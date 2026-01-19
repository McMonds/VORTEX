use vortex_io::ring::RingDriver;
use vortex_io::memory::BufferPool;
use vortex_io::net::VortexListener;
use crate::storage::wal::WalManager;
use crate::index::hnsw::HnswIndex;
use crate::index::VectorIndex;
use vortex_rpc::{VBP_MAGIC, ResponseHeader, STATUS_OK, STATUS_ERR};
use log::{info, error, debug, trace};
use io_uring::{opcode, types};
use std::os::unix::io::RawFd;
use std::time::Instant;
use std::path::Path;

/// User Data Tags to distinguish CQE types
const TAG_ACCEPT: u64 = 0xFFFF_0000;
const TAG_READ_PREFIX: u64 = 0xAAAA_0000;
const TAG_WAL_PREFIX: u64 = 0xBBBB_0000;
const TAG_WRITE_PREFIX: u64 = 0xCCCC_0000;

const CMD_UPSERT: u8 = 1;
const CMD_SEARCH: u8 = 5;

pub struct ShardReactor {
    shard_id: usize,
    ring: RingDriver,
    pool: BufferPool,
    listener: Option<VortexListener>,
    wal: WalManager,
    // Shard-local in-memory state (Rule 6: Share Nothing)
    index: HnswIndex,
    pending_submissions: u32,
    active_leases: Vec<Option<vortex_io::memory::BufferLease>>,
    // Map Lease Index -> Socket FD for zero-copy response
    active_fds: Vec<Option<RawFd>>,
    
    // Zero-Allocation Recycled Buffers
    completions_buffer: Vec<(u64, i32)>,
    scratch_query_buffer: Box<[f32; 128]>,
    
    // TCP Reassembly (Milestone 5 Hardening)
    accumulated_bytes: Vec<usize>, 
}

impl ShardReactor {
    pub fn new(shard_id: usize, ring_entries: u32, max_elements: usize, base_path: &str) -> Self {
        let ring = RingDriver::new(ring_entries).expect("Failed to init io_uring");
        let pool = BufferPool::new(ring_entries as usize, 4096);
        
        // Initialize WAL in requested directory (Rule #8/Milestone 4)
        let mut wal = WalManager::new(shard_id, base_path).expect("Failed to init WAL");
        
        // Dynamic dimension 128, capacity controlled by caller (Target 0 Scaling)
        let mut index = HnswIndex::new(128, max_elements);

        // --- THE RESURRECTION (Phase 4 Recovery) ---
        let wal_path = format!("./shard_{}.wal", shard_id);
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
            active_leases: vec![None; ring_entries as usize],
            active_fds: vec![None; ring_entries as usize],
            
            // Pre-allocate to avoid malloc in hot loop
            completions_buffer: Vec::with_capacity(ring_entries as usize),
            scratch_query_buffer: Box::new([0.0f32; 128]),
            accumulated_bytes: vec![0; ring_entries as usize],
        }
    }

    pub fn listen(&mut self, port: u16) -> std::io::Result<()> {
        let listener = VortexListener::new_ingress(port)?;
        self.listener = Some(listener);
        self.submit_accept();
        Ok(())
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
                info!("Shard {} accepted connection (fd: {})", self.shard_id, result);
                self.submit_read(result as RawFd);
                self.submit_accept();
            } else if (tag & 0xFFFF_0000) == TAG_READ_PREFIX {
                let idx = (tag & 0x0000_FFFF) as usize;
                self.handle_ingress_complete(idx, result as usize);
            } else if (tag & 0xFFFF_0000) == TAG_WAL_PREFIX {
                let idx = (tag & 0x0000_FFFF) as usize;
                self.handle_wal_complete(idx, result as usize);
            } else if (tag & 0xFFFF_0000) == TAG_WRITE_PREFIX {
                let idx = (tag & 0x0000_FFFF) as usize;
                self.handle_write_complete(idx, result as usize);
            }
        }
        
        true
    }

    fn submit_read(&mut self, fd: RawFd) {
        if let Some(lease) = self.pool.lease() {
            let idx = lease.index;
            self.accumulated_bytes[idx] = 0; // Fresh connection/request
            self.submit_read_at(fd, idx, 0);
            self.active_leases[idx] = Some(lease);
            self.active_fds[idx] = Some(fd);
        }
    }

    fn submit_read_at(&mut self, fd: RawFd, idx: usize, offset: usize) {
        let page = self.pool.get_page_mut(idx);
        let buf = page.as_slice_mut();
        
        if offset >= buf.len() {
            error!("Shard {} Buffer Overflow prevention: read offset {} exceeds page size {}", self.shard_id, offset, buf.len());
            return;
        }

        let tag = TAG_READ_PREFIX | (idx as u64);
        let read_e = opcode::Read::new(types::Fd(fd), unsafe { buf.as_mut_ptr().add(offset) }, (buf.len() - offset) as u32)
            .build()
            .user_data(tag);

        self.push_submission(&read_e);
    }

    /// Formats the response buffer using the *same* lease (Zero-Copy recycle).
    fn prepare_response_buffer(&mut self, idx: usize, opcode: u8, status: u8, req_id: u64) {
        let page = self.pool.get_page_mut(idx);
        let data = page.as_slice_mut();
        
        let header = ResponseHeader {
            magic: VBP_MAGIC,
            status,
            opcode,
            payload_len: 0, // 0 For now (Ack only)
            request_id: req_id,
        };
        
        // SAFETY: ResponseHeader is #[repr(C)] fixed size.
        unsafe {
            let ptr = data.as_mut_ptr() as *mut ResponseHeader;
            *ptr = header;
        }
    }

    /// Submits a write to the socket.
    /// We reuse the ingress FD which implies we need to track it.
    /// WAIT: In the current accept loop, we don't store the FD in the lease/struct!
    /// We only passed it via 'result' in completions.
    /// FIX: We need to store the socket FD in the active_leases or similar map?
    /// OR: We assume the FD is stable? No, we need it.
    /// 
    /// For Phase 3, we must store the FD.
    /// Current `active_leases` is just `Option<BufferLease>`.
    /// 
    /// STRATEGY UPDATE: The `TAG` encodes the index. We need a map of `Index -> FD`.
    /// Or we can hack it: The FD is lost after Read submit if we don't save it.
    ///
    /// Let's check `submit_read`. It takes `fd`.
    /// We need to store `fd` when we submit_read.
    /// `active_leases` should be tuple `(Lease, Fd)`.
    /// 
    /// BUT for this atomic step, I'll update `active_leases` to store metadata?
    /// Or add `fds: Vec<RawFd>` parallel array.
    /// 
    /// Let's add `active_fds: Vec<Option<RawFd>>` to ShardReactor struct.
    fn submit_write(&mut self, idx: usize) {
        let fd = self.active_fds[idx].expect("Lost FD for active lease!");
        let page = self.pool.get_page_mut(idx);
        let buf = page.as_slice_mut();
        
        // We only write the header (16 bytes) for now since payload_len=0.
        let write_len = std::mem::size_of::<ResponseHeader>();
        
        let tag = TAG_WRITE_PREFIX | (idx as u64);
        let write_e = opcode::Write::new(types::Fd(fd), buf.as_ptr(), write_len as u32)
            .build()
            .user_data(tag);
            
         self.push_submission(&write_e);
    }

    fn handle_ingress_complete(&mut self, idx: usize, bytes: usize) {
        // 1. Handle Client Death (EOF)
        if bytes == 0 {
            trace!("Shard {} Ingress -> Client disconnected (EOF). Releasing lease.", self.shard_id);
            if let Some(lease) = self.active_leases[idx].take() {
                self.pool.release(lease);
            }
            self.active_fds[idx] = None;
            self.accumulated_bytes[idx] = 0;
            return;
        }

        self.accumulated_bytes[idx] += bytes;
        let total = self.accumulated_bytes[idx];

        // 2. Protocol Reassembly (Target 3 Hardening)
        // We need at least 16 bytes to know the payload length
        if total < 16 {
            debug!("Shard {} Partial Read ({} bytes). Waiting for header...", self.shard_id, total);
            let fd = self.active_fds[idx].unwrap();
            self.submit_read_at(fd, idx, total);
            return;
        }

        // Peek at header to find expected length
        let header = {
            let page = self.pool.get_page_mut(idx);
            let data = page.as_slice_mut();
            unsafe { &*(data.as_ptr() as *const vortex_rpc::RequestHeader) }
        };

        let expected = 16 + header.payload_len as usize;
        debug!("Shard {} Ingress -> Received frame. Logical Len: {}, Physical Len: {}", self.shard_id, expected, total);

        if total < expected {
            let fd = self.active_fds[idx].unwrap();
            self.submit_read_at(fd, idx, total);
            return;
        }
        
        // 3. Cast & Check (The Contract) - SCOPED BORROW
        // We extract the metadata and pointer efficiently to drop the borrow on self.pool
        let (opcode, req_id, data_ptr) = {
            let page = self.pool.get_page_mut(idx);
            let data = page.as_slice_mut();
            
            match vortex_rpc::verify_header(&data[0..total]) {
                Ok(h) => {
                    (h.opcode, h.request_id, data.as_ptr())
                },
                Err(e) => {
                    error!("Shard {} Ingress -> Protocol Violation: {}. Closing connection.", self.shard_id, e);
                    // Strict firewall: Drop connection on Magic Mismatch
                    // We need to return early, but we are inside a block.
                    // We'll return a special tuple to signal exit.
                    // 0 is invalid opcode.
                    (0, 0, std::ptr::null())
                }
            }
        };

        // Handle Protocol Violation Exit
        if data_ptr.is_null() {
             if let Some(lease) = self.active_leases[idx].take() {
                self.pool.release(lease);
            }
            return;
        }

        // 4. Decision Logic (Self is free now)
        match opcode {
            CMD_SEARCH => {
                // Rule 10: Speed over Durability for search. No WAL.
                info!("Shard {} Ingress -> Received SEARCH command (Log Only for Phase 2).", self.shard_id);
                
                // Zero-Allocation: Use scratch buffer
                let results = self.index.search(self.scratch_query_buffer.as_slice(), 10);
                debug!("Shard {} SEARCH complete. Found {} results.", self.shard_id, results.len());
                
                // Phase 3 Target 2: Respond with Zero-Copy
                // Reuse the same lease/buffer for the response
                self.prepare_response_buffer(idx, CMD_SEARCH, STATUS_OK, req_id);
                self.submit_write(idx);
            },
            CMD_UPSERT => {
                // Rule 9: Persistence Precedes Response. 
                // CRITICAL: O_DIRECT requires PAGE_SIZE (4KB) alignment for both OFFSET and LENGTH.
                let tag = TAG_WAL_PREFIX | (idx as u64);
                
                // Align length to 4096 bytes if necessary
                let aligned_len = if total % 4096 != 0 {
                    let new_len = ((total / 4096) + 1) * 4096;
                    // Zero out the padding area in the buffer
                    let page = self.pool.get_page_mut(idx);
                    let data = page.as_slice_mut();
                    if new_len <= data.len() {
                        for b in &mut data[total..new_len] { *b = 0; }
                        new_len
                    } else {
                        total // Fallback if record somehow exceeds page size
                    }
                } else {
                    total
                };

                let wal_e = self.wal.write_entry(data_ptr, aligned_len as u32, tag);
                self.push_submission(&wal_e);
            },
            _ => {
                debug!("Shard {} Ingress -> Unknown command {}", self.shard_id, opcode);
                // In production, we should write an error response.
                self.prepare_response_buffer(idx, opcode, STATUS_ERR, req_id);
                self.submit_write(idx);
            }
        }
    }

    fn handle_wal_complete(&mut self, idx: usize, bytes: usize) {
        debug!("Shard {} WAL Persisted ({} bytes). Finalizing command.", self.shard_id, bytes);
        
        // 1. Retrieve Payload
        // The buffer contains [RequestHeader (16b)] [ID (8b)] [Vector (N * 4b)]
        let page = self.pool.get_page_mut(idx);
        let data = page.as_slice_mut();
        
        let header_size = std::mem::size_of::<vortex_rpc::RequestHeader>();
        
        // Safety Check (Protocol Guard II)
        if bytes < header_size + 8 {
             error!("Shard {} WAL Complete: Payload too short for Vector ID.", self.shard_id);
             self.prepare_response_buffer(idx, CMD_UPSERT, STATUS_ERR, 0);
             self.submit_write(idx);
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
             self.submit_write(idx);
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
        self.submit_write(idx);
        
        // CRITICAL: Do NOT drop lease here. 
        // Logic flows to handle_write_complete.
    }
    
    fn handle_write_complete(&mut self, idx: usize, _res: usize) {
        trace!("Shard {} Egress -> Response sent. Recycling lease.", self.shard_id);
        
        // CRITICAL FIX: Retrieve FD BEFORE clearing it
        // We need to re-arm the connection for the next request (keep-alive)
        let fd = self.active_fds[idx];
        
        // Release the current lease (response buffer is done)
        if let Some(lease) = self.active_leases[idx].take() {
            self.pool.release(lease);
        }
        self.active_fds[idx] = None;
        
        // Re-arm the Reader (The Keep-Alive Loop Fix)
        // Submit a new read on the same FD to handle the next request
        if let Some(fd) = fd {
            trace!("Shard {} Re-arming connection (fd: {}) for next request.", self.shard_id, fd);
            self.submit_read(fd);
        }
    }
}
