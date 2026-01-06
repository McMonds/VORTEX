use std::os::unix::io::RawFd;
use io_uring::{opcode, types};
use log::info;
use std::net::ToSocketAddrs;

pub struct VortexListener {
    fd: RawFd,
}

impl VortexListener {
    pub fn bind(addr: &str) -> std::io::Result<Self> {
        let socket_addr = addr.to_socket_addrs()?.next().ok_or(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid address"))?;
        
        unsafe {
            let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
            if fd < 0 { return Err(std::io::Error::last_os_error()); }

            // Enable SO_REUSEADDR and SO_REUSEPORT
            let optval: libc::c_int = 1;
            libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, &optval as *const _ as *const libc::c_void, std::mem::size_of::<libc::c_int>() as libc::socklen_t);
            libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEPORT, &optval as *const _ as *const libc::c_void, std::mem::size_of::<libc::c_int>() as libc::socklen_t);

            // Bind
            let mut sockaddr_in: libc::sockaddr_in = std::mem::zeroed();
            sockaddr_in.sin_family = libc::AF_INET as libc::sa_family_t;
            sockaddr_in.sin_port = socket_addr.port().to_be();
            match socket_addr.ip() {
                std::net::IpAddr::V4(v4) => {
                    sockaddr_in.sin_addr.s_addr = u32::from_ne_bytes(v4.octets());
                }
                _ => return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "IPv6 not supported in Milestone 6")),
            }

            if libc::bind(fd, &sockaddr_in as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t) < 0 {
                let err = std::io::Error::last_os_error();
                libc::close(fd);
                return Err(err);
            }

            // Listen
            if libc::listen(fd, 128) < 0 {
                let err = std::io::Error::last_os_error();
                libc::close(fd);
                return Err(err);
            }

            // Non-blocking
            let flags = libc::fcntl(fd, libc::F_GETFL, 0);
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);

            info!("VBP Listener bound to {} (fd: {}) with HARDWARE REUSEPORT", addr, fd);
            Ok(Self { fd })
        }
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
