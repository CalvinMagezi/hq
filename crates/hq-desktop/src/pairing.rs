//! Pairing from the app's side: show a QR code, take the six digits the person types, list the
//! paired devices and revoke one.
//!
//! One long-term `Bridge` serves for as long as the person leaves devices switched on; revoking
//! goes through it, because only it holds the connections a revoke has to close. The digits the
//! joining device shows are compared here, in Rust: the window gets the request but never the
//! expected digits, so it cannot show them, and a click cannot stand in for typing them.

use crate::qr::{self, QrMatrix};
use anyhow::{Context, Result, anyhow};
use hq_bridge::{
    Bridge, BridgeConfig, EndpointId, PairRequest, RelayMode, StreamHandler, Tuning,
    code::printable, echo_handler,
};
use hq_db::{Database, bridge as peers};
use serde::Serialize;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;
use tokio::sync::{oneshot, watch};

/// Wrong tries at the digits before the device is declined. A typo should not cost a new QR code;
/// three tries on digits only the person can read off the other screen is no help to a guesser.
pub const MAX_DIGIT_TRIES: u32 = 3;
const SAS_DIGITS: usize = 6;
const SHOWN_NAME_CHARS: usize = 60;
const SHORT_ID_CHARS: usize = 8;

pub struct PairingConfig {
    pub key: PathBuf,
    /// Where the long-term endpoint listens (UDP).
    pub bind: Vec<SocketAddr>,
    pub relay: RelayMode,
    pub name: String,
    pub with_addrs: bool,
    pub tuning: Tuning,
}

type OpenDb = Box<dyn Fn() -> Result<Database> + Send + Sync>;

pub struct PairingController {
    cfg: PairingConfig,
    open_db: OpenDb,
    db: Mutex<Option<Database>>,
    handler: StreamHandler,
    bridge: tokio::sync::Mutex<Option<Bridge>>,
    session: Arc<Mutex<Session>>,
    changed: watch::Sender<u64>,
}

#[derive(Default)]
struct Session {
    ticket: Option<ActiveTicket>,
    pending: Vec<Pending>,
    next_ticket: u64,
    next_request: u64,
}

struct ActiveTicket {
    seq: u64,
    view: TicketView,
    cancel: Option<oneshot::Sender<()>>,
}

struct Pending {
    id: u64,
    joiner: EndpointId,
    name: String,
    sas: String,
    decide: Option<oneshot::Sender<bool>>,
    tries_left: u32,
    received_at: i64,
    expires_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TicketView {
    /// The `hqpair://` link: the QR code's text and the copy-paste form.
    pub link: String,
    pub qr: Option<QrMatrix>,
    pub expires_at: i64,
}

/// A device waiting for the person's digits. It has no field for the expected digits.
#[derive(Debug, Clone, Serialize)]
pub struct PendingView {
    pub id: u64,
    /// What the device calls itself: unchecked text, made printable. Never a reason to trust it.
    pub name: String,
    pub joiner_id: String,
    pub joiner_short: String,
    pub received_at: i64,
    pub expires_at: i64,
    pub tries_left: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostCard {
    pub id: String,
    pub short_id: String,
    pub name: String,
    pub addrs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PeerView {
    pub id: String,
    pub short_id: String,
    pub name: String,
    pub paired_at: i64,
    pub last_seen_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PairingSnapshot {
    /// Whether this machine is switched on for paired devices.
    pub running: bool,
    pub host: Option<HostCard>,
    pub ticket: Option<TicketView>,
    pub pending: Vec<PendingView>,
    pub peers: Vec<PeerView>,
    /// Why the paired list could not be read, if it could not.
    pub peers_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Submitted {
    /// The digits matched and the device was accepted; it appears in the list in a moment.
    Accepted,
    /// The digits did not match; the person may try again.
    WrongDigits { tries_left: u32 },
    /// Too many wrong tries: the device was declined.
    Declined,
    /// The answer came after the device stopped waiting. Nothing was paired.
    TooLate,
    /// Not six digits.
    NotDigits,
    /// No such request, or it was already answered.
    Unknown,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn short(id: &str) -> String {
    id.chars().take(SHORT_ID_CHARS).collect()
}

impl PairingController {
    pub fn new(
        cfg: PairingConfig,
        open_db: impl Fn() -> Result<Database> + Send + Sync + 'static,
        handler: Option<StreamHandler>,
    ) -> Self {
        Self {
            cfg,
            open_db: Box::new(open_db),
            db: Mutex::new(None),
            handler: handler.unwrap_or_else(echo_handler),
            bridge: tokio::sync::Mutex::new(None),
            session: Arc::default(),
            changed: watch::channel(0).0,
        }
    }

    /// Ticks whenever the screen should read the snapshot again.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    fn bump(&self) {
        self.changed.send_modify(|v| *v += 1);
    }

    fn db(&self) -> Result<Database> {
        let mut slot = self.db.lock().unwrap();
        if slot.is_none() {
            *slot = Some((self.open_db)().context("opening the HQ database")?);
        }
        Ok(slot.clone().expect("just set"))
    }

    /// Switches this machine on for paired devices. Nothing listens before this is called.
    pub async fn start(&self) -> Result<()> {
        let mut bridge = self.bridge.lock().await;
        self.ensure_started(&mut bridge).await?;
        drop(bridge);
        self.bump();
        Ok(())
    }

    async fn ensure_started<'a>(&self, slot: &'a mut Option<Bridge>) -> Result<&'a Bridge> {
        if slot.is_none() {
            let bridge = Bridge::start(
                BridgeConfig {
                    key: self.cfg.key.clone(),
                    bind: self.cfg.bind.clone(),
                    relay: self.cfg.relay.clone(),
                    name: self.cfg.name.clone(),
                    with_addrs: self.cfg.with_addrs,
                    advertise: Vec::new(),
                },
                self.db()?,
                Some(self.handler.clone()),
            )
            .await?;
            *slot = Some(bridge);
        }
        Ok(slot.as_ref().expect("just set"))
    }

    /// Switches this machine off for paired devices and closes everything open.
    pub async fn stop(&self) {
        self.cancel_ticket();
        let bridge = self.bridge.lock().await.take();
        if let Some(bridge) = bridge {
            bridge.shutdown().await;
        }
        self.session.lock().unwrap().pending.clear();
        self.bump();
    }

    /// Opens a ticket (one at a time: a new one replaces the old) and returns what to show.
    pub async fn begin(&self) -> Result<TicketView> {
        let mut slot = self.bridge.lock().await;
        let bridge = self.ensure_started(&mut slot).await?;
        self.cancel_ticket();
        let tuning = self.cfg.tuning.clone();
        let ttl = tuning.ttl;
        let decision = tuning.decision_timeout;
        let mut open = bridge.open_ticket(tuning).await?;
        let link = open.ticket.link();
        let view = TicketView {
            qr: qr::matrix(&link),
            link,
            expires_at: now() + ttl.as_secs() as i64,
        };
        let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
        let seq = {
            let mut session = self.session.lock().unwrap();
            session.next_ticket += 1;
            let seq = session.next_ticket;
            session.ticket = Some(ActiveTicket {
                seq,
                view: view.clone(),
                cancel: Some(cancel_tx),
            });
            seq
        };
        let session = self.session.clone();
        let changed = self.changed.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = &mut cancel_rx => {
                        open.cancel().await;
                        break;
                    }
                    request = open.requests.recv() => match request {
                        Some(request) => {
                            add_pending(&session, request, decision);
                            changed.send_modify(|v| *v += 1);
                        }
                        None => {
                            open.closed().await;
                            break;
                        }
                    }
                }
            }
            let mut s = session.lock().unwrap();
            if s.ticket.as_ref().is_some_and(|t| t.seq == seq) {
                s.ticket = None;
            }
            drop(s);
            changed.send_modify(|v| *v += 1);
        });
        drop(slot);
        self.bump();
        Ok(view)
    }

    /// Closes the open ticket, if there is one.
    pub fn cancel_ticket(&self) {
        let cancel = self
            .session
            .lock()
            .unwrap()
            .ticket
            .as_mut()
            .and_then(|t| t.cancel.take());
        if let Some(cancel) = cancel {
            let _ = cancel.send(());
        }
    }

    /// The person typed `typed` for request `id`. The device is accepted only if the digits are
    /// the ones the joining device shows.
    pub fn submit_digits(&self, id: u64, typed: &str) -> Submitted {
        let Some(digits) = normalize_digits(typed) else {
            return Submitted::NotDigits;
        };
        let mut session = self.session.lock().unwrap();
        prune(&mut session);
        let Some(at) = session.pending.iter().position(|p| p.id == id) else {
            return Submitted::Unknown;
        };
        let matches = digits_match(&digits, &session.pending[at].sas);
        let outcome = if matches {
            decide(&mut session, at, true)
        } else {
            let p = &mut session.pending[at];
            p.tries_left = p.tries_left.saturating_sub(1);
            if p.tries_left > 0 {
                Submitted::WrongDigits {
                    tries_left: p.tries_left,
                }
            } else {
                decide(&mut session, at, false);
                Submitted::Declined
            }
        };
        drop(session);
        self.bump();
        outcome
    }

    /// The person declines without typing anything.
    pub fn decline(&self, id: u64) -> bool {
        let mut session = self.session.lock().unwrap();
        let Some(at) = session.pending.iter().position(|p| p.id == id) else {
            return false;
        };
        decide(&mut session, at, false);
        drop(session);
        self.bump();
        true
    }

    /// Revokes a device and closes every connection it has open, at once.
    pub async fn revoke(&self, id: &str) -> Result<bool> {
        let id: EndpointId = id.parse().map_err(|_| anyhow!("{id} is not a device id"))?;
        let bridge = self.bridge.lock().await;
        let changed = match bridge.as_ref() {
            Some(bridge) => bridge.revoke(&id).await?,
            None => {
                let db = self.db()?;
                let key = id.to_string();
                tokio::task::spawn_blocking(move || peers::revoke(&db, &key)).await??
            }
        };
        drop(bridge);
        self.bump();
        Ok(changed)
    }

    pub async fn snapshot(&self) -> PairingSnapshot {
        let host = self.bridge.lock().await.as_ref().map(|b| {
            let card = b.card();
            HostCard {
                short_id: short(&card.id),
                id: card.id,
                name: printable(&card.name, SHOWN_NAME_CHARS),
                addrs: card.addrs,
            }
        });
        let (peers, peers_error) = match self.list_peers().await {
            Ok(list) => (list, None),
            Err(e) => (Vec::new(), Some(format!("{e:#}"))),
        };
        let mut session = self.session.lock().unwrap();
        prune(&mut session);
        PairingSnapshot {
            running: host.is_some(),
            host,
            ticket: session.ticket.as_ref().map(|t| t.view.clone()),
            pending: session.pending.iter().map(pending_view).collect(),
            peers,
            peers_error,
        }
    }

    async fn list_peers(&self) -> Result<Vec<PeerView>> {
        let db = self.db()?;
        let rows = tokio::task::spawn_blocking(move || peers::list(&db, false)).await??;
        Ok(rows
            .into_iter()
            .map(|p| PeerView {
                short_id: short(&p.id),
                name: printable(&p.label, SHOWN_NAME_CHARS),
                id: p.id,
                paired_at: p.paired_at,
                last_seen_at: p.last_seen_at,
            })
            .collect())
    }
}

fn add_pending(session: &Mutex<Session>, request: PairRequest, decision: std::time::Duration) {
    let mut s = session.lock().unwrap();
    prune(&mut s);
    s.next_request += 1;
    let id = s.next_request;
    let received_at = now();
    s.pending.push(Pending {
        id,
        joiner: request.joiner,
        name: printable(&request.name, SHOWN_NAME_CHARS),
        sas: request.sas,
        decide: Some(request.decide),
        tries_left: MAX_DIGIT_TRIES,
        received_at,
        expires_at: received_at + decision.as_secs() as i64,
    });
}

/// Forgets requests whose device stopped waiting.
fn prune(session: &mut Session) {
    session
        .pending
        .retain(|p| p.decide.as_ref().is_some_and(|d| !d.is_closed()));
}

/// Sends the answer and forgets the request.
fn decide(session: &mut Session, at: usize, accept: bool) -> Submitted {
    let mut p = session.pending.remove(at);
    let sent = p.decide.take().map(|d| d.send(accept));
    match (sent, accept) {
        (Some(Ok(())), true) => Submitted::Accepted,
        (Some(Ok(())), false) => Submitted::Declined,
        _ => Submitted::TooLate,
    }
}

fn pending_view(p: &Pending) -> PendingView {
    let id = p.joiner.to_string();
    PendingView {
        id: p.id,
        name: p.name.clone(),
        joiner_short: short(&id),
        joiner_id: id,
        received_at: p.received_at,
        expires_at: p.expires_at,
        tries_left: p.tries_left,
    }
}

/// The six digits, however the person spaced them. `None` when it is not exactly six digits.
fn normalize_digits(typed: &str) -> Option<String> {
    let digits: String = typed
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    (digits.len() == SAS_DIGITS && digits.bytes().all(|b| b.is_ascii_digit())).then_some(digits)
}

fn digits_match(typed: &str, expected: &str) -> bool {
    typed.as_bytes().ct_eq(expected.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_are_read_however_they_are_spaced_and_nothing_else() {
        assert_eq!(normalize_digits("123456").as_deref(), Some("123456"));
        assert_eq!(normalize_digits(" 123 456 ").as_deref(), Some("123456"));
        assert_eq!(normalize_digits("123-456").as_deref(), Some("123456"));
        for bad in ["", "12345", "1234567", "12345a", "١٢٣٤٥٦", "123 45\u{0661}"] {
            assert_eq!(normalize_digits(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn only_equal_digits_match() {
        assert!(digits_match("918273", "918273"));
        assert!(!digits_match("918273", "918274"));
        assert!(!digits_match("91827", "918273"));
    }
}
