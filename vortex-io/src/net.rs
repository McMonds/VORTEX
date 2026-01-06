use std::os::unix::io::{AsRawFd, RawFd};
use std::net::{Ipv4Addr, SocketAddr};
use io_uring::{opcode, types};
use socket2::{Socket, Domain, Type, Protocol};
use log::info;

/// VORTEX TCP Ingress Listener.
/// Uses socket2 for safe hardware-level configuration (REUSEPORT, NODELAY).
pub struct VortexListener {
    socket: Socket,
}

impl VortexListener {
    /// Creates a new high-performance ingress listener for the VBP protocol.
    /// 
    /// # Performance
    /// - Enables `SO_REUSEPORT` for linear multi-core scaling.
    /// - Enables `TCP_NODELAY` to minimize latency for small vector packets.
    /// - Sets `O_NONBLOCK` for compatibility with `io_uring`.
    /// 
    /// # Errors
    /// Returns `std::io::Error` if the socket cannot be created, configured, or bound.
    pub fn new_ingress(port: u16) -> std::io::Result<Self> {
        let domain = Domain::IPV4;
        let socket_type = Type::STREAM;
        let protocol = Some(Protocol::TCP);

        let socket = Socket::new(domain, socket_type, protocol)?;

        // Performance & Clustering Configuration
        socket.set_reuse_address(true)?;
        socket.set_reuse_port(true)?;
        socket.set_nodelay(true)?;
        socket.set_nonblocking(true)?;

        // Bind to all interfaces (Global Ingress)
        let addr = SocketAddr::new(Ipv4Addr::new(0, 0, 0, 0).into(), port);
        socket.bind(&addr.into())?;

        // Listen with production-grade backlog
        const LISTEN_BACKLOG: i32 = 4096;
        socket.listen(LISTEN_BACKLOG)?;

        info!("VBP Ingress active on port {} (fd: {}) [REUSEPORT=ON, NODELAY=ON]", port, socket.as_raw_fd());
        
        Ok(Self { socket })
    }

    /// Exposes the raw file descriptor for the io_uring submission queue.
    #[inline]
    pub fn as_raw_fd(&self) -> RawFd {
        self.socket.as_raw_fd()
    }

    /// Generates an io_uring Accept SQE for this listener.
    /// 
    /// # SAFETY
    /// The caller must ensure `addr` and `addrlen` are valid pointers or null.
    pub fn accept_sqe(&self, addr: *mut libc::sockaddr, addrlen: *mut libc::socklen_t, tag: u64) -> io_uring::squeue::Entry {
        opcode::Accept::new(types::Fd(self.as_raw_fd()), addr, addrlen)
            .build()
            .user_data(tag)
    }
}

impl AsRawFd for VortexListener {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_fd()
    }
}
