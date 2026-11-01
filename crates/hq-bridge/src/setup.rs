//! Setup after pairing (`hq/setup/1`): one exchange on the long-term bridge that wires a hub (the
//! machine that holds HQ, such as the phone app) and a machine (one that runs coding agents for
//! it) with no configuration. The rules and their tests are in `docs/security/PAIRING.md`,
//! section 15; the design is `docs/plans/pairing-autoconfig.md`.
//!
//! The machine speaks first, and only when it has no setup with that peer yet:
//!
//! ```text
//! machine  {v, role:"machine", name, gate, mcp_port}   mcp_port: its forward to the hub's web
//! hub      {v, role:"hub", name, web, mcp_key, gate_key}
//! ```
//!
//! Each side picks its own loopback ports and stores what it set up in `bridge_setup`. A message
//! carries no address and no port the receiver must listen on. Only a serving hub accepts the
//! protocol, and the gate in `bridge.rs` admits paired peers only.

use crate::bridge::{
    Dialer, Live, LiveGuard, REASON_REVOKED, REJECT_REVOKED, peer_revoked_us, still_paired,
};
use crate::code::printable;
use crate::services::run_forward;
use crate::wire::MAX_NAME_CHARS;
use anyhow::{Context, Result, anyhow, bail};
use hq_db::{
    Database, bridge as peers,
    bridge_setup::{self as rows, ROLE_HUB, ROLE_MACHINE, Setup},
};
use iroh::{
    EndpointId,
    endpoint::{Connection, RecvStream, SendStream},
    protocol::{AcceptError, ProtocolHandler},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{HashMap, HashSet},
    net::{Ipv4Addr, SocketAddr},
    ops::RangeInclusive,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::TcpListener, sync::Notify, task::JoinHandle};

/// The setup protocol. Its own name, so a peer that does not speak it is told so at the handshake
/// and the pairing stays as it is.
pub const SETUP_ALPN: &[u8] = b"hq/setup/1";
pub const SETUP_VERSION: u16 = 1;
/// The longest message either side reads.
pub const MAX_SETUP_BYTES: usize = 4_096;
/// Where every setup port is picked from, on both sides.
pub const PORT_RANGE: RangeInclusive<u16> = 47_400..=47_999;
/// The hub's web app and `/mcp` (its paired-devices door).
pub const SVC_WEB: &str = "web";
/// The machine's host gate.
pub const SVC_GATE: &str = "gate";
/// Labels of the gate lines setup writes: this prefix and the first hex of the hub's id.
pub const GATE_LABEL_PREFIX: &str = "bridge:";
const LABEL_ID_CHARS: usize = 8;
/// The whole exchange, dial included. A peer that is not serving costs this long at most.
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(15);
/// How often the forwards and the gate lines are checked against the rows, besides on a wake.
const RECONCILE_EVERY: Duration = Duration::from_secs(2);
/// A setup name is a host or remote MCP name: [`crate::services::valid_name`] rules, this long
/// before an id suffix is added.
const MAX_SLUG: usize = 23;
const FALLBACK_SLUG: &str = "device";
/// Names the host list already gives a meaning.
const BUILT_IN_NAMES: [&str; 2] = ["native", "local"];
/// The close code of a setup the hub refused.
const REJECT_SETUP: u32 = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Holds HQ (the phone app, or a computer given `--as hub`): offers `web`, answers setup.
    Hub,
    /// Runs coding agents for a hub: offers `gate`, starts setup.
    Machine,
}

/// What a machine says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineHello {
    pub v: u16,
    pub role: String,
    pub name: String,
    pub gate: bool,
    pub mcp_port: u16,
}

/// What a hub answers. `mcp_key` is empty when the machine already holds a live one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HubReply {
    pub v: u16,
    pub role: String,
    pub name: String,
    pub web: bool,
    pub mcp_key: String,
    pub gate_key: String,
}

/// This side's part in setup. Every path is given, so nothing here reaches into a home directory.
#[derive(Debug, Clone)]
pub struct SetupConfig {
    pub role: Role,
    /// Machine: whether its gate is offered as [`SVC_GATE`] (a loopback gate that admits 127.0.0.1).
    pub gate: bool,
    /// Hub: whether [`SVC_WEB`] is offered (`bridge.web_listen` is set).
    pub web: bool,
    /// Machine: the gate's authorized keys file, where the hub's key goes.
    pub authorized_keys: PathBuf,
    /// Hub: where its gate keys live, one `<host>.key` per machine.
    pub gate_key_dir: PathBuf,
    /// Names setup may not take: configured hosts and remote MCP entries, which win.
    pub reserved_names: Vec<String>,
    /// Ports setup may not take: configured forwards.
    pub reserved_ports: Vec<u16>,
    /// Peer and service pairs a configured forward already covers; setup opens none for them.
    pub configured_forwards: Vec<(String, String)>,
    /// Where ports are picked from; [`PORT_RANGE`] unless a test narrows it.
    pub ports: RangeInclusive<u16>,
}

impl SetupConfig {
    pub fn new(role: Role, authorized_keys: PathBuf, gate_key_dir: PathBuf) -> Self {
        Self {
            role,
            gate: false,
            web: false,
            authorized_keys,
            gate_key_dir,
            reserved_names: Vec::new(),
            reserved_ports: Vec::new(),
            configured_forwards: Vec::new(),
            ports: PORT_RANGE,
        }
    }
}

/// What a machine's setup with one hub came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupOutcome {
    pub setup: Setup,
    /// Whether a message went out; a setup already in place sends nothing.
    pub sent: bool,
}

impl SetupOutcome {
    /// The hub kept the key it minted earlier and this machine has lost its copy: pairing again
    /// gets a new one (a new pairing starts setup over on both sides).
    pub fn key_missing(&self) -> bool {
        self.setup.mcp_key.is_none()
    }
}

/// The running part of setup, shared by the bridge, the protocol handler and the forwards task.
pub(crate) struct SetupState {
    pub cfg: SetupConfig,
    pub name: String,
    /// Ports picked here and held open until their forward takes them, so nothing else on this
    /// machine (another test, another setup) can pick the same one in between.
    held: Mutex<HashMap<u16, std::net::TcpListener>>,
    wake: Notify,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl SetupState {
    pub(crate) fn new(cfg: SetupConfig, name: String) -> Self {
        Self {
            cfg,
            name,
            held: Mutex::default(),
            wake: Notify::new(),
            task: Mutex::default(),
        }
    }

    pub(crate) fn wake(&self) {
        self.wake.notify_one();
    }

    pub(crate) fn stop(&self) {
        if let Some(task) = self.task.lock().unwrap().take() {
            task.abort();
        }
    }

    /// Picks a free loopback port that no row, configured forward or held port has, and holds it.
    fn reserve(&self, db: &Database) -> Result<u16> {
        let mut taken: HashSet<u16> = rows::list(db)?.iter().map(|s| s.local_port).collect();
        taken.extend(&self.cfg.reserved_ports);
        let mut held = self.held.lock().unwrap();
        taken.extend(held.keys());
        let (low, high) = (*self.cfg.ports.start(), *self.cfg.ports.end());
        let span = u32::from(high.saturating_sub(low)) + 1;
        let start = rand::random::<u32>() % span;
        for step in 0..span {
            let port = low + ((start + step) % span) as u16;
            if taken.contains(&port) {
                continue;
            }
            if let Ok(listener) = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
                held.insert(port, listener);
                return Ok(port);
            }
        }
        bail!("no free loopback port from {low} to {high}")
    }

    fn release(&self, port: u16) {
        self.held.lock().unwrap().remove(&port);
    }

    fn take_held(&self, port: u16) -> Option<std::net::TcpListener> {
        self.held.lock().unwrap().remove(&port)
    }
}

/// The gate line label for the hub `peer`.
pub fn gate_label(peer: &str) -> String {
    let prefix: String = peer.chars().take(LABEL_ID_CHARS).collect();
    format!("{GATE_LABEL_PREFIX}{prefix}")
}

/// Whether a label is one setup writes (`bridge:` and eight lowercase hex characters).
fn is_setup_label(label: &str) -> bool {
    label.strip_prefix(GATE_LABEL_PREFIX).is_some_and(|rest| {
        rest.len() == LABEL_ID_CHARS
            && rest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// A host or remote MCP name from a name a peer chose: lowercase letters, digits and `-`, never a
/// path, and with the peer's id prefix added when the plain one is taken.
pub fn slug_name(raw: &str, peer: &str, taken: &[String]) -> String {
    let mut slug = String::new();
    for c in raw.chars().flat_map(char::to_lowercase) {
        let c = if c.is_ascii_lowercase() || c.is_ascii_digit() {
            c
        } else {
            '-'
        };
        if !(c == '-' && (slug.is_empty() || slug.ends_with('-'))) {
            slug.push(c);
        }
    }
    slug.truncate(MAX_SLUG);
    let slug = match slug.trim_end_matches('-') {
        "" => FALLBACK_SLUG.to_string(),
        s => s.to_string(),
    };
    let clashes = |s: &str| BUILT_IN_NAMES.contains(&s) || taken.iter().any(|t| t == s);
    if !clashes(&slug) {
        return slug;
    }
    let id: String = peer.chars().take(LABEL_ID_CHARS).collect();
    format!("{slug}-{id}")
}

/// Every name in use for a peer other than `peer`, plus the reserved ones.
fn taken_names(db: &Database, cfg: &SetupConfig, peer: &str) -> Result<Vec<String>> {
    let mut taken: Vec<String> = rows::list(db)?
        .into_iter()
        .filter(|s| s.peer_id != peer)
        .map(|s| s.peer_name)
        .collect();
    taken.extend(cfg.reserved_names.iter().cloned());
    Ok(taken)
}

// ---- the wire

async fn write_msg<T: Serialize>(send: &mut SendStream, msg: &T) -> Result<()> {
    let body = serde_json::to_vec(msg)?;
    let len = u16::try_from(body.len())
        .ok()
        .filter(|n| usize::from(*n) <= MAX_SETUP_BYTES)
        .ok_or_else(|| anyhow!("the setup message is too long"))?;
    send.write_all(&len.to_be_bytes()).await?;
    send.write_all(&body).await?;
    Ok(())
}

async fn read_msg<T: DeserializeOwned>(recv: &mut RecvStream) -> Result<T> {
    let mut len = [0u8; 2];
    recv.read_exact(&mut len).await?;
    parse_msg(&read_body(recv, usize::from(u16::from_be_bytes(len))).await?)
}

async fn read_body(recv: &mut RecvStream, len: usize) -> Result<Vec<u8>> {
    if len == 0 || len > MAX_SETUP_BYTES {
        bail!("a setup message of {len} bytes is refused");
    }
    let mut body = vec![0u8; len];
    recv.read_exact(&mut body).await?;
    Ok(body)
}

/// Strict: unknown fields, missing ones and wrong types are all refused.
pub fn parse_msg<T: DeserializeOwned>(body: &[u8]) -> Result<T> {
    if body.len() > MAX_SETUP_BYTES {
        bail!("a setup message of {} bytes is refused", body.len());
    }
    serde_json::from_slice(body).context("not a setup message")
}

fn check_name(name: &str) -> Result<()> {
    if name.chars().count() > MAX_NAME_CHARS {
        bail!("the name is longer than {MAX_NAME_CHARS} characters");
    }
    Ok(())
}

/// A machine's message as a hub accepts it.
pub fn check_hello(hello: &MachineHello) -> Result<()> {
    if hello.v != SETUP_VERSION {
        bail!("setup version {} is not spoken here", hello.v);
    }
    if hello.role != ROLE_MACHINE {
        bail!("only a machine starts setup");
    }
    check_name(&hello.name)?;
    if !PORT_RANGE.contains(&hello.mcp_port) {
        bail!("mcp_port {} is outside the setup range", hello.mcp_port);
    }
    Ok(())
}

/// A hub's answer as a machine accepts it. The gate key's own format is checked where it is
/// written, by the gate's code.
pub fn check_reply(reply: &HubReply) -> Result<()> {
    if reply.v != SETUP_VERSION {
        bail!("setup version {} is not spoken here", reply.v);
    }
    if reply.role != ROLE_HUB {
        bail!("the peer is not a hub, so there is nothing to set up");
    }
    check_name(&reply.name)?;
    if !reply.mcp_key.is_empty() && !rows::is_device_key(&reply.mcp_key) {
        bail!("the hub's key is not a device key");
    }
    if reply.gate_key.is_empty() || reply.gate_key.len() > MAX_SETUP_BYTES {
        bail!("the hub sent no gate key");
    }
    Ok(())
}

// ---- the hub

/// The setup protocol's handler on a hub.
#[derive(Clone)]
pub(crate) struct SetupProtocol {
    pub db: Database,
    pub live: Arc<Live>,
    pub state: Arc<SetupState>,
    pub revoker: crate::bridge::Revoker,
}

impl std::fmt::Debug for SetupProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SetupProtocol")
    }
}

impl ProtocolHandler for SetupProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let id = conn.remote_id();
        let token = self.live.register(id, conn.weak_handle());
        let _guard = LiveGuard::new(self.live.clone(), id, token);
        if !still_paired(&self.db, id).await {
            conn.close(REJECT_REVOKED.into(), REASON_REVOKED);
            return Ok(());
        }
        let answered = tokio::time::timeout(EXCHANGE_TIMEOUT, self.answer(&conn, id)).await;
        let why = match answered {
            Ok(Ok(())) => {
                // Wait for the machine to read the answer and close, or closing would cut it off.
                let _ = tokio::time::timeout(EXCHANGE_TIMEOUT, conn.closed()).await;
                if peer_revoked_us(conn.close_reason().as_ref()) {
                    let _ = self.revoker.revoke(&id).await;
                }
                return Ok(());
            }
            Ok(Err(e)) => format!("{e:#}"),
            Err(_) => "it timed out".to_string(),
        };
        if peer_revoked_us(conn.close_reason().as_ref()) {
            let _ = self.revoker.revoke(&id).await;
        }
        tracing::warn!(peer = %id.fmt_short(), "bridge setup refused: {why}");
        conn.close(REJECT_SETUP.into(), b"setup refused");
        Ok(())
    }
}

impl SetupProtocol {
    async fn answer(&self, conn: &Connection, id: EndpointId) -> Result<()> {
        let (mut send, mut recv) = conn.accept_bi().await?;
        let hello: MachineHello = read_msg(&mut recv).await?;
        check_hello(&hello)?;
        let (db, state, peer) = (self.db.clone(), self.state.clone(), id.to_string());
        let reply =
            tokio::task::spawn_blocking(move || hub_record(&db, &state, &peer, &hello)).await??;
        write_msg(&mut send, &reply).await?;
        send.finish()?;
        self.state.wake();
        Ok(())
    }
}

/// Stores what the machine said and builds the answer. A peer the hub already set up keeps its
/// host name, its port and its key: no second key is minted while one is live.
fn hub_record(
    db: &Database,
    state: &SetupState,
    peer: &str,
    hello: &MachineHello,
) -> Result<HubReply> {
    let existing = rows::get(db, peer)?.filter(|s| !s.is_hub());
    let host = match &existing {
        Some(s) => s.peer_name.clone(),
        None => slug_name(&hello.name, peer, &taken_names(db, &state.cfg, peer)?),
    };
    let (port, picked) = match &existing {
        Some(s) => (s.local_port, false),
        None => (state.reserve(db)?, true),
    };
    let result = (|| {
        let key = (!rows::has_live_key(db, peer)?).then(rows::new_device_key);
        let gate_key = hub_gate_key(&state.cfg, &host)?;
        let setup = Setup {
            peer_id: peer.to_string(),
            peer_role: ROLE_MACHINE.into(),
            peer_name: host.clone(),
            local_port: port,
            peer_offers: hello.gate,
            peer_port: Some(hello.mcp_port),
            mcp_key: None,
            offered: false,
        };
        if !rows::record(db, &setup, key.as_deref())? {
            bail!("the peer was revoked during setup");
        }
        Ok(HubReply {
            v: SETUP_VERSION,
            role: ROLE_HUB.into(),
            name: state.name.chars().take(MAX_NAME_CHARS).collect(),
            web: state.cfg.web,
            mcp_key: key.unwrap_or_default(),
            gate_key,
        })
    })();
    if result.is_err() && picked {
        state.release(port);
    }
    result
}

#[cfg(unix)]
fn hub_gate_key(cfg: &SetupConfig, host: &str) -> Result<String> {
    let path = cfg.gate_key_dir.join(format!("{host}.key"));
    Ok(hq_gate::ClientKey::load_or_create(&path)
        .map_err(|e| anyhow!("the gate key for {host}: {e}"))?
        .public_key())
}

#[cfg(not(unix))]
fn hub_gate_key(_cfg: &SetupConfig, _host: &str) -> Result<String> {
    bail!("a hub's gate keys need a unix system")
}

// ---- the machine

/// Sets this machine up with the hub `peer`, or returns the setup already in place without a
/// word on the wire. It runs again only when this machine now offers a gate it did not offer then.
pub(crate) async fn machine_setup(
    dialer: &Dialer,
    state: &SetupState,
    peer: &str,
) -> Result<SetupOutcome> {
    let db = dialer.db().clone();
    let existing = rows::get(&db, peer)?.filter(Setup::is_hub);
    if let Some(setup) = existing.clone().filter(|s| s.offered || !state.cfg.gate) {
        return Ok(SetupOutcome { setup, sent: false });
    }
    let (port, picked) = match &existing {
        Some(s) => (s.local_port, false),
        None => (state.reserve(&db)?, true),
    };
    let hello = MachineHello {
        v: SETUP_VERSION,
        role: ROLE_MACHINE.into(),
        name: state.name.chars().take(MAX_NAME_CHARS).collect(),
        gate: state.cfg.gate,
        mcp_port: port,
    };
    let result = match tokio::time::timeout(EXCHANGE_TIMEOUT, exchange(dialer, peer, &hello)).await
    {
        Ok(Ok(reply)) => machine_record(&db, state, peer, port, existing.as_ref(), &reply),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(anyhow!("the peer did not answer setup in time")),
    };
    if result.is_err() && picked {
        state.release(port);
    }
    let setup = result?;
    state.wake();
    Ok(SetupOutcome { setup, sent: true })
}

async fn exchange(dialer: &Dialer, peer: &str, hello: &MachineHello) -> Result<HubReply> {
    let conn = dialer
        .dial(peer, SETUP_ALPN)
        .await
        .context("the peer does not answer setup (it may not be a hub, or not be serving)")?;
    let (mut send, mut recv) = match conn.open_bi().await {
        Ok(streams) => streams,
        Err(e) => {
            if peer_revoked_us(conn.close_reason().as_ref())
                && let Ok(id) = peer.parse::<EndpointId>()
            {
                let _ = dialer.revoke(&id).await;
            }
            return Err(e.into());
        }
    };
    write_msg(&mut send, hello).await?;
    send.finish()?;
    let reply: HubReply = match read_msg(&mut recv).await {
        Ok(reply) => reply,
        Err(e) => {
            if peer_revoked_us(conn.close_reason().as_ref())
                && let Ok(id) = peer.parse::<EndpointId>()
            {
                let _ = dialer.revoke(&id).await;
            }
            return Err(e);
        }
    };
    conn.close(0u32.into(), b"done");
    check_reply(&reply)?;
    Ok(reply)
}

fn machine_record(
    db: &Database,
    state: &SetupState,
    peer: &str,
    port: u16,
    existing: Option<&Setup>,
    reply: &HubReply,
) -> Result<Setup> {
    authorize_hub(&state.cfg, peer, &reply.gate_key)?;
    let name = match existing {
        Some(s) => s.peer_name.clone(),
        None => slug_name(&reply.name, peer, &taken_names(db, &state.cfg, peer)?),
    };
    let mcp_key = match reply.mcp_key.as_str() {
        "" => existing.and_then(|s| s.mcp_key.clone()),
        key => Some(key.to_string()),
    };
    let setup = Setup {
        peer_id: peer.to_string(),
        peer_role: ROLE_HUB.into(),
        peer_name: name,
        local_port: port,
        peer_offers: reply.web,
        peer_port: None,
        mcp_key,
        offered: state.cfg.gate,
    };
    if !rows::record(db, &setup, None)? {
        prune_gate_lines(db, &state.cfg.authorized_keys)?;
        bail!("the peer was revoked during setup");
    }
    Ok(setup)
}

#[cfg(unix)]
fn authorize_hub(cfg: &SetupConfig, peer: &str, key: &str) -> Result<()> {
    let from = [std::net::IpAddr::V4(Ipv4Addr::LOCALHOST)];
    hq_gate::append_authorized(&cfg.authorized_keys, key, &from, &gate_label(peer))
        .map_err(|e| anyhow!("authorizing the hub's gate key: {e}"))?;
    Ok(())
}

#[cfg(not(unix))]
fn authorize_hub(_cfg: &SetupConfig, _peer: &str, _key: &str) -> Result<()> {
    bail!("the host gate needs a unix system")
}

/// Removes every gate line setup wrote for a hub that no longer has a setup row here (revoked,
/// from this process or another). Lines the owner wrote are left alone. Returns how many went.
#[cfg(unix)]
pub fn prune_gate_lines(db: &Database, authorized_keys: &std::path::Path) -> Result<usize> {
    let keep: HashSet<String> = rows::list(db)?
        .iter()
        .filter(|s| s.is_hub())
        .map(|s| gate_label(&s.peer_id))
        .collect();
    hq_gate::remove_authorized(authorized_keys, |_, label| {
        is_setup_label(label) && !keep.contains(label)
    })
    .map_err(|e| anyhow!("{}: {e}", authorized_keys.display()))
}

#[cfg(not(unix))]
pub fn prune_gate_lines(_db: &Database, _authorized_keys: &std::path::Path) -> Result<usize> {
    Ok(0)
}

// ---- the forwards

/// The service a setup row's forward reaches on its peer.
fn forward_service(setup: &Setup) -> &'static str {
    if setup.is_hub() { SVC_WEB } else { SVC_GATE }
}

/// Keeps one forward open per setup row and closes the forward of a row that is gone, checking
/// on every wake and every [`RECONCILE_EVERY`]. On a machine it also prunes the gate lines.
pub(crate) fn spawn_forwards(state: Arc<SetupState>, dialer: Dialer) {
    let task = tokio::spawn({
        let state = state.clone();
        async move {
            let mut open: HashMap<u16, (String, JoinHandle<()>)> = HashMap::new();
            loop {
                reconcile(&state, &dialer, &mut open).await;
                tokio::select! {
                    _ = tokio::time::sleep(RECONCILE_EVERY) => {}
                    _ = state.wake.notified() => {}
                }
            }
        }
    });
    if let Some(old) = state.task.lock().unwrap().replace(task) {
        old.abort();
    }
}

async fn reconcile(
    state: &SetupState,
    dialer: &Dialer,
    open: &mut HashMap<u16, (String, JoinHandle<()>)>,
) {
    let db = dialer.db().clone();
    let listed = tokio::task::spawn_blocking(move || rows::list(&db)).await;
    let Ok(Ok(rows)) = listed else {
        return;
    };
    let wanted: HashMap<u16, &Setup> = rows
        .iter()
        .filter(|s| !configured(state, s))
        .map(|s| (s.local_port, s))
        .collect();
    open.retain(|port, (peer, task)| {
        let keep = wanted.get(port).is_some_and(|s| &s.peer_id == peer) && !task.is_finished();
        if !keep {
            task.abort();
        }
        keep
    });
    for (port, setup) in wanted {
        if open.contains_key(&port) {
            continue;
        }
        match bind_forward(state, port).await {
            Ok(listener) => {
                let task = tokio::spawn(run_forward(
                    dialer.clone(),
                    setup.peer_id.clone(),
                    forward_service(setup).to_string(),
                    listener,
                ));
                open.insert(port, (setup.peer_id.clone(), task));
            }
            Err(e) => tracing::warn!(port, "bridge setup forward: {e:#}"),
        }
    }
    if state.cfg.role == Role::Machine {
        let (db, path) = (dialer.db().clone(), state.cfg.authorized_keys.clone());
        let _ = tokio::task::spawn_blocking(move || prune_gate_lines(&db, &path)).await;
    }
}

/// Config wins: a configured forward to the same peer and service, or on the same port, means
/// setup opens none.
fn configured(state: &SetupState, setup: &Setup) -> bool {
    let service = forward_service(setup);
    state.cfg.reserved_ports.contains(&setup.local_port)
        || state
            .cfg
            .configured_forwards
            .iter()
            .any(|(peer, svc)| peer == &setup.peer_id && svc == service)
}

async fn bind_forward(state: &SetupState, port: u16) -> Result<TcpListener> {
    if let Some(held) = state.take_held(port) {
        held.set_nonblocking(true)?;
        return Ok(TcpListener::from_std(held)?);
    }
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpListener::bind(addr)
        .await
        .with_context(|| format!("could not listen on {addr}"))
}

/// Every paired peer that has no setup here yet, for a machine to try at start.
pub(crate) fn peers_without_setup(db: &Database) -> Result<Vec<String>> {
    let set: HashSet<String> = rows::list(db)?.into_iter().map(|s| s.peer_id).collect();
    Ok(peers::list(db, false)?
        .into_iter()
        .map(|p| p.id)
        .filter(|id| !set.contains(id))
        .collect())
}

/// A peer's name as it may be shown to a person.
pub fn shown(name: &str) -> String {
    printable(name, MAX_NAME_CHARS)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEER: &str = "0123456789abcdef";

    #[test]
    fn a_chosen_name_becomes_a_plain_slug_and_never_a_path() {
        assert_eq!(
            slug_name("Calvin's MacBook Pro", PEER, &[]),
            "calvin-s-macbook-pro"
        );
        for raw in ["../../x", "/etc/passwd", "..", "a/../../b", "\u{202e}evil"] {
            let s = slug_name(raw, PEER, &[]);
            assert!(crate::services::valid_name(&s), "{raw:?} gave {s:?}");
            assert!(!s.contains('.') && !s.contains('/'), "{s}");
        }
        assert_eq!(slug_name("", PEER, &[]), FALLBACK_SLUG);
        assert_eq!(slug_name("Native", PEER, &[]), "native-01234567");
        assert_eq!(
            slug_name("laptop", PEER, &["laptop".to_string()]),
            "laptop-01234567"
        );
        let long = slug_name(&"x".repeat(200), PEER, &[]);
        assert!(crate::services::valid_name(&long) && long.len() <= MAX_SLUG);
    }

    #[test]
    fn only_labels_setup_writes_are_setup_labels() {
        assert!(is_setup_label(&gate_label("abcdef0123456789")));
        for not in [
            "bridge:",
            "bridge:abc",
            "bridge:ABCDEF01",
            "mac",
            "bridge:abcdef012",
        ] {
            assert!(!is_setup_label(not), "{not}");
        }
    }

    fn hello() -> MachineHello {
        MachineHello {
            v: SETUP_VERSION,
            role: ROLE_MACHINE.into(),
            name: "laptop".into(),
            gate: true,
            mcp_port: 47_500,
        }
    }

    fn reply() -> HubReply {
        HubReply {
            v: SETUP_VERSION,
            role: ROLE_HUB.into(),
            name: "phone".into(),
            web: true,
            mcp_key: rows::new_device_key(),
            gate_key: "k".into(),
        }
    }

    #[test]
    fn messages_with_unknown_fields_a_wrong_version_or_too_many_bytes_are_refused() {
        let good = serde_json::to_vec(&hello()).unwrap();
        assert!(check_hello(&parse_msg::<MachineHello>(&good).unwrap()).is_ok());
        let mut extra = serde_json::to_value(hello()).unwrap();
        extra["addr"] = "10.0.0.1:22".into();
        assert!(parse_msg::<MachineHello>(&serde_json::to_vec(&extra).unwrap()).is_err());
        let mut missing = serde_json::to_value(hello()).unwrap();
        missing.as_object_mut().unwrap().remove("gate");
        assert!(parse_msg::<MachineHello>(&serde_json::to_vec(&missing).unwrap()).is_err());
        let huge = vec![b' '; MAX_SETUP_BYTES + 1];
        assert!(parse_msg::<MachineHello>(&huge).is_err());

        let mut h = hello();
        h.v = 2;
        assert!(check_hello(&h).is_err());
        let mut h = hello();
        h.role = ROLE_HUB.into();
        assert!(check_hello(&h).is_err(), "a hub does not start setup");
        let mut h = hello();
        h.mcp_port = 22;
        assert!(check_hello(&h).is_err(), "a port outside the range");
        let mut h = hello();
        h.name = "x".repeat(MAX_NAME_CHARS + 1);
        assert!(check_hello(&h).is_err());

        assert!(check_reply(&reply()).is_ok());
        let mut r = reply();
        r.v = 0;
        assert!(check_reply(&r).is_err());
        let mut r = reply();
        r.role = ROLE_MACHINE.into();
        assert!(check_reply(&r).is_err(), "two machines set nothing up");
        let mut r = reply();
        r.mcp_key = "AGENTHQ_API_KEY".into();
        assert!(check_reply(&r).is_err());
        let mut r = reply();
        r.mcp_key.clear();
        assert!(
            check_reply(&r).is_ok(),
            "a hub that keeps a live key sends none"
        );
        let mut extra = serde_json::to_value(reply()).unwrap();
        extra["listen"] = "0.0.0.0:80".into();
        assert!(parse_msg::<HubReply>(&serde_json::to_vec(&extra).unwrap()).is_err());
    }
}
