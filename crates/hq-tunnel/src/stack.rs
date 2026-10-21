//! The TCP side: a smoltcp interface with this device's tunnel address, the serve map (a port on
//! that address handed to a loopback port, for named tunnel sources only) and the reach map (a
//! loopback connection opened towards a fixed tunnel destination). Packets go in and come out as
//! byte vectors; there is no socket and no clock in here except the `Instant` the caller passes,
//! so the whole thing runs in a test with two stacks facing each other.

use crate::config::TunnelConfig;
use crate::error::{Result, invalid};
use crate::join::JoinConfig;
use crate::packet::{self, PROTO_TCP};
use crate::pipe::{Connector, Pipe};
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::socket::tcp;
use smoltcp::time::{Duration as SDuration, Instant as SInstant};
use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr, Ipv4Address};
use std::collections::{HashSet, VecDeque};
use std::hash::{BuildHasher, Hasher};
use std::net::{Ipv4Addr, SocketAddr};
use std::os::fd::RawFd;
use std::time::{Duration, Instant};

/// WireGuard's own overhead is 80 bytes on IPv4; this is the inner MTU `hq net` plans for.
const MTU: usize = 1420;
/// A prefix wide enough to hold a default plane; everything else goes via the hub.
const ON_LINK_PREFIX: u8 = 24;
/// Packets waiting for the stack. More than this are dropped: the sender will retransmit.
const MAX_QUEUED_PACKETS: usize = 1024;
const FIRST_EPHEMERAL_PORT: u16 = 49_152;
const LAST_EPHEMERAL_PORT: u16 = 65_535;
const KEEP_ALIVE: SDuration = SDuration::from_secs(60);
const USER_TIMEOUT: SDuration = SDuration::from_secs(180);
/// A serve connection that has not been handed to its target by now is torn down: a peer that
/// opens a connection and goes quiet must not hold a slot.
const ATTACH_TIMEOUT: Duration = Duration::from_secs(10);
/// Connections handed to their targets in one poll. A connect to loopback is quick, but a wedged
/// target costs the connector's timeout, and the loop must not stand still for 32 of them.
const ATTACH_PER_POLL: usize = 4;
/// All entries together, whatever their own caps say.
const MAX_TOTAL_CONNECTIONS: usize = 256;
/// A finished socket stays this long so smoltcp can send the last acknowledgement it owes (its
/// delayed ACK is 10 ms) before the socket is dropped and the peer's retransmissions meet a reset.
const CLOSE_LINGER: Duration = Duration::from_millis(200);
/// A finished socket whose target has not yet taken all the bytes the tunnel delivered waits this
/// long for it.
const FLUSH_LINGER: Duration = Duration::from_secs(10);

struct Queues {
    rx: VecDeque<Vec<u8>>,
    tx: VecDeque<Vec<u8>>,
}
struct Rx(Vec<u8>);
struct Tx<'a>(&'a mut VecDeque<Vec<u8>>);
impl RxToken for Rx {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        f(&self.0)
    }
}
impl TxToken for Tx<'_> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut buf = vec![0; len];
        let out = f(&mut buf);
        self.0.push_back(buf);
        out
    }
}
impl Device for Queues {
    type RxToken<'a> = Rx;
    type TxToken<'a> = Tx<'a>;
    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ip;
        caps.max_transmission_unit = MTU;
        caps
    }
    fn receive(&mut self, _now: SInstant) -> Option<(Rx, Tx<'_>)> {
        let packet = self.rx.pop_front()?;
        Some((Rx(packet), Tx(&mut self.tx)))
    }
    fn transmit(&mut self, _now: SInstant) -> Option<Tx<'_>> {
        Some(Tx(&mut self.tx))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    Serve(usize),
    Reach(usize),
}

struct Conn {
    handle: SocketHandle,
    origin: Origin,
    pipe: Option<Box<dyn Pipe>>,
    /// Bytes read from the tunnel that the pipe has not taken yet.
    to_pipe: Vec<u8>,
    pipe_eof: bool,
    pipe_shut: bool,
    opened: Instant,
    last_moved: Instant,
    /// When the socket was first seen finished (closed or in TIME-WAIT).
    closed_since: Option<Instant>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct StackStats {
    /// Serve connections handed to their target.
    pub accepted: u64,
    /// Serve connections from a source the entry does not allow, that were never handed to their
    /// target in time, or whose target refused.
    pub denied: u64,
    /// Packets dropped before the stack: wrong source, wrong destination, not plain TCP over IPv4.
    pub dropped: u64,
    pub reach_opened: u64,
}

/// A descriptor the caller should wait on for this connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interest {
    pub fd: RawFd,
    pub read: bool,
    pub write: bool,
}

struct ServeState {
    port: u16,
    target: SocketAddr,
    from: Vec<Ipv4Addr>,
}

struct ReachState {
    to: (Ipv4Addr, u16),
}

pub struct Stack {
    iface: Interface,
    queues: Queues,
    sockets: SocketSet<'static>,
    epoch: Instant,
    own: Ipv4Addr,
    serve: Vec<ServeState>,
    reach: Vec<ReachState>,
    /// Where replies to a reach connection come from. A packet for a serve port is checked against
    /// that entry's own list instead.
    reach_sources: HashSet<Ipv4Addr>,
    idle: Vec<(SocketHandle, usize)>,
    conns: Vec<Conn>,
    buffer: usize,
    idle_listeners: usize,
    max_connections: usize,
    reach_idle: Duration,
    next_port: u16,
    /// Set when `poll` left an established serve connection for the next one.
    attach_waiting: bool,
    pub stats: StackStats,
}

fn socket(buffer: usize) -> tcp::Socket<'static> {
    let mut s = tcp::Socket::new(
        tcp::SocketBuffer::new(vec![0; buffer]),
        tcp::SocketBuffer::new(vec![0; buffer]),
    );
    s.set_keep_alive(Some(KEEP_ALIVE));
    s.set_timeout(Some(USER_TIMEOUT));
    s
}

fn smol_addr(a: Ipv4Addr) -> Ipv4Address {
    let o = a.octets();
    Ipv4Address::new(o[0], o[1], o[2], o[3])
}

impl Stack {
    pub fn new(join: &JoinConfig, config: &TunnelConfig, now: Instant) -> Result<Self> {
        config.validate(join)?;
        let mut queues = Queues {
            rx: VecDeque::new(),
            tx: VecDeque::new(),
        };
        let mut iface_config = Config::new(HardwareAddress::Ip);
        // Initial sequence numbers and the first local port come from the operating system's
        // randomness, so a peer cannot guess them from the addresses.
        let seed = std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish();
        iface_config.random_seed = seed;
        let mut iface = Interface::new(iface_config, &mut queues, SInstant::from_micros(0));
        iface.update_ip_addrs(|addrs| {
            let _ = addrs.push(IpCidr::new(
                IpAddress::Ipv4(smol_addr(join.address)),
                ON_LINK_PREFIX,
            ));
        });
        if iface
            .routes_mut()
            .add_default_ipv4_route(smol_addr(join.hub_address))
            .is_err()
        {
            return invalid("the stack could not take a route to the hub");
        }
        let mut reach_sources: HashSet<Ipv4Addr> = HashSet::new();
        let serve: Vec<ServeState> = config
            .serve
            .iter()
            .map(|e| ServeState {
                port: e.port,
                target: e.to,
                from: e.from.clone(),
            })
            .collect();
        let reach: Vec<ReachState> = config
            .reach
            .iter()
            .map(|e| {
                reach_sources.insert(*e.to.ip());
                ReachState {
                    to: (*e.to.ip(), e.to.port()),
                }
            })
            .collect();
        Ok(Self {
            iface,
            queues,
            sockets: SocketSet::new(vec![]),
            epoch: now,
            own: join.address,
            serve,
            reach,
            reach_sources,
            idle: vec![],
            conns: vec![],
            buffer: config.tcp_buffer_kb * 1024,
            idle_listeners: config.idle_listeners,
            max_connections: config.max_connections,
            reach_idle: Duration::from_secs(config.reach_idle_secs),
            next_port: FIRST_EPHEMERAL_PORT
                + (seed % u64::from(LAST_EPHEMERAL_PORT - FIRST_EPHEMERAL_PORT)) as u16,
            attach_waiting: false,
            stats: StackStats::default(),
        })
    }

    fn smol(&self, now: Instant) -> SInstant {
        SInstant::from_micros(now.saturating_duration_since(self.epoch).as_micros() as i64)
    }

    /// An IP packet from the hub. Returns whether the stack took it. Only plain TCP over IPv4,
    /// addressed to this device, gets in, and only from a source that may speak to the port it is
    /// going to: the entry's own list for a serve port, a reach destination for anything else.
    /// Doing this before the stack means an unlisted source cannot even start a connection that
    /// holds one of the entry's slots.
    pub fn ingress(&mut self, packet: &[u8]) -> bool {
        let allowed = packet::inspect(packet).is_some_and(|m| {
            m.protocol == PROTO_TCP
                && m.dst == self.own
                && m.tcp_dst_port
                    .is_some_and(|port| self.source_may_reach(m.src, port))
                && !self.repeats_a_syn(&m)
        });
        if !allowed || self.queues.rx.len() >= MAX_QUEUED_PACKETS {
            self.stats.dropped += 1;
            return false;
        }
        self.queues.rx.push_back(packet.to_vec());
        true
    }

    /// A SYN for a connection that already exists is the client retrying. Left to smoltcp, a
    /// listener could take it and start a second half-open connection for the same client; the
    /// original answers with its own SYN-ACK retransmissions instead.
    fn repeats_a_syn(&self, m: &packet::Ipv4Meta) -> bool {
        m.tcp_syn
            && self.conns.iter().any(|c| {
                let s = self.sockets.get::<tcp::Socket>(c.handle);
                let from_there = s.remote_endpoint().is_some_and(|e| {
                    e.addr == IpAddress::Ipv4(smol_addr(m.src)) && Some(e.port) == m.tcp_src_port
                });
                from_there && Some(s.local_endpoint().map_or(0, |e| e.port)) == m.tcp_dst_port
            })
    }

    fn source_may_reach(&self, source: Ipv4Addr, port: u16) -> bool {
        match self.serve.iter().find(|e| e.port == port) {
            Some(entry) => entry.from.contains(&source),
            None => self.reach_sources.contains(&source),
        }
    }

    /// IP packets for the hub, oldest first.
    pub fn take_egress(&mut self) -> Vec<Vec<u8>> {
        self.queues.tx.drain(..).collect()
    }

    pub fn has_queued(&self) -> bool {
        !self.queues.rx.is_empty() || !self.queues.tx.is_empty()
    }

    /// How long the stack can sleep before it needs to run again.
    pub fn poll_delay(&mut self, now: Instant) -> Option<Duration> {
        if self.has_queued() || self.attach_waiting {
            return Some(Duration::ZERO);
        }
        let at = self.smol(now);
        let tcp = self
            .iface
            .poll_delay(at, &self.sockets)
            .map(|d| Duration::from_micros(d.total_micros()));
        match (tcp, self.next_reaper(now)) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// When the stack's own housekeeping next has something to do: a half-open connection to give
    /// up on, a quiet reach connection to close, a finished socket to drop.
    fn next_reaper(&self, now: Instant) -> Option<Duration> {
        self.conns
            .iter()
            .filter_map(|c| {
                let due = match (c.closed_since, c.pipe.is_none(), c.origin) {
                    (Some(since), _, _) => since + linger(c),
                    (None, true, Origin::Serve(_)) => c.opened + ATTACH_TIMEOUT,
                    (None, false, Origin::Reach(_)) => c.last_moved + self.reach_idle,
                    _ => return None,
                };
                Some(due.saturating_duration_since(now))
            })
            .min()
    }

    pub fn connections(&self) -> usize {
        self.conns.len()
    }

    /// Whether another connection may be opened towards this reach entry.
    pub fn can_open_reach(&self, entry: usize) -> bool {
        self.count(Origin::Reach(entry)) < self.max_connections
            && self.conns.len() < MAX_TOTAL_CONNECTIONS
    }

    fn count(&self, origin: Origin) -> usize {
        self.conns.iter().filter(|c| c.origin == origin).count()
    }

    /// Descriptors to wait on. A connection that wants nothing is left out: a closed descriptor in
    /// a poll set reports hang-up forever and would keep the loop awake.
    pub fn interests(&self) -> Vec<Interest> {
        self.conns
            .iter()
            .filter_map(|c| {
                let fd = c.pipe.as_ref()?.raw_fd()?;
                let s = self.sockets.get::<tcp::Socket>(c.handle);
                let read = !c.pipe_eof && s.can_send();
                let write = !c.to_pipe.is_empty();
                (read || write).then_some(Interest { fd, read, write })
            })
            .collect()
    }

    /// Opens a connection from `pipe` (an accepted loopback stream) to reach entry `entry`.
    pub fn open_reach(&mut self, entry: usize, pipe: Box<dyn Pipe>, now: Instant) -> Result<()> {
        let Some(state) = self.reach.get(entry) else {
            return invalid("no such reach entry");
        };
        if !self.can_open_reach(entry) {
            return invalid("too many connections");
        }
        let (ip, port) = state.to;
        let local = self.take_port();
        let mut s = socket(self.buffer);
        let remote = (IpAddress::Ipv4(smol_addr(ip)), port);
        if s.connect(self.iface.context(), remote, local).is_err() {
            return invalid("the stack could not start the connection");
        }
        let handle = self.sockets.add(s);
        self.conns.push(Conn {
            handle,
            origin: Origin::Reach(entry),
            pipe: Some(pipe),
            to_pipe: vec![],
            pipe_eof: false,
            pipe_shut: false,
            opened: now,
            last_moved: now,
            closed_since: None,
        });
        self.stats.reach_opened += 1;
        Ok(())
    }

    fn take_port(&mut self) -> u16 {
        loop {
            let port = self.next_port;
            self.next_port = if port == LAST_EPHEMERAL_PORT {
                FIRST_EPHEMERAL_PORT
            } else {
                port + 1
            };
            let used = self.serve.iter().any(|e| e.port == port)
                || self.conns.iter().any(|c| {
                    self.sockets
                        .get::<tcp::Socket>(c.handle)
                        .local_endpoint()
                        .is_some_and(|e| e.port == port)
                });
            if !used {
                return port;
            }
        }
    }

    /// SYNs waiting in the queue for this serve port. A burst of connections must each find a
    /// listener, so the pool is topped up for them before the packets are processed.
    fn waiting_syns(&self, port: u16) -> usize {
        self.queues
            .rx
            .iter()
            .filter_map(|p| packet::inspect(p))
            .filter(|m| m.tcp_syn && m.tcp_dst_port == Some(port))
            .count()
    }

    fn top_up_listeners(&mut self) {
        for entry in 0..self.serve.len() {
            let origin = Origin::Serve(entry);
            let busy = self.count(origin);
            let want = self
                .idle_listeners
                .max(self.waiting_syns(self.serve[entry].port));
            loop {
                let idle = self.idle.iter().filter(|(_, e)| *e == entry).count();
                let room = idle + busy < self.max_connections
                    && self.idle.len() + self.conns.len() < MAX_TOTAL_CONNECTIONS;
                if idle >= want || !room {
                    break;
                }
                let mut s = socket(self.buffer);
                if s.listen(self.serve[entry].port).is_err() {
                    break;
                }
                self.idle.push((self.sockets.add(s), entry));
            }
        }
    }

    fn promote(&mut self, now: Instant) {
        let mut i = 0;
        while i < self.idle.len() {
            let (handle, entry) = self.idle[i];
            // Anything that left Listen: a connection, or one that came and went inside one poll.
            if self.sockets.get::<tcp::Socket>(handle).state() != tcp::State::Listen {
                self.idle.swap_remove(i);
                self.conns.push(Conn {
                    handle,
                    origin: Origin::Serve(entry),
                    pipe: None,
                    to_pipe: vec![],
                    pipe_eof: false,
                    pipe_shut: false,
                    opened: now,
                    last_moved: now,
                    closed_since: None,
                });
            } else {
                i += 1;
            }
        }
    }

    /// Runs the stack: takes queued packets, advances timers, moves bytes between each tunnel
    /// connection and its pipe, and queues the packets that result.
    pub fn poll(&mut self, now: Instant, connector: &mut dyn Connector) {
        self.top_up_listeners();
        let at = self.smol(now);
        self.iface.poll(at, &mut self.queues, &mut self.sockets);
        self.promote(now);
        let mut dead = vec![];
        let mut budget = ATTACH_PER_POLL;
        self.attach_waiting = false;
        for index in 0..self.conns.len() {
            if self.step(index, now, connector, &mut budget) {
                dead.push(index);
            }
        }
        // The last poll goes out before a dead socket is removed, so an abort still sends its reset.
        self.iface.poll(at, &mut self.queues, &mut self.sockets);
        for index in dead.into_iter().rev() {
            let conn = self.conns.swap_remove(index);
            self.sockets.remove(conn.handle);
        }
    }

    /// One connection's turn. Returns whether it is finished and can be dropped.
    fn step(
        &mut self,
        index: usize,
        now: Instant,
        connector: &mut dyn Connector,
        budget: &mut usize,
    ) -> bool {
        let handle = self.conns[index].handle;
        if self.conns[index].pipe.is_none() && !self.attach(index, now, connector, budget) {
            let conn = &mut self.conns[index];
            return finished(self.sockets.get::<tcp::Socket>(handle), conn, now);
        }
        let reach_idle = self.reach_idle;
        let conn = &mut self.conns[index];
        let s = self.sockets.get_mut::<tcp::Socket>(handle);
        let moved = pump(s, conn);
        if moved {
            conn.last_moved = now;
        } else if matches!(conn.origin, Origin::Reach(_))
            && now.duration_since(conn.last_moved) > reach_idle
        {
            s.abort();
        }
        finished(s, conn, now)
    }

    /// Gives a new serve connection its pipe once it is established, after the fence. Returns
    /// whether the connection now has one.
    fn attach(
        &mut self,
        index: usize,
        now: Instant,
        connector: &mut dyn Connector,
        budget: &mut usize,
    ) -> bool {
        let Origin::Serve(entry) = self.conns[index].origin else {
            return false;
        };
        let handle = self.conns[index].handle;
        let s = self.sockets.get_mut::<tcp::Socket>(handle);
        // A peer that sends its data and closes right behind the handshake is already in
        // CloseWait by the time it is looked at; it is still a customer.
        let ready = matches!(s.state(), tcp::State::Established | tcp::State::CloseWait);
        if !ready {
            if now.duration_since(self.conns[index].opened) > ATTACH_TIMEOUT {
                self.stats.denied += 1;
                s.abort();
            }
            return false;
        }
        if *budget == 0 {
            self.attach_waiting = true;
            return false;
        }
        *budget -= 1;
        let source = match s.remote_endpoint().map(|e| e.addr) {
            Some(IpAddress::Ipv4(a)) => Some(Ipv4Addr::from(a.octets())),
            _ => None,
        };
        let allowed = source.is_some_and(|a| self.serve[entry].from.contains(&a));
        let s = self.sockets.get_mut::<tcp::Socket>(handle);
        if !allowed {
            self.stats.denied += 1;
            s.abort();
            return false;
        }
        match connector.connect(self.serve[entry].target) {
            Ok(pipe) => {
                self.conns[index].pipe = Some(pipe);
                self.stats.accepted += 1;
                true
            }
            Err(_) => {
                self.stats.denied += 1;
                s.abort();
                false
            }
        }
    }
}

/// Whether a connection can be dropped. A finished socket lingers briefly so its last
/// acknowledgement goes out, and longer while the target still has bytes to take.
fn finished(s: &tcp::Socket<'_>, c: &mut Conn, now: Instant) -> bool {
    if s.is_open() {
        c.closed_since = None;
        return false;
    }
    let since = *c.closed_since.get_or_insert(now);
    let wait = if s.state() == tcp::State::Closed && c.to_pipe.is_empty() {
        Duration::ZERO
    } else {
        linger(c)
    };
    now.duration_since(since) >= wait
}

/// How long a finished connection is kept. The reaper timer and `finished` must agree, or the
/// loop wakes at the shorter one and finds nothing to do until the longer one passes.
fn linger(c: &Conn) -> Duration {
    if c.to_pipe.is_empty() {
        CLOSE_LINGER
    } else {
        FLUSH_LINGER
    }
}

/// Moves bytes both ways until nothing moves. smoltcp's rings are contiguous slices that wrap, so
/// a single pass can leave data behind and the window would never reopen.
fn pump(s: &mut tcp::Socket<'_>, c: &mut Conn) -> bool {
    let Some(pipe) = c.pipe.as_mut() else {
        return false;
    };
    let mut any = false;
    let mut moved = true;
    while moved {
        moved = false;
        if s.can_recv() && c.to_pipe.len() < s.recv_capacity() {
            let room = s.recv_capacity() - c.to_pipe.len();
            let n = s
                .recv(|data| {
                    let n = data.len().min(room);
                    c.to_pipe.extend_from_slice(&data[..n]);
                    (n, n)
                })
                .unwrap_or(0);
            moved |= n > 0;
        }
        while !c.to_pipe.is_empty() {
            match pipe.write(&c.to_pipe) {
                Ok(n) if n > 0 => {
                    c.to_pipe.drain(..n);
                    moved = true;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                // A pipe that takes nothing and says nothing is as good as broken.
                _ => {
                    c.to_pipe.clear();
                    s.abort();
                    return any;
                }
            }
        }
        if !c.pipe_eof && s.can_send() {
            let n = s
                .send(|buf| match pipe.read(buf) {
                    Ok(n) if n > 0 => (n, n),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => (0, 0),
                    // The end of the stream, or an error that will not clear: either way, no more.
                    _ => {
                        c.pipe_eof = true;
                        (0, 0)
                    }
                })
                .unwrap_or(0);
            moved |= n > 0;
        }
        any |= moved;
    }
    // While connecting, the receive side is not open yet, but it is not finished either: closing
    // the pipe's write side then would hang up on the caller before the first byte.
    let connecting = matches!(
        s.state(),
        tcp::State::SynSent | tcp::State::SynReceived | tcp::State::Listen
    );
    if !connecting && !s.may_recv() && !s.can_recv() && c.to_pipe.is_empty() && !c.pipe_shut {
        pipe.shutdown_write();
        c.pipe_shut = true;
    }
    if c.pipe_eof && s.send_queue() == 0 && s.may_send() {
        s.close();
    }
    any
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::join::parse;
    use crate::pipe::{Pipe, mem::MemConnector};

    const OWN: Ipv4Addr = Ipv4Addr::new(10, 77, 0, 4);
    const HUB: Ipv4Addr = Ipv4Addr::new(10, 77, 0, 1);
    const REACHED: Ipv4Addr = Ipv4Addr::new(10, 77, 0, 9);
    const STRANGER: Ipv4Addr = Ipv4Addr::new(10, 77, 0, 77);
    const SERVE_PORT: u16 = 8443;
    const SYN: u8 = 0x02;

    fn stack(now: Instant) -> Stack {
        let join = parse(&format!(
            "[Interface]\nAddress = {OWN}/32\n\n[Peer]\nPublicKey = {}\nEndpoint = vpn.example.com:51820\nAllowedIPs = {HUB}/32\n",
            hq_net::key::encode(&[7; 32])
        ))
        .unwrap();
        let config = TunnelConfig::from_toml(&format!(
            "[[serve]]\nport = {SERVE_PORT}\nto = \"127.0.0.1:9000\"\nfrom = [\"{HUB}\"]\n\n[[reach]]\nlisten = \"127.0.0.1:47801\"\nto = \"{REACHED}:1\"\n"
        ))
        .unwrap();
        Stack::new(&join, &config, now).unwrap()
    }

    /// The Internet checksum of the chunks, as the headers carry it.
    fn checksum(chunks: &[&[u8]]) -> u16 {
        let mut sum: u32 = 0;
        for chunk in chunks {
            for pair in chunk.chunks(2) {
                sum += u32::from(u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]));
            }
        }
        while sum > 0xffff {
            sum = (sum & 0xffff) + (sum >> 16);
        }
        !(sum as u16)
    }

    /// An IPv4 packet with a TCP header and a payload, checksums included so smoltcp accepts it.
    #[allow(clippy::too_many_arguments)]
    fn segment(
        src: Ipv4Addr,
        dst: Ipv4Addr,
        ports: (u16, u16),
        flags: u8,
        seq: u32,
        ack: u32,
        payload: &[u8],
    ) -> Vec<u8> {
        let total = 40 + payload.len();
        let mut p = vec![0u8; total];
        p[0] = 0x45;
        p[2..4].copy_from_slice(&(total as u16).to_be_bytes());
        p[8] = 64;
        p[9] = PROTO_TCP;
        p[12..16].copy_from_slice(&src.octets());
        p[16..20].copy_from_slice(&dst.octets());
        let ip_sum = checksum(&[&p[..20]]);
        p[10..12].copy_from_slice(&ip_sum.to_be_bytes());
        p[20..22].copy_from_slice(&ports.0.to_be_bytes());
        p[22..24].copy_from_slice(&ports.1.to_be_bytes());
        p[24..28].copy_from_slice(&seq.to_be_bytes());
        p[28..32].copy_from_slice(&ack.to_be_bytes());
        p[32] = 0x50;
        p[33] = flags;
        p[34..36].copy_from_slice(&65_535u16.to_be_bytes());
        p[40..].copy_from_slice(payload);
        let mut pseudo = vec![];
        pseudo.extend_from_slice(&src.octets());
        pseudo.extend_from_slice(&dst.octets());
        pseudo.extend_from_slice(&[0, PROTO_TCP]);
        pseudo.extend_from_slice(&((total - 20) as u16).to_be_bytes());
        let tcp_sum = checksum(&[&pseudo, &p[20..]]);
        p[36..38].copy_from_slice(&tcp_sum.to_be_bytes());
        p
    }

    fn tcp(src: Ipv4Addr, dst: Ipv4Addr, sport: u16, dport: u16, flags: u8) -> Vec<u8> {
        segment(src, dst, (sport, dport), flags, 1000, 0, &[])
    }

    #[test]
    fn a_client_that_sends_and_closes_in_one_batch_behind_the_handshake_is_served() {
        const ACK: u8 = 0x10;
        const PSH: u8 = 0x08;
        const FIN: u8 = 0x01;
        let base = Instant::now();
        let mut s = stack(base);
        let mut connector = MemConnector::default();
        s.poll(base, &mut connector);
        assert!(s.ingress(&tcp(HUB, OWN, 40_000, SERVE_PORT, SYN)));
        s.poll(base + Duration::from_millis(10), &mut connector);
        let synack = s.take_egress().pop().expect("a SYN-ACK");
        let isn = u32::from_be_bytes([synack[24], synack[25], synack[26], synack[27]]);
        // The last step of the handshake, the data and the close arrive together, so the first
        // look at the connection finds it already in CloseWait.
        let closing = segment(
            HUB,
            OWN,
            (40_000, SERVE_PORT),
            ACK | PSH | FIN,
            1001,
            isn + 1,
            b"hi",
        );
        assert!(s.ingress(&closing));
        s.poll(base + Duration::from_millis(20), &mut connector);
        assert_eq!(connector.opened.len(), 1, "handed to its target");
        let (_, far) = &mut connector.opened[0];
        let mut got = [0u8; 8];
        assert_eq!(far.read(&mut got).unwrap(), 2);
        assert_eq!(&got[..2], b"hi");
    }

    #[test]
    fn a_packet_gets_in_only_from_a_source_that_may_speak_to_its_port() {
        let mut s = stack(Instant::now());
        // A serve port: only that entry's list.
        assert!(s.ingress(&tcp(HUB, OWN, 40_000, SERVE_PORT, SYN)));
        assert!(
            !s.ingress(&tcp(REACHED, OWN, 40_000, SERVE_PORT, SYN)),
            "known, but not for this entry"
        );
        assert!(!s.ingress(&tcp(STRANGER, OWN, 40_000, SERVE_PORT, SYN)));
        // Anything else: only a reach destination, whose replies come to an ephemeral port.
        assert!(s.ingress(&tcp(REACHED, OWN, 1, 50_000, SYN)));
        assert!(!s.ingress(&tcp(HUB, OWN, 1, 50_000, SYN)));
        assert!(!s.ingress(&tcp(STRANGER, OWN, 1, 50_000, SYN)));
        assert_eq!(s.stats.dropped, 4);
    }

    #[test]
    fn only_plain_tcp_to_this_device_gets_in() {
        let mut s = stack(Instant::now());
        let good = tcp(HUB, OWN, 40_000, SERVE_PORT, SYN);
        assert!(s.ingress(&good));
        let mut elsewhere = good.clone();
        elsewhere[16..20].copy_from_slice(&[10, 77, 0, 5]);
        let mut udp = good.clone();
        udp[9] = 17;
        let mut fragment = good.clone();
        fragment[6] = 0x20;
        let mut v6 = good.clone();
        v6[0] = 0x65;
        let mut truncated = good.clone();
        truncated.truncate(22);
        for (name, packet) in [
            ("another destination", elsewhere),
            ("not TCP", udp),
            ("a fragment", fragment),
            ("not IPv4", v6),
            ("too short for ports", truncated),
        ] {
            assert!(!s.ingress(&packet), "{name}");
        }
    }

    #[test]
    fn a_syn_from_an_unlisted_source_takes_no_slot() {
        let base = Instant::now();
        let mut s = stack(base);
        let mut connector = MemConnector::default();
        s.poll(base, &mut connector);
        assert!(!s.ingress(&tcp(REACHED, OWN, 40_000, SERVE_PORT, SYN)));
        s.poll(base + Duration::from_millis(10), &mut connector);
        assert_eq!(s.connections(), 0);
        assert!(
            s.take_egress().is_empty(),
            "no answer, so a scan learns nothing"
        );
    }

    #[test]
    fn a_connection_that_never_completes_is_torn_down() {
        let base = Instant::now();
        let mut s = stack(base);
        let mut connector = MemConnector::default();
        s.poll(base, &mut connector);
        assert!(s.ingress(&tcp(HUB, OWN, 40_000, SERVE_PORT, SYN)));
        s.poll(base + Duration::from_millis(10), &mut connector);
        assert_eq!(s.connections(), 1, "half open, holding a slot");
        s.poll(base + Duration::from_secs(5), &mut connector);
        assert_eq!(s.connections(), 1);
        s.poll(
            base + ATTACH_TIMEOUT + Duration::from_secs(1),
            &mut connector,
        );
        s.poll(
            base + ATTACH_TIMEOUT + Duration::from_secs(2),
            &mut connector,
        );
        assert_eq!(s.connections(), 0);
        assert_eq!(s.stats.denied, 1);
        assert!(connector.opened.is_empty());
    }

    const ACK: u8 = 0x10;
    const RST: u8 = 0x04;
    const SYN_ACK: u8 = SYN | ACK;

    fn flags(packet: &[u8]) -> u8 {
        packet[33]
    }

    fn isn_of(synack: &[u8]) -> u32 {
        u32::from_be_bytes([synack[24], synack[25], synack[26], synack[27]])
    }

    fn open_connection(
        s: &mut Stack,
        connector: &mut MemConnector,
        at: Instant,
        sport: u16,
    ) -> u32 {
        assert!(s.ingress(&tcp(HUB, OWN, sport, SERVE_PORT, SYN)));
        s.poll(at, connector);
        let synack = s
            .take_egress()
            .into_iter()
            .find(|p| flags(p) == SYN_ACK)
            .expect("a SYN-ACK");
        isn_of(&synack)
    }

    #[test]
    fn many_abortive_closes_do_not_cost_the_entry_its_listeners() {
        let base = Instant::now();
        let mut s = stack(base);
        let mut connector = MemConnector::default();
        s.poll(base, &mut connector);
        // More abortive closes than the pool holds, each handshake and reset in one batch.
        for n in 0..8u16 {
            let sport = 41_000 + n;
            let t = base + Duration::from_millis(u64::from(n) * 400);
            let isn = open_connection(&mut s, &mut connector, t, sport);
            assert!(s.ingress(&segment(
                HUB,
                OWN,
                (sport, SERVE_PORT),
                ACK,
                1001,
                isn + 1,
                &[]
            )));
            assert!(s.ingress(&segment(
                HUB,
                OWN,
                (sport, SERVE_PORT),
                RST | ACK,
                1001,
                isn + 1,
                &[]
            )));
            s.poll(t + Duration::from_millis(10), &mut connector);
            s.poll(t + Duration::from_millis(300), &mut connector);
        }
        // The entry still answers a fresh connection.
        let t = base + Duration::from_secs(10);
        open_connection(&mut s, &mut connector, t, 42_000);
    }

    #[test]
    fn a_closed_connection_with_bytes_its_target_has_not_taken_does_not_keep_the_loop_awake() {
        let base = Instant::now();
        let mut s = stack(base);
        let mut connector = MemConnector {
            capacity: Some(16),
            ..MemConnector::default()
        };
        s.poll(base, &mut connector);
        let isn = open_connection(&mut s, &mut connector, base, 43_000);
        let payload = [7u8; 200];
        let data = segment(
            HUB,
            OWN,
            (43_000, SERVE_PORT),
            ACK | 0x08,
            1001,
            isn + 1,
            &payload,
        );
        assert!(s.ingress(&data));
        s.poll(base + Duration::from_millis(10), &mut connector);
        // The target took 16 bytes and is not reading; the client then resets.
        let reset = segment(
            HUB,
            OWN,
            (43_000, SERVE_PORT),
            RST | ACK,
            1001 + payload.len() as u32,
            isn + 1,
            &[],
        );
        assert!(s.ingress(&reset));
        s.poll(base + Duration::from_millis(20), &mut connector);
        assert_eq!(
            s.connections(),
            1,
            "kept while bytes are left for the target"
        );
        let later = base + Duration::from_millis(400);
        s.poll(later, &mut connector);
        let delay = s.poll_delay(later).expect("a reaper is due");
        assert!(
            delay > Duration::from_secs(1),
            "the loop would spin for {delay:?}"
        );
    }

    #[test]
    fn a_target_that_takes_a_few_bytes_at_a_time_still_gets_every_byte_in_order() {
        let base = Instant::now();
        let mut s = stack(base);
        let mut connector = MemConnector {
            capacity: Some(16),
            ..MemConnector::default()
        };
        s.poll(base, &mut connector);
        let isn = open_connection(&mut s, &mut connector, base, 44_000);
        let payload: Vec<u8> = (0..=199u8).collect();
        let data = segment(
            HUB,
            OWN,
            (44_000, SERVE_PORT),
            ACK | 0x08,
            1001,
            isn + 1,
            &payload,
        );
        assert!(s.ingress(&data));
        let mut got = vec![];
        for step in 1..200u64 {
            s.poll(base + Duration::from_millis(step * 5), &mut connector);
            let (_, far) = &mut connector.opened[0];
            let mut chunk = [0u8; 64];
            while let Ok(n) = far.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&chunk[..n]);
            }
            if got.len() >= payload.len() {
                break;
            }
        }
        assert_eq!(got, payload);
    }

    #[test]
    fn a_refused_connection_gets_a_reset_not_silence() {
        let base = Instant::now();
        let mut s = stack(base);
        let mut connector = MemConnector {
            refuse: true,
            ..MemConnector::default()
        };
        s.poll(base, &mut connector);
        let isn = open_connection(
            &mut s,
            &mut connector,
            base + Duration::from_millis(10),
            40_000,
        );
        assert!(s.ingress(&segment(
            HUB,
            OWN,
            (40_000, SERVE_PORT),
            ACK,
            1001,
            isn + 1,
            &[]
        )));
        s.poll(base + Duration::from_millis(20), &mut connector);
        let sent = s.take_egress();
        assert!(
            sent.iter().any(|p| flags(p) & RST != 0),
            "the caller is told at once"
        );
        assert_eq!(s.stats.denied, 1);
        assert_eq!(connector.opened.len(), 0);
    }

    #[test]
    fn a_burst_of_connections_larger_than_the_pool_is_still_answered() {
        let base = Instant::now();
        let mut s = stack(base);
        let mut connector = MemConnector::default();
        s.poll(base, &mut connector);
        for n in 0..10u16 {
            assert!(s.ingress(&tcp(HUB, OWN, 43_000 + n, SERVE_PORT, SYN)));
        }
        s.poll(base + Duration::from_millis(10), &mut connector);
        let answers = s
            .take_egress()
            .iter()
            .filter(|p| flags(p) == SYN_ACK)
            .count();
        assert_eq!(answers, 10, "every SYN found a listener");
        assert_eq!(s.connections(), 10);
    }

    #[test]
    fn a_retried_syn_does_not_start_a_second_connection() {
        let base = Instant::now();
        let mut s = stack(base);
        let mut connector = MemConnector::default();
        s.poll(base, &mut connector);
        let syn = tcp(HUB, OWN, 40_000, SERVE_PORT, SYN);
        assert!(s.ingress(&syn));
        s.poll(base + Duration::from_millis(10), &mut connector);
        assert_eq!(s.connections(), 1);
        assert!(
            !s.ingress(&syn),
            "the same client asking again is the first connection's to answer"
        );
        s.poll(base + Duration::from_millis(20), &mut connector);
        assert_eq!(s.connections(), 1);
    }
}
