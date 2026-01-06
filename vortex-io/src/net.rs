use std::net::TcpListener;
use std::os::unix::io::{AsRawFd, RawFd};
use io_uring::{opcode, types};
use log::info;

pub struct VortexListener {
    fd: RawFd,
}

impl VortexListener {
    pub fn bind(addr: &str) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        let fd = listener.as_raw_fd();
        // Leak the listener to let io_uring own the FD
        std::mem::forget(listener);
        info!("VBP Listener bound to {} (fd: {})", addr, fd);
        Ok(Self { fd })
    }

    pub fn fd(&self) -> RawFd {
        self.fd
    }

    /// Prepare an Accept SQE for io_uring
    pub fn accept_sqe(&self, addr: *mut libc::sockaddr, addrlen: *mut libc::socklen_t, user_data: u64) -> io_uring::squeue::Entry {
        opcode::Accept::new(types::Fd(self.fd), addr, addrlen)
            .build()
            .user_data(user_data)
    }
}
