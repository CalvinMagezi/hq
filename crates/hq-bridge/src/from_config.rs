//! The services a machine offers and the forwards it opens, built from `bridge:` in the HQ config.
//! `hq bridge serve` and the desktop app both start a bridge from the same config, so the checks
//! live here once.

use crate::{Bridge, DoorTarget, ForwardListener, Services, run_forward};
use anyhow::{Context, Result, anyhow, bail};
use hq_core::config::{BridgeServicesConfig, DoorTarget as ConfigTarget};
use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use tokio::task::JoinHandle;

/// The services in `bridge.services`, checked, or `None` when there are none. A service may not
/// lead to the main web port (`ws_port`): paired devices get the web app through
/// `bridge.web_listen`, the door that keeps the connector's paths and credentials out.
pub fn services_from_config(
    config: &BridgeServicesConfig,
    ws_port: u16,
) -> Result<Option<Services>> {
    let checked = config.services().map_err(|e| anyhow!(e))?;
    if checked.is_empty() {
        return Ok(None);
    }
    let mut out = BTreeMap::new();
    let mut peers = BTreeMap::new();
    for (name, service) in checked {
        if let Some(ids) = service.peers {
            let ids = ids
                .iter()
                .map(|id| id.parse::<iroh::EndpointId>())
                .collect::<Result<BTreeSet<_>, _>>()
                .with_context(|| format!("bridge.services.{name}.peers"))?;
            peers.insert(name.clone(), ids);
        }
        let target = match service.target {
            ConfigTarget::Unix(path) => DoorTarget::Unix(path),
            ConfigTarget::Tcp(addr) if addr.port() == ws_port => bail!(
                "bridge.services.{name} leads to the web server's own port ({ws_port}); point it at bridge.web_listen"
            ),
            ConfigTarget::Tcp(addr) => DoorTarget::Tcp(addr),
        };
        out.insert(name, target);
    }
    Ok(Some(Services {
        peers,
        ..Services::new(out)
    }))
}

/// One configured forward and whether this process listens for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardStatus {
    pub peer: String,
    pub service: String,
    pub listen: ConfigTarget,
    /// The port or socket is already taken by something else, typically a `hq bridge serve` on
    /// this machine that opened the same forward. Nothing was started for it here.
    pub running_elsewhere: bool,
}

/// The forwards a bridge opened. Each is a task that lives until [`Forwards::abort`] ends it.
pub struct Forwards {
    pub status: Vec<ForwardStatus>,
    tasks: Vec<JoinHandle<()>>,
}

impl Forwards {
    /// Stops listening on every forward this process opened and frees their ports. A socket file
    /// stays behind; the next start replaces it as stale.
    pub fn abort(&self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Binds a forward's local end. A socket's directory is made private when it is missing.
async fn bind_forward(listen: &ConfigTarget) -> std::io::Result<ForwardListener> {
    match listen {
        ConfigTarget::Tcp(addr) => Ok(tokio::net::TcpListener::bind(addr).await?.into()),
        #[cfg(unix)]
        ConfigTarget::Unix(path) => {
            Ok(hq_core::fs_private::bind_private_socket(path, true)?.into())
        }
        #[cfg(not(unix))]
        ConfigTarget::Unix(_) => Err(std::io::Error::other(
            "a socket forward needs a unix machine; listen on a loopback ip:port",
        )),
    }
}

/// Opens every forward in `bridge.forwards` on `bridge`. A port or socket that is taken is
/// reported as running elsewhere; any other bind failure is an error.
pub async fn start_forwards(bridge: &Bridge, config: &BridgeServicesConfig) -> Result<Forwards> {
    let mut forwards = Forwards {
        status: Vec::new(),
        tasks: Vec::new(),
    };
    for f in config.forwards().map_err(|e| anyhow!(e))? {
        let mut status = ForwardStatus {
            peer: f.peer.clone(),
            service: f.service.clone(),
            listen: f.listen.clone(),
            running_elsewhere: false,
        };
        match bind_forward(&f.listen).await {
            Ok(listener) => forwards.tasks.push(tokio::spawn(run_forward(
                bridge.dialer(),
                f.peer,
                f.service,
                listener,
                f.fail_fast,
            ))),
            Err(e) if e.kind() == ErrorKind::AddrInUse => status.running_elsewhere = true,
            Err(e) => {
                forwards.abort();
                return Err(e).with_context(|| {
                    format!("bridge.forwards: could not listen on {}", status.listen)
                });
            }
        }
        forwards.status.push(status);
    }
    Ok(forwards)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(services: &[(&str, &str)]) -> BridgeServicesConfig {
        BridgeServicesConfig {
            services: services
                .iter()
                .map(|(k, v)| (k.to_string(), (*v).into()))
                .collect(),
            ..BridgeServicesConfig::default()
        }
    }

    #[test]
    fn a_service_may_not_lead_to_the_main_web_port() {
        assert!(services_from_config(&config(&[]), 5678).unwrap().is_none());
        let web = |target| services_from_config(&config(&[("web", target)]), 5678);
        assert!(web("127.0.0.1:5678").is_err());
        assert!(web("127.0.0.1:5690").unwrap().is_some());
        assert!(web("0.0.0.0:5690").is_err());
    }

    #[test]
    fn a_peers_list_becomes_the_set_of_machines_that_may_open_the_service() {
        let id = iroh::SecretKey::generate().public();
        let mut config = config(&[("web", "127.0.0.1:5690")]);
        config.services.insert(
            "llm".into(),
            hq_core::config::ServiceEntry::Limited(hq_core::config::LimitedService {
                to: "127.0.0.1:11434".into(),
                peers: vec![id.to_string()],
            }),
        );
        let services = services_from_config(&config, 5678).unwrap().unwrap();
        assert_eq!(services.peers.len(), 1, "web stays open to every peer");
        assert_eq!(services.peers["llm"], BTreeSet::from([id]));
        services.validate().unwrap();
    }
}
