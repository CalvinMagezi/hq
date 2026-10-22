//! The machine that shows the QR code. Each ticket gets its own throwaway endpoint, made for it
//! and dropped with it, so a stranger who burns a ticket learns nothing about the long-term
//! identity, and the long-term endpoint never speaks the pairing protocol at all. In front of the
//! protocol sit the gate (closed unless a ticket is open, and to sources that used up their
//! strikes) and the per-source limits.

use crate::code::{
    self, Code, DEFAULT_GLOBAL_ATTEMPTS, DEFAULT_TTL, MAX_TTL, SasInputs, Verdict, Window,
    bind_host, new_bind_key, new_code, new_nonce, opens, printable,
};
use crate::limits::{Admission, LimitConfig, Limiter, Source};
use crate::ticket::{MAX_TICKET_ITEMS, Purpose, Ticket};
use crate::wire::{
    Card, Details, Final, Hello, MAX_NAME_CHARS, Outcome, Reply, Reveal, b32, clean_addrs,
    clean_relays, commitment_from, nonce_from, read_line, send_line, unb32,
};
use anyhow::{Result, anyhow, bail};
use iroh::{
    Endpoint, EndpointId, RelayMode, SecretKey, TransportAddr,
    endpoint::{AfterHandshakeOutcome, Connection, EndpointHooks, presets},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Notify, mpsc, oneshot};

const REJECT_CLOSED: u32 = 410;
const REJECT_BLOCKED: u32 = 403;
const REJECT_BUSY: u32 = 429;
const REJECT_SLOW: u32 = 408;
const REJECT_BAD: u32 = 400;
/// How long a pairing connection has to open its stream and finish each step of the exchange.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the person at the host has to type the digits and answer.
pub const DECISION_TIMEOUT: Duration = Duration::from_secs(120);
const ONLINE_WAIT: Duration = Duration::from_secs(5);
const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// A device asking to be paired, waiting for the person at the host. The person types the digits
/// shown on the other device; `decide(true)` is sent only if they match `sas`. The pairing is
/// recorded by the handler once it receives `true` in time: a `send` that fails means it was too
/// late.
pub struct PairRequest {
    pub joiner: EndpointId,
    /// What the other device calls itself. Unchecked text, made printable; never a reason to trust.
    pub name: String,
    pub sas: String,
    pub decide: oneshot::Sender<bool>,
}

/// A device the person accepted, handed to the registrar to be recorded.
#[derive(Debug, Clone)]
pub struct Joiner {
    pub id: EndpointId,
    /// What the device calls itself: unchecked text, made printable.
    pub name: String,
    /// The address the pairing connection came from, when it came directly. The joiner dialed
    /// from its long-term endpoint, so this is a way to dial it back that needs no relay and no
    /// word from the joiner.
    pub observed: Option<SocketAddr>,
}

/// What a purpose does with a device the person accepted. Pairing writes the peer into the
/// database (`PeerRegistrar`); an `hq edge` registers the device under a name.
pub trait Registrar: Send + Sync + 'static {
    /// Records the joiner. The value, if any, is sent back to it. An error becomes "error" for the
    /// joiner, and nothing is recorded.
    fn accept(&self, joiner: &Joiner) -> Result<Option<serde_json::Value>>;

    /// How to dial the joiner back, once it has checked the host's card and sent it. May never be
    /// called (the joiner left first), so nothing may depend on it. Relays and addresses are
    /// already parsed and capped.
    fn details(&self, _joiner: &EndpointId, _relays: &[String], _addrs: &[String]) {}
}

/// The limits and timeouts of one ticket.
#[derive(Debug, Clone)]
pub struct Tuning {
    pub ttl: Duration,
    /// The backstop on wrong codes across every source.
    pub global_attempts: u32,
    pub limits: LimitConfig,
    pub hello_timeout: Duration,
    pub decision_timeout: Duration,
}

impl Tuning {
    /// The longest one exchange may take, whichever of its waits stalls: a slot is never held
    /// longer than this. Seven waits can each use the step timeout (a stream, the hello, the
    /// reply, the reveal, the request to the person, the final answer, the details), then the
    /// person's decision and the two close waits.
    pub fn exchange_budget(&self) -> Duration {
        self.hello_timeout
            .saturating_mul(7)
            .saturating_add(self.decision_timeout)
            .saturating_add(CLOSE_GRACE * 2)
    }
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            ttl: DEFAULT_TTL,
            global_attempts: DEFAULT_GLOBAL_ATTEMPTS,
            limits: LimitConfig::default(),
            hello_timeout: HELLO_TIMEOUT,
            decision_timeout: DECISION_TIMEOUT,
        }
    }
}

pub struct PairingSpec {
    pub purpose: Purpose,
    /// The long-term identity of the machine showing the ticket, sent to the joiner only after the
    /// digits matched.
    pub card: Card,
    pub registrar: Arc<dyn Registrar>,
    /// Where the throwaway endpoint listens. Use port 0.
    pub bind: Vec<SocketAddr>,
    pub relay: RelayMode,
    /// Whether the ticket lists the throwaway endpoint's addresses. Without them it holds only the
    /// id and the relay, which is smaller and shows no IP address; the joiner finds the direct
    /// path through the relay.
    pub with_addrs: bool,
    pub advertise: Vec<SocketAddr>,
    pub tuning: Tuning,
}

/// Counters the tests and the host's screen read.
#[derive(Debug, Default)]
pub struct Stats {
    /// Connections closed at the gate, before any handler ran.
    pub refused: AtomicU64,
    /// Connections that reached the pairing handler.
    pub reached: AtomicU64,
}

struct Shared {
    window: Mutex<Window>,
    limiter: Mutex<Limiter>,
    in_flight: AtomicUsize,
    changed: Notify,
    stats: Arc<Stats>,
}

impl Shared {
    fn window_open(&self) -> bool {
        self.window.lock().unwrap().is_open(Instant::now())
    }

    fn strike(&self, source: &Source) {
        self.limiter.lock().unwrap().strike(source, Instant::now());
    }
}

/// A place in the limiter and in the in-flight count, given back when the exchange ends.
struct Slot {
    shared: Arc<Shared>,
    source: Source,
}

impl Slot {
    fn take(shared: &Arc<Shared>, source: Source) -> Result<Slot, Admission> {
        let admission = shared
            .limiter
            .lock()
            .unwrap()
            .admit(&source, Instant::now());
        if admission != Admission::Allowed {
            return Err(admission);
        }
        shared.in_flight.fetch_add(1, Ordering::SeqCst);
        Ok(Slot {
            shared: shared.clone(),
            source,
        })
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.shared.limiter.lock().unwrap().release(&self.source);
        self.shared.in_flight.fetch_sub(1, Ordering::SeqCst);
        self.shared.changed.notify_one();
    }
}

/// The source of a connection: the address of the path it arrived on.
pub(crate) fn source_of(conn: &Connection) -> Source {
    let paths = conn.paths();
    let path = paths
        .iter()
        .find(|p| p.is_selected())
        .or_else(|| paths.iter().next());
    let Some(path) = path else {
        return Source::Other;
    };
    match path.remote_addr() {
        TransportAddr::Ip(addr) => Source::from_socket(*addr),
        TransportAddr::Relay(url) => Source::Relay(url.to_string()),
        _ => Source::Other,
    }
}

/// The socket address of the direct path a connection arrived on, if it has one.
fn observed_addr(conn: &Connection) -> Option<SocketAddr> {
    let paths = conn.paths();
    let direct = |selected_only: bool| {
        paths.iter().find_map(|p| match p.remote_addr() {
            TransportAddr::Ip(addr) if p.is_selected() || !selected_only => Some(*addr),
            _ => None,
        })
    };
    direct(true).or_else(|| direct(false))
}

#[derive(Debug, Clone)]
struct PairGate {
    purpose: Purpose,
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Shared")
    }
}

impl EndpointHooks for PairGate {
    async fn after_handshake(&self, conn: &Connection) -> AfterHandshakeOutcome {
        if conn.side().is_client() {
            return AfterHandshakeOutcome::accept();
        }
        let reject = |code: u32, reason: &[u8]| {
            self.shared.stats.refused.fetch_add(1, Ordering::SeqCst);
            AfterHandshakeOutcome::Reject {
                error_code: code.into(),
                reason: reason.to_vec(),
            }
        };
        if Purpose::from_alpn(conn.alpn()).map(|(p, _)| p) != Some(self.purpose) {
            return reject(REJECT_BAD, b"wrong protocol");
        }
        if !self.shared.window_open() {
            return reject(REJECT_CLOSED, b"pairing is closed");
        }
        if self
            .shared
            .limiter
            .lock()
            .unwrap()
            .is_blocked(&source_of(conn))
        {
            return reject(REJECT_BLOCKED, b"too many wrong tries");
        }
        AfterHandshakeOutcome::accept()
    }
}

#[derive(Clone)]
struct PairHandler {
    shared: Arc<Shared>,
    pairing_endpoint: EndpointId,
    host_id: EndpointId,
    card: Card,
    code: Code,
    registrar: Arc<dyn Registrar>,
    requests: mpsc::Sender<PairRequest>,
    tuning: Tuning,
}

impl std::fmt::Debug for PairHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PairHandler")
    }
}

impl PairHandler {
    async fn step<T>(&self, what: &str, f: impl Future<Output = Result<T>>) -> Result<T> {
        match tokio::time::timeout(self.tuning.hello_timeout, f).await {
            Ok(result) => result,
            Err(_) => bail!("timed out waiting for {what}"),
        }
    }

    /// Writes a line. A peer that never reads (a tiny flow-control window) must not hold a slot.
    async fn say<T: serde::Serialize>(
        &self,
        send: &mut iroh::endpoint::SendStream,
        value: &T,
    ) -> Result<()> {
        self.step("the other side to read", send_line(send, value))
            .await
    }

    async fn refuse(
        &self,
        conn: &Connection,
        send: &mut iroh::endpoint::SendStream,
        why: &str,
    ) -> Result<()> {
        let reply = Reply {
            ok: false,
            why: why.into(),
            ..Reply::default()
        };
        self.say(send, &reply).await?;
        send.finish()?;
        let _ = tokio::time::timeout(CLOSE_GRACE, conn.closed()).await;
        Ok(())
    }

    /// Reads the hello. Anything that goes wrong here is a strike against the source: an honest
    /// device opens a stream and speaks at once.
    async fn read_hello(
        &self,
        conn: &Connection,
        source: &Source,
    ) -> Result<(
        iroh::endpoint::SendStream,
        iroh::endpoint::RecvStream,
        Hello,
    )> {
        let result = async {
            let (send, mut recv) = self
                .step("a stream", async { Ok(conn.accept_bi().await?) })
                .await?;
            let line = self.step("the hello", read_line(&mut recv)).await?;
            let hello: Hello = serde_json::from_str(&line)?;
            Ok((send, recv, hello))
        }
        .await;
        if result.is_err() {
            self.shared.strike(source);
        }
        result
    }

    async fn exchange(&self, conn: &Connection, source: &Source) -> Result<()> {
        let joiner = conn.remote_id();
        let alpn = conn.alpn().to_vec();
        let (mut send, mut recv, hello) = self.read_hello(conn, source).await?;
        let version = Purpose::from_alpn(&alpn).map(|(_, v)| v);
        let commitment = commitment_from(&hello.commit);
        if Some(hello.v) != version || commitment.is_none() {
            self.shared.strike(source);
            return self
                .refuse(conn, &mut send, "unsupported pairing version")
                .await;
        }
        let Some(commitment) = commitment else {
            unreachable!("checked above")
        };
        let verdict = {
            let mut window = self.shared.window.lock().unwrap();
            let verdict = window.check(Instant::now(), &unb32(&hello.code));
            if !window.is_open(Instant::now()) {
                self.shared.changed.notify_one();
            }
            verdict
        };
        if verdict != Verdict::Accepted {
            if verdict == Verdict::WrongCode {
                self.shared.strike(source);
            }
            return self.refuse(conn, &mut send, verdict.reason()).await;
        }

        let host_nonce = new_nonce();
        let bind_key = new_bind_key();
        let binding = bind_host(&self.host_id, &host_nonce, &bind_key);
        self.say(
            &mut send,
            &Reply {
                ok: true,
                nonce: b32(&host_nonce),
                bind: b32(&binding),
                ..Reply::default()
            },
        )
        .await?;
        let reveal = match self.read_reveal(&mut recv).await {
            Ok(reveal) => reveal,
            Err(e) => {
                self.shared.strike(source);
                return Err(e);
            }
        };
        let Some(joiner_nonce) = nonce_from(&reveal.nonce).filter(|n| opens(&alpn, &commitment, n))
        else {
            self.shared.strike(source);
            conn.close(REJECT_BAD.into(), b"bad reveal");
            bail!(
                "{} revealed a nonce that does not match its commitment",
                joiner.fmt_short()
            );
        };
        let sas = code::sas(&SasInputs {
            alpn: &alpn,
            pairing_endpoint: &self.pairing_endpoint,
            host_binding: &binding,
            joiner: &joiner,
            code: &self.code,
            joiner_nonce: &joiner_nonce,
            host_nonce: &host_nonce,
        });
        let who = Joiner {
            id: joiner,
            name: printable(&reveal.name, MAX_NAME_CHARS),
            observed: observed_addr(conn),
        };
        let (decide, answer) = oneshot::channel();
        self.step("the host person", async {
            self.requests
                .send(PairRequest {
                    joiner,
                    name: who.name.clone(),
                    sas,
                    decide,
                })
                .await
                .map_err(|_| anyhow!("host is gone"))
        })
        .await?;
        let (result, info) = match tokio::time::timeout(self.tuning.decision_timeout, answer).await
        {
            Ok(Ok(true)) => self.record(conn, who).await,
            Ok(Ok(false)) => (Outcome::Declined, None),
            _ => (Outcome::Timeout, None),
        };
        let accepted = result == Outcome::Accepted;
        let last = Final {
            result,
            card: accepted.then(|| self.card.clone()),
            bind_key: if accepted {
                b32(&bind_key)
            } else {
                String::new()
            },
            info,
        };
        self.say(&mut send, &last).await?;
        send.finish()?;
        if accepted {
            self.take_details(&mut recv, joiner).await;
            conn.close(0u32.into(), b"done");
        } else {
            // Closing now could cut the answer off before the joiner has read it.
            let _ = tokio::time::timeout(CLOSE_GRACE, conn.closed()).await;
        }
        Ok(())
    }

    async fn read_reveal(&self, recv: &mut iroh::endpoint::RecvStream) -> Result<Reveal> {
        let line = self.step("the reveal", read_line(recv)).await?;
        Ok(serde_json::from_str(&line)?)
    }

    /// The joiner's dial-back details, if it sends them in time. Not a strike when it does not:
    /// the device is already recorded and can be dialed by id.
    async fn take_details(&self, recv: &mut iroh::endpoint::RecvStream, joiner: EndpointId) {
        let Ok(line) = self.step("the details", read_line(recv)).await else {
            return;
        };
        let Ok(details) = serde_json::from_str::<Details>(&line) else {
            return;
        };
        let registrar = self.registrar.clone();
        let (relays, addrs) = (clean_relays(details.relays), clean_addrs(details.addrs));
        let _ =
            tokio::task::spawn_blocking(move || registrar.details(&joiner, &relays, &addrs)).await;
    }

    /// Records the device only now, after an answer arrived in time and while the joiner is still
    /// connected, so a late answer or a vanished device leaves things unchanged.
    async fn record(&self, conn: &Connection, who: Joiner) -> (Outcome, Option<serde_json::Value>) {
        if conn.close_reason().is_some() {
            return (Outcome::Gone, None);
        }
        let registrar = self.registrar.clone();
        let id = who.id;
        match tokio::task::spawn_blocking(move || registrar.accept(&who)).await {
            Ok(Ok(info)) => (Outcome::Accepted, info),
            Ok(Err(e)) => {
                tracing::warn!(device = %id.fmt_short(), "could not record a pairing: {e:#}");
                (Outcome::Error, None)
            }
            Err(_) => (Outcome::Error, None),
        }
    }
}

impl ProtocolHandler for PairHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        self.shared.stats.reached.fetch_add(1, Ordering::SeqCst);
        let source = source_of(&conn);
        let _slot = match Slot::take(&self.shared, source.clone()) {
            Ok(slot) => slot,
            Err(admission) => {
                let code = if admission == Admission::Blocked {
                    REJECT_BLOCKED
                } else {
                    REJECT_BUSY
                };
                conn.close(code.into(), b"busy");
                return Ok(());
            }
        };
        let budget = self.tuning.exchange_budget();
        let outcome = match tokio::time::timeout(budget, self.exchange(&conn, &source)).await {
            Ok(outcome) => outcome,
            Err(_) => Err(anyhow!("the exchange took longer than {budget:?}")),
        };
        if let Err(e) = outcome {
            tracing::debug!(
                "[pair] exchange with {} failed: {}",
                conn.remote_id().fmt_short(),
                printable(&format!("{e:#}"), 200)
            );
            conn.close(REJECT_SLOW.into(), b"pairing did not finish");
        }
        Ok(())
    }
}

/// An open ticket: the throwaway endpoint, the ticket that names it, and the requests of devices
/// that passed the code and wait for the person's digits.
pub struct OpenTicket {
    pub ticket: Ticket,
    pub requests: mpsc::Receiver<PairRequest>,
    pub stats: Arc<Stats>,
    pairing_endpoint: Endpoint,
    shared: Arc<Shared>,
    lifecycle: Option<tokio::task::JoinHandle<()>>,
}

impl OpenTicket {
    pub async fn open(spec: PairingSpec) -> Result<Self> {
        let host_id: EndpointId = spec
            .card
            .id
            .parse()
            .map_err(|_| anyhow!("the host card names no machine"))?;
        let code = new_code();
        let stats = Arc::new(Stats::default());
        let mut tuning = spec.tuning.clone();
        tuning.ttl = tuning.ttl.min(MAX_TTL);
        let shared = Arc::new(Shared {
            window: Mutex::new(Window::new(
                code,
                Instant::now(),
                tuning.ttl,
                tuning.global_attempts,
            )),
            limiter: Mutex::new(Limiter::new(tuning.limits)),
            in_flight: AtomicUsize::new(0),
            changed: Notify::new(),
            stats: stats.clone(),
        });
        let gate = PairGate {
            purpose: spec.purpose,
            shared: shared.clone(),
        };
        let mut builder = Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::generate())
            .relay_mode(spec.relay.clone())
            .clear_ip_transports()
            .hooks(gate);
        for addr in &spec.bind {
            builder = builder.bind_addr(*addr)?;
        }
        let endpoint = builder.bind().await?;
        if !matches!(spec.relay, RelayMode::Disabled) {
            let _ = tokio::time::timeout(ONLINE_WAIT, endpoint.online()).await;
        }
        let (tx, requests) = mpsc::channel(1);
        let handler = PairHandler {
            shared: shared.clone(),
            pairing_endpoint: endpoint.id(),
            host_id,
            card: spec.card.clone(),
            code,
            registrar: spec.registrar,
            requests: tx,
            tuning,
        };
        let mut router = Router::builder(endpoint.clone());
        for alpn in spec.purpose.alpns() {
            router = router.accept(alpn, handler.clone());
        }
        let router = router.spawn();

        let ticket = Ticket::new(
            spec.purpose,
            endpoint.id(),
            &code,
            relay_urls(&endpoint),
            ticket_addrs(
                &endpoint,
                spec.with_addrs,
                &spec.advertise,
                MAX_TICKET_ITEMS,
            ),
        );
        let deadline = shared.window.lock().unwrap().deadline();
        let lifecycle = tokio::spawn(lifecycle(shared.clone(), router, deadline));
        Ok(Self {
            ticket,
            requests,
            stats,
            pairing_endpoint: endpoint,
            shared,
            lifecycle: Some(lifecycle),
        })
    }

    /// The throwaway endpoint's id: what the ticket names.
    pub fn pairing_id(&self) -> EndpointId {
        self.pairing_endpoint.id()
    }

    /// Whether the throwaway endpoint has been closed (it refuses everything). Its socket is
    /// released when this ticket is dropped, as the endpoint handle lives here.
    pub fn endpoint_closed(&self) -> bool {
        self.pairing_endpoint.is_closed()
    }

    /// Whether the ticket can still be used.
    pub fn is_open(&self) -> bool {
        self.shared.window_open()
    }

    /// Resolves when the throwaway endpoint is gone: the ticket was used, expired or burned, and
    /// no exchange is still running.
    pub async fn closed(&mut self) {
        if let Some(task) = self.lifecycle.take() {
            let _ = task.await;
        }
    }

    /// Closes the ticket now. An exchange already past the code is allowed to finish.
    pub async fn cancel(mut self) {
        self.shared.window.lock().unwrap().close();
        self.shared.changed.notify_one();
        self.closed().await;
    }
}

impl Drop for OpenTicket {
    fn drop(&mut self) {
        self.shared.window.lock().unwrap().close();
        self.shared.changed.notify_one();
    }
}

fn relay_urls(endpoint: &Endpoint) -> Vec<String> {
    endpoint
        .addr()
        .relay_urls()
        .map(|r| r.to_string())
        .collect()
}

/// The addresses a ticket or card lists, the ones the person named first, then the sockets bound
/// to one address, then what the machine found, with no repeats and no more than `limit` (a
/// machine with many interfaces would otherwise make a ticket the other side refuses).
pub(crate) fn ticket_addrs(
    endpoint: &Endpoint,
    with_addrs: bool,
    advertise: &[SocketAddr],
    limit: usize,
) -> Vec<String> {
    if !with_addrs {
        return Vec::new();
    }
    // A socket bound to one address is reachable at it; one bound to every address is not named
    // by that, so the person gives the address to advertise.
    let mut addrs: Vec<String> = advertise.iter().map(|a| a.to_string()).collect();
    addrs.extend(
        endpoint
            .bound_sockets()
            .iter()
            .filter(|a| !a.ip().is_unspecified())
            .map(|a| a.to_string()),
    );
    addrs.extend(endpoint.addr().ip_addrs().map(|a| a.to_string()));
    first_distinct(addrs, limit)
}

/// The first `limit` entries, in order, without repeats.
fn first_distinct(items: Vec<String>, limit: usize) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.clone()))
        .take(limit)
        .collect()
}

/// Keeps the throwaway endpoint up exactly as long as the ticket can be used: until the window
/// closes (spent, expired or burned), then until the exchanges that already started have ended.
async fn lifecycle(shared: Arc<Shared>, router: Router, deadline: Instant) {
    let deadline = tokio::time::Instant::from_std(deadline);
    loop {
        let changed = shared.changed.notified();
        if !shared.window_open() {
            break;
        }
        tokio::select! {
            _ = changed => {}
            _ = tokio::time::sleep_until(deadline) => break,
        }
    }
    loop {
        let changed = shared.changed.notified();
        if shared.in_flight.load(Ordering::SeqCst) == 0 {
            break;
        }
        changed.await;
    }
    let _ = router.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ticket_never_lists_more_addresses_than_the_other_side_will_read() {
        let many: Vec<String> = (0..20).map(|n| format!("192.0.2.{n}:7000")).collect();
        let kept = first_distinct(many.clone(), MAX_TICKET_ITEMS);
        assert_eq!(kept, many[..MAX_TICKET_ITEMS]);
        let repeated = vec!["a".to_string(), "b".into(), "a".into(), "c".into()];
        assert_eq!(first_distinct(repeated, 10), ["a", "b", "c"]);
    }
}
