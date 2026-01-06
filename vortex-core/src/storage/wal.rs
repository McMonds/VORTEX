use vortex_io::storage::DirectFile;
use log::info;
use io_uring;

/// Manages the Write-Ahead Log (WAL) for a specific Shard.
/// 
/// # Purpose
/// Ensures ACID durability by appending mutations to a disk-resident log file
/// using strict O_DIRECT / O_DSYNC semantics before they are applied to the
/// in-memory index.
///
/// # Thread Safety
/// This struct is intended to be owned by a single `ShardReactor` thread.
/// It is NOT `Sync` and should not be shared across threads (Rule #6).
pub struct WalManager {
    file: DirectFile,
    current_offset: u64,
}

impl WalManager {
    /// Initializes a new WAL Manager.
    ///
    /// # Arguments
    /// * `shard_id` - The physical core ID this shard belongs to.
    /// * `base_path` - The directory where WAL files will be stored.
    ///
    /// # Errors
    /// Returns `std::io::Result` if the file cannot be opened or created.
    pub fn new(shard_id: usize, base_path: &str) -> std::io::Result<Self> {
        let wal_path = format!("{}/shard_{}.wal", base_path, shard_id);
        
        // Open with kernel-bypass flags (O_DIRECT | O_DSYNC)
        let file = DirectFile::open_wal(&wal_path)?;
        
        // In a Production Replay scenario, we would allow seeking to end.
        // For Milestone 4 replacement, start at 0 (Truncate behavior implied by fresh run).
        let current_offset = 0;
        
        info!("Shard {} WAL Manager initialized at {} (Offset: 0)", shard_id, wal_path);
        
        Ok(Self {
            file,
            current_offset,
        })
    }

    /// Prepares a Write SQE for the io_uring submission queue.
    ///
    /// # Logic
    /// Creates an `io_uring::opcode::Write` entry pointing to `buf`.
    /// Does NOT submit the entry; the Reactor must push it to the ring.
    ///
    /// # Safety
    /// * `buf` must be a valid pointer to memory that will NOT be dropped 
    ///   until the completion event is received by the Reactor (Rule #8).
    /// * `len` should ideally be 4096-aligned for optimal O_DIRECT performance.
    pub fn write_entry(&mut self, buf: *const u8, len: u32, user_data: u64) -> io_uring::squeue::Entry {
        // Prepare the IO uring entry
        let entry = self.file.write_sqe(buf, len, self.current_offset, user_data);
        
        // Advance offset state immediately (Optimistic Append)
        self.current_offset += len as u64;
        
        entry
    }

    /// Returns the current write offset (file size).
    pub fn current_offset(&self) -> u64 {
        self.current_offset
    }
}
