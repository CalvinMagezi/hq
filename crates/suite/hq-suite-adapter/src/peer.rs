//! A sink kept by a paired machine.
//!
//! The peer runs a small blob store on its loopback address (`hq suite sink peer-serve`), which
//! HQ's existing pairing exposes as a service. This machine reaches it through a forward to that
//! service, so what it dials is also a loopback address. The originals are sealed here, under a
//! key made from the recovery code, before they leave: the peer holds ciphertext and never the
//! code. The hash an original is filed under is the hash of the plain bytes, as in a folder sink.
//!
//! ```text
//! PUT  /o/<sha256>   store a sealed original (409 if one is already filed under that name)
//! GET  /o/<sha256>   fetch it
//! HEAD /o/<sha256>   is it there
//! GET  /list         every name, one per line
//! PUT  /meta         the sink's open description (once); GET /meta reads it back
//! ```

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use hq_suite_sinks::{Listable, Meta, open_object, seal_object, valid_hash};
use hq_suite_store::{ColdError, ColdResult, ColdStore, fsx};

/// The largest sealed original the store takes.
pub const MAX_OBJECT_BYTES: usize = 256 * 1024 * 1024;
/// The largest description file.
const MAX_META_BYTES: usize = 4096;
const MAX_HEADER_BYTES: usize = 8 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CONNECTIONS: usize = 8;
const ACCEPT_POLL: Duration = Duration::from_millis(40);
const OBJECTS: &str = "objects";
const META: &str = "meta.json";
const EXTENSION: &str = "hqo";

fn cold(msg: impl Into<String>) -> ColdError {
    ColdError(msg.into())
}

struct Reply {
    status: u16,
    body: Vec<u8>,
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        _ => "Error",
    }
}

fn write_reply(stream: &mut TcpStream, r: &Reply, head_only: bool) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        r.status,
        reason(r.status),
        r.body.len()
    )?;
    if !head_only {
        stream.write_all(&r.body)?;
    }
    stream.flush()
}

/// The request line and headers, read with a cap. Returns the method, the path and the declared
/// body length.
fn read_head<R: BufRead>(reader: &mut R) -> Result<(String, String, usize), u16> {
    let mut total = 0usize;
    let mut line = String::new();
    let mut next = |line: &mut String| -> Result<(), u16> {
        line.clear();
        let n = reader.read_line(line).map_err(|_| 400u16)?;
        total += n;
        if n == 0 || total > MAX_HEADER_BYTES {
            return Err(400);
        }
        Ok(())
    };
    next(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, path) = match (parts.next(), parts.next(), parts.next()) {
        (Some(m), Some(p), Some(v)) if v.starts_with("HTTP/1.") => (m.to_string(), p.to_string()),
        _ => return Err(400),
    };
    let mut length = 0usize;
    loop {
        next(&mut line)?;
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().map_err(|_| 400u16)?;
        }
    }
    Ok((method, path, length))
}

fn object_path(dir: &Path, hash: &str) -> PathBuf {
    dir.join(OBJECTS).join(format!("{hash}.{EXTENSION}"))
}

fn list_hashes(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir.join(OBJECTS))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|f| {
            let name = f.file_name().to_string_lossy().into_owned();
            let hash = name.strip_suffix(&format!(".{EXTENSION}"))?.to_string();
            (valid_hash(&hash) && f.metadata().is_ok_and(|m| m.len() > 0)).then_some(hash)
        })
        .collect();
    out.sort();
    out
}

fn store_new(path: &Path, body: &[u8]) -> Reply {
    if std::fs::metadata(path).is_ok_and(|m| m.len() > 0) {
        return Reply {
            status: 409,
            body: Vec::new(),
        };
    }
    match fsx::atomic_write(path, body) {
        Ok(()) => Reply {
            status: 201,
            body: Vec::new(),
        },
        Err(_) => Reply {
            status: 400,
            body: Vec::new(),
        },
    }
}

fn route(dir: &Path, method: &str, path: &str, body: &[u8]) -> Reply {
    let empty = |status| Reply {
        status,
        body: Vec::new(),
    };
    if path == "/list" && method == "GET" {
        let text = list_hashes(dir).join("\n");
        return Reply {
            status: 200,
            body: text.into_bytes(),
        };
    }
    if path == "/meta" {
        let file = dir.join(META);
        return match method {
            "GET" => std::fs::read(&file).map_or(empty(404), |body| Reply { status: 200, body }),
            "PUT" => store_new(&file, body),
            _ => empty(400),
        };
    }
    let Some(hash) = path.strip_prefix("/o/").filter(|h| valid_hash(h)) else {
        return empty(404);
    };
    let file = object_path(dir, hash);
    match method {
        "PUT" => store_new(&file, body),
        "GET" | "HEAD" => match std::fs::read(&file) {
            Ok(body) if !body.is_empty() => Reply { status: 200, body },
            _ => empty(404),
        },
        _ => empty(400),
    }
}

fn handle(dir: &Path, mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(read_half);
    let (method, path, length) = match read_head(&mut reader) {
        Ok(h) => h,
        Err(status) => {
            let _ = write_reply(
                &mut stream,
                &Reply {
                    status,
                    body: Vec::new(),
                },
                false,
            );
            return;
        }
    };
    let cap = if path == "/meta" {
        MAX_META_BYTES
    } else {
        MAX_OBJECT_BYTES
    };
    if length > cap {
        let _ = write_reply(
            &mut stream,
            &Reply {
                status: 413,
                body: Vec::new(),
            },
            false,
        );
        return;
    }
    let mut body = vec![0u8; length];
    if reader.read_exact(&mut body).is_err() {
        let _ = write_reply(
            &mut stream,
            &Reply {
                status: 400,
                body: Vec::new(),
            },
            false,
        );
        return;
    }
    let reply = route(dir, &method, &path, &body);
    let _ = write_reply(&mut stream, &reply, method == "HEAD");
}

/// Serves the blob store in `dir` on `listener` until `stop` is set. The listener must be bound to
/// a loopback address: this is a store for the service forward, never for the network.
pub fn serve(dir: &Path, listener: TcpListener, stop: Arc<AtomicBool>) -> std::io::Result<()> {
    let local = listener.local_addr()?;
    if !local.ip().is_loopback() {
        return Err(std::io::Error::other(
            "a sink store only listens on a loopback address",
        ));
    }
    std::fs::create_dir_all(dir.join(OBJECTS))?;
    listener.set_nonblocking(true)?;
    let live = Arc::new(AtomicUsize::new(0));
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, from)) => {
                if !from.ip().is_loopback() || live.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                    continue;
                }
                let _ = stream.set_nonblocking(false);
                live.fetch_add(1, Ordering::Relaxed);
                let (dir, live) = (dir.to_path_buf(), Arc::clone(&live));
                std::thread::spawn(move || {
                    handle(&dir, stream);
                    live.fetch_sub(1, Ordering::Relaxed);
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(ACCEPT_POLL);
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// One request to the peer's store, and its status and body.
fn request(addr: SocketAddr, method: &str, path: &str, body: &[u8]) -> ColdResult<(u16, Vec<u8>)> {
    let unreachable = |e: std::io::Error| cold(format!("the peer's sink cannot be reached: {e}"));
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(unreachable)?;
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(unreachable)?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(unreachable)?;
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: sink\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .and_then(|()| stream.write_all(body))
    .and_then(|()| stream.flush())
    .map_err(unreachable)?;
    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader.read_line(&mut status_line).map_err(unreachable)?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| cold("the peer's sink answered in a way HQ does not understand"))?;
    let mut length = 0usize;
    let mut header = String::new();
    let mut seen = 0usize;
    loop {
        header.clear();
        let n = reader.read_line(&mut header).map_err(unreachable)?;
        seen += n;
        if n == 0 || seen > MAX_HEADER_BYTES {
            return Err(cold(
                "the peer's sink answered in a way HQ does not understand",
            ));
        }
        if header == "\r\n" || header == "\n" {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    if method == "HEAD" {
        return Ok((status, Vec::new()));
    }
    if length > MAX_OBJECT_BYTES {
        return Err(cold("the peer's sink sent more than a sink may hold"));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).map_err(unreachable)?;
    Ok((status, body))
}

/// A sink on a paired machine, reached through a loopback forward.
pub struct PeerSink {
    id: String,
    key: hq_backup::object::ObjectKey,
    addr: SocketAddr,
}

/// The address of a peer's store: a loopback address and a port, as a forward gives them.
pub fn parse_addr(text: &str) -> ColdResult<SocketAddr> {
    let addr: SocketAddr = text
        .parse()
        .map_err(|_| cold("a peer's address is written like 127.0.0.1:5700"))?;
    if addr.ip() != IpAddr::from([127, 0, 0, 1]) && !addr.ip().is_loopback() {
        return Err(cold(
            "a peer's sink is reached through a loopback forward, so its address is 127.0.0.1 and a port",
        ));
    }
    Ok(addr)
}

impl PeerSink {
    /// Opens the sink described by `meta` with the recovery code, on the peer at `addr`.
    pub fn open(meta: &Meta, code: &str, addr: SocketAddr) -> ColdResult<PeerSink> {
        let key = meta.unlock(code)?;
        Ok(PeerSink {
            id: meta.id().to_string(),
            key,
            addr,
        })
    }

    /// Whether the peer's store answers now.
    pub fn reachable(addr: SocketAddr) -> bool {
        request(addr, "GET", "/list", &[]).is_ok_and(|(s, _)| s == 200)
    }

    /// Files the sink's open description on the peer, so a new phone can find it again with only
    /// the recovery code. A description already there is left alone.
    pub fn publish_meta(addr: SocketAddr, meta: &Meta) -> ColdResult<()> {
        let text = serde_json::to_vec(meta).map_err(|e| cold(e.to_string()))?;
        match request(addr, "PUT", "/meta", &text)? {
            (201 | 409, _) => Ok(()),
            _ => Err(cold("the peer's sink did not take the description")),
        }
    }

    /// The description a peer's store holds, for registering the sink on a new phone.
    pub fn fetch_meta(addr: SocketAddr) -> ColdResult<Meta> {
        match request(addr, "GET", "/meta", &[])? {
            (200, body) => serde_json::from_slice(&body)
                .map_err(|_| cold("the description the peer holds is damaged")),
            _ => Err(cold("that peer holds no sink description")),
        }
    }
}

impl ColdStore for PeerSink {
    fn id(&self) -> &str {
        &self.id
    }

    fn put(&self, sha256: &str, bytes: &[u8]) -> ColdResult<()> {
        let sealed = seal_object(&self.key, sha256, bytes)?;
        if self.has(sha256) && self.get(sha256).is_ok() {
            return Ok(());
        }
        match request(self.addr, "PUT", &format!("/o/{sha256}"), &sealed)? {
            (201, _) => {}
            (409, _) => {
                return Err(cold(
                    "the peer already holds something under that name that this code cannot open; clear it on the peer first",
                ));
            }
            (413, _) => return Err(cold("that original is larger than a peer's sink takes")),
            _ => return Err(cold("the peer's sink did not take it")),
        }
        // Read back through the same path a restore takes, so "stored" means "restorable".
        self.get(sha256)
            .map(drop)
            .map_err(|e| cold(format!("the peer did not keep it: {e}")))
    }

    fn get(&self, sha256: &str) -> ColdResult<Vec<u8>> {
        if !valid_hash(sha256) {
            return Err(cold("that is not a hash this sink files by"));
        }
        match request(self.addr, "GET", &format!("/o/{sha256}"), &[])? {
            (200, sealed) => open_object(&self.key, sha256, &sealed),
            _ => Err(cold("that original is not in this sink")),
        }
    }

    fn has(&self, sha256: &str) -> bool {
        valid_hash(sha256)
            && request(self.addr, "HEAD", &format!("/o/{sha256}"), &[]).is_ok_and(|(s, _)| s == 200)
    }
}

impl Listable for PeerSink {
    fn list_hashes(&self) -> Vec<String> {
        match request(self.addr, "GET", "/list", &[]) {
            Ok((200, body)) => String::from_utf8_lossy(&body)
                .lines()
                .filter(|l| valid_hash(l))
                .map(str::to_string)
                .collect(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_suite_sinks::restore_drill;
    use sha2::{Digest, Sha256};

    fn hash(b: &[u8]) -> String {
        hex::encode(Sha256::digest(b))
    }

    struct Store {
        addr: SocketAddr,
        dir: tempfile::TempDir,
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Store {
        fn start() -> Store {
            let dir = tempfile::tempdir().unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let (d, s) = (dir.path().to_path_buf(), Arc::clone(&stop));
            let thread = std::thread::spawn(move || {
                serve(&d, listener, s).unwrap();
            });
            Store {
                addr,
                dir,
                stop,
                thread: Some(thread),
            }
        }
    }

    impl Drop for Store {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    fn sink(store: &Store) -> (PeerSink, Meta, String) {
        let (meta, _, code) = Meta::make("phone-backup").unwrap();
        let sink = PeerSink::open(&meta, &code, store.addr).unwrap();
        (sink, meta, code)
    }

    #[test]
    fn an_original_is_sealed_before_it_leaves_and_comes_back_whole() {
        let store = Store::start();
        let (sink, _, _) = sink(&store);
        let original = b"the original picture bytes, large and precious".repeat(200);
        let h = hash(&original);
        assert!(!sink.has(&h));
        sink.put(&h, &original).unwrap();
        assert!(sink.has(&h));
        assert_eq!(sink.get(&h).unwrap(), original);
        sink.put(&h, &original).unwrap();
        let on_disk = std::fs::read(object_path(store.dir.path(), &h)).unwrap();
        assert!(
            !on_disk
                .windows(20)
                .any(|w| original.windows(20).any(|o| o == w)),
            "the peer holds ciphertext, not the original"
        );
        assert_eq!(sink.list_hashes(), vec![h]);
    }

    #[test]
    fn the_wrong_code_and_the_wrong_name_are_refused() {
        let store = Store::start();
        let (sink, meta, _) = sink(&store);
        let data = b"x".repeat(100);
        let h = hash(&data);
        sink.put(&h, &data).unwrap();
        let other = PeerSink::open(&meta, "not the code", store.addr);
        assert!(
            other.is_err(),
            "a wrong code is refused before anything is read"
        );
        assert!(
            sink.put(&hash(b"other"), &data).is_err(),
            "bytes must hash to their name"
        );
        assert!(sink.get("../../etc/passwd").is_err());
        assert!(!sink.has("not-a-hash"));
    }

    #[test]
    fn the_store_takes_only_names_that_are_hashes_and_never_overwrites() {
        let store = Store::start();
        let a = store.addr;
        assert_eq!(request(a, "PUT", "/o/../x", b"data").unwrap().0, 404);
        assert_eq!(
            request(a, "PUT", &format!("/o/{}", "A".repeat(64)), b"d")
                .unwrap()
                .0,
            404
        );
        let h = "a".repeat(64);
        assert_eq!(
            request(a, "PUT", &format!("/o/{h}"), b"first").unwrap().0,
            201
        );
        assert_eq!(
            request(a, "PUT", &format!("/o/{h}"), b"second").unwrap().0,
            409
        );
        assert_eq!(
            request(a, "GET", &format!("/o/{h}"), &[]).unwrap().1,
            b"first"
        );
        assert_eq!(
            request(a, "DELETE", &format!("/o/{h}"), &[]).unwrap().0,
            400
        );
        assert_eq!(request(a, "GET", "/etc/passwd", &[]).unwrap().0, 404);
    }

    #[test]
    fn a_description_on_the_peer_lets_a_new_phone_find_the_sink_with_only_the_code() {
        let store = Store::start();
        let (sink_a, meta, code) = sink(&store);
        PeerSink::publish_meta(store.addr, &meta).unwrap();
        let data = b"kept across a wipe".repeat(50);
        let h = hash(&data);
        sink_a.put(&h, &data).unwrap();
        let found = PeerSink::fetch_meta(store.addr).unwrap();
        let fresh = PeerSink::open(&found, &code, store.addr).unwrap();
        assert_eq!(fresh.get(&h).unwrap(), data);
        let drill = restore_drill(&fresh, 5);
        assert_eq!((drill.checked, drill.failed.len()), (1, 0));
    }

    #[test]
    fn a_damaged_object_fails_the_drill_and_an_unreachable_peer_says_so() {
        let store = Store::start();
        let (sink, _, _) = sink(&store);
        let data = b"y".repeat(80);
        let h = hash(&data);
        sink.put(&h, &data).unwrap();
        std::fs::write(object_path(store.dir.path(), &h), b"garbage").unwrap();
        let drill = restore_drill(&sink, 5);
        assert_eq!(drill.failed, vec![h]);
        let dead = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap()
        };
        assert!(!PeerSink::reachable(dead));
        assert!(
            parse_addr("192.168.1.9:5700").is_err(),
            "only a loopback forward is dialled"
        );
        assert!(parse_addr("127.0.0.1:5700").is_ok());
    }

    #[test]
    fn an_oversized_or_malformed_request_is_refused_not_buffered() {
        let store = Store::start();
        let mut s = TcpStream::connect(store.addr).unwrap();
        write!(
            s,
            "PUT /o/{} HTTP/1.1\r\nContent-Length: 999999999999\r\n\r\n",
            "b".repeat(64)
        )
        .unwrap();
        let mut out = String::new();
        BufReader::new(s).read_line(&mut out).unwrap();
        assert!(out.contains("413") || out.contains("400"), "{out}");
        let mut s2 = TcpStream::connect(store.addr).unwrap();
        s2.write_all(b"NOT HTTP AT ALL\r\n\r\n").unwrap();
        let mut out2 = String::new();
        BufReader::new(s2).read_line(&mut out2).unwrap();
        assert!(out2.contains("400"), "{out2}");
    }
}
