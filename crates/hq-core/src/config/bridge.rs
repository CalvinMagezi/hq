use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, net::SocketAddr};

use super::mcp_connector::{DoorListen, DoorTarget};

/// Longest service name a stream may ask for.
pub const MAX_SERVICE_NAME: usize = 32;

/// What this machine offers its paired devices over the bridge, and which of theirs it reaches
/// through local ports. Empty by default: nothing is offered and nothing is forwarded.
/// Docs: `docs/PAIRING.md` ("Services and forwards").
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BridgeServicesConfig {
    /// Where the web server opens a second door for paired devices: a loopback `ip:port` that
    /// serves the web app, `/api`, `/ws` and `/mcp`, and never the connector's OAuth paths. Unset:
    /// no such door. The web server refuses to open it unless the web token is set.
    #[serde(default)]
    pub web_listen: Option<String>,

    /// Named services a paired device may open, each to a fixed loopback `ip:port` or an absolute
    /// unix socket path on this machine. The device names a service; it never names an address.
    #[serde(default)]
    pub services: BTreeMap<String, String>,

    /// Local ports that reach a paired device's service.
    #[serde(default)]
    pub forwards: Vec<BridgeForward>,
}

/// One local port that reaches a service on a paired device.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BridgeForward {
    /// The paired device's machine id (`hq bridge peers`).
    pub peer: String,
    /// The service name on that device.
    pub service: String,
    /// The loopback `ip:port` this machine listens on.
    pub listen: String,
}

/// A forward with its address checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedForward {
    pub peer: String,
    pub service: String,
    pub listen: SocketAddr,
}

/// A service name is short lowercase ASCII, so it never needs escaping in logs or on the wire.
pub fn valid_service_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_SERVICE_NAME
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn loopback_port(text: &str, what: &str) -> Result<SocketAddr, String> {
    match DoorListen::parse(Some(text)) {
        Ok(DoorListen::Tcp(addr)) => Ok(addr),
        Ok(_) => Err(format!("{what}: {text:?} must be a loopback ip:port")),
        Err(e) => Err(format!("{what}: {e}")),
    }
}

impl BridgeServicesConfig {
    /// The paired-devices web door's address, checked.
    pub fn web_listen(&self) -> Result<Option<SocketAddr>, String> {
        self.web_listen
            .as_deref()
            .map(|t| loopback_port(t, "bridge.web_listen"))
            .transpose()
    }

    /// The services, checked: valid names, and each target a loopback port or a socket path.
    pub fn services(&self) -> Result<BTreeMap<String, DoorTarget>, String> {
        let mut out = BTreeMap::new();
        for (name, target) in &self.services {
            if !valid_service_name(name) {
                return Err(format!(
                    "bridge.services: {name:?} is not a service name (lowercase letters, digits and -, at most {MAX_SERVICE_NAME})"
                ));
            }
            let target = match DoorListen::parse(Some(target)) {
                Ok(DoorListen::Default) => {
                    return Err(format!("bridge.services.{name}: an address is required"));
                }
                Ok(DoorListen::Tcp(addr)) => DoorTarget::Tcp(addr),
                Ok(DoorListen::Unix(path)) => DoorTarget::Unix(path),
                Err(e) => return Err(format!("bridge.services.{name}: {e}")),
            };
            out.insert(name.clone(), target);
        }
        Ok(out)
    }

    /// The forwards, checked: valid service names, loopback listen ports, no port used twice.
    pub fn forwards(&self) -> Result<Vec<CheckedForward>, String> {
        let mut out: Vec<CheckedForward> = Vec::new();
        for f in &self.forwards {
            if !valid_service_name(&f.service) {
                return Err(format!(
                    "bridge.forwards: {:?} is not a service name",
                    f.service
                ));
            }
            if f.peer.trim().is_empty() {
                return Err("bridge.forwards: peer is required".into());
            }
            let listen = loopback_port(&f.listen, "bridge.forwards")?;
            if out.iter().any(|o| o.listen == listen) {
                return Err(format!("bridge.forwards: {listen} is listed twice"));
            }
            out.push(CheckedForward {
                peer: f.peer.trim().to_string(),
                service: f.service.clone(),
                listen,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> BridgeServicesConfig {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn empty_by_default() {
        let c = BridgeServicesConfig::default();
        assert_eq!(c.web_listen().unwrap(), None);
        assert!(c.services().unwrap().is_empty());
        assert!(c.forwards().unwrap().is_empty());
    }

    #[test]
    fn services_point_at_loopback_ports_or_socket_paths_only() {
        let ok = parse("services:\n  web: 127.0.0.1:5690\n  gate: /run/hq/gate.sock\n");
        let s = ok.services().unwrap();
        assert_eq!(s["web"], DoorTarget::Tcp("127.0.0.1:5690".parse().unwrap()));
        assert_eq!(s["gate"], DoorTarget::Unix("/run/hq/gate.sock".into()));
        for bad in [
            "services:\n  web: 0.0.0.0:5690\n",
            "services:\n  web: 192.168.1.2:80\n",
            "services:\n  web: 127.0.0.1:0\n",
            "services:\n  web: relative.sock\n",
            "services:\n  web: \"\"\n",
            "services:\n  Web: 127.0.0.1:5690\n",
            "services:\n  \"a b\": 127.0.0.1:5690\n",
        ] {
            assert!(parse(bad).services().is_err(), "{bad}");
        }
    }

    #[test]
    fn forwards_listen_on_loopback_once_each() {
        let ok = parse("forwards:\n  - peer: abc\n    service: web\n    listen: 127.0.0.1:47480\n");
        assert_eq!(
            ok.forwards().unwrap()[0].listen,
            "127.0.0.1:47480".parse().unwrap()
        );
        for bad in [
            "forwards:\n  - peer: abc\n    service: web\n    listen: 0.0.0.0:47480\n",
            "forwards:\n  - peer: abc\n    service: web\n    listen: /tmp/x.sock\n",
            "forwards:\n  - peer: \" \"\n    service: web\n    listen: 127.0.0.1:1\n",
            "forwards:\n  - peer: a\n    service: WEB\n    listen: 127.0.0.1:1\n",
            "forwards:\n  - {peer: a, service: web, listen: \"127.0.0.1:1\"}\n  - {peer: b, service: gate, listen: \"127.0.0.1:1\"}\n",
        ] {
            assert!(parse(bad).forwards().is_err(), "{bad}");
        }
    }

    #[test]
    fn the_web_door_is_a_loopback_port() {
        assert!(parse("web_listen: 127.0.0.1:5690\n").web_listen().is_ok());
        assert!(parse("web_listen: 0.0.0.0:5690\n").web_listen().is_err());
        assert!(parse("web_listen: /tmp/web.sock\n").web_listen().is_err());
    }

    #[test]
    fn unknown_keys_are_refused() {
        assert!(serde_yaml::from_str::<BridgeServicesConfig>("servics: {}\n").is_err());
    }
}
