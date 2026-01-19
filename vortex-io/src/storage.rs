use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, RawFd};
use io_uring::{opcode, types};
use log::info;

pub struct DirectFile {
    _file: File,
    fd: RawFd,
}

// [REMOVED] Unused AlignedPadding

impl DirectFile {
    /// Open a file with O_DIRECT | O_DSYNC for kernel-bypass persistence (BP 10)
    pub fn open_wal(path: &str) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .custom_flags(libc::O_DIRECT | libc::O_DSYNC)
            .open(path)?;
        
        let fd = file.as_raw_fd();
        info!("WAL File opened with O_DIRECT at {} (fd: {})", path, fd);
        Ok(Self { _file: file, fd })
    }

    pub fn fd(&self) -> RawFd {
        self.fd
    }

    /// Returns the current size of the underlying file.
    pub fn file_size(&self) -> std::io::Result<u64> {
        self._file.metadata().map(|m| m.len())
    }

    /// Truncates the file to a specific size.
    /// Used during recovery to prune corrupted tails.
    pub fn truncate(&self, size: u64) -> std::io::Result<()> {
        self._file.set_len(size)
    }

    pub fn write_sqe(&self, buf: *const u8, len: u32, offset: u64, user_data: u64) -> io_uring::squeue::Entry {
        opcode::Write::new(types::Fd(self.fd), buf, len)
            .offset(offset)
            .build()
            .user_data(user_data)
    }
}
