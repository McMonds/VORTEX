use vortex_io::ring::RingDriver;
use vortex_io::memory::BufferPool;
use vortex_io::net::VortexListener;
use crate::storage::wal::WalManager;
use crate::index::hnsw::HnswIndex;
use crate::index::VectorIndex;
use vortex_rpc::VBP_MAGIC;
use log::{info, error};
use io_uring::{opcode, types};
use std::os::unix::io::RawFd;

/// User Data Tags to distinguish CQE types
const TAG_ACCEPT: u64 = 0xFFFF_0000;
const TAG_READ_PREFIX: u64 = 0xAAAA_0000;
const TAG_WAL_PREFIX: u64 = 0xBBBB_0000;

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
        }
    }

    pub fn listen(&mut self, addr: &str) -> std::io::Result<()> {
        let listener = VortexListener::bind(addr)?;
        self.listener = Some(listener);
        self.submit_accept();
        Ok(())
    }

    fn submit_accept(&mut self) {
        if let Some(ref listener) = self.listener {
            let entry = listener.accept_sqe(std::ptr::null_mut(), std::ptr::null_mut(), TAG_ACCEPT);
            unsafe {
                self.ring.submission_queue().push(&entry).expect("Ring full");
            }
            self.pending_submissions += 1;
        }
    }

    pub fn run_tick(&mut self) -> bool {
        if let Err(e) = self.ring.submit_and_wait(1) {
            error!("Shard {} Ring Error: {}", self.shard_id, e);
            return false;
        }

        let mut completions = Vec::with_capacity(64);
        {
            let mut cq = self.ring.completion_queue();
            while let Some(cqe) = cq.next() {
                self.pending_submissions -= 1;
                completions.push((cqe.user_data() as u64, cqe.result()));
            }
        }

        for (tag, result) in completions {
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

            unsafe {
                self.ring.submission_queue().push(&read_e).expect("Ring full");
            }
            self.active_leases[idx] = Some(lease);
            self.pending_submissions += 1;
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
        
        if cmd_code == 5 { // Search (Rule 10: Speed over Durability for search)
            info!("Shard {} Ingress -> Received SEARCH command.", self.shard_id);
            let dummy_query = vec![0.1f32; 128];
            let results = self.index.search(&dummy_query, 10);
            info!("Shard {} SEARCH complete. Found {} results.", self.shard_id, results.len());
            
            // Release lease since search is read-only
            if let Some(lease) = self.active_leases[idx].take() {
                self.pool.release(lease);
            }
        } else { // Upsert or other (Rule 9: Persistence Precedes Response)
            info!("Shard {} Ingress -> Submitting to WAL ({} bytes)", self.shard_id, bytes);
            let tag = TAG_WAL_PREFIX | (idx as u64);
            let wal_e = self.wal.write_entry(data.as_ptr(), 4096, tag);
            
            unsafe {
                self.ring.submission_queue().push(&wal_e).expect("Ring full");
            }
            self.pending_submissions += 1;
        }
    }

    fn handle_wal_complete(&mut self, idx: usize, bytes: usize) {
        info!("Shard {} WAL Persisted ({} bytes). Finalizing command.", self.shard_id, bytes);
        
        // Finalizing: Indexing after persistence (Milestone 5)
        let page = self.pool.get_page_mut(idx);
        let _data = page.as_slice_mut();
        
        // In a real VBP packet, we would extract the vector here.
        // For demonstration, we'll index a dummy vector.
        let dummy_id = 999;
        let dummy_vec = vec![0.1f32; 128];
        self.index.insert(dummy_id, &dummy_vec);
        
        info!("Shard {} indexed vector id {}. Lifecycle complete.", self.shard_id, dummy_id);
        
        if let Some(lease) = self.active_leases[idx].take() {
            self.pool.release(lease);
        }
    }
}
