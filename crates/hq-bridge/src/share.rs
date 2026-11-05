//! A machine lends its model to its hubs (`hq/share/1`). The machine says what it shares, or
//! that it stopped; the hub stores it on the machine's setup row, opens a fail-fast forward to
//! the machine's `llm` service on a socket only HQ can open, and lists it in
//! `shared-models.json` beside its key, which HQ's backend chains put in front of the configured
//! primary (`HqConfig::backends_in_force`).
//!
//! ```text
//! machine  {v, model, tools, context}    model null: nothing shared
//! hub      {v, ok}                        ok false: this hub did not set the machine up
//! ```
//!
//! Only a peer the hub set up as a machine may share, or every paired device could make itself
//! the hub's model and read every prompt. The rules are in `docs/security/PAIRING.md`, section 17.

use crate::bridge::{Dialer, Live, LiveGuard, refused};
use crate::services::{ForwardListener, ForwardOptions, run_forward};
use crate::setup::{read_msg, write_msg};
use anyhow::{Context, Result, bail};
use hq_core::config::{SHARED_MODEL_SERVICE, SHARED_MODELS_FILE, SharedBackend};
use hq_db::{Database, bridge_setup as rows};
use iroh::{
    EndpointId,
    endpoint::Connection,
    protocol::{AcceptError, ProtocolHandler},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{task::JoinHandle, time::Instant};

pub const SHARE_ALPN: &[u8] = b"hq/share/1";
pub const SHARE_VERSION: u16 = 1;
/// Longest model name a hub stores. Ollama's names are far shorter.
pub const MAX_MODEL_CHARS: usize = 128;
/// How long a lent model may take to answer before the hub's chain moves on, and how quickly
/// later requests move on while it stays away (`ForwardOptions::fail_fast`).
pub const SHARED_FAIL_FAST: Duration = Duration::from_secs(3);
/// How often the hub checks its rows and the machine checks what it shares.
const TICK: Duration = Duration::from_secs(2);
/// A hub that could not be told is tried again after this long, then twice as long each time.
const RETRY: Duration = Duration::from_secs(30);
const MAX_BACKOFF: Duration = Duration::from_secs(10 * 60);
/// A hub that was told is told again this often, in case it lost what it held.
const REFRESH: Duration = Duration::from_secs(10 * 60);
/// A hub that does not speak the protocol (an older release) is asked again only this rarely.
const UNSUPPORTED_RETRY: Duration = Duration::from_secs(60 * 60);
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(15);
const REJECT_SHARE: u32 = 403;
/// The socket a hub's forward to `name`'s model listens on, in the bridge directory.
const SOCKET_SUFFIX: &str = "-llm.sock";

/// What a machine says it lends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareOffer {
    pub v: u16,
    pub model: Option<String>,
    pub tools: Option<bool>,
    pub context: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareAck {
    pub v: u16,
    pub ok: bool,
}

/// The model a machine lends now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offered {
    pub model: String,
    pub tools: Option<bool>,
    pub context: Option<u64>,
}

/// Asked every tick for what the machine lends now; `None`: nothing.
pub type ShareSource = Arc<dyn Fn() -> Option<Offered> + Send + Sync>;

/// A model name is printable and short, since the hub writes it into a file and a request.
pub fn valid_model(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_MODEL_CHARS
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._:/-@+".contains(c))
}

fn check_offer(offer: &ShareOffer) -> Result<()> {
    if offer.v != SHARE_VERSION {
        bail!("share version {} is not {SHARE_VERSION}", offer.v);
    }
    if let Some(model) = &offer.model
        && !valid_model(model)
    {
        bail!("{model:?} is not a model name");
    }
    Ok(())
}

/// The hub's side of the protocol.
pub(crate) struct ShareProtocol {
    pub db: Database,
    pub live: Arc<Live>,
}

impl std::fmt::Debug for ShareProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ShareProtocol")
    }
}

impl ProtocolHandler for ShareProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let id = conn.remote_id();
        let token = self.live.register(id, conn.weak_handle());
        let _guard = LiveGuard::new(self.live.clone(), id, token);
        if refused(&self.db, &conn, id).await {
            return Ok(());
        }
        match tokio::time::timeout(EXCHANGE_TIMEOUT, self.answer(&conn, id)).await {
            Ok(Ok(())) => {
                let _ = tokio::time::timeout(EXCHANGE_TIMEOUT, conn.closed()).await;
            }
            Ok(Err(e)) => {
                tracing::warn!(peer = %id.fmt_short(), "bridge share refused: {e:#}");
                conn.close(REJECT_SHARE.into(), b"share refused");
            }
            Err(_) => conn.close(REJECT_SHARE.into(), b"share timed out"),
        }
        Ok(())
    }
}

impl ShareProtocol {
    async fn answer(&self, conn: &Connection, id: EndpointId) -> Result<()> {
        let (mut send, mut recv) = conn.accept_bi().await?;
        let offer: ShareOffer = read_msg(&mut recv).await?;
        check_offer(&offer)?;
        let shared = offer.model.map(|model| rows::SharedModel {
            model,
            tools: offer.tools,
            context: offer.context,
        });
        let (db, peer) = (self.db.clone(), id.to_string());
        let ok = tokio::task::spawn_blocking(move || rows::set_shared(&db, &peer, shared.as_ref()))
            .await??;
        if !ok {
            tracing::warn!(peer = %id.fmt_short(), "bridge share: not a machine this hub set up");
        }
        write_msg(
            &mut send,
            &ShareAck {
                v: SHARE_VERSION,
                ok,
            },
        )
        .await?;
        send.finish()?;
        Ok(())
    }
}

/// Tells `hub` what this machine lends now.
pub async fn announce(dialer: &Dialer, hub: &str, offered: Option<&Offered>) -> Result<()> {
    let exchange = async {
        let conn = dialer.dial(hub, SHARE_ALPN).await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        let offer = ShareOffer {
            v: SHARE_VERSION,
            model: offered.map(|o| o.model.clone()),
            tools: offered.and_then(|o| o.tools),
            context: offered.and_then(|o| o.context),
        };
        write_msg(&mut send, &offer).await?;
        send.finish()?;
        let ack: ShareAck = read_msg(&mut recv).await?;
        conn.close(0u32.into(), b"done");
        if !ack.ok {
            bail!("the hub did not set this machine up, so it takes no model from it");
        }
        Ok(())
    };
    tokio::time::timeout(EXCHANGE_TIMEOUT, exchange)
        .await
        .context("the hub did not answer in time")?
}

/// Whether `e` says the hub does not speak `hq/share/1` (an older release): the handshake
/// fails on the protocol name (TLS alert 120, "no application protocol").
pub fn is_unsupported(e: &anyhow::Error) -> bool {
    let text = format!("{e:#}").to_ascii_lowercase();
    [
        "alpn",
        "no application protocol",
        "known protocol",
        "error 120",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

/// What the announcer knows about one hub.
#[derive(Clone)]
struct Told {
    /// What the hub holds, `None` while unknown.
    held: Option<Option<Offered>>,
    /// After a failure, no new try before this.
    retry_at: Instant,
    /// The wait after the next failure.
    backoff: Duration,
    /// When the hub is told again even if nothing changed.
    refresh_at: Instant,
}

impl Told {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            held: None,
            retry_at: now,
            backoff: RETRY,
            refresh_at: now,
        }
    }

    fn due(&self, wanted: &Option<Offered>) -> bool {
        let now = Instant::now();
        now >= self.retry_at && (self.held.as_ref() != Some(wanted) || now >= self.refresh_at)
    }
}

/// Keeps every hub of this machine told what it lends: at start, whenever `source` changes, and
/// again every [`REFRESH`] in case the hub lost it (a revoke and a new pairing clear its row).
/// Hubs are told at once, so one that is away delays no other; one that does not answer is asked
/// again after a wait that doubles up to [`MAX_BACKOFF`]. `also` lists hubs set up by hand
/// (configured forwards to their `web`); the rest are the `hub` rows of `bridge_setup`.
pub fn spawn_announcer(dialer: Dialer, source: ShareSource, also: Vec<String>) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Unknown at start, so every hub is told once, even "nothing", which clears a model it
        // kept from before this machine stopped sharing while its bridge was off.
        let mut told: HashMap<String, Told> = HashMap::new();
        loop {
            let wanted = source();
            let due: Vec<(String, Told)> = hubs(dialer.db(), &also)
                .await
                .into_iter()
                .map(|hub| {
                    let state = told.get(&hub).cloned().unwrap_or_else(Told::new);
                    (hub, state)
                })
                .filter(|(_, state)| state.due(&wanted))
                .collect();
            let mut asks = tokio::task::JoinSet::new();
            for (hub, state) in due {
                let (dialer, wanted) = (dialer.clone(), wanted.clone());
                asks.spawn(async move {
                    let result = announce(&dialer, &hub, wanted.as_ref()).await;
                    (hub, state, wanted, result)
                });
            }
            while let Some(Ok((hub, state, wanted, result))) = asks.join_next().await {
                let now = Instant::now();
                let next = match result {
                    Ok(()) => Told {
                        held: Some(wanted),
                        retry_at: now,
                        backoff: RETRY,
                        refresh_at: now + REFRESH,
                    },
                    Err(e) if is_unsupported(&e) => Told {
                        retry_at: now + UNSUPPORTED_RETRY,
                        ..state
                    },
                    Err(e) => {
                        tracing::debug!(hub = %hub, "bridge share: {e:#}");
                        Told {
                            retry_at: now + state.backoff,
                            backoff: (state.backoff * 2).min(MAX_BACKOFF),
                            ..state
                        }
                    }
                };
                told.insert(hub, next);
            }
            tokio::time::sleep(TICK).await;
        }
    })
}

async fn hubs(db: &Database, also: &[String]) -> Vec<String> {
    let db = db.clone();
    let mut out: Vec<String> = tokio::task::spawn_blocking(move || rows::list(&db))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.is_hub())
        .map(|s| s.peer_id)
        .collect();
    for id in also {
        let id = id.trim().to_ascii_lowercase();
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// The socket a hub's forward to the machine set up as `name` listens on.
pub fn shared_socket(bridge_dir: &Path, name: &str) -> PathBuf {
    bridge_dir.join(format!("{name}{SOCKET_SUFFIX}"))
}

/// The hub's side: a forward for every machine that lends a model, and `shared-models.json` in
/// `bridge_dir` listing them, both following the rows (a revoke removes the row, then both).
pub fn spawn_shared_forwards(dialer: Dialer, bridge_dir: PathBuf) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut open = OpenShared {
            dir: bridge_dir,
            forwards: HashMap::new(),
            failed: HashMap::new(),
        };
        let mut written: Option<Vec<SharedBackend>> = None;
        loop {
            let db = dialer.db().clone();
            if let Ok(Ok(shared)) = tokio::task::spawn_blocking(move || rows::shared(&db)).await {
                let wanted: Vec<(String, SharedBackend)> = shared
                    .into_iter()
                    .map(|(setup, model)| {
                        let backend = SharedBackend {
                            socket: shared_socket(&open.dir, &setup.peer_name),
                            name: setup.peer_name,
                            model: model.model,
                            prompted_tools: model.tools == Some(false),
                        };
                        (setup.peer_id, backend)
                    })
                    .collect();
                open.reconcile(&dialer, &wanted);
                let listed: Vec<SharedBackend> =
                    open.forwards.values().map(|(b, _)| b.clone()).collect();
                if written.as_ref() != Some(&listed) {
                    match write_shared(&open.dir, &listed) {
                        Ok(()) => written = Some(listed),
                        Err(e) => tracing::warn!("bridge share: {e:#}"),
                    }
                }
            }
            tokio::time::sleep(TICK).await;
        }
    })
}

/// The forwards the hub opened for lent models. Dropped with its task (the bridge stops), it
/// stops them and removes their sockets and the listing, so HQ never names a dead socket.
struct OpenShared {
    dir: PathBuf,
    forwards: HashMap<String, (SharedBackend, JoinHandle<()>)>,
    /// Peers whose socket could not be opened, and when to try again, so a lasting failure is
    /// logged once rather than every tick.
    failed: HashMap<String, Instant>,
}

impl Drop for OpenShared {
    fn drop(&mut self) {
        for (backend, task) in self.forwards.values() {
            task.abort();
            let _ = std::fs::remove_file(&backend.socket);
        }
        let _ = write_shared(&self.dir, &[]);
    }
}

impl OpenShared {
    fn reconcile(&mut self, dialer: &Dialer, wanted: &[(String, SharedBackend)]) {
        self.forwards.retain(|peer, (backend, task)| {
            let keep = wanted.iter().any(|(p, b)| p == peer && b == backend) && !task.is_finished();
            if !keep {
                task.abort();
                let _ = std::fs::remove_file(&backend.socket);
            }
            keep
        });
        self.failed
            .retain(|peer, _| wanted.iter().any(|(p, _)| p == peer));
        for (peer, backend) in wanted {
            let waiting = self.failed.get(peer).is_some_and(|at| Instant::now() < *at);
            if self.forwards.contains_key(peer) || waiting {
                continue;
            }
            match listen(&backend.socket) {
                Ok(listener) => {
                    let options = ForwardOptions {
                        fail_fast: Some(SHARED_FAIL_FAST),
                        direct_only: false,
                    };
                    let task = tokio::spawn(run_forward(
                        dialer.clone(),
                        peer.clone(),
                        SHARED_MODEL_SERVICE.to_string(),
                        listener,
                        options,
                    ));
                    self.failed.remove(peer);
                    self.forwards.insert(peer.clone(), (backend.clone(), task));
                }
                Err(e) => {
                    if !self.failed.contains_key(peer) {
                        tracing::warn!(socket = %backend.socket.display(), "bridge share forward: {e:#}");
                    }
                    self.failed
                        .insert(peer.clone(), Instant::now() + MAX_BACKOFF);
                }
            }
        }
    }
}

#[cfg(unix)]
fn listen(socket: &Path) -> Result<ForwardListener> {
    Ok(hq_core::fs_private::bind_private_socket(socket, true)?.into())
}

#[cfg(not(unix))]
fn listen(_socket: &Path) -> Result<ForwardListener> {
    Err(anyhow!(
        "a lent model needs a unix socket, which this machine has not"
    ))
}

/// Writes the list HQ reads, owner-only, or removes it when nothing is lent.
fn write_shared(dir: &Path, shared: &[SharedBackend]) -> Result<()> {
    let path = dir.join(SHARED_MODELS_FILE);
    if shared.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        };
    }
    let tmp = dir.join(format!("{SHARED_MODELS_FILE}.tmp"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    {
        use std::io::Write;
        let mut file = options.open(&tmp).context("writing the shared models")?;
        file.write_all(&serde_json::to_vec_pretty(shared)?)?;
    }
    std::fs::rename(&tmp, &path).context("saving the shared models")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_names_are_short_and_plain() {
        for ok in [
            "qwen3:8b",
            "gemma4:e2b",
            "library/llama3.1:70b-instruct-q4_K_M",
        ] {
            assert!(valid_model(ok), "{ok}");
        }
        for bad in [
            "",
            "a b",
            "x\n",
            "\"x\"",
            "é",
            &"m".repeat(MAX_MODEL_CHARS + 1),
        ] {
            assert!(!valid_model(bad), "{bad:?}");
        }
        let offer = |model: Option<&str>, v| ShareOffer {
            v,
            model: model.map(str::to_string),
            tools: None,
            context: None,
        };
        assert!(check_offer(&offer(Some("qwen3:8b"), SHARE_VERSION)).is_ok());
        assert!(check_offer(&offer(None, SHARE_VERSION)).is_ok());
        assert!(check_offer(&offer(Some("a b"), SHARE_VERSION)).is_err());
        assert!(check_offer(&offer(None, 2)).is_err());
    }

    #[test]
    fn the_list_is_owner_only_and_removed_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let one = SharedBackend {
            name: "laptop".into(),
            model: "qwen3:8b".into(),
            socket: shared_socket(dir.path(), "laptop"),
            prompted_tools: false,
        };
        write_shared(dir.path(), std::slice::from_ref(&one)).unwrap();
        let path = dir.path().join(SHARED_MODELS_FILE);
        assert_eq!(hq_core::config::read_shared_backends(&path), vec![one]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0);
        }
        write_shared(dir.path(), &[]).unwrap();
        assert!(!path.exists());
        write_shared(dir.path(), &[]).unwrap();
    }
}
