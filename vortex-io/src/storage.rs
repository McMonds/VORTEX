use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, RawFd};
use io_uring::{opcode, types};
use log::info;

pub struct DirectFile {
    file: File,
    fd: RawFd,
}

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
        Ok(Self { file, fd })
    }

    pub fn fd(&self) -> RawFd {
        self.fd
    }

    /// Prepare a Write SQE. Buffer MUST be page-aligned (Constitution Rule 9)
    pub fn write_sqe(&self, buf: *const u8, len: u32, offset: u64, user_data: u64) -> io_uring::squeue::Entry {
        opcode::Write::new(types::Fd(self.fd), buf, len)
            .offset(offset)
            .build()
            .user_data(user_data)
    }
}
