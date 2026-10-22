//! The household `iroh-relay`, hosted inside the edge process.
//!
//! The relay ends its own TLS. It binds loopback only, and the edge's one public listener routes
//! the relay's server name to it, so a Let's Encrypt TLS-ALPN-01 challenge for that name arrives on
//! port 443 like any other connection and reaches the relay's ACME code. The relay admits the edge
//! and registered devices and nobody else, so it is not an open relay.

use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use iroh::{EndpointId, RelayUrl};
use iroh_relay::server::{
    Access, AccessControl, AcmeConfig, CertConfig, ClientRequest, ConnectionId, QuicConfig,
    RelayConfig, Server, ServerConfig, TlsConfig,
};
use tokio::task::JoinHandle;

use crate::registry::Registry;

const LOOPBACK: Ipv4Addr = Ipv4Addr::LOCALHOST;
/// How often relay sessions of devices that have left the registry are cut.
const REVOKE_SWEEP: Duration = Duration::from_secs(1);

/// Endpoints with a live relay session, and how many sessions each has.
type Admitted = Arc<Mutex<HashMap<EndpointId, usize>>>;

/// How the relay gets its certificate.
pub enum RelayTls {
    /// Let's Encrypt, answered through the edge's own listener. What the config selects.
    LetsEncrypt {
        email: String,
        production: bool,
        cache_dir: PathBuf,
    },
    /// No TLS: the relay speaks plain HTTP on loopback. For tests and local development only;
    /// no configuration reaches this, because the edge forwards TLS and nothing else to the relay.
    PlainHttp,
}

pub struct RelayParams {
    /// The server name the edge routes to the relay.
    pub hostname: String,
    pub tls: RelayTls,
    /// UDP address for QUIC address discovery; needs TLS.
    pub quic_bind: Option<SocketAddr>,
}

/// A running relay and the loopback address the edge forwards its connections to.
pub struct RelayHost {
    _server: Option<Server>,
    hostname: String,
    target: SocketAddr,
    url: RelayUrl,
    denied: Arc<AtomicU64>,
    admitted: Admitted,
    sweeper: Option<JoinHandle<()>>,
}

impl Drop for RelayHost {
    fn drop(&mut self) {
        if let Some(sweeper) = &self.sweeper {
            sweeper.abort();
        }
    }
}

/// Chooses ring as the process-wide rustls provider, once. The relay's ACME code
/// (`tokio-rustls-acme`) builds a `ServerConfig` with the process default, which rustls resolves by
/// itself only when exactly one provider is compiled in. A build that includes `hq-web` has two
/// (it pulls in aws-lc-rs), so without this the relay panics at start-up. If the process already
/// installed a provider, that one stays: either works for ACME.
fn ensure_default_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

impl RelayHost {
    /// Starts the relay. `edge_id` is the edge's own endpoint, which it must be able to use to
    /// reach devices; everyone else must be in `registry`.
    pub async fn spawn(
        params: RelayParams,
        registry: Arc<dyn Registry>,
        edge_id: EndpointId,
    ) -> Result<Self> {
        ensure_default_crypto_provider();
        let denied = Arc::new(AtomicU64::new(0));
        let admitted = Admitted::default();
        let mut relay = RelayConfig::new((LOOPBACK, 0));
        relay.access = Arc::new(RegisteredOnly {
            registry: registry.clone(),
            edge_id,
            denied: denied.clone(),
            admitted: admitted.clone(),
        });
        let mut config = ServerConfig::default();
        match params.tls {
            RelayTls::LetsEncrypt {
                email,
                production,
                cache_dir,
            } => {
                crate::identity::ensure_private_dir(&cache_dir)?;
                let acme = AcmeConfig::letsencrypt(production)
                    .domains(vec![params.hostname.clone()])
                    .contact(vec![format!("mailto:{email}")])
                    .cache_path(cache_dir);
                let builder = rustls::ServerConfig::builder_with_provider(Arc::new(
                    rustls::crypto::ring::default_provider(),
                ))
                .with_safe_default_protocol_versions()
                .context("the relay's TLS versions")?
                .with_no_client_auth();
                relay.tls = Some(TlsConfig::new(
                    (LOOPBACK, 0),
                    CertConfig::LetsEncrypt {
                        acme_config: acme,
                        server_config_builder: builder,
                    },
                ));
                config.quic = params.quic_bind.map(QuicConfig::new);
            }
            RelayTls::PlainHttp => {}
        }
        // No metrics listener: the edge has one public listener and no admin surface.
        config.metrics_addr = None;
        config.relay = Some(relay);
        let server = Server::spawn(config)
            .await
            .map_err(|e| anyhow!("starting the relay: {e}"))?;
        let (target, url) = match (server.https_addr(), server.http_addr()) {
            (Some(https), _) => (
                https,
                format!("https://{}", params.hostname).parse::<RelayUrl>()?,
            ),
            (None, Some(http)) => (http, format!("http://{http}").parse::<RelayUrl>()?),
            (None, None) => return Err(anyhow!("the relay bound no address")),
        };
        let sweeper = server.relay_service().cloned().map(|service| {
            tokio::spawn(sweep_revoked(service, registry, edge_id, admitted.clone()))
        });
        Ok(Self {
            _server: Some(server),
            hostname: params.hostname,
            target,
            url,
            denied,
            admitted,
            sweeper,
        })
    }

    /// Stands in for a relay that is already running at `target`, for tests of the edge's routing
    /// and caps. It serves nothing itself.
    #[doc(hidden)]
    pub fn at(hostname: &str, target: SocketAddr, url: RelayUrl) -> Self {
        Self {
            _server: None,
            hostname: hostname.to_owned(),
            target,
            url,
            denied: Arc::default(),
            admitted: Admitted::default(),
            sweeper: None,
        }
    }

    /// Whether `id` has a live session on the relay.
    pub fn is_connected(&self, id: &EndpointId) -> bool {
        self.admitted
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(id)
    }

    /// The server name the edge routes to this relay.
    pub fn hostname(&self) -> &str {
        &self.hostname
    }

    /// Where the edge forwards connections for [`Self::hostname`].
    pub fn target(&self) -> SocketAddr {
        self.target
    }

    /// The URL devices use as their home relay.
    pub fn url(&self) -> &RelayUrl {
        &self.url
    }

    /// How many relay connections were refused because the endpoint is not registered.
    pub fn denied(&self) -> u64 {
        self.denied.load(Ordering::Relaxed)
    }
}

/// Cuts the relay sessions of devices that were removed from the registry, so revoking a device
/// ends its relay session too and not only its edge connections.
async fn sweep_revoked(
    service: iroh_relay::server::RelayService,
    registry: Arc<dyn Registry>,
    edge_id: EndpointId,
    admitted: Admitted,
) {
    loop {
        tokio::time::sleep(REVOKE_SWEEP).await;
        if !registry.is_readable() {
            continue;
        }
        let stale: Vec<EndpointId> = admitted
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .filter(|id| **id != edge_id && !registry.is_registered(id))
            .copied()
            .collect();
        for id in stale {
            service.clients().disconnect(id, None);
        }
    }
}

/// Admits the edge and registered devices.
#[derive(Debug)]
struct RegisteredOnly {
    registry: Arc<dyn Registry>,
    edge_id: EndpointId,
    denied: Arc<AtomicU64>,
    admitted: Admitted,
}

impl std::fmt::Debug for dyn Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Registry")
    }
}

impl AccessControl for RegisteredOnly {
    async fn on_connect(&self, request: &ClientRequest) -> Access {
        let id = request.endpoint_id();
        if id == self.edge_id || self.registry.is_registered(&id) {
            *self
                .admitted
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(id)
                .or_insert(0) += 1;
            return Access::Allow;
        }
        self.denied.fetch_add(1, Ordering::Relaxed);
        Access::Deny { reason: None }
    }

    fn on_disconnect(&self, endpoint_id: EndpointId, _connection_id: ConnectionId) {
        let mut admitted = self.admitted.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = admitted.get_mut(&endpoint_id) {
            *count -= 1;
            if *count == 0 {
                admitted.remove(&endpoint_id);
            }
        }
    }
}
