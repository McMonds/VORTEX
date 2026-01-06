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

/// Standard Page Size for NVMe/SSD alignment (4KB).
pub const PAGE_SIZE: usize = 4096;

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
        
        // RECOVERY LOGIC: Seek to the end of the file to determine the append cursor.
        // This allows the system to restart and continue appending to the existing log
        // without overwriting committed data.
        let current_offset = file.file_size()?;
        
        info!("Shard {} WAL Manager initialized at {} (Offset: {})", shard_id, wal_path, current_offset);
        
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

    /// Truncates the WAL to a specific offset.
    /// Used during recovery to prune corrupted tails.
    pub fn truncate(&mut self, offset: u64) -> std::io::Result<()> {
        self.file.truncate(offset)?;
        self.current_offset = offset;
        Ok(())
    }

    /// Returns the current write offset (file size).
    pub fn current_offset(&self) -> u64 {
        self.current_offset
    }

    /// Creates a blocking iterator for WAL replay during boot.
    ///
    /// # Rule #8 Exception
    /// This uses synchronous blocking I/O (`std::fs::File`) which is normally
    /// forbidden by the 12 Commandments. However, during boot (before the
    /// Reactor is online), blocking is acceptable and simpler than async.
    ///
    /// # Returns
    /// A `WalIterator` that yields WAL entries sequentially from offset 0.
    pub fn replay_iter(&self, wal_path: &str) -> std::io::Result<WalIterator> {
        WalIterator::new(wal_path)
    }
}

/// Iterator for sequentially reading WAL entries during crash recovery.
///
/// # Boot-Time Only
/// This struct uses blocking `std::fs::File::read_exact` which violates Rule #8.
/// It is ONLY safe to use during boot before the Reactor starts.
pub struct WalIterator {
    file: std::fs::File,
    bytes_read: u64,
}

impl WalIterator {
    fn new(wal_path: &str) -> std::io::Result<Self> {
        let file = std::fs::File::open(wal_path)?;
        Ok(Self {
            file,
            bytes_read: 0,
        })
    }

    /// Returns the total number of bytes processed so far.
    pub fn bytes_read(&self) -> u64 {
        self.bytes_read
    }
}

/// A single WAL entry read during replay.
pub struct WalEntry {
    /// The raw request header (16 bytes)
    pub header: vortex_rpc::RequestHeader,
    /// The payload (ID + Vector bytes)
    pub payload: Vec<u8>,
}

impl Iterator for WalIterator {
    type Item = std::io::Result<WalEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        use std::io::Read;

        let entry_start_offset = self.bytes_read;

        // 1. Read Header (16 bytes)
        let mut header_buf = [0u8; 16];
        match self.file.read_exact(&mut header_buf) {
            Ok(_) => {},
            Err(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // EOF: Clean termination
                return None;
            },
            Err(e) => {
                // Partial read = Corruption
                return Some(Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("WAL Truncation detected at offset {}: {}", entry_start_offset, e)
                )));
            }
        }

        // 2. Parse Header
        // SAFETY: RequestHeader is #[repr(C)] with fixed layout
        let header = unsafe {
            std::ptr::read(header_buf.as_ptr() as *const vortex_rpc::RequestHeader)
        };

        // 3. Validate Magic
        if header.magic != vortex_rpc::VBP_MAGIC {
            return Some(Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("WAL Corruption: Invalid magic 0x{:x} at offset {}", header.magic, entry_start_offset)
            )));
        }

        // Advance count only after header is fully validated
        self.bytes_read += 16;

        // 4. Read Payload
        let payload_len = header.payload_len as usize;
        let mut payload = vec![0u8; payload_len];

        if let Err(e) = self.file.read_exact(&mut payload) {
            return Some(Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("WAL Truncation in payload at offset {}: {}", self.bytes_read, e)
            )));
        }

        self.bytes_read += payload_len as u64;

        // 5. Yield Entry
        Some(Ok(WalEntry { header, payload }))
    }
}
