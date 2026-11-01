//! The control channel of `hq bridge run`: one JSON object per line in, one per line out.
//!
//! The phone app starts `hq bridge run` as a child process and talks to it over its standard input
//! and output. There is no socket and no port, so nothing but the parent can reach it, and the
//! process ends when the parent goes away (end of input). The protocol is documented in
//! `docs/HQ_MOBILE.md`. The loop here takes any reader and writer, so a host that wants it
//! in-process later (the edge stream, workstream W1) can serve it over a channel instead.
//!
//! The six digits of a pairing never leave this process: the person types what the other device
//! shows, the app forwards the typed text, and the comparison happens here. A button that says
//! "yes" cannot be built on this protocol.

use anyhow::{Result, anyhow};
use hq_bridge::{
    Bridge, Connection, EndpointId, JoinResult, Ticket, Tuning, code::printable, echo,
    host::PairRequest, path_kind,
};
use hq_db::bridge as peers;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{mpsc, watch},
    task::JoinHandle,
    time::{sleep, timeout},
};

/// Bumped when a command or an event changes meaning. The app refuses a newer one it does not know.
pub const PROTOCOL_VERSION: u32 = 1;
const EVENT_QUEUE: usize = 64;
const DIGITS_QUEUE: usize = 4;
const MAX_COMMAND_BYTES: u64 = 64 * 1024;
const SHOWN_CHARS: usize = 120;
const SHOWN_ERROR_CHARS: usize = 200;
const MAX_PINGS: usize = 20;
const DEFAULT_PINGS: usize = 3;
/// One attempt to reach a peer. The transport gives up on its own much later.
const DIAL_ATTEMPT: Duration = Duration::from_secs(5);
/// How long `ping` and `keep` keep trying to reach a peer before they say so.
const DIAL_WINDOW: Duration = Duration::from_secs(60);
/// Between two failed attempts, unless the network changes first.
const REDIAL_PAUSE: Duration = Duration::from_secs(1);
const KEEP_INTERVAL: Duration = Duration::from_secs(1);
/// Longer than any stall seen on a network switch (about 18 s), so a stall shows as one slow call
/// and not as a loss.
const KEEP_ECHO_LIMIT: Duration = Duration::from_secs(40);
const SLOW_ECHO: Duration = Duration::from_millis(1500);
/// How long shutdown waits for the last events to be written.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);
/// The most failures in a row that lengthen the pause between `keep` attempts.
const KEEP_MAX_BACKOFF: u32 = 30;
/// How long a fresh probe may take after a network change before the connection is given up.
const PROBE_LIMIT: Duration = Duration::from_millis(1500);

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Command {
    Status,
    Open { ttl_secs: Option<u64> },
    Cancel,
    Digits { value: String },
    Join { link: String },
    Peers,
    Revoke { id: String },
    NetworkChange,
    Ping { id: String, count: Option<usize> },
    Keep { id: String },
    KeepStop,
    Quit,
}

/// The ticket this process is showing, so a second `open` repeats it and `digits` find their way.
struct TicketSlot {
    /// Tells the ticket's task and a waiting answer to stop; a plain permit would be used up by whichever of them looked first.
    id: u64,
    link: String,
    ends_at: Instant,
    stop: watch::Sender<bool>,
    digits: mpsc::Sender<String>,
}

#[derive(Default)]
struct Slots {
    ticket: Option<TicketSlot>,
    join: Option<JoinHandle<()>>,
    keep: Option<JoinHandle<()>>,
    /// Pings run on their own until they answer or give up; shutdown ends the ones still going.
    pings: tokio::task::JoinSet<()>,
}

pub struct Control {
    bridge: Bridge,
    events: mpsc::Sender<Value>,
    relay: String,
    name: String,
    slots: Mutex<Slots>,
    next_ticket: AtomicU64,
    /// Counts network changes. A dial that is waiting between attempts hears it and tries at once.
    network: watch::Sender<u64>,
}

/// Runs until `quit` or the end of `input`. Every event goes to `output`, one line each; nothing
/// else is ever written there.
pub async fn serve<R, W>(
    bridge: Bridge,
    relay: String,
    name: String,
    mut input: R,
    output: W,
) -> Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (events, queue) = mpsc::channel(EVENT_QUEUE);
    let writer = tokio::spawn(write_events(output, queue));
    let (network, _) = watch::channel(0);
    // A revoke the peer told us of refreshes the app's list as a local one does. The callback runs
    // before the bridge's own revoke, so it writes the revoke first: the list the app asks for next
    // must not still show the peer. The bridge's revoke after it closes connections and sweeps.
    let cleanup = bridge.on_revoked();
    // Weak: tasks that outlive the loop hold the callback, and must not keep the event queue open.
    let (learned, db) = (events.downgrade(), bridge.db().clone());
    bridge.set_on_revoked(Arc::new(move |peer: &str| {
        if let Some(cleanup) = &cleanup {
            cleanup(peer);
        }
        let changed = peers::revoke(&db, peer).unwrap_or(false);
        if let Some(events) = learned.upgrade() {
            let _ = events.try_send(json!({"event": "revoked", "id": peer, "changed": changed}));
        }
    }));
    let control = Arc::new(Control {
        bridge,
        events,
        relay,
        name,
        slots: Mutex::new(Slots::default()),
        next_ticket: AtomicU64::new(0),
        network,
    });
    control
        .emit(json!({
            "event": "ready", "version": PROTOCOL_VERSION, "id": control.bridge.id().to_string(),
            "name": control.name, "relay": control.relay,
        }))
        .await;
    let mut line = String::new();
    loop {
        line.clear();
        let read = (&mut input)
            .take(MAX_COMMAND_BYTES)
            .read_line(&mut line)
            .await;
        match read {
            Ok(0) => break,
            // Not text: the bytes are gone from the pipe, so say so and carry on.
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                control.fail("", "that is not a command").await;
                continue;
            }
            Err(_) => break,
            Ok(n) if n as u64 >= MAX_COMMAND_BYTES && !line.ends_with('\n') => {
                control.fail("", "that line is too long").await;
                break;
            }
            Ok(_) => {}
        }
        if line.trim().is_empty() {
            continue;
        }
        if !control.handle(line.trim()).await {
            break;
        }
    }
    control.stop_all();
    control.bridge.endpoint().close().await;
    control.emit(json!({"event": "bye"})).await;
    drop(control);
    // Tasks that still hold the event queue (a ticket closing) get a moment, not forever.
    let _ = timeout(SHUTDOWN_GRACE, writer).await;
    Ok(())
}

async fn write_events<W: AsyncWrite + Unpin>(mut output: W, mut queue: mpsc::Receiver<Value>) {
    while let Some(event) = queue.recv().await {
        let mut line = event.to_string();
        line.push('\n');
        if output.write_all(line.as_bytes()).await.is_err() || output.flush().await.is_err() {
            return;
        }
    }
}

impl Control {
    async fn emit(&self, event: Value) {
        let _ = self.events.send(event).await;
    }

    fn emit_now(&self, event: Value) {
        let _ = self.events.try_send(event);
    }

    async fn fail(&self, cmd: &str, why: impl std::fmt::Display) {
        self.emit(json!({
            "event": "error", "cmd": cmd, "message": printable(&why.to_string(), SHOWN_ERROR_CHARS),
        }))
        .await;
    }

    /// Returns false when the loop should end.
    async fn handle(self: &Arc<Self>, line: &str) -> bool {
        // The text of a bad line is not echoed: it may be a link, and a link holds the code.
        let Ok(command) = serde_json::from_str::<Command>(line) else {
            self.fail("", "that is not a command").await;
            return true;
        };
        match command {
            Command::Status => self.status().await,
            Command::Open { ttl_secs } => self.open(ttl_secs).await,
            Command::Cancel => self.cancel(),
            Command::Digits { value } => self.digits(value).await,
            Command::Join { link } => self.join(link),
            Command::Peers => self.peers().await,
            Command::Revoke { id } => self.revoke(&id).await,
            Command::NetworkChange => self.network_change().await,
            Command::Ping { id, count } => self.ping(id, count),
            Command::Keep { id } => self.keep(id),
            Command::KeepStop => self.keep_stop(),
            Command::Quit => return false,
        }
        true
    }

    fn stop_all(&self) {
        let mut slots = self.slots.lock().unwrap();
        if let Some(ticket) = slots.ticket.take() {
            let _ = ticket.stop.send(true);
        }
        for task in [slots.join.take(), slots.keep.take()].into_iter().flatten() {
            task.abort();
        }
        slots.pings.abort_all();
    }

    async fn status(&self) {
        let ticket_open = self.slots.lock().unwrap().ticket.is_some();
        let db = self.bridge.db().clone();
        let count = tokio::task::spawn_blocking(move || peers::list(&db, false).map(|p| p.len()))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(0);
        self.emit(json!({
            "event": "status", "id": self.bridge.id().to_string(), "name": self.name,
            "relay": self.relay, "ticket_open": ticket_open, "peers": count,
        }))
        .await;
    }

    // ---- showing a ticket

    async fn open(self: &Arc<Self>, ttl_secs: Option<u64>) {
        if let Some(repeat) = self.open_ticket_event() {
            self.emit(repeat).await;
            return;
        }
        let mut tuning = Tuning::default();
        if let Some(secs) = ttl_secs {
            tuning.ttl = Duration::from_secs(secs).clamp(MIN_TTL, MAX_TTL);
        }
        let ttl = tuning.ttl;
        let open = match self.bridge.open_ticket(tuning).await {
            Ok(open) => open,
            Err(e) => return self.fail("open", format!("{e:#}")).await,
        };
        let link = open.ticket.link();
        let (digits, digits_in) = mpsc::channel(DIGITS_QUEUE);
        let (stop, stop_in) = watch::channel(false);
        let id = self.next_ticket.fetch_add(1, Ordering::SeqCst);
        self.slots.lock().unwrap().ticket = Some(TicketSlot {
            id,
            link: link.clone(),
            ends_at: Instant::now() + ttl,
            stop,
            digits,
        });
        self.emit(json!({"event": "ticket", "link": link, "expires_in_secs": ttl.as_secs()}))
            .await;
        tokio::spawn(self.clone().run_ticket(id, open, digits_in, stop_in));
    }

    fn open_ticket_event(&self) -> Option<Value> {
        let slots = self.slots.lock().unwrap();
        let ticket = slots.ticket.as_ref()?;
        let left = ticket.ends_at.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        Some(json!({"event": "ticket", "link": ticket.link, "expires_in_secs": left.as_secs()}))
    }

    /// Ends the ticket and a pairing in progress. The ticket leaves the slot at once, so an `open` that
    /// follows makes a new one instead of being handed the dead link.
    fn cancel(&self) {
        let mut slots = self.slots.lock().unwrap();
        if let Some(ticket) = slots.ticket.take() {
            let _ = ticket.stop.send(true);
        }
        if let Some(task) = slots.join.take() {
            task.abort();
        }
    }

    /// Owns the ticket until it ends. A device that passed the code waits here for the digits.
    async fn run_ticket(
        self: Arc<Self>,
        id: u64,
        mut open: hq_bridge::OpenTicket,
        mut digits: mpsc::Receiver<String>,
        mut stop: watch::Receiver<bool>,
    ) {
        // A device whose digits were accepted: whether it is on the list is looked at once the exchange
        // has ended (the host records it before it finishes), not by asking while it is being written.
        let mut accepted: Option<String> = None;
        loop {
            let request = tokio::select! {
                request = open.requests.recv() => request,
                _ = stop.wait_for(|stopped| *stopped) => None,
            };
            let Some(request) = request else { break };
            while digits.try_recv().is_ok() {}
            if let Some(joiner) = self.answer(request, &mut digits, &mut stop).await {
                accepted = Some(joiner);
            }
            if *stop.borrow() {
                break;
            }
        }
        // A request that arrived as the ticket stopped is dropped here, which declines it at once; left in
        // the queue it would hold the exchange (and this task) for the person's whole two minutes.
        open.requests.close();
        while open.requests.try_recv().is_ok() {}
        // Out of the slot before the slow close, so a new `open` is not given this ticket's dead link.
        {
            let mut slots = self.slots.lock().unwrap();
            if slots.ticket.as_ref().is_some_and(|t| t.id == id) {
                slots.ticket = None;
            }
        }
        // Closing waits for the exchange to end, so the device is on the list by now if it is going to be.
        open.cancel().await;
        let mut paired = false;
        if let Some(joiner) = accepted {
            paired = self.on_list(&joiner).await;
            let result = if paired { "paired" } else { "too_late" };
            self.emit(json!({"event": "decision", "joiner": joiner, "result": result}))
                .await;
        }
        self.emit(json!({"event": "ticket_closed", "paired": paired}))
            .await;
    }

    /// Waits for the person to type the digits, then pairs only if they match. Returns whether it paired.
    async fn answer(
        &self,
        request: PairRequest,
        digits: &mut mpsc::Receiver<String>,
        stop: &mut watch::Receiver<bool>,
    ) -> Option<String> {
        let joiner = request.joiner.to_string();
        self.emit(json!({
            "event": "request", "joiner": joiner, "name": printable(&request.name, SHOWN_CHARS),
            "window_secs": hq_bridge::host::DECISION_TIMEOUT.as_secs(),
        }))
        .await;
        let PairRequest {
            sas, mut decide, ..
        } = request;
        let typed = tokio::select! {
            typed = digits.recv() => typed,
            _ = decide.closed() => None,
            _ = stop.wait_for(|stopped| *stopped) => None,
        };
        let accept = typed.as_deref().map(str::trim) == Some(sas.as_str());
        let result = match typed {
            None => "too_late",
            Some(_) => match decide.send(accept) {
                // Delivered. Whether it was recorded is for the end of the ticket to say.
                Ok(()) if accept => return Some(joiner),
                Ok(()) => "declined",
                Err(_) => "too_late",
            },
        };
        self.emit(json!({"event": "decision", "joiner": joiner, "result": result}))
            .await;
        None
    }

    /// Whether the device is on the paired list: the answer reaching it is not the same thing, since it
    /// can leave in the instant the host records it.
    async fn on_list(&self, joiner: &str) -> bool {
        let db = self.bridge.db().clone();
        let key = joiner.to_string();
        tokio::task::spawn_blocking(move || peers::is_paired(&db, &key))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(false)
    }

    async fn digits(&self, value: String) {
        let sender = self
            .slots
            .lock()
            .unwrap()
            .ticket
            .as_ref()
            .map(|t| t.digits.clone());
        match sender {
            Some(sender) if sender.try_send(value).is_ok() => {}
            _ => self.fail("digits", "no device is waiting for digits").await,
        }
    }

    // ---- scanning a ticket

    fn join(self: &Arc<Self>, link: String) {
        let mut slots = self.slots.lock().unwrap();
        if slots.join.as_ref().is_some_and(|t| !t.is_finished()) {
            drop(slots);
            return self.emit_now(json!({
                "event": "error", "cmd": "join", "message": "a pairing is already in progress",
            }));
        }
        slots.join = Some(tokio::spawn(self.clone().run_join(link)));
    }

    async fn run_join(self: Arc<Self>, link: String) {
        let ticket = match Ticket::parse(&link) {
            Ok(ticket) => ticket,
            Err(e) => return self.join_failed(format!("{e:#}")).await,
        };
        let result = self
            .bridge
            .join(&ticket, &Tuning::default(), |sas| {
                // Queued with a wait, not dropped when the queue is full: without them the person has
                // nothing to type and the pairing times out.
                let events = self.events.clone();
                let digits = json!({"event": "digits", "value": sas});
                tokio::spawn(async move {
                    let _ = events.send(digits).await;
                });
            })
            .await;
        match result {
            Ok(JoinResult::Paired(joined)) => {
                self.emit(json!({
                    "event": "joined", "id": joined.card.id,
                    "name": printable(&joined.card.name, SHOWN_CHARS),
                }))
                .await
            }
            Ok(JoinResult::Refused(why)) => self.join_failed(why).await,
            Ok(JoinResult::Declined { .. }) => {
                self.join_failed("the other device did not accept the digits".into())
                    .await
            }
            Err(e) => self.join_failed(format!("{e:#}")).await,
        }
    }

    async fn join_failed(&self, why: String) {
        self.emit(json!({
            "event": "join_failed", "why": printable(&why, SHOWN_ERROR_CHARS),
        }))
        .await;
    }

    // ---- the paired list

    async fn peers(&self) {
        let db = self.bridge.db().clone();
        let rows = tokio::task::spawn_blocking(move || peers::list(&db, false)).await;
        match rows {
            Ok(Ok(rows)) => {
                let list: Vec<Value> = rows
                    .iter()
                    .map(|p| {
                        json!({
                            "id": p.id, "name": printable(&p.label, SHOWN_CHARS),
                            "paired_at": p.paired_at, "last_seen_at": p.last_seen_at,
                            "revoked_at": p.revoked_at,
                        })
                    })
                    .collect();
                self.emit(json!({"event": "peers", "peers": list})).await;
            }
            Ok(Err(e)) => self.fail("peers", format!("{e:#}")).await,
            Err(e) => self.fail("peers", e).await,
        }
    }

    async fn revoke(&self, id: &str) {
        let Ok(peer) = id.parse::<EndpointId>() else {
            return self.fail("revoke", "that is not a device id").await;
        };
        // The bridge closes the peer's open connections as well as writing the revoke.
        match self.bridge.revoke(&peer).await {
            Ok(changed) => {
                self.emit(json!({"event": "revoked", "id": peer.to_string(), "changed": changed}))
                    .await
            }
            Err(e) => self.fail("revoke", format!("{e:#}")).await,
        }
    }

    // ---- the network

    /// Android does not tell native code when the network changes, so the app does. iroh re-probes
    /// its paths and its relay, and a dial that is waiting between attempts tries again at once.
    async fn network_change(&self) {
        self.bridge.endpoint().network_change().await;
        self.network.send_modify(|count| *count += 1);
    }

    /// Connects to a paired peer, trying again until `window` ends. A peer that is not paired is
    /// refused at once: retrying cannot change that.
    async fn dial(&self, peer: &str, window: Duration) -> Result<Connection> {
        let _: EndpointId = peer
            .parse()
            .map_err(|_| anyhow!("that is not a device id"))?;
        if !self.on_list(peer).await {
            return Err(anyhow!("that device is not paired"));
        }
        let end = Instant::now() + window;
        let mut changes = self.network.subscribe();
        loop {
            let attempt = DIAL_ATTEMPT.min(end.saturating_duration_since(Instant::now()));
            let res = timeout(attempt, self.bridge.connect(peer)).await;
            if let Ok(Ok(conn)) = res {
                return Ok(conn);
            }
            if let Ok(Err(e)) = res {
                let revoked = hq_bridge::peer_revoked_us(hq_bridge::error_close_reason(&e));
                if revoked || !self.on_list(peer).await {
                    return Err(e);
                }
            }
            if Instant::now() >= end {
                return Err(anyhow!("could not reach that device"));
            }
            tokio::select! {
                _ = sleep(REDIAL_PAUSE) => {}
                _ = changes.changed() => {}
            }
        }
    }

    fn ping(self: &Arc<Self>, id: String, count: Option<usize>) {
        let count = count.unwrap_or(DEFAULT_PINGS).clamp(1, MAX_PINGS);
        let this = self.clone();
        let mut slots = self.slots.lock().unwrap();
        while slots.pings.try_join_next().is_some() {}
        slots.pings.spawn(async move {
            let started = Instant::now();
            let conn = match this.dial(&id, DIAL_WINDOW).await {
                Ok(conn) => conn,
                Err(e) => {
                    return this
                        .emit(json!({"event": "ping", "id": id, "error": format!("{e:#}")}))
                        .await;
                }
            };
            let connect_ms = started.elapsed().as_millis() as u64;
            let mut times = Vec::new();
            for _ in 0..count {
                let t = Instant::now();
                match timeout(KEEP_ECHO_LIMIT, echo(&conn, b"ping")).await {
                    Ok(Ok(())) => times.push(t.elapsed()),
                    // A peer that revoked us is heard by the bridge, which says `revoked`.
                    _ => break,
                }
            }
            times.sort();
            let median = times.get(times.len() / 2).map(|d| d.as_millis() as u64);
            this.emit(json!({
                "event": "ping", "id": id, "connect_ms": connect_ms, "median_ms": median,
                "path": path_kind(&conn), "error": median.is_none().then_some("no reply"),
            }))
            .await;
        });
    }

    /// Holds one connection to a peer and echoes on it once a second. For diagnosis: it prints a
    /// line per echo to standard error and reports a slow one or a loss as an event.
    fn keep(self: &Arc<Self>, id: String) {
        let mut slots = self.slots.lock().unwrap();
        if let Some(task) = slots.keep.take() {
            task.abort();
        }
        slots.keep = Some(tokio::spawn(self.clone().run_keep(id)));
    }

    fn keep_stop(&self) {
        if let Some(task) = self.slots.lock().unwrap().keep.take() {
            task.abort();
        }
    }

    async fn run_keep(self: Arc<Self>, id: String) {
        let begun = Instant::now();
        let mut changes = self.network.subscribe();
        let mut conn: Option<Connection> = None;
        let mut last_ok = Instant::now();
        let mut failures: u32 = 0;
        loop {
            let c = match conn.take() {
                Some(c) => c,
                None => match self.dial(&id, DIAL_WINDOW).await {
                    Ok(c) => {
                        keep_trace(begun, &format!("connected path={}", path_kind(&c)));
                        c
                    }
                    Err(e) => {
                        return self
                            .emit(json!({"event": "keep", "id": id, "state": "down", "error": format!("{e:#}")}))
                            .await;
                    }
                },
            };
            let t = Instant::now();
            match echo_unless_stale(&c, &mut changes).await {
                Some(rtt) => {
                    keep_trace(
                        begun,
                        &format!("rtt={}ms path={}", rtt.as_millis(), path_kind(&c)),
                    );
                    // Back up means an echo came back, not just a handshake (a device that revoked us completes one).
                    if rtt >= SLOW_ECHO || failures > 0 {
                        self.emit(json!({
                            "event": "keep", "id": id, "state": "up", "rtt_ms": rtt.as_millis() as u64,
                            "down_ms": t.duration_since(last_ok).as_millis() as u64, "path": path_kind(&c),
                        }))
                        .await;
                    }
                    last_ok = Instant::now();
                    failures = 0;
                    conn = Some(c);
                }
                None => {
                    failures = failures.saturating_add(1);
                    keep_trace(begun, "lost");
                    self.emit(json!({
                        "event": "keep", "id": id, "state": "down",
                        "down_ms": last_ok.elapsed().as_millis() as u64,
                    }))
                    .await;
                }
            }
            // A device that revoked us still lets the handshake through and then closes the stream:
            // each failure in a row waits a little longer, so that does not become a loop.
            tokio::select! {
                _ = sleep(KEEP_INTERVAL.saturating_mul(failures.clamp(1, KEEP_MAX_BACKOFF))) => {}
                _ = changes.changed() => {}
            }
        }
    }
}

/// One echo on `conn`. When the network changes while it waits, a fresh probe says whether the
/// connection is still good: the old path of a connection may be dead for many seconds before
/// the transport gives up on it, but a new connection is made in about a second. A probe that
/// is not answered closes the connection and returns `None`, so the caller dials again.
async fn echo_unless_stale(
    conn: &Connection,
    changes: &mut watch::Receiver<u64>,
) -> Option<Duration> {
    let began = Instant::now();
    let call = timeout(KEEP_ECHO_LIMIT, echo(conn, b"keep"));
    tokio::pin!(call);
    loop {
        tokio::select! {
            done = &mut call => return matches!(done, Ok(Ok(()))).then(|| began.elapsed()),
            Ok(()) = changes.changed() => {
                let probe = timeout(PROBE_LIMIT, echo(conn, b"probe")).await;
                if !matches!(probe, Ok(Ok(()))) {
                    conn.close(0u32.into(), b"network changed");
                    return None;
                }
            }
        }
    }
}

/// One line of the `keep` trace on standard error: the wall clock in milliseconds (so it lines up
/// with a network switch made from outside), then the time since `keep` began.
fn keep_trace(begun: Instant, what: &str) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    // `eprintln!` panics when the write fails (a full disk), and a panic aborts the process.
    use std::io::Write;
    let _ = writeln!(
        std::io::stderr(),
        "keep ts={now} t={:.1}s {what}",
        begun.elapsed().as_secs_f32()
    );
}

const MIN_TTL: Duration = Duration::from_secs(10);
const MAX_TTL: Duration = Duration::from_secs(3_600);

#[cfg(test)]
mod tests {
    use super::*;
    use hq_bridge::{RelayMode, echo_handler};
    use hq_db::Database;
    use tokio::io::{BufReader, DuplexStream, Lines};

    const LOOPBACK: &str = "127.0.0.1:0";
    const GIVE_UP: Duration = Duration::from_secs(20);

    /// One machine running the control loop, driven through in-memory pipes.
    struct Side {
        commands: DuplexStream,
        events: Lines<BufReader<DuplexStream>>,
        task: JoinHandle<Result<()>>,
        db: Database,
        _dir: tempfile::TempDir,
    }

    impl Side {
        async fn new(name: &str) -> Self {
            Self::with_handler(name, echo_handler()).await
        }

        async fn with_handler(name: &str, handler: hq_bridge::StreamHandler) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let db = Database::open_memory().unwrap();
            let bridge = Bridge::start(
                hq_bridge::BridgeConfig {
                    key: dir.path().join("bridge.key"),
                    bind: vec![LOOPBACK.parse().unwrap()],
                    relay: RelayMode::Disabled,
                    name: name.into(),
                    with_addrs: true,
                    advertise: vec![],
                },
                db.clone(),
                Some(handler),
            )
            .await
            .unwrap();
            let (commands, command_in) = tokio::io::duplex(64 * 1024);
            let (event_out, events) = tokio::io::duplex(64 * 1024);
            let task = tokio::spawn(serve(
                bridge,
                "none".into(),
                name.into(),
                BufReader::new(command_in),
                event_out,
            ));
            let mut side = Self {
                commands,
                events: BufReader::new(events).lines(),
                task,
                db,
                _dir: dir,
            };
            assert_eq!(side.next().await["event"], "ready");
            side
        }

        async fn send(&mut self, command: Value) {
            let mut line = command.to_string();
            line.push('\n');
            self.commands.write_all(line.as_bytes()).await.unwrap();
        }

        async fn next(&mut self) -> Value {
            let line = timeout(GIVE_UP, self.events.next_line())
                .await
                .expect("no event in time")
                .unwrap()
                .expect("the output ended");
            serde_json::from_str(&line).unwrap()
        }

        /// Skips events until one with this name arrives.
        async fn until(&mut self, event: &str) -> Value {
            loop {
                let value = self.next().await;
                if value["event"] == event {
                    return value;
                }
            }
        }

        async fn id(&mut self) -> String {
            self.send(json!({"cmd": "status"})).await;
            self.until("status").await["id"]
                .as_str()
                .unwrap()
                .to_string()
        }
    }

    /// `host` shows a ticket, `phone` joins it. Returns the digits the phone showed.
    async fn start_pairing(host: &mut Side, phone: &mut Side) -> String {
        host.send(json!({"cmd": "open"})).await;
        let link = host.until("ticket").await["link"]
            .as_str()
            .unwrap()
            .to_string();
        phone.send(json!({"cmd": "join", "link": link})).await;
        phone.until("digits").await["value"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// A paired pair of machines: returns the host's id and the phone's.
    async fn pair_up(host: &mut Side, phone: &mut Side) -> (String, String) {
        let digits = start_pairing(host, phone).await;
        host.until("request").await;
        host.send(json!({"cmd": "digits", "value": digits})).await;
        phone.until("joined").await;
        (host.id().await, phone.id().await)
    }

    /// A service that takes a stream and never answers it: a connection whose path has gone quiet.
    fn silent_handler() -> hq_bridge::StreamHandler {
        Arc::new(|stream: hq_bridge::PeerStream| {
            Box::pin(async move {
                let _held = stream;
                sleep(Duration::from_secs(120)).await;
            })
        })
    }

    #[tokio::test]
    async fn an_empty_bridge_reports_no_peers() {
        let mut side = Side::new("desk").await;
        side.send(json!({"cmd": "peers"})).await;
        assert_eq!(side.until("peers").await["peers"], json!([]));
        side.send(json!({"cmd": "status"})).await;
        let status = side.until("status").await;
        assert_eq!(status["peers"], 0);
        assert_eq!(status["ticket_open"], false);
    }

    #[tokio::test]
    async fn a_bad_line_is_an_error_that_does_not_repeat_it() {
        let mut side = Side::new("desk").await;
        let secret = "hqpair://ticket/SECRETCODE";
        side.commands
            .write_all(format!("join {secret}\n").as_bytes())
            .await
            .unwrap();
        let error = side.until("error").await;
        assert!(!error.to_string().contains("SECRETCODE"), "{error}");
        side.send(json!({"cmd": "digits", "value": "123456"})).await;
        assert_eq!(side.until("error").await["cmd"], "digits");
    }

    #[tokio::test]
    async fn opening_twice_shows_the_same_ticket_once() {
        let mut side = Side::new("desk").await;
        side.send(json!({"cmd": "open", "ttl_secs": 60})).await;
        let first = side.until("ticket").await;
        side.send(json!({"cmd": "open"})).await;
        let second = side.until("ticket").await;
        assert_eq!(first["link"], second["link"]);
        side.send(json!({"cmd": "cancel"})).await;
        assert_eq!(side.until("ticket_closed").await["paired"], false);
    }

    #[tokio::test]
    async fn typed_digits_pair_two_machines_through_the_control_channel() {
        let mut host = Side::new("desk").await;
        let mut phone = Side::new("phone").await;
        let digits = start_pairing(&mut host, &mut phone).await;
        assert_eq!(digits.len(), 6);

        let request = host.until("request").await;
        assert_eq!(request["name"], "phone");
        // Nothing the host says holds the digits: the person has to read them off the phone.
        assert!(!request.to_string().contains(&digits));
        host.send(json!({"cmd": "digits", "value": digits})).await;
        assert_eq!(host.until("decision").await["result"], "paired");
        assert_eq!(host.until("ticket_closed").await["paired"], true);

        let joined = phone.until("joined").await;
        assert_eq!(joined["id"], host.id().await);
        host.send(json!({"cmd": "peers"})).await;
        let listed = host.until("peers").await;
        assert_eq!(listed["peers"][0]["name"], "phone");
        assert_eq!(listed["peers"][0]["id"], phone.id().await);
    }

    #[tokio::test]
    async fn wrong_digits_decline_and_pair_nothing() {
        let mut host = Side::new("desk").await;
        let mut phone = Side::new("phone").await;
        let digits = start_pairing(&mut host, &mut phone).await;
        host.until("request").await;
        let wrong = if digits == "000000" {
            "111111"
        } else {
            "000000"
        };
        host.send(json!({"cmd": "digits", "value": wrong})).await;
        assert_eq!(host.until("decision").await["result"], "declined");
        assert_eq!(phone.until("join_failed").await["event"], "join_failed");
        host.send(json!({"cmd": "peers"})).await;
        assert_eq!(host.until("peers").await["peers"], json!([]));
    }

    #[tokio::test]
    async fn a_paired_peer_answers_a_ping_and_loses_access_when_revoked() {
        let mut host = Side::new("desk").await;
        let mut phone = Side::new("phone").await;
        let digits = start_pairing(&mut host, &mut phone).await;
        host.until("request").await;
        host.send(json!({"cmd": "digits", "value": digits})).await;
        phone.until("joined").await;
        let host_id = host.id().await;
        let phone_id = phone.id().await;

        phone
            .send(json!({"cmd": "ping", "id": host_id, "count": 2}))
            .await;
        let ping = phone.until("ping").await;
        assert!(ping["error"].is_null(), "{ping}");
        assert_eq!(ping["path"], "direct");

        host.send(json!({"cmd": "revoke", "id": phone_id})).await;
        assert_eq!(host.until("revoked").await["changed"], true);
        phone.send(json!({"cmd": "ping", "id": host_id})).await;
        // The host's refusal says "revoked": the phone revokes too and tells the app, in either
        // order with the failed ping.
        let (mut learned, mut ping) = (None, None);
        while learned.is_none() || ping.is_none() {
            let event = phone.next().await;
            match event["event"].as_str() {
                Some("revoked") => learned = Some(event),
                Some("ping") => ping = Some(event),
                _ => {}
            }
        }
        let learned = learned.unwrap();
        assert_eq!(
            (&learned["id"], &learned["changed"]),
            (&json!(host_id), &json!(true))
        );
        assert!(ping.unwrap()["error"].is_string());
        assert_eq!(peers::list(&host.db, false).unwrap().len(), 0);
        phone.send(json!({"cmd": "peers"})).await;
        assert_eq!(phone.until("peers").await["peers"], json!([]));
    }

    #[tokio::test]
    async fn a_ping_to_an_unpaired_id_fails_at_once_without_retrying() {
        let mut host = Side::new("desk").await;
        let stranger = host.id().await;
        let started = Instant::now();
        host.send(json!({"cmd": "ping", "id": stranger})).await;
        assert!(host.until("ping").await["error"].is_string());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn a_network_change_is_accepted_and_quit_ends_the_loop() {
        let mut side = Side::new("desk").await;
        side.send(json!({"cmd": "network_change"})).await;
        side.send(json!({"cmd": "status"})).await;
        assert_eq!(side.next().await["event"], "status");
        side.send(json!({"cmd": "quit"})).await;
        assert_eq!(side.until("bye").await["event"], "bye");
        timeout(GIVE_UP, side.task).await.unwrap().unwrap().unwrap();
    }

    #[tokio::test]
    async fn the_end_of_input_ends_the_loop() {
        let side = Side::new("desk").await;
        let Side { commands, task, .. } = side;
        drop(commands);
        timeout(GIVE_UP, task).await.unwrap().unwrap().unwrap();
    }

    #[tokio::test]
    async fn a_network_change_ends_a_stalled_connection_without_waiting_for_the_echo_limit() {
        let mut host = Side::with_handler("desk", silent_handler()).await;
        let mut phone = Side::new("phone").await;
        let (host_id, _) = pair_up(&mut host, &mut phone).await;

        phone.send(json!({"cmd": "keep", "id": host_id})).await;
        sleep(Duration::from_secs(2)).await;
        let changed = Instant::now();
        phone.send(json!({"cmd": "network_change"})).await;
        let down = phone.until("keep").await;
        assert_eq!(down["state"], "down", "{down}");
        // A loss on a connection that was up, not a failure to dial in the first place.
        assert!(down["error"].is_null(), "{down}");
        assert!(down["down_ms"].is_u64(), "{down}");
        // The echo limit is 40 s; a network change cuts a stalled call down to the probe's limit.
        assert!(
            changed.elapsed() < Duration::from_secs(10),
            "{:?}",
            changed.elapsed()
        );
    }

    #[tokio::test]
    async fn a_network_change_leaves_a_healthy_connection_alone() {
        let mut host = Side::new("desk").await;
        let mut phone = Side::new("phone").await;
        let (host_id, _) = pair_up(&mut host, &mut phone).await;

        phone.send(json!({"cmd": "keep", "id": host_id})).await;
        sleep(Duration::from_secs(2)).await;
        phone.send(json!({"cmd": "network_change"})).await;
        sleep(Duration::from_secs(4)).await;
        phone.send(json!({"cmd": "keep_stop"})).await;
        phone.send(json!({"cmd": "status"})).await;
        // A healthy connection answers the probe: no loss is reported between the change and now.
        loop {
            let event = phone.next().await;
            assert_ne!(event["event"], "keep", "{event}");
            if event["event"] == "status" {
                break;
            }
        }
    }

    #[tokio::test]
    async fn a_new_ticket_after_a_cancel_is_a_new_ticket() {
        let mut side = Side::new("desk").await;
        side.send(json!({"cmd": "open"})).await;
        let first = side.until("ticket").await["link"].clone();
        side.send(json!({"cmd": "cancel"})).await;
        side.send(json!({"cmd": "open"})).await;
        let second = side.until("ticket").await["link"].clone();
        assert_ne!(first, second, "the dead ticket's link was handed out again");
    }

    #[tokio::test]
    async fn cancelling_while_a_device_waits_for_its_digits_ends_the_ticket() {
        let mut host = Side::new("desk").await;
        let mut phone = Side::new("phone").await;
        start_pairing(&mut host, &mut phone).await;
        host.until("request").await;
        host.send(json!({"cmd": "cancel"})).await;
        assert_eq!(host.until("decision").await["result"], "too_late");
        assert_eq!(host.until("ticket_closed").await["paired"], false);
        host.send(json!({"cmd": "peers"})).await;
        assert_eq!(host.until("peers").await["peers"], json!([]));
    }

    #[tokio::test]
    async fn cancelling_ends_a_join_in_progress_and_allows_the_next() {
        let mut host = Side::new("desk").await;
        let mut phone = Side::new("phone").await;
        start_pairing(&mut host, &mut phone).await;
        host.until("request").await;
        // The host never types the digits; the phone gives up.
        phone.send(json!({"cmd": "cancel"})).await;
        host.send(json!({"cmd": "open"})).await;
        let link = host.until("ticket").await["link"]
            .as_str()
            .unwrap()
            .to_string();
        phone.send(json!({"cmd": "join", "link": link})).await;
        let next = phone.next().await;
        assert_ne!(next["event"], "error", "a second join was refused: {next}");
    }

    #[tokio::test]
    async fn a_join_with_a_link_that_is_not_a_ticket_fails_with_a_message() {
        let mut phone = Side::new("phone").await;
        phone
            .send(json!({"cmd": "join", "link": "hqpair://ticket/not-a-real-ticket-body"}))
            .await;
        let failed = phone.until("join_failed").await;
        assert!(
            failed["why"].as_str().is_some_and(|why| !why.is_empty()),
            "{failed}"
        );
    }
}
