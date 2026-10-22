//! The household `iroh-relay`, hosted inside the edge process.
//!
//! The relay ends its own TLS. It binds loopback only, and the edge's one public listener routes
//! the relay's server name to it, so a Let's Encrypt TLS-ALPN-01 challenge for that name arrives on
//! port 443 like any other connection and reaches the relay's ACME code. The relay admits the edge
//! and registered devices and nobody else, so it is not an open relay.

use std::{
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result, anyhow};
use iroh::{EndpointId, RelayUrl};
use iroh_relay::server::{
    Access, AccessControl, AcmeConfig, CertConfig, ClientRequest, QuicConfig, RelayConfig, Server,
    ServerConfig, TlsConfig,
};

use crate::registry::Registry;

const LOOPBACK: Ipv4Addr = Ipv4Addr::LOCALHOST;

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
    _server: Server,
    hostname: String,
    target: SocketAddr,
    url: RelayUrl,
    denied: Arc<AtomicU64>,
}

impl RelayHost {
    /// Starts the relay. `edge_id` is the edge's own endpoint, which it must be able to use to
    /// reach devices; everyone else must be in `registry`.
    pub async fn spawn(
        params: RelayParams,
        registry: Arc<dyn Registry>,
        edge_id: EndpointId,
    ) -> Result<Self> {
        let denied = Arc::new(AtomicU64::new(0));
        let mut relay = RelayConfig::new((LOOPBACK, 0));
        relay.access = Arc::new(RegisteredOnly {
            registry,
            edge_id,
            denied: denied.clone(),
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
        Ok(Self {
            _server: server,
            hostname: params.hostname,
            target,
            url,
            denied,
        })
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

/// Admits the edge and registered devices.
#[derive(Debug)]
struct RegisteredOnly {
    registry: Arc<dyn Registry>,
    edge_id: EndpointId,
    denied: Arc<AtomicU64>,
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
            return Access::Allow;
        }
        self.denied.fetch_add(1, Ordering::Relaxed);
        Access::Deny { reason: None }
    }
}
