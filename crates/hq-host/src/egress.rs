//! The egress proxy for sandboxed agents. A sandbox can only say "connect to
//! this local port"; it cannot filter by domain. So each agent gets its own
//! loopback listener here, the sandbox allows exactly that port, and this decides
//! what the agent may reach: only hosts its rules name, resolved by the host
//! itself, never a private, loopback or link-local address unless a rule says so.
//!
//! One listener per agent means the sandbox rule for one agent's port does not
//! open another agent's, and every decision is attributable to an agent.
//!
//! It speaks the two things an HTTP client sends a proxy: `CONNECT host:port`
//! (tunnelled TLS) and an absolute-form `GET`/`POST http://host/...` (plain HTTP,
//! one request per connection so a second request cannot name another host).

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Longest request line plus headers read from a client.
const MAX_HEAD_BYTES: usize = 16 * 1024;
/// Connections one agent may hold open at once.
const MAX_CONNECTIONS: usize = 32;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// A tunnel with no traffic in either direction this long is closed.
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
const DRAIN_TIMEOUT: Duration = Duration::from_millis(200);
const ACCEPT_POLL: Duration = Duration::from_millis(50);
/// Decisions kept per agent for `decisions`.
const LOG_CAPACITY: usize = 200;
const DEFAULT_PORT: u16 = 443;
/// Longest TLS ClientHello record accepted before the tunnel is judged.
const MAX_HELLO_BYTES: usize = 16 * 1024;

/// One thing an agent may reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// An exact host name, or `*.suffix` for any subdomain (not the bare suffix).
    pub host: String,
    /// Allowed ports; empty means 443 only.
    pub ports: Vec<u16>,
    /// The host may resolve to a private, loopback or link-local address. Set only
    /// for an endpoint the operator named, such as the HQ MCP address on a tailnet.
    pub allow_private: bool,
    /// A CONNECT tunnel must open with a TLS ClientHello whose server name is the
    /// host the request named, so a tunnel to an allowed address cannot reach a
    /// different site by naming it inside TLS. On by default.
    pub check_sni: bool,
}

impl Rule {
    pub fn new(host: &str) -> Self {
        Self {
            host: host.to_ascii_lowercase(),
            ports: Vec::new(),
            allow_private: false,
            check_sni: true,
        }
    }

    pub fn port(mut self, port: u16) -> Self {
        self.ports.push(port);
        self
    }

    /// Lets a tunnel carry any protocol (plain TCP to an operator-named endpoint).
    pub fn any_protocol(mut self) -> Self {
        self.check_sni = false;
        self
    }

    pub fn private(mut self) -> Self {
        self.allow_private = true;
        self
    }

    fn matches(&self, host: &str, port: u16) -> bool {
        let ports_ok = if self.ports.is_empty() {
            port == DEFAULT_PORT
        } else {
            self.ports.contains(&port)
        };
        let host_ok = match self.host.strip_prefix("*.") {
            Some(suffix) => host
                .strip_suffix(suffix)
                .is_some_and(|rest| rest.ends_with('.') && rest.len() > 1),
            None => host == self.host,
        };
        ports_ok && host_ok
    }
}

/// What the proxy decided about one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow(SocketAddr),
    Deny(String),
}

/// One decision, kept for the operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub host: String,
    pub port: u16,
    pub allowed: bool,
    pub reason: String,
}

/// Whether `ip` is somewhere an agent must not reach by default: this machine,
/// the local network, link-local, carrier-grade NAT (tailnets live there),
/// multicast, or unspecified.
pub fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_broadcast()
                || (o[0] == 100 && (64..128).contains(&o[1]))
                || (o[0] == 198 && (o[1] & 0xfe) == 18)
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || o[0] == 0
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private(IpAddr::V4(v4));
            }
            let seg = v6.segments();
            // Names for an IPv4 address inside IPv6: NAT64, 6to4, v4-compatible.
            let embeds_v4 = (seg[0] == 0x64 && seg[1] == 0xff9b && seg[2..6] == [0; 4])
                || seg[0] == 0x2002
                || seg[..6] == [0; 6];
            embeds_v4
                || (seg[0] & 0xffc0) == 0xfec0
                || v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (seg[0] & 0xfe00) == 0xfc00
                || (seg[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Decides one request. `resolve` turns a name into addresses (injected so the
/// logic is testable). Every address must be acceptable: a name that resolves to
/// both a public and a private address is refused, so the choice of address the
/// connection later makes cannot matter.
pub fn decide(
    rules: &[Rule],
    host: &str,
    port: u16,
    resolve: impl Fn(&str, u16) -> Vec<IpAddr>,
) -> Verdict {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.parse::<IpAddr>().is_ok() {
        // A literal address is never a name a rule can vouch for by suffix; only an
        // exact rule for it (with `allow_private` if it is private) lets it through.
        return match rules
            .iter()
            .find(|r| r.host == host && r.matches(&host, port))
        {
            Some(rule) => finish(rule, &host, port, &resolve),
            None => Verdict::Deny(format!("{host}:{port} is an address, not an allowed name")),
        };
    }
    match rules.iter().find(|r| r.matches(&host, port)) {
        Some(rule) => finish(rule, &host, port, &resolve),
        None => Verdict::Deny(format!("{host}:{port} is not on the allowlist")),
    }
}

fn finish(
    rule: &Rule,
    host: &str,
    port: u16,
    resolve: &impl Fn(&str, u16) -> Vec<IpAddr>,
) -> Verdict {
    let addrs = resolve(host, port);
    // IPv4 first: a listener bound on one family is not reachable on the other.
    let first = addrs.iter().find(|ip| ip.is_ipv4()).or(addrs.first());
    let Some(first) = first else {
        return Verdict::Deny(format!("{host} did not resolve"));
    };
    if !rule.allow_private && addrs.iter().any(|ip| is_private(*ip)) {
        return Verdict::Deny(format!("{host} resolves to a private or local address"));
    }
    Verdict::Allow(SocketAddr::new(*first, port))
}

fn system_resolve(host: &str, port: u16) -> Vec<IpAddr> {
    (host, port)
        .to_socket_addrs()
        .map(|it| it.map(|a| a.ip()).collect())
        .unwrap_or_default()
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

struct AgentProxy {
    port: u16,
    stop: Arc<AtomicBool>,
}

type Log = Arc<Mutex<VecDeque<Decision>>>;

/// The agents' listeners.
#[derive(Default)]
pub struct Egress {
    agents: Mutex<HashMap<String, AgentProxy>>,
    /// Kept after a listener closes so the operator can see why an agent that
    /// has exited was refused; dropped by `forget` or replaced by the next `open`.
    logs: Mutex<HashMap<String, Log>>,
}

impl Egress {
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens a loopback listener for `agent` applying `rules` and returns its port.
    /// Opening again for the same agent replaces the earlier listener.
    pub fn open(&self, agent: &str, rules: Vec<Rule>) -> std::io::Result<u16> {
        self.open_with_bridge(agent, rules, None)
    }

    /// Like `open`, and also serves `bridge` (a unix socket) that forwards to the
    /// listener, for sandboxes with no network of their own.
    pub fn open_with_bridge(
        &self,
        agent: &str,
        rules: Vec<Rule>,
        bridge: Option<&std::path::Path>,
    ) -> std::io::Result<u16> {
        self.close(agent);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let log: Log = Arc::new(Mutex::new(VecDeque::new()));
        lock(&self.logs).insert(agent.to_string(), log.clone());
        let (rules, open) = (Arc::new(rules), Arc::new(AtomicUsize::new(0)));
        if let Some(path) = bridge {
            crate::relay::serve_bridge(path, port, stop.clone())?;
        }
        let (accept_stop, accept_log) = (stop.clone(), log);
        std::thread::Builder::new()
            .name(format!("egress-{agent}"))
            .spawn(move || {
                while !accept_stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((client, _)) => {
                            admit(client, &rules, &open, &accept_log, &accept_stop);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(ACCEPT_POLL);
                        }
                        Err(_) => std::thread::sleep(ACCEPT_POLL),
                    }
                }
            })?;
        lock(&self.agents).insert(agent.to_string(), AgentProxy { port, stop });
        Ok(port)
    }

    /// Stops `agent`'s listener; its open tunnels end at their next read.
    pub fn close(&self, agent: &str) {
        if let Some(proxy) = lock(&self.agents).remove(agent) {
            proxy.stop.store(true, Ordering::SeqCst);
        }
    }

    /// Stops `agent`'s listener only if it is still the one on `port`, so a late
    /// exit of an old process cannot close its replacement's listener.
    pub fn close_port(&self, agent: &str, port: u16) {
        let mut agents = lock(&self.agents);
        let current = agents.get(agent).is_some_and(|p| p.port == port);
        if let (true, Some(proxy)) = (current, agents.remove(agent)) {
            proxy.stop.store(true, Ordering::SeqCst);
        }
    }

    /// The most recent decisions for `agent`, oldest first.
    pub fn decisions(&self, agent: &str) -> Vec<Decision> {
        lock(&self.logs)
            .get(agent)
            .map(|log| lock(log).iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Stops `agent`'s listener and drops its log.
    pub fn forget(&self, agent: &str) {
        self.close(agent);
        lock(&self.logs).remove(agent);
    }
}

impl Drop for Egress {
    fn drop(&mut self) {
        for proxy in lock(&self.agents).values() {
            proxy.stop.store(true, Ordering::SeqCst);
        }
    }
}

/// One slot of an agent's connection budget, returned on drop.
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn admit(
    client: TcpStream,
    rules: &Arc<Vec<Rule>>,
    open: &Arc<AtomicUsize>,
    log: &Arc<Mutex<VecDeque<Decision>>>,
    stop: &Arc<AtomicBool>,
) {
    if open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
        open.fetch_sub(1, Ordering::SeqCst);
        let mut client = client;
        let _ = client.write_all(b"HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\n\r\n");
        return;
    }
    let slot = Slot(open.clone());
    let (rules, log, stop) = (rules.clone(), log.clone(), stop.clone());
    let _ = std::thread::Builder::new().spawn(move || {
        let _slot = slot;
        let _ = serve(client, &rules, &log, &stop);
    });
}

/// A parsed request head: the method, the target and the raw header lines.
struct Head {
    method: String,
    target: String,
    headers: Vec<String>,
}

fn read_head(client: &mut TcpStream) -> std::io::Result<Option<(Head, Vec<u8>)>> {
    let started = Instant::now();
    let mut raw = Vec::new();
    let mut buf = [0u8; 2048];
    let end = loop {
        // The whole head has HEAD_TIMEOUT, not each read, so a trickle cannot hold a slot.
        let left = HEAD_TIMEOUT.saturating_sub(started.elapsed());
        if left.is_zero() {
            return Ok(None);
        }
        client.set_read_timeout(Some(left))?;
        if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
        if raw.len() > MAX_HEAD_BYTES {
            return Ok(None);
        }
        let n = client.read(&mut buf)?;
        if n == 0 {
            return Ok(None);
        }
        raw.extend_from_slice(&buf[..n]);
    };
    if end > MAX_HEAD_BYTES {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&raw[..end]).into_owned();
    let mut lines = text.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let (Some(method), Some(target), Some(_version)) = (first.next(), first.next(), first.next())
    else {
        return Ok(None);
    };
    let headers = lines
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    let head = Head {
        method: method.to_string(),
        target: target.to_string(),
        headers,
    };
    Ok(Some((head, raw[end..].to_vec())))
}

/// Answers and closes. Unread request bytes are drained first so closing does not
/// reset the connection and discard the answer.
fn refuse(client: &mut TcpStream, status: &str) {
    let _ = client.write_all(format!("HTTP/1.1 {status}\r\nConnection: close\r\n\r\n").as_bytes());
    let _ = client.shutdown(Shutdown::Write);
    let _ = client.set_read_timeout(Some(DRAIN_TIMEOUT));
    let mut sink = [0u8; 4096];
    let mut drained = 0;
    while drained < MAX_HEAD_BYTES * 4 {
        match client.read(&mut sink) {
            Ok(0) | Err(_) => break,
            Ok(n) => drained += n,
        }
    }
}

fn record(log: &Mutex<VecDeque<Decision>>, host: &str, port: u16, verdict: &Verdict) {
    let (allowed, reason) = match verdict {
        Verdict::Allow(addr) => (true, format!("to {addr}")),
        Verdict::Deny(why) => (false, why.clone()),
    };
    let mut log = lock(log);
    if log.len() >= LOG_CAPACITY {
        log.pop_front();
    }
    log.push_back(Decision {
        host: host.to_string(),
        port,
        allowed,
        reason,
    });
}

/// `host` and `port` from a CONNECT target (`host:port`, `[v6]:port`) or an
/// absolute URL's authority. Anything with userinfo, a path or odd characters is
/// refused, so what is checked is what is connected to.
fn split_authority(authority: &str, default_port: Option<u16>) -> Option<(String, u16)> {
    if authority.is_empty()
        || authority.contains(['@', '/', '\\', ' ', '?', '#', '%'])
        || !authority.is_ascii()
    {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.ends_with(']') || h.starts_with('[') => (
            h.trim_matches(['[', ']']).to_string(),
            p.parse::<u16>().ok()?,
        ),
        _ => (
            authority.trim_matches(['[', ']']).to_string(),
            default_port?,
        ),
    };
    (!host.is_empty()).then_some((host, port))
}

fn serve(
    mut client: TcpStream,
    rules: &[Rule],
    log: &Mutex<VecDeque<Decision>>,
    stop: &AtomicBool,
) -> std::io::Result<()> {
    // BSD kernels hand an accepted socket the listener's non-blocking flag.
    client.set_nonblocking(false)?;
    let Some((head, leftover)) = read_head(&mut client)? else {
        refuse(&mut client, "400 Bad Request");
        return Ok(());
    };
    let connect = head.method == "CONNECT";
    let plain = matches!(
        head.method.as_str(),
        "GET" | "POST" | "PUT" | "DELETE" | "HEAD" | "PATCH"
    );
    if !connect && !plain {
        refuse(&mut client, "405 Method Not Allowed");
        return Ok(());
    }
    let (authority, path) = if connect {
        (head.target.clone(), String::new())
    } else {
        match head.target.strip_prefix("http://") {
            Some(rest) => match rest.split_once('/') {
                Some((a, p)) => (a.to_string(), format!("/{p}")),
                None => (rest.to_string(), "/".to_string()),
            },
            None => {
                refuse(&mut client, "400 Bad Request");
                return Ok(());
            }
        }
    };
    let Some((host, port)) = split_authority(&authority, (!connect).then_some(80)) else {
        refuse(&mut client, "400 Bad Request");
        return Ok(());
    };
    let verdict = decide(rules, &host, port, system_resolve);
    record(log, &host, port, &verdict);
    let Verdict::Allow(addr) = verdict else {
        refuse(&mut client, "403 Forbidden");
        return Ok(());
    };
    if connect {
        client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
        let wants_sni = host.parse::<IpAddr>().is_err()
            && rules
                .iter()
                .find(|r| r.matches(&host.to_ascii_lowercase(), port))
                .is_none_or(|r| r.check_sni);
        let first = if wants_sni {
            match first_tls_record(&mut client, leftover) {
                Some(bytes) if sni_of(&bytes).is_some_and(|n| n.eq_ignore_ascii_case(&host)) => {
                    bytes
                }
                _ => {
                    // Overwrites the allow just logged for this tunnel.
                    let why = Verdict::Deny(format!(
                        "{host}: the tunnel does not open with a TLS hello naming it"
                    ));
                    record(log, &host, port, &why);
                    return Ok(());
                }
            }
        } else {
            leftover
        };
        // Only a tunnel that passed the check reaches the upstream.
        let mut upstream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
        upstream.write_all(&first)?;
        tunnel(client, upstream, stop);
        return Ok(());
    }
    let Ok(mut upstream) = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) else {
        refuse(&mut client, "502 Bad Gateway");
        return Ok(());
    };
    // One request per connection: the headers say so, and the connection ends
    // when the upstream's reply does, so a second request cannot name another host.
    let mut req = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
        head.method, path, authority
    );
    for line in &head.headers {
        let name = line.split(':').next().unwrap_or("").to_ascii_lowercase();
        if !matches!(
            name.as_str(),
            "host" | "connection" | "proxy-connection" | "proxy-authorization"
        ) {
            req.push_str(line);
            req.push_str("\r\n");
        }
    }
    req.push_str("\r\n");
    upstream.write_all(req.as_bytes())?;
    upstream.write_all(&leftover)?;
    tunnel(client, upstream, stop);
    Ok(())
}

/// Copies both ways until the upstream closes, goes idle, or the agent's listener
/// is closed. Plain HTTP ends with the upstream's reply (`Connection: close`).
fn tunnel(client: TcpStream, upstream: TcpStream, stop: &AtomicBool) {
    let (Ok(mut c_read), Ok(mut u_read)) = (client.try_clone(), upstream.try_clone()) else {
        return;
    };
    let (mut c_write, mut u_write) = (client, upstream);
    let _ = c_read.set_read_timeout(Some(ACCEPT_POLL * 4));
    let _ = u_read.set_read_timeout(Some(ACCEPT_POLL * 4));
    let mut idle_since = Instant::now();
    let mut buf = vec![0u8; 16 * 1024];
    let mut client_open = true;
    loop {
        if stop.load(Ordering::SeqCst) || idle_since.elapsed() > IDLE_TIMEOUT {
            break;
        }
        let mut moved = false;
        if client_open {
            match c_read.read(&mut buf) {
                Ok(0) => {
                    client_open = false;
                    let _ = u_write.shutdown(Shutdown::Write);
                }
                Ok(n) => {
                    if u_write.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    moved = true;
                }
                Err(e) if is_timeout(&e) => {}
                Err(_) => break,
            }
        }
        match u_read.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if c_write.write_all(&buf[..n]).is_err() {
                    break;
                }
                moved = true;
            }
            Err(e) if is_timeout(&e) => {}
            Err(_) => break,
        }
        if moved {
            idle_since = Instant::now();
        }
    }
    let _ = c_write.shutdown(Shutdown::Both);
}

/// The first TLS record the client sends, complete, or None if it is not a
/// handshake record or does not arrive in time.
fn first_tls_record(client: &mut TcpStream, mut bytes: Vec<u8>) -> Option<Vec<u8>> {
    const HANDSHAKE: u8 = 0x16;
    const RECORD_HEADER: usize = 5;
    let started = Instant::now();
    let mut buf = [0u8; 2048];
    loop {
        if bytes.len() >= RECORD_HEADER {
            if bytes[0] != HANDSHAKE {
                return None;
            }
            let need = RECORD_HEADER + u16::from_be_bytes([bytes[3], bytes[4]]) as usize;
            if need > MAX_HELLO_BYTES {
                return None;
            }
            if bytes.len() >= need {
                return Some(bytes);
            }
        }
        let left = HEAD_TIMEOUT.saturating_sub(started.elapsed());
        if left.is_zero() {
            return None;
        }
        client.set_read_timeout(Some(left)).ok()?;
        match client.read(&mut buf) {
            Ok(0) | Err(_) => return None,
            Ok(n) => bytes.extend_from_slice(&buf[..n]),
        }
    }
}

/// The server name a TLS ClientHello asks for, if it is one and names a host.
fn sni_of(record: &[u8]) -> Option<String> {
    fn take<'a>(data: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
        let (head, rest) = data.split_at_checked(n)?;
        *data = rest;
        Some(head)
    }
    fn take_u16(data: &mut &[u8]) -> Option<usize> {
        let b = take(data, 2)?;
        Some(u16::from_be_bytes([b[0], b[1]]) as usize)
    }
    const CLIENT_HELLO: u8 = 1;
    const SERVER_NAME: usize = 0;
    const HOST_NAME: u8 = 0;
    let mut data = record.get(5..)?;
    if take(&mut data, 1)? != [CLIENT_HELLO] {
        return None;
    }
    take(&mut data, 3 + 2 + 32)?; // length, version, random
    let session = *take(&mut data, 1)?.first()? as usize;
    take(&mut data, session)?;
    let suites = take_u16(&mut data)?;
    take(&mut data, suites)?;
    let compression = *take(&mut data, 1)?.first()? as usize;
    take(&mut data, compression)?;
    let total = take_u16(&mut data)?;
    let mut extensions = take(&mut data, total)?;
    while !extensions.is_empty() {
        let kind = take_u16(&mut extensions)?;
        let len = take_u16(&mut extensions)?;
        let mut body = take(&mut extensions, len)?;
        if kind != SERVER_NAME {
            continue;
        }
        take_u16(&mut body)?;
        if take(&mut body, 1)?.first() != Some(&HOST_NAME) {
            return None;
        }
        let name_len = take_u16(&mut body)?;
        return String::from_utf8(take(&mut body, name_len)?.to_vec()).ok();
    }
    None
}

fn is_timeout(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

#[cfg(test)]
mod tests;
