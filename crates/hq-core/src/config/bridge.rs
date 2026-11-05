use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, net::SocketAddr};

use super::mcp_connector::{DoorListen, DoorTarget};

/// Longest service name a stream may ask for.
pub const MAX_SERVICE_NAME: usize = 32;

/// The service a hub offers for its web app, and the one a laptop offers for its coding-agent
/// host (`docs/PAIRING.md`, "Services and forwards").
pub const HUB_WEB_SERVICE: &str = "web";
pub const HUB_GATE_SERVICE: &str = "gate";

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
    pub services: BTreeMap<String, ServiceEntry>,

    /// Local ports that reach a paired device's service.
    #[serde(default)]
    pub forwards: Vec<BridgeForward>,
}

/// Where a service leads and who may open it: `name: addr` lets every paired device in,
/// `name: {to: addr, peers: [id, ...]}` only the devices listed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ServiceEntry {
    Addr(String),
    Limited(LimitedService),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LimitedService {
    pub to: String,
    /// The machine ids (`hq bridge peers`) that may open this service.
    pub peers: Vec<String>,
}

impl ServiceEntry {
    pub fn to(&self) -> &str {
        match self {
            Self::Addr(to) | Self::Limited(LimitedService { to, .. }) => to,
        }
    }
}

impl From<String> for ServiceEntry {
    fn from(to: String) -> Self {
        Self::Addr(to)
    }
}

impl From<&str> for ServiceEntry {
    fn from(to: &str) -> Self {
        Self::Addr(to.to_string())
    }
}

/// A service with its address and peers checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedService {
    pub target: DoorTarget,
    /// `None`: every paired device. Otherwise only these machine ids, lowercase hex.
    pub peers: Option<Vec<String>>,
}

/// A machine id as `hq bridge peers` prints it: 64 hex digits.
const MACHINE_ID_LEN: usize = 64;

fn machine_id(text: &str) -> Option<String> {
    let id = text.trim().to_ascii_lowercase();
    (id.len() == MACHINE_ID_LEN && id.bytes().all(|b| b.is_ascii_hexdigit())).then_some(id)
}

/// One local port that reaches a service on a paired device.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BridgeForward {
    /// The paired device's machine id (`hq bridge peers`).
    pub peer: String,
    /// The service name on that device.
    pub service: String,
    /// Where this machine listens: a loopback `ip:port`, which any local process can open, or an
    /// absolute unix socket path in a directory only this user can enter, which only this user's
    /// processes can open (use it on a phone, where every app shares the loopback).
    pub listen: String,
    /// Give up on the device after this many seconds, and while it stays unreachable close new
    /// connections at once, trying it again in the background. For a service with a fallback,
    /// such as a model the backend chain can replace with another; unset, a connection waits for
    /// the device as long as a dial may take.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fail_fast_secs: Option<u64>,
    /// `direct`: open new connections to the device only over a direct path (the same network,
    /// or a hole punched through), and end them at once while only a relay reaches it, so a
    /// prompt does not cross a metered link. The dial's handshake, keepalives and a stream already
    /// open when the path falls back may still use the relay. `any`, the default, takes whichever
    /// path there is.
    #[serde(default, skip_serializing_if = "ForwardWhen::is_any")]
    pub when: ForwardWhen,
}

/// Which paths a forward may use to reach its device.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ForwardWhen {
    #[default]
    Any,
    Direct,
}

impl ForwardWhen {
    pub fn is_any(&self) -> bool {
        *self == Self::Any
    }
}

/// Longest `fail_fast_secs`: past this a forward already waits that long without the option.
pub const MAX_FAIL_FAST_SECS: u64 = 20;

/// A forward with its address checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedForward {
    pub peer: String,
    pub service: String,
    pub listen: DoorTarget,
    pub fail_fast: Option<std::time::Duration>,
    pub direct_only: bool,
}

/// An empty list would shut the service to everyone, which is almost surely a mistake: say so.
fn checked_peers(name: &str, peers: &[String]) -> Result<Vec<String>, String> {
    if peers.is_empty() {
        return Err(format!(
            "bridge.services.{name}.peers: list at least one machine id, or use the `{name}: addr` form to let every paired device in"
        ));
    }
    peers
        .iter()
        .map(|p| {
            machine_id(p).ok_or_else(|| {
                format!("bridge.services.{name}.peers: {p:?} is not a machine id (64 hex digits, `hq bridge peers`)")
            })
        })
        .collect()
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

    /// The services, checked: valid names, each target a loopback port or a socket path, and each
    /// peers list a non-empty list of machine ids.
    pub fn services(&self) -> Result<BTreeMap<String, CheckedService>, String> {
        let mut out = BTreeMap::new();
        for (name, entry) in &self.services {
            if !valid_service_name(name) {
                return Err(format!(
                    "bridge.services: {name:?} is not a service name (lowercase letters, digits and -, at most {MAX_SERVICE_NAME})"
                ));
            }
            let target = match DoorListen::parse(Some(entry.to())) {
                Ok(DoorListen::Default) => {
                    return Err(format!("bridge.services.{name}: an address is required"));
                }
                Ok(DoorListen::Tcp(addr)) => DoorTarget::Tcp(addr),
                Ok(DoorListen::Unix(path)) => DoorTarget::Unix(path),
                Err(e) => return Err(format!("bridge.services.{name}: {e}")),
            };
            let peers = match entry {
                ServiceEntry::Addr(_) => None,
                ServiceEntry::Limited(limited) => Some(checked_peers(name, &limited.peers)?),
            };
            out.insert(name.clone(), CheckedService { target, peers });
        }
        Ok(out)
    }

    /// Where this machine reaches a paired hub's web app: the loopback address of the first
    /// forward to the `web` service. `None` when there is no such forward or the forwards are
    /// not valid.
    pub fn hub_web_listen(&self) -> Option<SocketAddr> {
        self.forwards()
            .ok()?
            .into_iter()
            .find_map(|f| match f.listen {
                DoorTarget::Tcp(addr) if f.service == HUB_WEB_SERVICE => Some(addr),
                _ => None,
            })
    }

    /// Whether this machine serves a hub: it offers its host gate and reaches the hub's web app,
    /// so HQ itself runs elsewhere (on the phone) and only coding agents run here.
    pub fn serves_hub(&self) -> bool {
        self.services.contains_key(HUB_GATE_SERVICE) && self.hub_web_listen().is_some()
    }

    /// The forwards, checked: valid service names, loopback listen ports or socket paths, none used
    /// twice.
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
            let listen = match DoorListen::parse(Some(&f.listen)) {
                Ok(DoorListen::Tcp(addr)) => DoorTarget::Tcp(addr),
                Ok(DoorListen::Unix(path)) => DoorTarget::Unix(path),
                Ok(DoorListen::Default) => {
                    return Err("bridge.forwards: listen is required".into());
                }
                Err(e) => return Err(format!("bridge.forwards: {e}")),
            };
            if out.iter().any(|o| o.listen == listen) {
                return Err(format!("bridge.forwards: {listen} is listed twice"));
            }
            let fail_fast = match f.fail_fast_secs {
                None => None,
                Some(secs @ 1..=MAX_FAIL_FAST_SECS) => Some(std::time::Duration::from_secs(secs)),
                Some(secs) => {
                    return Err(format!(
                        "bridge.forwards: fail_fast_secs {secs} must be between 1 and {MAX_FAIL_FAST_SECS}"
                    ));
                }
            };
            out.push(CheckedForward {
                fail_fast,
                direct_only: f.when == ForwardWhen::Direct,
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
        assert_eq!(
            s["web"].target,
            DoorTarget::Tcp("127.0.0.1:5690".parse().unwrap())
        );
        assert_eq!(
            s["gate"].target,
            DoorTarget::Unix("/run/hq/gate.sock".into())
        );
        assert_eq!(s["web"].peers, None);
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
    fn a_service_may_name_the_peers_that_open_it() {
        let id = "ab".repeat(32);
        let yaml = format!(
            "services:\n  web: 127.0.0.1:5690\n  llm:\n    to: 127.0.0.1:11434\n    peers: [\"{}\"]\n",
            id.to_uppercase()
        );
        let config = parse(&yaml);
        let s = config.services().unwrap();
        assert_eq!(
            s["web"].peers, None,
            "the plain form still admits every peer"
        );
        assert_eq!(s["llm"].peers, Some(vec![id.clone()]), "ids are lowercased");
        assert_eq!(
            serde_yaml::from_str::<BridgeServicesConfig>(&serde_yaml::to_string(&config).unwrap())
                .unwrap(),
            config,
            "both forms survive a write"
        );
        for bad in [
            "services:\n  llm:\n    to: 127.0.0.1:11434\n    peers: []\n".to_string(),
            "services:\n  llm:\n    to: 127.0.0.1:11434\n    peers: [abc]\n".to_string(),
            format!("services:\n  llm:\n    to: 0.0.0.0:11434\n    peers: [\"{id}\"]\n"),
        ] {
            assert!(parse(&bad).services().is_err(), "{bad}");
        }
        for unparsable in [
            format!("services:\n  llm:\n    to: 127.0.0.1:11434\n    peer: [\"{id}\"]\n"),
            "services:\n  llm:\n    to: 127.0.0.1:11434\n".to_string(),
        ] {
            assert!(
                serde_yaml::from_str::<BridgeServicesConfig>(&unparsable).is_err(),
                "{unparsable}"
            );
        }
    }

    #[test]
    fn a_forward_takes_any_path_unless_told_direct() {
        let with = |when: &str| {
            parse(&format!(
                "forwards:\n  - peer: abc\n    service: llm\n    listen: 127.0.0.1:47600\n{when}"
            ))
        };
        assert!(!with("").forwards().unwrap()[0].direct_only);
        assert!(!with("    when: any\n").forwards().unwrap()[0].direct_only);
        let direct = with("    when: direct\n");
        assert!(direct.forwards().unwrap()[0].direct_only);
        assert!(
            serde_yaml::to_string(&direct)
                .unwrap()
                .contains("when: direct")
        );
        assert!(!serde_yaml::to_string(&with("")).unwrap().contains("when"));
        assert!(serde_yaml::from_str::<BridgeServicesConfig>(
            "forwards:\n  - peer: abc\n    service: llm\n    listen: 127.0.0.1:47600\n    when: lan\n"
        )
        .is_err());
    }

    #[test]
    fn fail_fast_is_off_unless_set_and_bounded_when_it_is() {
        let with = |secs: &str| {
            parse(&format!(
                "forwards:\n  - peer: abc\n    service: llm\n    listen: 127.0.0.1:47600\n{secs}"
            ))
            .forwards()
        };
        assert_eq!(with("").unwrap()[0].fail_fast, None);
        assert_eq!(
            with("    fail_fast_secs: 3\n").unwrap()[0].fail_fast,
            Some(std::time::Duration::from_secs(3))
        );
        for bad in ["0", "21"] {
            assert!(
                with(&format!("    fail_fast_secs: {bad}\n")).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn forwards_listen_on_loopback_once_each() {
        let ok = parse(
            "forwards:\n  - peer: abc\n    service: web\n    listen: 127.0.0.1:47480\n  - peer: abc\n    service: llm\n    listen: /data/hq/bridge/llm.sock\n",
        );
        let forwards = ok.forwards().unwrap();
        assert_eq!(
            forwards[0].listen,
            DoorTarget::Tcp("127.0.0.1:47480".parse().unwrap())
        );
        assert_eq!(
            forwards[1].listen,
            DoorTarget::Unix("/data/hq/bridge/llm.sock".into())
        );
        assert_eq!(
            ok.hub_web_listen(),
            Some("127.0.0.1:47480".parse().unwrap())
        );
        for bad in [
            "forwards:\n  - peer: abc\n    service: web\n    listen: 0.0.0.0:47480\n",
            "forwards:\n  - peer: abc\n    service: web\n    listen: x.sock\n",
            "forwards:\n  - peer: abc\n    service: web\n    listen: \"\"\n",
            "forwards:\n  - {peer: a, service: web, listen: /s/a.sock}\n  - {peer: b, service: llm, listen: \"unix:/s/a.sock\"}\n",
            "forwards:\n  - peer: \" \"\n    service: web\n    listen: 127.0.0.1:1\n",
            "forwards:\n  - peer: a\n    service: WEB\n    listen: 127.0.0.1:1\n",
            "forwards:\n  - {peer: a, service: web, listen: \"127.0.0.1:1\"}\n  - {peer: b, service: gate, listen: \"127.0.0.1:1\"}\n",
        ] {
            assert!(parse(bad).forwards().is_err(), "{bad}");
        }
    }

    #[test]
    fn a_laptop_serves_a_hub_when_it_offers_the_gate_and_reaches_the_web_app() {
        let hub = parse(
            "services:\n  gate: 127.0.0.1:7747\nforwards:\n  - peer: abc\n    service: web\n    listen: 127.0.0.1:47480\n",
        );
        assert!(hub.serves_hub());
        assert_eq!(
            hub.hub_web_listen(),
            Some("127.0.0.1:47480".parse().unwrap())
        );
        let no_forward = parse("services:\n  gate: 127.0.0.1:7747\n");
        assert!(!no_forward.serves_hub());
        let no_gate = parse("forwards:\n  - {peer: a, service: web, listen: \"127.0.0.1:1\"}\n");
        assert!(!no_gate.serves_hub());
        assert_eq!(
            no_gate.hub_web_listen(),
            Some("127.0.0.1:1".parse().unwrap())
        );
        assert!(!BridgeServicesConfig::default().serves_hub());
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
