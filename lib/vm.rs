use anyhow::Result;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::net::UnixDatagram;

pub struct VM {
    sock: UnixDatagram,
}

impl VM {
    pub fn new(vm_fd: RawFd) -> Result<VM> {
        let sock = unsafe { UnixDatagram::from_raw_fd(vm_fd) };
        sock.set_nonblocking(true)?;

        Ok(VM { sock })
    }

    pub fn write(&self, pkt: &[u8]) -> std::io::Result<usize> {
        self.sock.send(pkt)
    }

    pub fn read(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.sock.recv(buf)
    }
}

impl AsRawFd for VM {
    fn as_raw_fd(&self) -> RawFd {
        self.sock.as_raw_fd()
    }
}
