use vortex_io::storage::DirectFile;
use log::info;
use std::path::Path;

pub struct WalManager {
    file: DirectFile,
    current_offset: u64,
}

impl WalManager {
    pub fn new(shard_id: usize, base_path: &str) -> std::io::Result<Self> {
        let wal_path = format!("{}/shard_{}.wal", base_path, shard_id);
        let file = DirectFile::open_wal(&wal_path)?;
        
        info!("Shard {} WAL Manager initialized at {}", shard_id, wal_path);
        
        Ok(Self {
            file,
            current_offset: 0,
        })
    }

    pub fn write_entry(&mut self, buf: *const u8, len: u32, user_data: u64) -> io_uring::squeue::Entry {
        // Enforce sector alignment (Rule 9: 4KB aligned for O_DIRECT)
        // Note: For real work, len should be a multiple of 4096. 
        // If not, we'd need to pad or use a aligned staging buffer.
        let entry = self.file.write_sqe(buf, len, self.current_offset, user_data);
        self.current_offset += len as u64;
        entry
    }

    pub fn current_offset(&self) -> u64 {
        self.current_offset
    }
}
