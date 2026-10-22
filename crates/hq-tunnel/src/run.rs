//! The loop. One thread, one `poll(2)`: it sleeps until the earliest of the stack's own timer, the
//! next WireGuard deadline, and a descriptor becoming ready, so an idle tunnel wakes about once per
//! keepalive. It owns the UDP socket to the hub, the loopback listeners of the reach map and the
//! control channel from the app; it never listens on anything else.

use crate::config::TunnelConfig;
use crate::error::{Error, Result};
use crate::join::{Endpoint, JoinConfig};
use crate::pipe::{LoopbackConnector, TcpPipe};
use crate::secret::Secret32;
use crate::session::{Decapsulated, Session};
use crate::stack::Stack;
use crate::status::{
    EventLog, MAX_EVENT_LOG_BYTES, STATUS_VERSION, Status, open_fd_count, process_rss_kb, unix_now,
    write_atomic,
};
use crate::throttle::RefusalBudget;
use crate::timers::{self, Inputs};
use std::io::ErrorKind;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, UdpSocket};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// A WireGuard datagram is at most an MTU plus 32 bytes.
const DATAGRAM_BYTES: usize = 2048;
const RESOLVE_RETRY: Duration = Duration::from_secs(5);
const STATUS_EVERY: Duration = Duration::from_secs(5);
const ACCEPT_RETRY: Duration = Duration::from_secs(1);
const CONTROL_LINE_MAX: usize = 256;
/// WireGuard stops using a session this long after its last handshake.
const REJECT_AFTER: Duration = Duration::from_secs(180);
/// An initiation nobody has answered for this long makes the state "down" rather than "connecting".
const DOWN_AFTER: Duration = Duration::from_secs(30);
const DENIED_LOG_EVERY: Duration = Duration::from_secs(10);
/// Datagrams taken in one wake. A flood must not keep the loop from everything else; what is left
/// stays in the socket and the next pass takes it.
const MAX_DATAGRAMS_PER_WAKE: usize = 128;
/// A jump in suspended time this big between two passes means the device slept.
const SUSPEND_GAP_SECS: f64 = 2.0;
/// With no endpoint there is nothing to send to; the resolver's answer or its retry wakes the loop.
const NO_ENDPOINT_WAIT: Duration = Duration::from_secs(30);

pub struct RunOptions {
    pub join: JoinConfig,
    pub config: TunnelConfig,
    pub private_key: Secret32,
    pub status_path: Option<PathBuf>,
    pub events_path: Option<PathBuf>,
    /// Lines from the app: `network-changed`, `quit`. End of input also stops the peer, because
    /// the app that started it is gone.
    pub control_fd: Option<RawFd>,
    /// Where the UDP socket binds. Anything but a test uses `0.0.0.0:0`.
    pub bind: SocketAddr,
    /// Use this address instead of resolving the endpoint (tests).
    pub endpoint_override: Option<SocketAddr>,
    /// Called once with the local UDP address after it is bound (tests).
    pub on_ready: Option<Box<dyn FnOnce(SocketAddr) + Send>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Quit,
    ControlClosed,
}

#[derive(Debug)]
pub struct Summary {
    pub exit: Exit,
    pub wakeups: u64,
    /// Datagrams the session turned away (bad MAC, unknown receiver, a replay).
    pub refused: u64,
    /// How many times reading the socket was stopped for turning away too many datagrams.
    pub read_pauses: u64,
}

#[derive(Debug, PartialEq, Eq)]
enum Control {
    NetworkChanged,
    Quit,
}

/// What the status says about the tunnel. "idle" is a session that has aged out with nothing in
/// flight: a peer with no keepalive that is left alone, which makes a new one when it is needed.
fn connection_state(
    age: Option<Duration>,
    unanswered: Option<Duration>,
    had_session: bool,
) -> &'static str {
    match (age, unanswered) {
        // A rekey that nobody has answered for this long means the hub is gone or does not know
        // this device, however fresh the old session still is.
        (_, Some(waited)) if waited >= DOWN_AFTER => "down",
        (Some(a), _) if a < REJECT_AFTER => "up",
        (_, Some(_)) => "connecting",
        (Some(_), None) => "idle",
        // boringtun forgets a session after 180 s, so an aged-out one reads as no age at all.
        (None, None) if had_session => "idle",
        (None, None) => "connecting",
    }
}

/// A handshake initiation: message type 1, 148 bytes.
fn is_initiation(datagram: &[u8]) -> bool {
    datagram.len() == 148 && datagram.starts_with(&[1, 0, 0, 0])
}

/// A cookie reply (type 3, 64 bytes): the session is under load and has not taken the handshake.
fn is_cookie_reply(datagram: &[u8]) -> bool {
    datagram.len() == 64 && datagram.starts_with(&[3, 0, 0, 0])
}

/// An initiation (148 bytes) or a response (92 bytes): the only datagrams that settle a handshake.
fn is_handshake_message(datagram: &[u8]) -> bool {
    is_initiation(datagram) || (datagram.len() == 92 && datagram.starts_with(&[2, 0, 0, 0]))
}

fn is_endpoint(endpoint: Option<SocketAddr>, from: SocketAddr) -> bool {
    endpoint == Some(from)
}

fn parse_control(line: &str) -> Option<Control> {
    match line.trim() {
        "network-changed" => Some(Control::NetworkChanged),
        "quit" => Some(Control::Quit),
        _ => None,
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn clock(id: libc::clockid_t) -> f64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid timespec and `id` is one of the two fixed clock ids used here.
    unsafe { libc::clock_gettime(id, &mut ts) };
    ts.tv_sec as f64 + ts.tv_nsec as f64 / 1e9
}

/// Seconds the CPU has spent suspended since boot: the boot-time clock counts sleep, the
/// monotonic clock does not.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn suspended_total() -> f64 {
    clock(libc::CLOCK_BOOTTIME) - clock(libc::CLOCK_MONOTONIC)
}

/// Other systems have no equivalent clock pair here; the field reads zero.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn suspended_total() -> f64 {
    0.0
}

struct ReachListener {
    listener: TcpListener,
    entry: usize,
    paused_until: Option<Instant>,
}

/// Resolves the endpoint's name on a thread of its own, so a slow resolver never freezes the loop.
/// The answer comes back over a channel and a byte on `wake_read` makes the loop notice.
struct Resolver {
    tx: mpsc::Sender<(u64, std::io::Result<SocketAddr>)>,
    rx: mpsc::Receiver<(u64, std::io::Result<SocketAddr>)>,
    wake_read: UnixStream,
    wake_write: UnixStream,
    /// Bumped on every request; an answer to an older one is ignored.
    generation: u64,
    in_flight: bool,
}

impl Resolver {
    fn new() -> std::io::Result<Self> {
        let (wake_read, wake_write) = UnixStream::pair()?;
        wake_read.set_nonblocking(true)?;
        wake_write.set_nonblocking(true)?;
        let (tx, rx) = mpsc::channel();
        Ok(Self {
            tx,
            rx,
            wake_read,
            wake_write,
            generation: 0,
            in_flight: false,
        })
    }

    fn start(&mut self, endpoint: Endpoint) {
        self.generation += 1;
        self.in_flight = true;
        let generation = self.generation;
        let tx = self.tx.clone();
        let wake = self.wake_write.try_clone();
        thread::spawn(move || {
            let _ = tx.send((generation, endpoint.resolve()));
            if let Ok(mut wake) = wake {
                let _ = wake.write(&[1]);
            }
        });
    }

    /// Forgets any request still running; its answer will be ignored.
    fn abandon(&mut self) {
        self.generation += 1;
        self.in_flight = false;
    }

    fn fd(&self) -> RawFd {
        self.wake_read.as_raw_fd()
    }

    fn answer(&mut self) -> Option<std::io::Result<SocketAddr>> {
        let mut sink = [0u8; 16];
        while self.wake_read.read(&mut sink).is_ok_and(|n| n > 0) {}
        while let Ok((generation, result)) = self.rx.try_recv() {
            if generation == self.generation {
                self.in_flight = false;
                return Some(result);
            }
        }
        None
    }
}

struct Peer {
    opts: RunOptions,
    resolver: Resolver,
    /// Send an initiation as soon as the endpoint is known. `true` forces one out even if one
    /// went a moment ago, which a change of network wants.
    handshake_when_resolved: Option<bool>,
    refused_total: u64,
    /// Limits how many refusals the loop spends time on; see `throttle`.
    refusals: RefusalBudget,
    refused_last: Option<String>,
    refused_logged: u64,
    refused_logged_at: Instant,
    /// Something the status shows has changed since it was last written.
    status_dirty: bool,
    written_state: &'static str,
    session: Session,
    stack: Stack,
    udp: UdpSocket,
    endpoint: Option<SocketAddr>,
    next_resolve: Instant,
    events: Option<EventLog>,
    started_unix: u64,
    suspended0: f64,
    last_tx: Instant,
    last_rx: Instant,
    last_activity: Instant,
    tx_bytes: u64,
    rx_bytes: u64,
    /// The last state that was written to the event log, so a change is logged once.
    logged_state: &'static str,
    /// When the oldest handshake initiation that nothing has answered was sent.
    unanswered_since: Option<Instant>,
    unanswered: Option<Duration>,
    /// A session has been up at some point, even if boringtun has forgotten it since.
    had_session: bool,
    last_error: Option<String>,
    denied_logged: u64,
    denied_logged_at: Instant,
    wakeups: u64,
}

impl Peer {
    fn log(&self, text: &str) {
        if let Some(log) = &self.events {
            log.append(text);
        }
    }

    /// Starts finding the endpoint. A test can fix the address; otherwise the name is resolved on
    /// the resolver's thread.
    fn request_resolve(&mut self, now: Instant) {
        if self.resolver.in_flight {
            return;
        }
        if let Some(addr) = self.opts.endpoint_override {
            self.apply_resolution(Ok(addr), now);
        } else {
            self.resolver.start(self.opts.join.endpoint.clone());
        }
    }

    fn apply_resolution(&mut self, found: std::io::Result<SocketAddr>, now: Instant) {
        self.status_dirty = true;
        let addr = match found {
            Ok(addr) => addr,
            Err(e) => {
                self.last_error = Some(format!("the endpoint did not resolve: {e}"));
                self.log("endpoint resolution failed");
                self.next_resolve = now + RESOLVE_RETRY;
                return;
            }
        };
        let moved = self.endpoint != Some(addr);
        if let Err(e) = self.udp.connect(addr) {
            self.last_error = Some(format!("could not use the endpoint: {:?}", e.kind()));
            // With no endpoint nothing is sent and nothing is believed, and the lookup is retried.
            self.endpoint = None;
            self.next_resolve = now + RESOLVE_RETRY;
            return;
        }
        // A connected socket takes datagrams from the hub's address and no one else's.
        self.endpoint = Some(addr);
        self.last_error = None;
        if moved {
            self.log(&format!("endpoint resolved to {addr}"));
        }
        let pending = self.handshake_when_resolved.take();
        let force = pending.unwrap_or(true);
        if (moved || pending.is_some())
            && let Some(d) = self.session.handshake(force)
        {
            self.send(&d, now);
        }
    }

    /// Takes a finished resolution, and asks again when the last try failed and the wait is over.
    fn service_resolver(&mut self, now: Instant) {
        if let Some(found) = self.resolver.answer() {
            self.apply_resolution(found, now);
        }
        if self.endpoint.is_none() && now >= self.next_resolve {
            self.request_resolve(now);
        }
    }

    fn send(&mut self, datagram: &[u8], now: Instant) {
        if self.endpoint.is_none() {
            return;
        }
        // An attempt counts even if it failed: the next try is a keepalive away, not a tight loop.
        self.last_tx = now;
        if is_initiation(datagram) && self.unanswered_since.is_none() {
            self.unanswered_since = Some(now);
        }
        match self.udp.send(datagram) {
            Ok(n) => {
                self.tx_bytes += n as u64;
                self.last_error = None;
            }
            // No network right now. The next timer or the next network change tries again.
            Err(e) => self.last_error = Some(format!("send failed: {:?}", e.kind())),
        }
    }

    fn send_all(&mut self, datagrams: Vec<Vec<u8>>, now: Instant) {
        for d in datagrams {
            self.send(&d, now);
        }
    }

    fn receive(&mut self, now: Instant) {
        if self.refusals.paused(now) {
            return;
        }
        let mut buf = [0u8; DATAGRAM_BYTES];
        for _ in 0..MAX_DATAGRAMS_PER_WAKE {
            let (n, from) = match self.udp.recv_from(&mut buf) {
                Ok(got) => got,
                Err(_) => return,
            };
            // The kernel already drops other senders on a connected socket. This check does not
            // depend on that: a socket whose connect failed is open to anyone.
            if !is_endpoint(self.endpoint, from) {
                continue;
            }
            self.rx_bytes += n as u64;
            let mut out = Decapsulated::default();
            self.session
                .decapsulate(Some(from.ip()), &buf[..n], &mut out);
            self.refused_total += out.refused.len() as u64;
            let stop = self.refusals.charge(now, out.refused.len());
            if out.refused.is_empty()
                && is_handshake_message(&buf[..n])
                && !out.replies.iter().any(|r| is_cookie_reply(r))
            {
                self.unanswered_since = None;
            }
            if let Some(reason) = out.refused.pop() {
                self.refused_last = Some(reason);
            }
            // Only a datagram the session accepted counts as the hub being there.
            if !out.replies.is_empty() || !out.packets.is_empty() {
                self.last_rx = now;
            }
            self.send_all(out.replies, now);
            if !out.packets.is_empty() {
                self.last_activity = now;
            }
            for packet in out.packets {
                self.stack.ingress(&packet);
            }
            if stop {
                // The rest of the flood waits in the kernel's buffer, which drops what does not fit.
                self.status_dirty = true;
                return;
            }
        }
    }

    /// The device moved to another network. The old endpoint address stays in use until the name
    /// has been looked up again, so a flaky resolver right after a roam does not leave the peer
    /// with nowhere to send.
    fn network_changed(&mut self, now: Instant) {
        self.status_dirty = true;
        self.log("network changed");
        match UdpSocket::bind(self.opts.bind) {
            Ok(udp) if udp.set_nonblocking(true).is_ok() => self.udp = udp,
            _ => self.log("could not open a new UDP socket"),
        }
        self.resolver.abandon();
        self.unanswered_since = None;
        let reconnected = self
            .endpoint
            .is_some_and(|addr| self.udp.connect(addr).is_ok());
        if reconnected {
            if let Some(d) = self.session.handshake(true) {
                self.send(&d, now);
            }
            self.handshake_when_resolved = None;
        } else {
            // Not connected means not trusted: forget the endpoint until a lookup connects again.
            self.endpoint = None;
            self.handshake_when_resolved = Some(true);
        }
        self.next_resolve = now;
        self.request_resolve(now);
    }

    /// If the device slept since the last pass, the loop's clock stood still while WireGuard's did
    /// not. TCP is left alone on purpose: its timers, like the kernel's, do not count time asleep,
    /// so a connection that was idle across the sleep is not declared dead. The session is
    /// checked at once.
    fn notice_sleep(&mut self, schedule: &mut Schedule, now: Instant) {
        let asleep = suspended_total();
        let gap = asleep - std::mem::replace(&mut schedule.last_suspended, asleep);
        if gap < SUSPEND_GAP_SECS {
            return;
        }
        self.log(&format!("woke after about {} s asleep", gap as u64));
        schedule.next_wg = now;
        self.unanswered_since = None;
        // Forced: a handshake that was in flight when the device went to sleep is stale.
        if let Some(d) = self.session.handshake(true) {
            self.send(&d, now);
        }
    }

    /// Sends what the stack produced, through the tunnel.
    fn forward_egress(&mut self, now: Instant) {
        let datagrams: Vec<Vec<u8>> = self
            .stack
            .take_egress()
            .iter()
            .filter_map(|p| self.session.encapsulate(p))
            .collect();
        self.send_all(datagrams, now);
    }

    /// Runs the protocol timers and works out when they next need to. The timers are cheap, so
    /// they run on every pass: that also keeps boringtun's own clock and its handshake rate
    /// limiter current; only the sleep is scheduled.
    fn run_timers(&mut self, schedule: &mut Schedule, now: Instant) {
        if let Some(d) = self.session.update_timers() {
            self.send(&d, now);
        }
        self.had_session |= self.session.is_established();
        self.unanswered = self
            .unanswered_since
            .map(|since| now.saturating_duration_since(since));
        schedule.next_wg = if self.endpoint.is_none() {
            now + NO_ENDPOINT_WAIT
        } else {
            timers::next_deadline(&Inputs {
                now,
                had_session: self.had_session,
                last_tx: self.last_tx,
                last_rx: self.last_rx,
                last_activity: self.last_activity,
                keepalive: self.keepalive(),
                handshake_unanswered: self.unanswered,
            })
        };
    }

    fn keepalive(&self) -> Option<Duration> {
        self.opts
            .join
            .keepalive_secs
            .map(|s| Duration::from_secs(u64::from(s)))
    }

    fn state(&self) -> &'static str {
        connection_state(
            self.session.handshake_age(),
            self.unanswered,
            self.had_session,
        )
    }

    fn write_status(&mut self, stopped: bool) {
        let state = if stopped { "stopped" } else { self.state() };
        self.written_state = state;
        if state != self.logged_state && !stopped {
            self.logged_state = state;
            self.log(&format!("session {state}"));
        }
        let stats = self.stack.stats;
        let now = Instant::now();
        if stats.denied > self.denied_logged
            && now.duration_since(self.denied_logged_at) >= DENIED_LOG_EVERY
        {
            self.log(&format!("{} connection(s) refused so far", stats.denied));
            self.denied_logged = stats.denied;
            self.denied_logged_at = now;
        }
        if self.refused_total > self.refused_logged
            && now.duration_since(self.refused_logged_at) >= DENIED_LOG_EVERY
        {
            let last = self.refused_last.as_deref().unwrap_or("");
            self.log(&format!(
                "{} datagram(s) refused so far, reading paused {} time(s), last: {last}",
                self.refused_total, self.refusals.pauses
            ));
            self.refused_logged = self.refused_total;
            self.refused_logged_at = now;
        }
        let Some(path) = &self.opts.status_path else {
            return;
        };
        let status = Status {
            version: STATUS_VERSION,
            state,
            address: self.opts.join.address.to_string(),
            endpoint: self.opts.join.endpoint.to_string(),
            started_unix: self.started_unix,
            updated_unix: unix_now(),
            max_age_s: timers::longest_sleep(self.keepalive())
                .max(NO_ENDPOINT_WAIT)
                .as_secs(),
            last_handshake_age_s: self.session.handshake_age().map(|a| a.as_secs()),
            connections: self.stack.connections(),
            accepted: stats.accepted,
            denied: stats.denied,
            dropped: stats.dropped,
            reach_opened: stats.reach_opened,
            udp_tx_bytes: self.tx_bytes,
            udp_rx_bytes: self.rx_bytes,
            suspended_s: (suspended_total() - self.suspended0).max(0.0),
            wakeups: self.wakeups,
            read_pauses: self.refusals.pauses,
            rss_kb: process_rss_kb(),
            open_fds: open_fd_count(),
            error: self.last_error.clone(),
        };
        let _ = write_atomic(path, &status);
    }
}

fn bind_udp(addr: SocketAddr) -> Result<UdpSocket> {
    let udp = UdpSocket::bind(addr)?;
    udp.set_nonblocking(true)?;
    Ok(udp)
}

fn bind_reach(config: &TunnelConfig) -> Result<Vec<ReachListener>> {
    config
        .reach
        .iter()
        .enumerate()
        .map(|(entry, e)| {
            let listener = TcpListener::bind(e.listen)?;
            listener.set_nonblocking(true)?;
            Ok(ReachListener {
                listener,
                entry,
                paused_until: None,
            })
        })
        .collect()
}

pub fn run(mut opts: RunOptions) -> Result<Summary> {
    opts.config.validate(&opts.join)?;
    let now = Instant::now();
    let udp = bind_udp(opts.bind)?;
    let mut listeners = bind_reach(&opts.config)?;
    if let Some(ready) = opts.on_ready.take() {
        ready(udp.local_addr()?);
    }
    // The session keeps the key from here on; the copy in the options is wiped at once.
    let private_key = std::mem::replace(&mut opts.private_key, Secret32::new([0; 32]));
    let session = Session::new(
        &private_key,
        opts.join.hub_key,
        opts.join.preshared_key.as_ref(),
        opts.join.keepalive_secs,
    );
    drop(private_key);
    let stack = Stack::new(&opts.join, &opts.config, now)?;
    let events = opts
        .events_path
        .clone()
        .map(|p| EventLog::new(p, MAX_EVENT_LOG_BYTES));
    let mut peer = Peer {
        resolver: Resolver::new()?,
        handshake_when_resolved: Some(false),
        refused_total: 0,
        refused_last: None,
        refused_logged: 0,
        refused_logged_at: now,
        refusals: RefusalBudget::new(now),
        status_dirty: false,
        written_state: "connecting",
        session,
        stack,
        udp,
        endpoint: None,
        next_resolve: now,
        events,
        started_unix: unix_now(),
        suspended0: suspended_total(),
        last_tx: now,
        last_rx: now,
        last_activity: now,
        tx_bytes: 0,
        rx_bytes: 0,
        logged_state: "",
        unanswered_since: None,
        unanswered: None,
        had_session: false,
        last_error: None,
        denied_logged: 0,
        denied_logged_at: now,
        wakeups: 0,
        opts,
    };
    peer.log("started");
    peer.request_resolve(now);
    let result = event_loop(&mut peer, &mut listeners);
    if let Err(e) = &result {
        peer.last_error = Some(match e {
            crate::error::Error::Io(io) => format!("stopped on an error: {:?}", io.kind()),
            crate::error::Error::Invalid(_) => "stopped on an error".to_string(),
        });
    }
    peer.write_status(true);
    peer.log("stopped");
    let exit = result?;
    Ok(Summary {
        exit,
        wakeups: peer.wakeups,
        refused: peer.refused_total,
        read_pauses: peer.refusals.pauses,
    })
}

fn accept_reach(peer: &mut Peer, listeners: &mut [ReachListener], now: Instant) {
    for l in listeners.iter_mut() {
        if l.paused_until.is_some_and(|t| now < t) {
            continue;
        }
        l.paused_until = None;
        while peer.stack.can_open_reach(l.entry) {
            match l.listener.accept() {
                Ok((stream, _)) => {
                    let Ok(pipe) = TcpPipe::new(stream) else {
                        continue;
                    };
                    if peer.stack.open_reach(l.entry, Box::new(pipe), now).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                // The caller went away between the handshake and the accept: ask again.
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::Interrupted | ErrorKind::ConnectionAborted
                    ) => {}
                Err(_) => {
                    // Out of descriptors or similar: stop asking for a moment rather than spin.
                    l.paused_until = Some(now + ACCEPT_RETRY);
                    break;
                }
            }
        }
    }
}

/// Lines from the app, one command each. A line longer than any command is skipped to its end, so
/// the tail of an overlong line is never taken for a command of its own.
#[derive(Default)]
struct ControlReader {
    pending: Vec<u8>,
    skipping: bool,
}

impl ControlReader {
    fn read(&mut self, fd: RawFd) -> std::io::Result<Vec<Control>> {
        let mut buf = [0u8; CONTROL_LINE_MAX];
        // SAFETY: `buf` is valid for `buf.len()` bytes; the descriptor is the caller's open pipe.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            return match e.kind() {
                ErrorKind::Interrupted | ErrorKind::WouldBlock => Ok(vec![]),
                _ => Err(e),
            };
        }
        if n == 0 {
            return Err(ErrorKind::UnexpectedEof.into());
        }
        Ok(self.feed(&buf[..n as usize]))
    }

    fn feed(&mut self, bytes: &[u8]) -> Vec<Control> {
        self.pending.extend_from_slice(bytes);
        let mut found = vec![];
        while let Some(at) = self.pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=at).collect();
            if std::mem::take(&mut self.skipping) {
                continue;
            }
            found.extend(parse_control(&String::from_utf8_lossy(&line)));
        }
        if self.pending.len() >= CONTROL_LINE_MAX {
            self.pending.clear();
            self.skipping = true;
        }
        found
    }
}

/// What the loop remembers between passes.
struct Schedule {
    next_wg: Instant,
    next_status: Instant,
    last_suspended: f64,
}

/// The descriptors to wait on, and which of them is the control channel.
fn wait_set(peer: &Peer, listeners: &[ReachListener]) -> (Vec<libc::pollfd>, Option<usize>) {
    let watch = |fd, events| libc::pollfd {
        fd,
        events,
        revents: 0,
    };
    // A socket that is being left alone stays out of the set: it would report data forever.
    let mut fds = Vec::new();
    if !peer.refusals.paused(Instant::now()) {
        fds.push(watch(peer.udp.as_raw_fd(), libc::POLLIN));
    }
    if peer.resolver.in_flight {
        fds.push(watch(peer.resolver.fd(), libc::POLLIN));
    }
    let control_slot = peer.opts.control_fd.map(|fd| {
        fds.push(watch(fd, libc::POLLIN));
        fds.len() - 1
    });
    let open = |l: &&ReachListener| l.paused_until.is_none() && peer.stack.can_open_reach(l.entry);
    for l in listeners.iter().filter(open) {
        fds.push(watch(l.listener.as_raw_fd(), libc::POLLIN));
    }
    for i in peer.stack.interests() {
        let read = if i.read { libc::POLLIN } else { 0 };
        let write = if i.write { libc::POLLOUT } else { 0 };
        fds.push(watch(i.fd, read | write));
    }
    (fds, control_slot)
}

/// How long the loop may sleep: until the earliest thing that needs it.
fn sleep_for(peer: &mut Peer, schedule: &Schedule, listeners: &[ReachListener]) -> Duration {
    let now = Instant::now();
    let mut due = vec![schedule.next_wg.saturating_duration_since(now)];
    due.extend(peer.stack.poll_delay(now));
    due.extend(peer.refusals.resumes_in(now));
    if peer.endpoint.is_none() && !peer.resolver.in_flight {
        due.push(peer.next_resolve.saturating_duration_since(now));
    }
    due.extend(
        listeners
            .iter()
            .filter_map(|l| l.paused_until)
            .map(|t| t.saturating_duration_since(now)),
    );
    due.into_iter().min().unwrap_or(NO_ENDPOINT_WAIT)
}

fn event_loop(peer: &mut Peer, listeners: &mut [ReachListener]) -> Result<Exit> {
    let mut connector = LoopbackConnector;
    let start = Instant::now();
    let mut schedule = Schedule {
        next_wg: start,
        next_status: start,
        last_suspended: suspended_total(),
    };
    let mut control = ControlReader::default();
    loop {
        let now = Instant::now();
        peer.notice_sleep(&mut schedule, now);
        peer.service_resolver(now);
        accept_reach(peer, listeners, now);
        peer.stack.poll(now, &mut connector);
        peer.forward_egress(now);
        peer.run_timers(&mut schedule, now);
        // Status is written when the loop is awake anyway, never from a timer of its own: on
        // a change that has just happened, or when it has not been written for a while.
        if peer.state() != peer.written_state {
            peer.status_dirty = true;
        }
        if now >= schedule.next_status || peer.status_dirty {
            peer.write_status(false);
            peer.status_dirty = false;
            schedule.next_status = now + STATUS_EVERY;
        }

        let timeout = sleep_for(peer, &schedule, listeners);
        let (mut fds, control_slot) = wait_set(peer, listeners);
        let wait_ms = if timeout.is_zero() {
            0
        } else {
            timeout.as_millis().clamp(1, i32::MAX as u128) as i32
        };
        // SAFETY: `fds` is a valid array of `fds.len()` pollfd structs for the duration of the call.
        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, wait_ms) };
        peer.wakeups += 1;

        peer.receive(Instant::now());
        let control_ready = control_slot.is_some_and(|slot| fds[slot].revents != 0);
        if control_ready
            && let Some(fd) = peer.opts.control_fd
            && let Some(exit) = handle_control(peer, fd, &mut control)?
        {
            return Ok(exit);
        }
    }
}

/// Acts on what the app said. `Some` means the peer should stop.
fn handle_control(peer: &mut Peer, fd: RawFd, reader: &mut ControlReader) -> Result<Option<Exit>> {
    match reader.read(fd) {
        Ok(commands) => {
            // Several reports in one read are one change.
            let mut changed = false;
            for command in commands {
                match command {
                    Control::Quit => return Ok(Some(Exit::Quit)),
                    Control::NetworkChanged => changed = true,
                }
            }
            if changed {
                peer.network_changed(Instant::now());
            }
            Ok(None)
        }
        Err(e) if e.kind() == ErrorKind::UnexpectedEof => Ok(Some(Exit::ControlClosed)),
        Err(e) => Err(Error::Io(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_follows_the_session_and_the_handshake_in_flight() {
        let s = Duration::from_secs;
        assert_eq!(connection_state(None, None, false), "connecting");
        assert_eq!(connection_state(Some(s(5)), None, true), "up");
        assert_eq!(connection_state(Some(s(179)), Some(s(29)), true), "up");
        assert_eq!(connection_state(Some(s(120)), Some(s(30)), true), "down");
        assert_eq!(connection_state(Some(s(179)), Some(s(100)), true), "down");
        assert_eq!(connection_state(None, Some(s(29)), false), "connecting");
        assert_eq!(connection_state(None, Some(s(30)), false), "down");
        assert_eq!(connection_state(None, Some(s(5)), true), "connecting");
        assert_eq!(connection_state(None, Some(s(600)), true), "down");
        // boringtun reports no age once a session is over 180 s old.
        assert_eq!(connection_state(None, None, true), "idle");
        assert_eq!(connection_state(Some(s(200)), None, true), "idle");
    }

    #[test]
    fn only_initiations_and_responses_settle_a_handshake() {
        let mut initiation = vec![0u8; 148];
        initiation[0] = 1;
        let mut response = vec![0u8; 92];
        response[0] = 2;
        let mut data = vec![0u8; 148];
        data[0] = 4;
        assert!(is_initiation(&initiation) && is_handshake_message(&initiation));
        assert!(!is_initiation(&response) && is_handshake_message(&response));
        assert!(!is_initiation(&data) && !is_handshake_message(&data));
        assert!(!is_initiation(&[1, 0, 0, 0]) && !is_handshake_message(&[]));
        let mut cookie = vec![0u8; 64];
        cookie[0] = 3;
        assert!(is_cookie_reply(&cookie) && !is_handshake_message(&cookie));
    }

    #[test]
    fn only_the_resolved_endpoint_is_believed() {
        let hub: SocketAddr = "192.0.2.1:51820".parse().unwrap();
        let other_port: SocketAddr = "192.0.2.1:51821".parse().unwrap();
        let other_host: SocketAddr = "192.0.2.2:51820".parse().unwrap();
        assert!(is_endpoint(Some(hub), hub));
        assert!(!is_endpoint(Some(hub), other_port));
        assert!(!is_endpoint(Some(hub), other_host));
        assert!(
            !is_endpoint(None, hub),
            "nobody is believed before a lookup"
        );
    }

    #[test]
    fn only_the_two_known_commands_are_understood() {
        assert_eq!(
            parse_control("network-changed"),
            Some(Control::NetworkChanged)
        );
        assert_eq!(parse_control("quit\r"), Some(Control::Quit));
        for other in [
            "",
            "QUIT",
            "keepalive 5",
            "quit now",
            "sh -c true",
            "network-changed please",
        ] {
            assert_eq!(parse_control(other), None, "{other}");
        }
    }

    #[test]
    fn the_tail_of_an_overlong_line_is_not_a_command() {
        let mut reader = ControlReader::default();
        assert!(reader.feed(&[b'x'; CONTROL_LINE_MAX]).is_empty());
        assert!(
            reader.feed(b"xxxx quit\n").is_empty(),
            "skipped to the end of the long line"
        );
        assert_eq!(
            reader.feed(b"quit\n"),
            vec![Control::Quit],
            "the next line is read normally"
        );
        // Commands split across reads, and several in one read.
        assert!(reader.feed(b"network-").is_empty());
        assert_eq!(
            reader.feed(b"changed\nquit\n"),
            vec![Control::NetworkChanged, Control::Quit]
        );
    }
}
