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
    Bridge, BridgeConfig, EndpointId, Forwards, PairRequest, RelayMode, StreamHandler, Tuning,
    code::printable, echo_handler, services_from_config, start_forwards,
};
use hq_core::config::BridgeServicesConfig;
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

/// What this machine offers its paired devices and which of their services it reaches through
/// local ports: `bridge:` in the HQ config.
#[derive(Debug, Clone, Default)]
pub struct HubServices {
    pub config: BridgeServicesConfig,
    /// HQ's own web port, which no service may lead to.
    pub ws_port: u16,
}

type OpenDb = Box<dyn Fn() -> Result<Database> + Send + Sync>;

pub struct PairingController {
    cfg: PairingConfig,
    open_db: OpenDb,
    db: Mutex<Option<Database>>,
    handler: StreamHandler,
    hub: HubServices,
    bridge: tokio::sync::Mutex<Option<Bridge>>,
    /// The forwards the bridge opened, kept to close their ports when it stops.
    forwards: Mutex<Option<Forwards>>,
    /// This machine's card while it is switched on. The screen reads it instead of the bridge, so
    /// a slow start or revoke (both can wait on a relay) never freezes the refresh.
    host: Mutex<Option<HostCard>>,
    /// The forwards' state, set and cleared with `host`.
    forward_views: Arc<Mutex<Vec<ForwardView>>>,
    offered: Mutex<Vec<String>>,
    /// Whether the bridge lends this computer's model to its hubs; flipped while it runs.
    share: hq_bridge::ServiceSwitch,
    /// The model hubs are told to use. Unset: the first one the server lists.
    shared_model: Mutex<Option<String>>,
    session: Arc<Mutex<Session>>,
    changed: watch::Sender<u64>,
    on_revoked: Mutex<Option<hq_bridge::RevokeCallback>>,
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

/// One local port that reaches a service on a paired device.
#[derive(Debug, Clone, Serialize)]
pub struct ForwardView {
    pub service: String,
    pub peer_short: String,
    pub listen: String,
    /// The port was already taken, typically by a `hq bridge serve` on this machine, so this app
    /// did not open it and does not own it.
    pub running_elsewhere: bool,
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
    /// The services this machine offers its paired devices, by name.
    pub services: Vec<String>,
    pub forwards: Vec<ForwardView>,
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
            hub: HubServices::default(),
            bridge: tokio::sync::Mutex::new(None),
            forwards: Mutex::new(None),
            host: Mutex::new(None),
            forward_views: Arc::new(Mutex::new(Vec::new())),
            offered: Mutex::new(Vec::new()),
            share: hq_bridge::ServiceSwitch::new(false),
            shared_model: Mutex::new(None),
            session: Arc::default(),
            changed: watch::channel(0).0,
            on_revoked: Mutex::new(None),
        }
    }

    /// Called when a peer tells us it revoked us, on a blocking thread. A revoke made here does
    /// not call it: its caller cleans up.
    pub fn with_on_revoked(self, cb: hq_bridge::RevokeCallback) -> Self {
        *self.on_revoked.lock().unwrap() = Some(cb);
        self
    }

    /// Offers `hub.config`'s services to paired devices and opens its forwards while the bridge is
    /// on. Without this, the bridge offers nothing and forwards nothing.
    pub fn with_services(mut self, hub: HubServices) -> Self {
        self.hub = hub;
        self
    }

    /// Lends this computer's model to its hubs while the bridge is on, or stops at once: open
    /// streams to the model end, as a revoke ends a peer's.
    pub fn set_sharing(&self, on: bool, model: Option<String>) {
        let model = model
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty());
        *self.shared_model.lock().unwrap() = model;
        self.share.set(on);
        self.bump();
    }

    pub fn sharing(&self) -> (bool, Option<String>) {
        (
            self.share.is_on(),
            self.shared_model.lock().unwrap().clone(),
        )
    }

    /// The model server the bridge lends: the config's `share_model.to`, else Ollama's address.
    pub fn model_server(&self) -> String {
        self.hub
            .config
            .share_model
            .as_ref()
            .map(|s| s.to.clone())
            .unwrap_or_else(|| hq_core::config::DEFAULT_MODEL_SERVER.to_string())
    }

    /// The owner offers `llm` by hand in `bridge.services`: the switch does not govern it.
    pub fn shared_by_hand(&self) -> bool {
        self.hub
            .config
            .services
            .contains_key(hq_core::config::SHARED_MODEL_SERVICE)
    }

    /// The config the bridge starts with: the owner's, plus the shared model behind the switch
    /// unless the owner already offers an `llm` service by hand. The service is registered even
    /// while the switch is off, so turning it on needs no restart; off, it answers like a name
    /// that is not offered.
    fn bridge_config(&self) -> hq_core::config::BridgeServicesConfig {
        let mut config = self.hub.config.clone();
        if !self.shared_by_hand() {
            let share = config.share_model.get_or_insert_with(Default::default);
            share.model = self.shared_model.lock().unwrap().clone();
        }
        config
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
            if let Some(dir) = self.cfg.key.parent() {
                crate::settings::create_private_dir(dir)?;
            }
            let mut services = services_from_config(&self.bridge_config(), self.hub.ws_port)?;
            if let Some(services) = services.as_mut()
                && services
                    .hubs_only
                    .contains_key(hq_core::config::SHARED_MODEL_SERVICE)
            {
                services.switches.insert(
                    hq_core::config::SHARED_MODEL_SERVICE.to_string(),
                    self.share.clone(),
                );
            }
            let offered: Vec<String> = services
                .as_ref()
                .map(|s| s.targets.keys().cloned().collect())
                .unwrap_or_default();
            let bridge = Bridge::start_with_services(
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
                None,
                services,
            )
            .await?;
            let on_revoked = self.on_revoked.lock().unwrap().clone();
            let changed = self.changed.clone();
            bridge.set_on_revoked(Arc::new(move |peer| {
                if let Some(ref cb) = on_revoked {
                    cb(peer);
                }
                changed.send_modify(|v| *v += 1);
            }));
            let forwards = match start_forwards(&bridge, &self.hub.config).await {
                Ok(forwards) => forwards,
                Err(e) => {
                    bridge.shutdown().await;
                    return Err(e);
                }
            };
            *self.forward_views.lock().unwrap() =
                forwards.status.iter().map(forward_view).collect();
            *self.forwards.lock().unwrap() = Some(forwards);
            *self.offered.lock().unwrap() = offered;
            let card = bridge.card();
            *self.host.lock().unwrap() = Some(HostCard {
                short_id: short(&card.id),
                id: card.id,
                name: printable(&card.name, SHOWN_NAME_CHARS),
                addrs: card.addrs,
            });
            *slot = Some(bridge);
        }
        Ok(slot.as_ref().expect("just set"))
    }

    /// Switches this machine off for paired devices and closes everything open.
    pub async fn stop(&self) {
        self.cancel_ticket();
        let bridge = self.bridge.lock().await.take();
        *self.host.lock().unwrap() = None;
        self.forward_views.lock().unwrap().clear();
        self.offered.lock().unwrap().clear();
        if let Some(forwards) = self.forwards.lock().unwrap().take() {
            forwards.abort();
        }
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
        let host = self.host.lock().unwrap().clone();
        let (peers, peers_error) = match self.list_peers().await {
            Ok(list) => (list, None),
            Err(e) => (Vec::new(), Some(format!("{e:#}"))),
        };
        let mut session = self.session.lock().unwrap();
        prune(&mut session);
        PairingSnapshot {
            running: host.is_some(),
            host,
            services: self
                .offered
                .lock()
                .unwrap()
                .iter()
                .filter(|name| {
                    name.as_str() != hq_core::config::SHARED_MODEL_SERVICE
                        || self.share.is_on()
                        || self.shared_by_hand()
                })
                .cloned()
                .collect(),
            forwards: self.forward_views.lock().unwrap().clone(),
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

/// Whether this app holds the forward listening on `listen`: `None` when pairing is off or it
/// opened no such forward, `Some(false)` when another program had the port first.
fn holds(views: &Mutex<Vec<ForwardView>>, listen: SocketAddr) -> Option<bool> {
    let wanted = listen.to_string();
    views
        .lock()
        .unwrap()
        .iter()
        .find(|f| f.listen == wanted)
        .map(|f| !f.running_elsewhere)
}

impl PairingController {
    /// Whether this app opened the forward on `listen` itself (see [`holds`]).
    pub fn holds_forward(&self, listen: SocketAddr) -> Option<bool> {
        holds(&self.forward_views, listen)
    }

    /// A gate for the proxy: open only while this app holds the forward on `listen`.
    pub fn forward_gate(&self, listen: SocketAddr) -> crate::proxy::UpstreamGate {
        let views = self.forward_views.clone();
        Arc::new(move || match holds(&views, listen) {
            Some(true) => None,
            Some(false) => Some(crate::proxy::HELD_PORT_LINE),
            None => Some(crate::desktop::FORWARD_WAIT_LINE),
        })
    }
}

fn forward_view(f: &hq_bridge::ForwardStatus) -> ForwardView {
    ForwardView {
        service: f.service.clone(),
        peer_short: short(&f.peer),
        listen: f.listen.to_string(),
        running_elsewhere: f.running_elsewhere,
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

    #[tokio::test]
    async fn the_screen_refresh_does_not_wait_for_the_bridge_lock() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open_memory().unwrap();
        let controller = PairingController::new(
            PairingConfig {
                key: dir.path().join("bridge.key"),
                bind: vec!["127.0.0.1:0".parse().unwrap()],
                relay: RelayMode::Disabled,
                name: "desk".into(),
                with_addrs: true,
                tuning: Tuning::default(),
            },
            move || Ok(db.clone()),
            None,
        );
        controller.start().await.unwrap();
        // A slow start or revoke holds this lock; the screen must still be able to refresh.
        let held = controller.bridge.lock().await;
        let snapshot =
            tokio::time::timeout(std::time::Duration::from_secs(2), controller.snapshot())
                .await
                .expect("the refresh waited for the bridge lock");
        assert!(snapshot.running && snapshot.host.is_some());
        drop(held);
        controller.stop().await;
    }

    #[test]
    fn only_equal_digits_match() {
        assert!(digits_match("918273", "918273"));
        assert!(!digits_match("918273", "918274"));
        assert!(!digits_match("91827", "918273"));
    }
}
