//! The far side of a forwarded connection: a byte stream the stack can read from and write to
//! without blocking. Real ones are loopback TCP streams; tests use an in-memory pair.

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::os::fd::{AsRawFd, RawFd};
use std::time::Duration;

/// Loopback answers at once or refuses at once; this only bounds a target that is wedged.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(100);

pub trait Pipe {
    /// `WouldBlock` when nothing is waiting, `Ok(0)` at the end of the stream.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    /// `WouldBlock` when the stream cannot take more now.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize>;
    fn shutdown_write(&mut self);
    /// The descriptor to wait on, if there is one.
    fn raw_fd(&self) -> Option<RawFd> {
        None
    }
}

pub trait Connector {
    fn connect(&mut self, to: SocketAddr) -> io::Result<Box<dyn Pipe>>;
}

pub struct TcpPipe(TcpStream);

impl TcpPipe {
    pub fn new(stream: TcpStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        stream.set_nodelay(true)?;
        Ok(Self(stream))
    }
}

impl Pipe for TcpPipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }
    fn shutdown_write(&mut self) {
        let _ = self.0.shutdown(Shutdown::Write);
    }
    fn raw_fd(&self) -> Option<RawFd> {
        Some(self.0.as_raw_fd())
    }
}

/// Connects only to loopback addresses, whatever it is asked.
pub struct LoopbackConnector;

impl Connector for LoopbackConnector {
    fn connect(&mut self, to: SocketAddr) -> io::Result<Box<dyn Pipe>> {
        if !to.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "only loopback targets are allowed",
            ));
        }
        let stream = TcpStream::connect_timeout(&to, CONNECT_TIMEOUT)?;
        Ok(Box::new(TcpPipe::new(stream)?))
    }
}

/// An in-memory pair, for tests and for callers with no sockets.
pub mod mem {
    use super::{Connector, Pipe};
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::io;
    use std::net::SocketAddr;
    use std::rc::Rc;

    #[derive(Default)]
    struct Half {
        bytes: VecDeque<u8>,
        closed: bool,
    }

    /// One end. What it writes, the other end reads.
    pub struct MemPipe {
        incoming: Rc<RefCell<Half>>,
        outgoing: Rc<RefCell<Half>>,
        capacity: usize,
    }

    /// Two ends. `capacity` bounds how much a write may leave unread (`WouldBlock` beyond it).
    pub fn pair(capacity: usize) -> (MemPipe, MemPipe) {
        let a = Rc::new(RefCell::new(Half::default()));
        let b = Rc::new(RefCell::new(Half::default()));
        (
            MemPipe {
                incoming: a.clone(),
                outgoing: b.clone(),
                capacity,
            },
            MemPipe {
                incoming: b,
                outgoing: a,
                capacity,
            },
        )
    }

    impl Pipe for MemPipe {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let mut half = self.incoming.borrow_mut();
            if half.bytes.is_empty() {
                return if half.closed {
                    Ok(0)
                } else {
                    Err(io::ErrorKind::WouldBlock.into())
                };
            }
            let n = buf.len().min(half.bytes.len());
            for slot in buf.iter_mut().take(n) {
                *slot = half.bytes.pop_front().unwrap_or_default();
            }
            Ok(n)
        }
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let mut half = self.outgoing.borrow_mut();
            let room = self.capacity.saturating_sub(half.bytes.len());
            if room == 0 {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let n = buf.len().min(room);
            half.bytes.extend(&buf[..n]);
            Ok(n)
        }
        fn shutdown_write(&mut self) {
            self.outgoing.borrow_mut().closed = true;
        }
    }

    impl Drop for MemPipe {
        fn drop(&mut self) {
            self.outgoing.borrow_mut().closed = true;
        }
    }

    /// Hands out the stack's end of a new pair and keeps the other end for the test to find.
    #[derive(Default)]
    pub struct MemConnector {
        pub opened: Vec<(SocketAddr, MemPipe)>,
        pub refuse: bool,
        /// How much a pipe takes before it blocks; a megabyte when unset.
        pub capacity: Option<usize>,
    }

    impl Connector for MemConnector {
        fn connect(&mut self, to: SocketAddr) -> io::Result<Box<dyn Pipe>> {
            if self.refuse {
                return Err(io::ErrorKind::ConnectionRefused.into());
            }
            let (ours, theirs) = pair(self.capacity.unwrap_or(1 << 20));
            self.opened.push((to, theirs));
            Ok(Box::new(ours))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_connector_refuses_anything_but_loopback_whatever_it_is_asked() {
        let mut connector = LoopbackConnector;
        for target in [
            "192.0.2.1:80",
            "10.77.0.2:80",
            "0.0.0.0:80",
            "[2001:db8::1]:80",
        ] {
            let err = connector.connect(target.parse().unwrap()).err().unwrap();
            assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "{target}");
        }
    }

    #[test]
    fn the_connector_reaches_a_loopback_listener_and_its_stream_does_not_block() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut pipe = LoopbackConnector
            .connect(listener.local_addr().unwrap())
            .unwrap();
        let (mut accepted, _) = listener.accept().unwrap();
        assert_eq!(
            pipe.read(&mut [0u8; 4]).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(pipe.raw_fd().is_some());
        pipe.write(b"hi").unwrap();
        let mut got = [0u8; 2];
        std::io::Read::read_exact(&mut accepted, &mut got).unwrap();
        assert_eq!(&got, b"hi");
    }
}
