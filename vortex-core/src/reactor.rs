use vortex_io::ring::RingDriver;
use vortex_io::memory::BufferPool;
use vortex_io::net::VortexListener;
use crate::storage::wal::WalManager;
use crate::storage::batch::BatchAccumulator;
use crate::index::hnsw::HnswIndex;
use crate::index::VectorIndex;
use vortex_rpc::{VBP_MAGIC, ResponseHeader, STATUS_OK, STATUS_ERR};
use log::{info, error, debug, trace, warn};
use io_uring::{opcode, types};
use std::os::unix::io::RawFd;
use std::time::{Instant, Duration};
use std::path::Path;

/// User Data Tags to distinguish CQE types
const TAG_ACCEPT: u64 = 0xFFFF_0000;
const TAG_READ_PREFIX: u64 = 0xAAAA_0000;
const TAG_WAL_PREFIX: u64 = 0xBBBB_0000;
const TAG_WRITE_PREFIX: u64 = 0xCCCC_0000;
const TAG_BATCH_WRITE: u64 = 0xDDDD_0000;

const CMD_UPSERT: u8 = 1;
const CMD_SEARCH: u8 = 5;

#[derive(Debug, Clone, Copy)]
pub enum FlushReason {
    Full,
    Eot,
}

impl std::fmt::Display for FlushReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlushReason::Full => write!(f, "Batch Full"),
            FlushReason::Eot => write!(f, "End-of-Tick"),
        }
    }
}

pub struct ShardReactor {
    shard_id: usize,
    ring: RingDriver,
    pool: BufferPool,
    listener: Option<VortexListener>,
    wal: WalManager,
    // Shard-local in-memory state (Rule 6: Share Nothing)
    index: HnswIndex,
    pending_submissions: u32,
    // Map Slot Index -> Socket FD for response
    active_fds: Vec<Option<RawFd>>,
    
    // Zero-Allocation Recycled Buffers
    completions_buffer: Vec<(u64, i32)>,
    scratch_query_buffer: Box<[f32; 128]>,
    
    // TCP Reassembly (Milestone 5 Hardening)
    accumulated_bytes: Vec<usize>, 
    consumed_bytes: Vec<usize>,
    pending_ops: Vec<usize>,

    // Mechanical Sympathy: Batching
    active_batch: BatchAccumulator,
    flushing_batch: Option<BatchAccumulator>,
    is_shutting_down: bool,
    ring_capacity: usize,
    paused_reads: Vec<usize>,
    write_in_flight: Vec<bool>,
    pending_acks: Vec<usize>,
    read_in_flight: Vec<bool>,
    
    // Phase 11: Foreman Telemetry
    backpressure_count: usize,
    last_backpressure_report: Instant,
    tick_search_micros: u64,
    tick_search_ops: usize,
    tick_ingress_ns: u64,
    tick_flush_ns: u64,
    last_pulse_report: Instant,
}

impl ShardReactor {
    pub fn new(shard_id: usize, ring_entries: u32, max_elements: usize, base_path: &str) -> Self {
        let ring = RingDriver::new(ring_entries).expect("Failed to init io_uring");
        // Rule #14 Optimization: Double pool for Shadow Response Buffers (RX/TX split)
        let pool = BufferPool::new((ring_entries * 2) as usize, 65536); 
        
        // Initialize WAL in requested directory (Rule #8/Milestone 4)
        let mut wal = WalManager::new(shard_id, base_path).expect("Failed to init WAL");
        
        // Dynamic dimension 128, capacity controlled by caller (Target 0 Scaling)
        let mut index = HnswIndex::new(128, max_elements);

        // --- THE RESURRECTION (Phase 4 Recovery) ---
        let wal_path = format!("{}/shard_{}.wal", base_path, shard_id);
        let start_time = Instant::now();
        let mut recovered_count = 0;

        if Path::new(&wal_path).exists() {
            // Replay iterator performs blocking I/O (allowed during boot per Rule #8 exception)
            if let Ok(mut iter) = wal.replay_iter(&wal_path) {
                for entry_res in &mut iter {
                    match entry_res {
                        Ok(entry) => {
                            if entry.header.opcode == CMD_UPSERT {
                                let payload = &entry.payload;
                                if payload.len() >= 8 {
                                    // Parse ID (8 bytes)
                                    let id = u64::from_le_bytes(payload[0..8].try_into().unwrap_or([0; 8]));
                                    // Parse Vector
                                    let vec_bytes = &payload[8..];
                                    let dim = vec_bytes.len() / 4;
                                    if dim > 0 {
                                        // SAFETY: WAL content is trusted for replay. Aligned in 4KB pages.
                                        let vec_slice: &[f32] = unsafe {
                                            std::slice::from_raw_parts(vec_bytes.as_ptr() as *const f32, dim)
                                        };
                                        index.insert(id, vec_slice);
                                        recovered_count += 1;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            let corruption_offset = iter.bytes_read();
                            error!("Shard {}: WAL Replay encountered corruption at offset {}: {}. Truncating log to prune corrupted tail.", 
                                shard_id, corruption_offset, e);
                            
                            // Self-Healing: Truncate the file to the last known good position
                            if let Err(te) = wal.truncate(corruption_offset) {
                                error!("Shard {}: Failed to truncate corrupted WAL: {}", shard_id, te);
                            }
                            break;
                        }
                    }
                }
            }
        }

        let duration = start_time.elapsed();
        if recovered_count > 0 {
            info!("Shard {}: Recovered {} records from WAL in {} ms.", 
                shard_id, recovered_count, duration.as_millis());
        }

        Self {
            shard_id,
            ring,
            pool,
            listener: None,
            wal,
            index,
            pending_submissions: 0,
            
            // Pre-allocate to avoid malloc in hot loop
            completions_buffer: Vec::with_capacity(ring_entries as usize),
            scratch_query_buffer: Box::new([0.0f32; 128]),
            active_fds: vec![None; 32],
            accumulated_bytes: vec![0; 32],
            consumed_bytes: vec![0; 32],
            pending_ops: vec![0; 32],
            active_batch: BatchAccumulator::new(),
            flushing_batch: None,
            is_shutting_down: false,
            ring_capacity: ring_entries as usize,
            paused_reads: Vec::with_capacity(32),
            write_in_flight: vec![false; ring_entries as usize * 2], // Direct mapping
            pending_acks: vec![0; 32],
            read_in_flight: vec![false; 32],
            backpressure_count: 0,
            last_backpressure_report: Instant::now(),
            tick_search_micros: 0,
            tick_search_ops: 0,
            tick_ingress_ns: 0,
            tick_flush_ns: 0,
            last_pulse_report: Instant::now(),
        }
    }

    pub fn listen(&mut self, port: u16) -> std::io::Result<()> {
        let listener = VortexListener::new_ingress(port)?;
        self.listener = Some(listener);
        self.submit_accept();
        Ok(())
    }

    pub fn shutdown(&mut self) {
        self.is_shutting_down = true;
        // Force drain all pending batches
        self.flush_active_batch(FlushReason::Eot);
    }

    /// Helper to submit an SQE with Backpressure handling.
    /// If the ring is full, it busy-loops on submit() until space opens up.
    fn push_submission(&mut self, entry: &io_uring::squeue::Entry) {
        loop {
            // SAFETY: Checked push to pre-allocated ring buffer. Entry is valid.
            unsafe {
                if self.ring.submission_queue().push(entry).is_ok() {
                    self.pending_submissions += 1;
                    return;
                }
            }

            // Backpressure Strategy: Flush to kernel to free up SQ slots.
            if let Err(e) = self.ring.submit() {
                error!("Critical Ring Submit Error during backpressure flush: {}", e);
            }
        }
    }

    fn submit_accept(&mut self) {
        if let Some(ref listener) = self.listener {
            let entry = listener.accept_sqe(std::ptr::null_mut(), std::ptr::null_mut(), TAG_ACCEPT);
            self.push_submission(&entry);
        }
    }

    pub fn run_tick(&mut self) -> bool {
        let _t_start = Instant::now();
        let mut _work_done = false;
        
        // 1Hz Pulse Telemetry (Rule 11/Constraint 1)
        if self.last_pulse_report.elapsed() >= Duration::from_secs(1) {
             let nodes = self.index.dist_calc_count.get();
             self.index.dist_calc_count.set(0);
             
             // Emit PULSE for dashboard parsing
             info!("PULSE Shard {} | [Search] ops={} time={}us dist={} | [Health] ingress={}ms flush={}ms",
                self.shard_id, 
                self.tick_search_ops, 
                self.tick_search_micros,
                nodes,
                self.tick_ingress_ns / 1_000_000,
                self.tick_flush_ns / 1_000_000
             );
             
             // Reset aggregators
             self.tick_search_ops = 0;
             self.tick_search_micros = 0;
             self.tick_ingress_ns = 0;
             self.tick_flush_ns = 0;
             self.last_pulse_report = Instant::now();
        }

        // 1. Process Completions
        // Opportunistic submit of any pending SQEs
        if let Err(e) = self.ring.submit_and_wait(1) {
            error!("Shard {} Ring Error: {}", self.shard_id, e);
            return false;
        }

        // Reuse the completions buffer (Zero Allocation)
        self.completions_buffer.clear();
        
        {
            let mut cq = self.ring.completion_queue();
            while let Some(cqe) = cq.next() {
                self.pending_submissions -= 1;
                self.completions_buffer.push((cqe.user_data() as u64, cqe.result()));
            }
        }

        // Iterate over the buffer (borrow checker happy now)
        for i in 0..self.completions_buffer.len() {
            let (tag, result) = self.completions_buffer[i];
            
            if result < 0 {
                let err = std::io::Error::from_raw_os_error(-result);
                if err.kind() == std::io::ErrorKind::WouldBlock { continue; }
                error!("Shard {} I/O Error on tag 0x{:x}: {}", self.shard_id, tag, err);
                continue;
            }

            if tag == TAG_ACCEPT {
                debug!("Shard {} accepted connection (fd: {})", self.shard_id, result);
                self.submit_read(result as RawFd);
                self.submit_accept();
            } else if (tag & 0xFFFF_0000) == TAG_READ_PREFIX {
                let idx = (tag & 0x0000_FFFF) as usize;
                self.handle_ingress(idx, result as usize);
            } else if (tag & 0xFFFF_0000) == TAG_WAL_PREFIX {
                let idx = (tag & 0x0000_FFFF) as usize;
                self.handle_wal_complete(idx, result as usize);
            } else if (tag & 0xFFFF_0000) == TAG_WRITE_PREFIX {
                let idx = (tag & 0x0000_FFFF) as usize;
                self.handle_write_complete(idx, result as usize);
            } else if tag == TAG_BATCH_WRITE {
                self.handle_batch_complete(result as usize);
            }
        }
        
        // EOT (End-Of-Tick) Flush: If we are idle and have pending data, COMMIT.
        if self.active_batch.is_dirty() && self.flushing_batch.is_none() {
            self.flush_active_batch(FlushReason::Eot);
        }

        // Shard Health Pulse
        trace!("Shard {} Heartbeat", self.shard_id);

        // Aggregated Backpressure Reporting
        if self.last_backpressure_report.elapsed() >= Duration::from_secs(1) {
            if self.backpressure_count > 0 {
                info!("Shard {} BACKPRESSURE Aggregator: {} stalls in last 1s.", self.shard_id, self.backpressure_count);
                self.backpressure_count = 0;
            }
            self.last_backpressure_report = Instant::now();
        }

        if self.is_shutting_down {
            return false;
        }

        true
    }

    fn submit_read(&mut self, fd: RawFd) {
        // Enforce 32-connection limit with STATIC mapping (Rule #7)
        // Connection i -> BufferPage[i] (ingress) and BufferPage[i+32] (shadow)
        for i in 0..32 {
            if self.active_fds[i].is_none() {
                self.active_fds[i] = Some(fd);
                self.accumulated_bytes[i] = 0; 
                self.consumed_bytes[i] = 0;
                self.pending_ops[i] = 0;
                self.submit_read_at(fd, i, 0);
                return;
            }
        }

        // Saturation Check: Refuse connection beyond static map capacity
        warn!("Shard {} Saturation: Disconnecting FD {} (Limit reached: 32).", self.shard_id, fd);
        unsafe { libc::close(fd); }
    }

    fn submit_read_at(&mut self, fd: RawFd, idx: usize, offset: usize) {
        if self.read_in_flight[idx] {
            return;
        }

        let page = self.pool.get_page_mut(idx);
        let buf = page.as_slice_mut();
        
        if offset >= buf.len() {
            // BACKPRESSURE: Buffer is full, wait for current request to commit and drain
            trace!("Shard {} Buffer Full (idx: {}). Backpressure engaged.", self.shard_id, idx);
            return;
        }

        self.read_in_flight[idx] = true;
        let tag = TAG_READ_PREFIX | (idx as u64);
        let read_len = (buf.len() - offset) as u32;
        // Cap read size to avoid overwhelming io_uring if the buffer is large
        let capped_read = std::cmp::min(read_len, 65536); 

        let read_e = opcode::Read::new(types::Fd(fd), unsafe { buf.as_mut_ptr().add(offset) }, capped_read)
            .build()
            .user_data(tag);

        self.push_submission(&read_e);
    }

    /// Formats the response buffer using the *shadow* page (RX/TX Split).
    fn prepare_response_buffer(&mut self, idx: usize, opcode: u8, status: u8, req_id: u64) {
        // Offset mapping: slot `idx` uses `idx + ring_capacity` for responses
        let tx_idx = idx + self.ring_capacity;
        let page = self.pool.get_page_mut(tx_idx);
        let data = page.as_slice_mut();
        
        // Phase 7.4: Use pending_acks as offset to allow queuing responses while write is in flight
        let offset = self.pending_acks[idx];
        let header = ResponseHeader {
            magic: VBP_MAGIC,
            status,
            opcode,
            payload_len: 0, 
            request_id: req_id,
        };
        
        // SAFETY: ResponseHeader is #[repr(C)] fixed size.
        unsafe {
            let ptr = data.as_mut_ptr().add(offset * 16) as *mut ResponseHeader;
            *ptr = header;
        }
        self.pending_acks[idx] += 1;
    }

    /// Submits a write to the socket from the shadow response lane.
    fn submit_write(&mut self, idx: usize, len: Option<usize>) {
        if let Some(fd) = self.active_fds[idx] {
            if self.write_in_flight[idx] {
                return;
            }

            let write_len = match len {
                Some(l) => l,
                None => {
                    let total = self.pending_acks[idx];
                    self.pending_acks[idx] = 0;
                    total * 16
                }
            };

            if write_len == 0 {
                return;
            }

            self.write_in_flight[idx] = true;
            let tx_idx = idx + self.ring_capacity;
            let page = self.pool.get_page_mut(tx_idx);
            let buf = page.as_slice_mut();
            
            let tag = TAG_WRITE_PREFIX | (idx as u64);
            let write_e = opcode::Write::new(types::Fd(fd), buf.as_ptr(), write_len as u32)
                .build()
                .user_data(tag);
                
             self.push_submission(&write_e);
        }
    }

    fn handle_ingress(&mut self, idx: usize, bytes: usize) {
        self.read_in_flight[idx] = false;

        // 1. Handle Client Death (EOF)
        if bytes == 0 {
            trace!("Shard {} Ingress -> Client disconnected (EOF).", self.shard_id);
            self.active_fds[idx] = None;
            
            // Only cleanup if no WAL/Write operations are in flight
            if self.pending_ops[idx] == 0 {
                self.accumulated_bytes[idx] = 0;
                self.consumed_bytes[idx] = 0;
            }
            return;
        }

        self.accumulated_bytes[idx] += bytes;
        
        // Phase 7.4: Removed pending_ops == 0 guard to enable pipelining.
        // process_ingress is guarded by read_in_flight to prevent buffer races.
        let i_start = Instant::now();
        self.process_ingress(idx);
        self.tick_ingress_ns += i_start.elapsed().as_nanos() as u64;
    }

    fn process_ingress(&mut self, idx: usize) {
        if self.read_in_flight[idx] {
            return;
        }

        loop {
            let total = self.accumulated_bytes[idx];
            let consumed = self.consumed_bytes[idx];
            let available = total - consumed;
            
            if available < 16 {
                if consumed > 0 {
                    let page = self.pool.get_page_mut(idx);
                    let data = page.as_slice_mut();
                    data.copy_within(consumed..total, 0);
                    self.accumulated_bytes[idx] = available;
                    self.consumed_bytes[idx] = 0;
                }
                
                // Phase 7.3.1: Only re-arm if the lock is held (implicit in submit_read_at)
                if self.pending_ops[idx] < 64 { 
                    if let Some(fd) = self.active_fds[idx] {
                        self.submit_read_at(fd, idx, self.accumulated_bytes[idx]);
                    }
                }
                break;
            }

            // Peek Header
            let (expected, opcode, req_id) = {
                let page = self.pool.get_page_mut(idx);
                let data = &page.as_slice_mut()[consumed..consumed + 16];
                let header = unsafe { &*(data.as_ptr() as *const vortex_rpc::RequestHeader) };
                
                if header.magic != vortex_rpc::VBP_MAGIC {
                    error!("Shard {} PROTOCOL CORRUPTION: Invalid Magic at consumed {}. Available {}.", self.shard_id, consumed, available);
                    self.active_fds[idx] = None;
                    return;
                }
                (16 + header.payload_len as usize, header.opcode, header.request_id)
            };

            if available < expected {
                if consumed > 0 {
                    let page = self.pool.get_page_mut(idx);
                    let data = page.as_slice_mut();
                    data.copy_within(consumed..total, 0);
                    self.accumulated_bytes[idx] = available;
                    self.consumed_bytes[idx] = 0;
                }
                
                if self.pending_ops[idx] < 64 {
                    if let Some(fd) = self.active_fds[idx] {
                        self.submit_read_at(fd, idx, self.accumulated_bytes[idx]);
                    }
                }
                break;
            }

            // Handle Request
            match opcode {
                CMD_SEARCH => {
                    let s_start = Instant::now();
                    let _results = self.index.search(self.scratch_query_buffer.as_slice(), 10);
                    let s_dur = s_start.elapsed();
                    
                    self.tick_search_ops += 1;
                    self.tick_search_micros += s_dur.as_micros() as u64;

                    self.pending_ops[idx] += 1;
                    self.prepare_response_buffer(idx, CMD_SEARCH, STATUS_OK, req_id);
                    self.submit_write(idx, None);
                },
                CMD_UPSERT => {
                    if self.pending_ops[idx] == 0 {
                        trace!("Shard {} Ingress -> First UPSERT for connection {}. Starting pipeline.", self.shard_id, idx);
                    }
                    let tag = idx as u64;
                    let push_res = {
                        let page = self.pool.get_page_mut(idx);
                        let data = &page.as_slice_mut()[consumed..consumed + expected];
                        self.active_batch.try_add(data, tag)
                    };

                    if let Err(_) = push_res {
                        if self.flushing_batch.is_none() {
                            self.flush_active_batch(FlushReason::Full);
                            // Retry in fresh batch
                            let page = self.pool.get_page_mut(idx);
                            let data = &page.as_slice_mut()[consumed..consumed + expected];
                            if let Err(_) = self.active_batch.try_add(data, tag) {
                                error!("Shard {} Command too big for batch: {} bytes", self.shard_id, expected);
                                self.prepare_response_buffer(idx, CMD_UPSERT, STATUS_ERR, req_id);
                                if !self.write_in_flight[idx] {
                                    self.submit_write(idx, None);
                                }
                                // Bytes are consumed below.
                            }
                        } else {
                            if !self.paused_reads.contains(&idx) {
                                self.paused_reads.push(idx);
                            }
                            self.backpressure_count += 1;
                            return;
                        }
                    }
                    self.pending_ops[idx] += 1;
                },
                _ => {
                    self.pending_ops[idx] += 1;
                    self.prepare_response_buffer(idx, opcode, STATUS_ERR, req_id);
                    self.submit_write(idx, None);
                }
            }

            self.consumed_bytes[idx] += expected;
        }
    }

    fn flush_active_batch(&mut self, reason: FlushReason) {
        let f_start = Instant::now();
        if !self.active_batch.is_dirty() { return; }
        if self.flushing_batch.is_some() { return; } // Pipeline full

        // Swap to Flushing
        let mut batch = std::mem::replace(&mut self.active_batch, BatchAccumulator::new());
        let (ptr, len) = batch.prepare_flush();
        
        info!("Shard {} Group Commit -> Flushing batch of {} bytes ({} requests) ({}).", self.shard_id, len, batch.tags.len(), reason);
        self.flushing_batch = Some(batch);
        
        let tag = TAG_BATCH_WRITE;
        let wal_e = self.wal.write_entry(ptr, len as u32, tag);
        self.push_submission(&wal_e);
        self.tick_flush_ns += f_start.elapsed().as_nanos() as u64;
    }

    fn handle_batch_complete(&mut self, bytes: usize) {
        let mut batch = self.flushing_batch.take().expect("Protocol Error: No flushing batch found.");
        let tags = batch.take_tags();
        
        trace!("Shard {} Group Commit -> {} bytes persisted. ACKing {} requests in batch.", self.shard_id, bytes, tags.len());
        
        // Group ACKs by connection to avoid Zero-Copy Hazards in egress
        let mut ack_counts = [0usize; 32];
        for idx_u64 in tags {
            let idx = idx_u64 as usize;
            if idx < 32 {
                ack_counts[idx] += 1;
            }
        }
        
        for idx in 0..32 {
            let count = ack_counts[idx];
            if count > 0 {
                // Prepare 'count' ACKs in the shadow TX buffer at the correct offset
                {
                    let tx_idx = idx + self.ring_capacity;
                    let page = self.pool.get_page_mut(tx_idx);
                    let data = page.as_slice_mut();
                    
                    // Phase 7.3: Use pending_acks as offset for deferred aggregation
                    let offset = self.pending_acks[idx];
                    for i in 0..count {
                        let header = vortex_rpc::ResponseHeader {
                            magic: vortex_rpc::VBP_MAGIC,
                            status: vortex_rpc::STATUS_OK,
                            opcode: CMD_UPSERT,
                            payload_len: 0,
                            request_id: 0, // In saturation mode, we sacrifice linearization for throughput
                        };
                        unsafe {
                            let ptr = data.as_mut_ptr().add((offset + i) * 16) as *mut vortex_rpc::ResponseHeader;
                            *ptr = header;
                        }
                    }
                }

                self.pending_acks[idx] += count;
                if self.write_in_flight[idx] {
                    continue;
                }
                
                // Submit ONE aggregated write for all ACKs of this connection
                self.submit_write(idx, None);
            }
        }

        // Phase 7.2: O(1) Wake-up Logic (Signal all paused readers)
        let pending = std::mem::take(&mut self.paused_reads);
        for idx in pending {
            self.process_ingress(idx);
        }
    }

    fn handle_wal_complete(&mut self, idx: usize, bytes: usize) {
        debug!("Shard {} WAL Persisted ({} bytes). Finalizing command.", self.shard_id, bytes);
        
        // 1. Retrieve Payload from SHADOW buffer
        let shadow_idx = idx + 32;
        let page = self.pool.get_page_mut(shadow_idx);
        let data = page.as_slice_mut();
        
        let header_size = std::mem::size_of::<vortex_rpc::RequestHeader>();
        
        // Safety Check (Protocol Guard II)
        if bytes < header_size + 8 {
             error!("Shard {} WAL Complete: Payload too short for Vector ID.", self.shard_id);
             self.prepare_response_buffer(idx, CMD_UPSERT, STATUS_ERR, 0);
             self.submit_write(idx, None);
             return;
        }

        // 2. Parse ID and Vector using LOGICAL length from the header
        // Header contains: magic(2) + status(1) + opcode(1) + payload_len(4) + request_id(8) = 16 bytes.
        // But we are reading the REQUEST header from the WAL: magic(2) + version(1) + opcode(1) + payload_len(4) + request_ids(8) = 16 bytes.
        let header = unsafe { &*(data.as_ptr() as *const vortex_rpc::RequestHeader) };
        let logical_payload_len = header.payload_len as usize;
        
        // Data is aligned to 4096, so offset 16 is aligned for u64 (8) and f32 (4).
        let payload_ptr = unsafe { data.as_ptr().add(header_size) };
        
        // Parse ID (8 bytes)
        let id = unsafe { *(payload_ptr as *const u64) };
        
        // Parse Vector (logical dimension)
        let vec_bytes = logical_payload_len - 8;
        let dim = vec_bytes / 4;
        
        if dim == 0 {
             error!("Shard {} WAL Complete: Logical vector dimension is 0.", self.shard_id);
             self.prepare_response_buffer(idx, CMD_UPSERT, STATUS_ERR, header.request_id);
             self.submit_write(idx, None);
             return;
        }

        let vector_slice = unsafe {
            std::slice::from_raw_parts(payload_ptr.add(8) as *const f32, dim)
        };
        
        // 3. Insert into Index
        // This is the "Brain Transplant" moment.
        self.index.insert(id, vector_slice);
        
        trace!("Shard {} indexed vector id {} (Dim: {}). Lifecycle complete.", self.shard_id, id, dim);
        
        // 4. Send Response (Closing the Circuit)
        // We need the original Request ID.
        // It's still in the buffer header!
        let req_id = match vortex_rpc::verify_header(&data[0..16]) {
            Ok(h) => h.request_id,
            Err(_) => 0,
        };
        
        self.prepare_response_buffer(idx, CMD_UPSERT, STATUS_OK, req_id);
        self.submit_write(idx, None);
        
        // CRITICAL: Do NOT drop lease here. 
        // Logic flows to handle_write_complete.
    }
    
    fn handle_write_complete(&mut self, idx: usize, _res: usize) {
        self.write_in_flight[idx] = false;

        // Result is handled by handle_write_complete and process_ingress for next steps
        self.submit_write(idx, None);
        
        if self.pending_ops[idx] > 0 {
            let acks_in_write = _res / 16;
            if acks_in_write > self.pending_ops[idx] {
                self.pending_ops[idx] = 0;
            } else {
                self.pending_ops[idx] -= acks_in_write;
            }
        }

        // Phase 7.3.1: Delegate all buffer sovereignty to process_ingress
        self.process_ingress(idx);
    }
}
