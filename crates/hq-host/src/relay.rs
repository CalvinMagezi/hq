//! The bridge that gets an agent's HTTP traffic out of a network-less sandbox
//! on Linux. bubblewrap gives the agent an empty network namespace, so the only
//! way out is a unix socket. Inside, `run_relay` listens on the proxy port on
//! loopback and forwards to that socket; outside, `serve_bridge` turns
//! connections on the socket into connections to the agent's egress proxy.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const SOCKET_MODE: u32 = 0o600;
const ACCEPT_POLL: Duration = Duration::from_millis(50);
/// How long `sandbox_init` waits for its relay to start listening.
const RELAY_READY: Duration = Duration::from_secs(3);
const COPY_BUF: usize = 16 * 1024;

/// One end of a bridged connection.
trait Side: Read + Write + Send + Sized + 'static {
    fn dup(&self) -> std::io::Result<Self>;
    fn close(&self);
}

impl Side for TcpStream {
    fn dup(&self) -> std::io::Result<Self> {
        self.try_clone()
    }
    fn close(&self) {
        let _ = self.shutdown(Shutdown::Both);
    }
}

impl Side for UnixStream {
    fn dup(&self) -> std::io::Result<Self> {
        self.try_clone()
    }
    fn close(&self) {
        let _ = self.shutdown(Shutdown::Both);
    }
}

/// Copies both ways until either side ends, then closes both.
fn pipe<A: Side, B: Side>(a: A, b: B) {
    let (Ok(mut a_read), Ok(mut b_read)) = (a.dup(), b.dup()) else {
        return;
    };
    let (mut a_write, mut b_write) = (a, b);
    let forward = std::thread::spawn(move || {
        let mut buf = vec![0u8; COPY_BUF];
        while let Ok(n) = a_read.read(&mut buf) {
            if n == 0 || b_write.write_all(&buf[..n]).is_err() {
                break;
            }
        }
        b_write.close();
    });
    let mut buf = vec![0u8; COPY_BUF];
    while let Ok(n) = b_read.read(&mut buf) {
        if n == 0 || a_write.write_all(&buf[..n]).is_err() {
            break;
        }
    }
    a_write.close();
    let _ = forward.join();
}

/// Inside the sandbox: forwards loopback `port` to the unix socket. Does not return.
pub fn run_relay(port: u16, unix: &Path) -> std::io::Result<()> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
    for client in listener.incoming() {
        let Ok(client) = client else { continue };
        let Ok(upstream) = UnixStream::connect(unix) else {
            client.close();
            continue;
        };
        std::thread::spawn(move || pipe(client, upstream));
    }
    Ok(())
}

/// Outside: accepts on a new unix socket at `path` and forwards each connection
/// to the egress proxy on loopback `port`, until `stop` is set. The socket file
/// is removed at the end.
pub fn serve_bridge(path: &Path, port: u16, stop: Arc<AtomicBool>) -> std::io::Result<()> {
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(SOCKET_MODE))?;
    listener.set_nonblocking(true)?;
    let path = path.to_path_buf();
    std::thread::Builder::new().name("egress-bridge".into()).spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((client, _)) => {
                    let _ = client.set_nonblocking(false);
                    let Ok(proxy) = TcpStream::connect((Ipv4Addr::LOCALHOST, port)) else {
                        client.close();
                        continue;
                    };
                    std::thread::spawn(move || pipe(client, proxy));
                }
                Err(_) => std::thread::sleep(ACCEPT_POLL),
            }
        }
        let _ = std::fs::remove_file(&path);
    })?;
    Ok(())
}

/// The first thing run inside a bubblewrap sandbox: starts the relay as a child
/// (it ends with the sandbox's pid namespace), waits until it listens, then
/// replaces this process with the agent. Returns only on failure.
pub fn sandbox_init(port: u16, unix: &Path, argv: &[String]) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    let Some((program, args)) = argv.split_first() else {
        return std::io::Error::other("sandbox-init needs a command to run");
    };
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => return e,
    };
    let spawned = std::process::Command::new(exe)
        .args(["host", "relay", "--port", &port.to_string(), "--unix"])
        .arg(unix)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Err(e) = spawned {
        return e;
    }
    let deadline = std::time::Instant::now() + RELAY_READY;
    while TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_err() {
        if std::time::Instant::now() > deadline {
            return std::io::Error::other("the egress relay did not start");
        }
        std::thread::sleep(ACCEPT_POLL);
    }
    std::process::Command::new(program).args(args).exec()
}
