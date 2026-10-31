//! The services a machine offers and the forwards it opens, built from `bridge:` in the HQ config.
//! `hq bridge serve` and the desktop app both start a bridge from the same config, so the checks
//! live here once.

use crate::{Bridge, DoorTarget, Services, run_forward};
use anyhow::{Context, Result, anyhow, bail};
use hq_core::config::{BridgeServicesConfig, DoorTarget as ConfigTarget};
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::net::SocketAddr;
use tokio::task::JoinHandle;

/// The services in `bridge.services`, checked, or `None` when there are none. A service may not
/// lead to the main web port (`ws_port`): paired devices get the web app through
/// `bridge.web_listen`, the door that keeps the connector's paths and credentials out.
pub fn services_from_config(
    config: &BridgeServicesConfig,
    ws_port: u16,
) -> Result<Option<Services>> {
    let targets = config.services().map_err(|e| anyhow!(e))?;
    if targets.is_empty() {
        return Ok(None);
    }
    let mut out = BTreeMap::new();
    for (name, target) in targets {
        let target = match target {
            ConfigTarget::Unix(path) => DoorTarget::Unix(path),
            ConfigTarget::Tcp(addr) if addr.port() == ws_port => bail!(
                "bridge.services.{name} leads to the web server's own port ({ws_port}); point it at bridge.web_listen"
            ),
            ConfigTarget::Tcp(addr) => DoorTarget::Tcp(addr),
        };
        out.insert(name, target);
    }
    Ok(Some(Services::new(out)))
}

/// One configured forward and whether this process listens for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardStatus {
    pub peer: String,
    pub service: String,
    pub listen: SocketAddr,
    /// The port is already taken by something else, typically a `hq bridge serve` on this machine
    /// that opened the same forward. Nothing was started for it here.
    pub running_elsewhere: bool,
}

/// The forwards a bridge opened. Each is a task that lives until [`Forwards::abort`] ends it.
pub struct Forwards {
    pub status: Vec<ForwardStatus>,
    tasks: Vec<JoinHandle<()>>,
}

impl Forwards {
    /// Stops listening on every forward this process opened and frees their ports.
    pub fn abort(&self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Opens every forward in `bridge.forwards` on `bridge`. A port that is taken is reported as
/// running elsewhere; any other bind failure is an error.
pub async fn start_forwards(bridge: &Bridge, config: &BridgeServicesConfig) -> Result<Forwards> {
    let mut forwards = Forwards {
        status: Vec::new(),
        tasks: Vec::new(),
    };
    for f in config.forwards().map_err(|e| anyhow!(e))? {
        let mut status = ForwardStatus {
            peer: f.peer.clone(),
            service: f.service.clone(),
            listen: f.listen,
            running_elsewhere: false,
        };
        match tokio::net::TcpListener::bind(f.listen).await {
            Ok(listener) => forwards.tasks.push(tokio::spawn(run_forward(
                bridge.dialer(),
                f.peer,
                f.service,
                listener,
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
                .map(|(k, v)| (k.to_string(), v.to_string()))
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
}
