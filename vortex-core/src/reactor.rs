use vortex_io::ring::RingDriver;
use vortex_io::memory::{BufferPool, BufferLease};
use vortex_io::net::VortexListener;
use vortex_rpc::VBP_MAGIC;
use log::{info, error, debug};
use io_uring::{opcode, types};
use std::os::unix::io::RawFd;

/// User Data Tags to distinguish CQE types
const TAG_ACCEPT: u64 = 0xFFFF_0000;
const TAG_READ_PREFIX: u64 = 0xAAAA_0000;

pub struct ShardReactor {
    shard_id: usize,
    ring: RingDriver,
    pool: BufferPool,
    listener: Option<VortexListener>,
    client_fds: Vec<Option<RawFd>>,
    pending_submissions: u32,
}

impl ShardReactor {
    pub fn new(shard_id: usize, ring_entries: u32) -> Self {
        let ring = RingDriver::new(ring_entries).expect("Failed to init io_uring");
        let pool = BufferPool::new(ring_entries as usize, 4096);
        
        Self {
            shard_id,
            ring,
            pool,
            listener: None,
            client_fds: vec![None; 1024], // Max 1024 concurrent clients per shard for now
            pending_submissions: 0,
        }
    }

    pub fn listen(&mut self, addr: &str) -> std::io::Result<()> {
        let listener = VortexListener::bind(addr)?;
        self.listener = Some(listener);
        // Start the accept loop
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
                completions.push((cqe.user_data(), cqe.result()));
            }
        }

        for (tag, result) in completions {
            if result < 0 {
                // Handle EWOULDBLOCK etc. gracefully in non-blocking mode
                let err = std::io::Error::from_raw_os_error(-result);
                if err.kind() == std::io::ErrorKind::WouldBlock {
                    continue; 
                }
                error!("Shard {} I/O Error on tag 0x{:x}: {}", self.shard_id, tag, err);
                continue;
            }

            if tag == TAG_ACCEPT {
                let client_fd = result;
                info!("Shard {} accepted new client (fd: {})", self.shard_id, client_fd);
                self.submit_read(client_fd as RawFd);
                self.submit_accept(); // Re-arm accept
            } else if (tag & 0xFFFF_0000) == TAG_READ_PREFIX {
                let lease_idx = (tag & 0x0000_FFFF) as usize;
                self.handle_read(lease_idx, result as usize);
            }
        }
        
        true
    }

    fn submit_read(&mut self, fd: RawFd) {
        if let Some(lease) = self.pool.lease() {
            let idx = lease.index;
            let page = self.pool.get_page_mut(idx);
            let buf = page.as_slice_mut();
            
            // Link this lease to this FD for high-speed dispatch
            let tag = TAG_READ_PREFIX | (idx as u64);
            
            let read_e = opcode::Read::new(types::Fd(fd), buf.as_mut_ptr(), buf.len() as u32)
                .build()
                .user_data(tag);

            unsafe {
                self.ring.submission_queue().push(&read_e).expect("Ring full");
            }
            self.pending_submissions += 1;
        }
    }

    fn handle_read(&mut self, lease_idx: usize, bytes: usize) {
        if bytes < 16 { return; }
        let page = self.pool.get_page_mut(lease_idx);
        let data = page.as_slice_mut();
        
        if u16::from_le_bytes([data[0], data[1]]) == VBP_MAGIC {
            info!("Shard {} Received Valid VBP Packet ({} bytes)", self.shard_id, bytes);
        }
    }
}
