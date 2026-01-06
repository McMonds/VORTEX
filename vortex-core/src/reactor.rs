use vortex_io::ring::RingDriver;
use vortex_io::memory::BufferPool;
use vortex_io::net::VortexListener;
use crate::storage::wal::WalManager;
use crate::index::hnsw::HnswIndex;
use crate::index::VectorIndex;
use vortex_rpc::VBP_MAGIC;
use log::{info, error, debug, trace};
use io_uring::{opcode, types};
use std::os::unix::io::RawFd;

/// User Data Tags to distinguish CQE types
const TAG_ACCEPT: u64 = 0xFFFF_0000;
const TAG_READ_PREFIX: u64 = 0xAAAA_0000;
const TAG_WAL_PREFIX: u64 = 0xBBBB_0000;

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
    
    // Zero-Allocation Recycled Buffers
    completions_buffer: Vec<(u64, i32)>,
    scratch_query_buffer: Box<[f32; 128]>,
}

impl ShardReactor {
    pub fn new(shard_id: usize, ring_entries: u32) -> Self {
        let ring = RingDriver::new(ring_entries).expect("Failed to init io_uring");
        let pool = BufferPool::new(ring_entries as usize, 4096);
        
        // Initialize WAL in current directory for now
        let wal = WalManager::new(shard_id, ".").expect("Failed to init WAL");
        
        // Default dimension 128 for Milestone 5
        let index = HnswIndex::new(128, 1_000_000);

        Self {
            shard_id,
            ring,
            pool,
            listener: None,
            wal,
            index,
            pending_submissions: 0,
            active_leases: vec![None; ring_entries as usize],
            
            // Pre-allocate to avoid malloc in hot loop
            completions_buffer: Vec::with_capacity(64),
            scratch_query_buffer: Box::new([0.0f32; 128]),
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
            }
        }
        
        true
    }

    fn submit_read(&mut self, fd: RawFd) {
        if let Some(lease) = self.pool.lease() {
            let idx = lease.index;
            let page = self.pool.get_page_mut(idx);
            let buf = page.as_slice_mut();
            let tag = TAG_READ_PREFIX | (idx as u64);
            
            let read_e = opcode::Read::new(types::Fd(fd), buf.as_mut_ptr(), buf.len() as u32)
                .build()
                .user_data(tag);

            self.push_submission(&read_e);
            self.active_leases[idx] = Some(lease);
        }
    }

    fn handle_ingress_complete(&mut self, idx: usize, bytes: usize) {
        if bytes < 16 { return; }
        
        let page = self.pool.get_page_mut(idx);
        let data = page.as_slice_mut();
        
        if u16::from_le_bytes([data[0], data[1]]) != VBP_MAGIC {
            return;
        }

        let cmd_code = data[3];
        
        // Rule 6: Linear Control Flow. Switch on opcode.
        match cmd_code {
            CMD_SEARCH => {
                // Rule 10: Speed over Durability for search. No WAL.
                trace!("Shard {} Ingress -> Received SEARCH command.", self.shard_id);
                
                // Zero-Allocation: Use scratch buffer
                let results = self.index.search(self.scratch_query_buffer.as_slice(), 10);
                debug!("Shard {} SEARCH complete. Found {} results.", self.shard_id, results.len());
                
                // Release lease since search is read-only
                if let Some(lease) = self.active_leases[idx].take() {
                    self.pool.release(lease);
                }
            },
            CMD_UPSERT => {
                // Rule 9: Persistence Precedes Response
                trace!("Shard {} Ingress -> Submitting to WAL ({} bytes)", self.shard_id, bytes);
                let tag = TAG_WAL_PREFIX | (idx as u64);
                let wal_e = self.wal.write_entry(data.as_ptr(), 4096, tag);
                
                self.push_submission(&wal_e);
            },
            _ => {
                debug!("Shard {} Ingress -> Unknown command {}", self.shard_id, cmd_code);
                // In production, we should write an error response.
                if let Some(lease) = self.active_leases[idx].take() {
                    self.pool.release(lease);
                }
            }
        }
    }

    fn handle_wal_complete(&mut self, idx: usize, bytes: usize) {
        trace!("Shard {} WAL Persisted ({} bytes). Finalizing command.", self.shard_id, bytes);
        
        // Finalizing: Indexing after persistence (Milestone 5)
        let page = self.pool.get_page_mut(idx);
        let _data = page.as_slice_mut();
        
        // In a real VBP packet, we would extract the vector here.
        // For demonstration, we'll index a dummy vector.
        let dummy_id = 999;
        // Use scratch buffer for insertion simulation too to avoid alloc
        self.index.insert(dummy_id, self.scratch_query_buffer.as_slice());
        
        trace!("Shard {} indexed vector id {}. Lifecycle complete.", self.shard_id, dummy_id);
        
        if let Some(lease) = self.active_leases[idx].take() {
            self.pool.release(lease);
        }
    }
}
