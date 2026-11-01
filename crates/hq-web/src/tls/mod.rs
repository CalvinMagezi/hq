//! The connector's own TLS door: the device ends TLS for its public name and gets the
//! certificate itself, so a forwarder in front (`hq edge`) only ever passes encrypted bytes.
//! Threat model, limits and operation: `docs/security/CONNECTOR_TLS.md`.
//!
//! Off by default (`mcp_connector.tls.enabled`), refused under profile `lite` like the connector
//! itself. This module takes streams and serves them; it opens no listener. Whoever owns the
//! stream source calls [`ConnectorTls::serve`] per connection and spawns [`ConnectorTls::run`]
//! once.

mod acceptor;
mod acme;
mod certs;
mod gate;
mod listener;
mod renew;
mod store;
#[cfg(test)]
pub(crate) mod tests;

pub use acceptor::{ConnectorAcceptor, MAX_CONNECTIONS, Served, Timeouts};
pub use gate::{
    CONNECTOR_PATHS, is_connector_path, is_connector_territory, refuse_connector_paths,
};
pub(crate) use gate::{STRIPPED_HEADERS, not_found};
pub(crate) use listener::spawn_door;

use crate::WsState;
use acme::{AcmeHttp, AcmeIssuer, AcmeSettings, Fresh, issued_from_bundle};
use axum::Router;
use certs::{ChallengeSlot, Issued, LiveCert};
use gate::Gate;
use hq_core::config::{ConnectorTlsSettings, McpConnectorConfig};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use store::PrivateDir;
use tokio::io::{AsyncRead, AsyncWrite};

/// The ALPN protocol a CA validates TLS-ALPN-01 with (RFC 8737).
pub(crate) const ACME_TLS_ALPN: &[u8] = b"acme-tls/1";

/// Why something on this door failed. Nothing here carries a secret, so every variant may be
/// logged and shown to the owner.
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("TLS configuration: {0}")]
    Config(String),
    #[error("certificate: {0}")]
    Certificate(String),
    #[error("private store: {0}")]
    Store(String),
    #[error("too many connections")]
    Busy,
    #[error("timed out waiting for {0}")]
    Timeout(&'static str),
    #[error("TLS handshake: {0}")]
    Handshake(String),
    #[error("refused: {0}")]
    Refused(&'static str),
    #[error("HTTP: {0}")]
    Http(String),
    #[error("ACME: {0}")]
    Acme(String),
    /// The CA's rate limit; the certificate in use, if any, is kept.
    #[error("the CA's rate limit was reached: {detail}")]
    RateLimited {
        retry_after: Option<Duration>,
        detail: String,
    },
}

/// Where the certificate stands, for `hq connector status` and the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TlsState {
    /// No certificate yet; one is being asked for.
    NoCertificate,
    Valid,
    /// A renewal is under way; the current certificate keeps serving.
    Renewing,
    /// The CA's limit is holding the next order; the current certificate keeps serving.
    RateLimited,
    /// The last order failed; it is tried again after a wait.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TlsStatus {
    pub state: TlsState,
    pub host: String,
    /// Unix seconds.
    pub not_after: Option<i64>,
    /// Unix seconds: the earliest the next order may start.
    pub next_attempt_at: Option<i64>,
    pub last_error: Option<String>,
}

struct Progress {
    state: TlsState,
    validity: Option<(i64, i64)>,
    not_before_retry: Option<i64>,
    failures: u32,
    last_error: Option<String>,
}

/// The connector's TLS door: the acceptor and the machinery that keeps its certificate fresh.
pub struct ConnectorTls {
    host: String,
    directory_url: String,
    acceptor: ConnectorAcceptor,
    live: Arc<LiveCert>,
    #[cfg(test)]
    challenge: Arc<ChallengeSlot>,
    issuer: AcmeIssuer,
    /// One order at a time: they share the challenge slot.
    order_lock: tokio::sync::Mutex<()>,
    store: PrivateDir,
    progress: Mutex<Progress>,
}

/// Everything `ConnectorTls` needs, spelled out so tests can point it at a test CA.
pub(crate) struct Parts {
    pub acme: AcmeSettings,
    pub http: AcmeHttp,
    pub store: PrivateDir,
    pub gate: Gate,
    pub router: Router,
}

impl ConnectorTls {
    /// The door for this server, or `None` when `mcp_connector.tls` is off. `router` is the
    /// one already built for the main listener; building a second would start the server's
    /// background watchers twice.
    pub fn for_state(
        state: &Arc<WsState>,
        router: Router,
        cfg: &McpConnectorConfig,
    ) -> Result<Option<Arc<Self>>, String> {
        let Some(tls) = cfg.tls_settings(state.web_auth_token.as_deref())? else {
            return Ok(None);
        };
        if state.profile().is_lite() {
            return Err("mcp_connector.tls is not available under profile: lite".into());
        }
        let connector = state
            .connector
            .as_ref()
            .ok_or("mcp_connector.tls needs the connector to be on and valid")?;
        if connector.public != format!("https://{}", tls.authority) {
            return Err("mcp_connector.tls is for a different origin than the connector".into());
        }
        if connector.consent == connector.public {
            return Err(
                "mcp_connector.tls needs authorize_url: the consent page is never served on the \
                 public name this door answers on"
                    .into(),
            );
        }
        let checking = state.clone();
        let live_grant = Arc::new(move |token: &str| {
            checking
                .db
                .with_conn(|c| {
                    hq_db::mcp_connector::grant_for_access_token(c, token, crate::oauth::now())
                })
                .ok()
                .flatten()
                .is_some()
        });
        let http = AcmeHttp::new(AcmeHttp::default_client().map_err(|e| e.to_string())?);
        let parts = Parts {
            acme: acme_settings(&tls),
            http,
            store: PrivateDir::under_vault(&state.vault_path),
            gate: Gate::new(&tls.authority, live_grant),
            router,
        };
        Self::from_parts(parts)
            .map(|t| Some(Arc::new(t)))
            .map_err(|e| e.to_string())
    }

    pub(crate) fn from_parts(parts: Parts) -> Result<Self, TlsError> {
        let host = parts.acme.host.to_ascii_lowercase();
        let directory_url = parts.acme.directory_url.clone();
        let live = Arc::new(LiveCert::new(&host));
        let challenge = Arc::new(ChallengeSlot::new(&host));
        let acceptor = ConnectorAcceptor::new(
            &host,
            live.clone(),
            challenge.clone(),
            parts.gate,
            parts.router,
        )?;
        let issuer = AcmeIssuer::new(
            parts.acme,
            parts.http,
            parts.store.clone(),
            challenge.clone(),
        );
        let this = Self {
            host,
            directory_url,
            acceptor,
            live,
            #[cfg(test)]
            challenge,
            issuer,
            order_lock: tokio::sync::Mutex::new(()),
            store: parts.store,
            progress: Mutex::new(Progress {
                state: TlsState::NoCertificate,
                validity: None,
                not_before_retry: None,
                failures: 0,
                last_error: None,
            }),
        };
        this.load_stored();
        Ok(this)
    }

    /// Serves one connection from the forwarder to its end. The caller spawns it per stream.
    pub async fn serve<S>(&self, stream: S) -> Result<Served, TlsError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        self.acceptor.serve(stream).await
    }

    #[cfg(test)]
    pub(crate) fn challenge_slot(&self) -> &ChallengeSlot {
        &self.challenge
    }

    #[cfg(test)]
    pub(crate) fn acceptor(&self) -> &ConnectorAcceptor {
        &self.acceptor
    }

    #[cfg(test)]
    pub(crate) fn acceptor_mut(&mut self) -> &mut ConnectorAcceptor {
        &mut self.acceptor
    }

    /// Keeps the certificate fresh for as long as it runs: orders one when there is none, and
    /// again two thirds of the way through each. Needs the acceptor taking connections, since
    /// the CA's validation arrives on it. Never returns.
    pub async fn run(&self) {
        loop {
            let (cert, retry) = {
                let p = self.progress.lock().unwrap_or_else(|e| e.into_inner());
                (p.validity, p.not_before_retry)
            };
            match renew::decide(now(), cert, retry) {
                renew::Next::Now => self.order_now().await,
                renew::Next::Wait(d) => tokio::time::sleep(d).await,
            }
        }
    }

    /// Orders a certificate now and installs it. A failure leaves the certificate in use, if
    /// there is one, and sets the wait before the next try.
    pub async fn order_now(&self) {
        let _one_at_a_time = self.order_lock.lock().await;
        self.begin_order();
        let outcome = self.issuer.issue().await;
        self.finish_order(outcome);
    }

    fn finish_order(&self, outcome: Result<Fresh, TlsError>) {
        // Disk first and outside the lock: a save is not a reason to hold up `status()`.
        if let Ok(fresh) = &outcome {
            let path = self.store.issued_path(&self.host, &self.directory_url);
            if let Err(e) = self.store.write(&path, fresh.bundle.expose()) {
                // Serving matters more than remembering: the next start orders again.
                tracing::warn!("connector tls: the new certificate was not saved: {e}");
            }
        }
        let remembered = {
            let mut p = self.progress.lock().unwrap_or_else(|e| e.into_inner());
            match outcome {
                Ok(fresh) => {
                    let Issued {
                        key,
                        not_before,
                        not_after,
                    } = fresh.issued;
                    p.validity = Some((not_before, not_after));
                    self.live.install(key);
                    p.state = TlsState::Valid;
                    p.failures = 0;
                    p.not_before_retry = renew::hold_after_success(now(), not_before, not_after);
                    p.last_error = None;
                    tracing::info!(host = %self.host, not_after, "connector tls: certificate installed");
                }
                Err(TlsError::RateLimited {
                    retry_after,
                    detail,
                }) => {
                    let wait = renew::rate_limit_wait(retry_after);
                    p.state = TlsState::RateLimited;
                    p.not_before_retry = Some(now() + wait_secs(wait));
                    p.last_error = Some(one_line(&format!(
                        "the CA's rate limit was reached: {detail}"
                    )));
                    tracing::warn!(host = %self.host, wait_secs = wait.as_secs(), "connector tls: rate limited");
                }
                Err(e) => {
                    p.failures = p.failures.saturating_add(1);
                    let wait = renew::backoff(p.failures);
                    p.state = TlsState::Failed;
                    p.not_before_retry = Some(now() + wait_secs(wait));
                    p.last_error = Some(one_line(&e.to_string()));
                    tracing::warn!(host = %self.host, wait_secs = wait.as_secs(), "connector tls: order failed: {e}");
                }
            }
            Remembered::of(&p)
        };
        self.remember(&remembered);
    }

    /// Keeps the wait and the failure count across a restart, so a process that starts over and
    /// over (a phone's service being restarted) does not order each time and trip the CA's limits.
    fn remember(&self, remembered: &Remembered) {
        let path = self.store.retry_path(&self.host, &self.directory_url);
        if remembered.failures == 0 && remembered.not_before_retry.is_none() {
            self.store.remove(&path);
            return;
        }
        let saved = serde_json::to_vec(remembered)
            .map_err(|e| TlsError::Store(format!("the order state: {e}")))
            .and_then(|json| self.store.write(&path, &json));
        if let Err(e) = saved {
            tracing::warn!("connector tls: the order state was not saved: {e}");
        }
    }

    /// While an order runs, the state says a renewal is under way if a certificate is serving.
    fn begin_order(&self) {
        let mut p = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        if p.validity.is_some() {
            p.state = TlsState::Renewing;
        }
    }

    pub fn status(&self) -> TlsStatus {
        let p = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        TlsStatus {
            state: p.state,
            host: self.host.clone(),
            not_after: p.validity.map(|(_, to)| to),
            next_attempt_at: p.not_before_retry,
            last_error: p.last_error.clone(),
        }
    }

    /// Picks up what a previous run saved: the certificate, so a restart serves at once instead
    /// of ordering again (the CA allows five certificates a week for one name), and the wait
    /// after a failure or a rate limit.
    fn load_stored(&self) {
        let path = self.store.issued_path(&self.host, &self.directory_url);
        let loaded = self.store.read(&path).and_then(|saved| {
            saved
                .map(|s| issued_from_bundle(&s, &self.host))
                .transpose()
        });
        let mut p = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        match loaded {
            Ok(Some(Issued {
                key,
                not_before,
                not_after,
            })) if not_after > now() => {
                self.live.install(key);
                p.validity = Some((not_before, not_after));
                p.state = TlsState::Valid;
            }
            // An expired certificate is of no use to a client; an order is placed at once.
            Ok(Some(_)) => tracing::info!("connector tls: the saved certificate has expired"),
            Ok(None) => {}
            Err(e) => tracing::warn!("connector tls: the saved certificate is unusable: {e}"),
        }
        let retry = self.store.retry_path(&self.host, &self.directory_url);
        let remembered = self
            .store
            .read(&retry)
            .ok()
            .flatten()
            .and_then(|saved| serde_json::from_slice::<Remembered>(saved.expose()).ok());
        if let Some(r) = remembered {
            // The count outlives the wait: a process restarted every few minutes still backs
            // off further each time instead of starting over.
            p.failures = r.failures.min(MAX_REMEMBERED_FAILURES);
            let wait = r
                .not_before_retry
                .filter(|t| *t > now() && *t - now() <= renew::max_wait_secs());
            if wait.is_some() {
                p.not_before_retry = wait;
                p.last_error = r.last_error.map(|e| one_line(&e));
                p.state = if r.rate_limited {
                    TlsState::RateLimited
                } else if r.failures > 0 {
                    TlsState::Failed
                } else {
                    p.state
                };
            }
        }
    }
}

/// A count read from disk is capped, so a damaged file cannot make the next wait absurd.
const MAX_REMEMBERED_FAILURES: u32 = 1_000;

/// What survives a restart of the order loop.
#[derive(Serialize, serde::Deserialize)]
struct Remembered {
    failures: u32,
    not_before_retry: Option<i64>,
    rate_limited: bool,
    last_error: Option<String>,
}

impl Remembered {
    fn of(p: &Progress) -> Self {
        Self {
            failures: p.failures,
            not_before_retry: p.not_before_retry,
            rate_limited: p.state == TlsState::RateLimited,
            last_error: p.last_error.clone(),
        }
    }
}

fn acme_settings(tls: &ConnectorTlsSettings) -> AcmeSettings {
    AcmeSettings {
        host: tls.host.clone(),
        directory_url: tls.directory_url.clone(),
        contact_email: tls.contact_email.clone(),
        // `tls_settings` refuses a configuration that has not agreed.
        agreed_to_terms: true,
    }
}

/// What the CA said, as one short line fit for a status screen and a small file.
fn one_line(text: &str) -> String {
    const MAX_CHARS: usize = 300;
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_CHARS)
        .collect();
    flat.trim().to_string()
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn wait_secs(wait: Duration) -> i64 {
    i64::try_from(wait.as_secs()).unwrap_or(i64::MAX)
}
