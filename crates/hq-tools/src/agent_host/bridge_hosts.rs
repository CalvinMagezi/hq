//! Hosts that bridge setup made (`hq/setup/1`, `docs/PAIRING.md` "What pairing sets up"): each
//! machine paired with this hub that offers its gate is a `tcp` host at its forward on loopback,
//! reached with this hub's gate key for that host (the default `~/.hq/gate/<name>.key`, which
//! setup created), and agents launched there call back to `/mcp` through that machine's forward.
//! A host configured by hand under `agent_host.hosts` wins over one of the same name.

use hq_core::config::{AgentHostConfig, HqConfig, RemoteHostConfig};
use hq_db::{Database, bridge_setup as rows};

/// One machine that setup made a host of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupHost {
    pub name: String,
    /// The hub's forward to the machine's gate.
    pub port: u16,
    /// Where agents launched there reach this hub's `/mcp`: the machine's own forward.
    pub mcp_url: Option<String>,
}

impl SetupHost {
    pub fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    fn remote(&self) -> RemoteHostConfig {
        serde_json::from_value(serde_json::json!({"transport": "tcp", "addr": self.addr()}))
            .expect("a tcp host entry")
    }
}

/// The hosts in a hub's setup rows: machines that offer their gate.
pub fn from_rows(setups: &[rows::Setup]) -> Vec<SetupHost> {
    setups
        .iter()
        .filter(|s| !s.is_hub() && s.peer_offers)
        .map(|s| SetupHost {
            name: s.peer_name.clone(),
            port: s.local_port,
            mcp_url: s.peer_port.map(|p| format!("http://127.0.0.1:{p}/mcp")),
        })
        .collect()
}

/// The setup hosts of this machine's database, read through a read-only connection that never
/// runs migrations: listing hosts must not write the owner's database (or create it). A failed
/// read, a missing file or a database without the setup table is no hosts, never an error: a
/// configured host must keep working whatever the database says.
pub fn load() -> Vec<SetupHost> {
    let Ok(config) = HqConfig::load() else {
        return Vec::new();
    };
    let flags =
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    rusqlite::Connection::open_with_flags(config.db_path(), flags)
        .ok()
        .and_then(|conn| rows::list_in(&conn).ok())
        .map(|setups| from_rows(&setups))
        .unwrap_or_default()
}

/// `cfg` with every setup host it does not already name added as a `tcp` host.
pub fn merged(mut cfg: AgentHostConfig, hosts: &[SetupHost]) -> AgentHostConfig {
    for host in hosts {
        cfg.hosts
            .entry(host.name.clone())
            .or_insert_with(|| host.remote());
    }
    cfg
}

/// The config one setup host is built with: its agents' MCP URL replaces the configured one (and
/// so reaches the sandbox's egress list), unless the host name is configured by hand.
pub fn for_host(cfg: &AgentHostConfig, hosts: &[SetupHost], name: &str) -> AgentHostConfig {
    let mut out = merged(cfg.clone(), hosts);
    if let Some(url) = mcp_url(cfg, hosts, name) {
        out.agent_mcp_url = Some(url);
    }
    out
}

/// Where agents launched on `name` reach this hub's `/mcp`, when `name` is a setup host that is
/// not configured by hand.
pub fn mcp_url(cfg: &AgentHostConfig, hosts: &[SetupHost], name: &str) -> Option<String> {
    if cfg.hosts.contains_key(name) {
        return None;
    }
    hosts.iter().find(|h| h.name == name)?.mcp_url.clone()
}

/// [`mcp_url`] read from `db`, for a caller that holds the database already.
pub fn mcp_url_in(db: &Database, cfg: &AgentHostConfig, name: &str) -> Option<String> {
    let setups = rows::list(db).ok()?;
    mcp_url(cfg, &from_rows(&setups), name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::config::HostTransport;

    fn row(name: &str, role: &str, offers: bool) -> rows::Setup {
        rows::Setup {
            peer_id: format!("{name}-id"),
            peer_role: role.into(),
            peer_name: name.into(),
            local_port: 47_410,
            peer_offers: offers,
            peer_port: Some(47_420),
            mcp_key: None,
            offered: false,
        }
    }

    #[test]
    fn a_machine_that_offers_its_gate_is_a_tcp_host_on_loopback() {
        let setups = [
            row("laptop", rows::ROLE_MACHINE, true),
            row("quiet", rows::ROLE_MACHINE, false),
            row("phone", rows::ROLE_HUB, true),
        ];
        let hosts = from_rows(&setups);
        assert_eq!(hosts.len(), 1, "{hosts:?}");
        let cfg = merged(AgentHostConfig::default(), &hosts);
        let laptop = &cfg.hosts["laptop"];
        assert_eq!(laptop.transport, HostTransport::Tcp);
        assert_eq!(laptop.addr.as_deref(), Some("127.0.0.1:47410"));
        assert!(
            laptop.gate_key_file.is_none(),
            "the default key path setup wrote"
        );
        let built = for_host(&AgentHostConfig::default(), &hosts, "laptop");
        assert_eq!(
            built.agent_mcp_url.as_deref(),
            Some("http://127.0.0.1:47420/mcp")
        );
    }

    #[test]
    fn a_host_configured_by_hand_wins() {
        let hosts = from_rows(&[row("laptop", rows::ROLE_MACHINE, true)]);
        let mut cfg = AgentHostConfig::default();
        cfg.hosts.insert(
            "laptop".into(),
            serde_yaml::from_str("ssh: \"me@100.64.0.1\"").unwrap(),
        );
        cfg.agent_mcp_url = Some("https://hq.example.com/mcp".into());
        let out = for_host(&cfg, &hosts, "laptop");
        assert_eq!(out.hosts["laptop"].transport, HostTransport::Ssh);
        assert_eq!(
            out.agent_mcp_url.as_deref(),
            Some("https://hq.example.com/mcp")
        );
        assert_eq!(mcp_url(&cfg, &hosts, "laptop"), None);
        assert_eq!(mcp_url(&cfg, &hosts, "other"), None);
    }

    #[test]
    fn the_mcp_url_is_read_from_the_database_for_a_paired_machine_only() {
        let db = Database::open_memory().unwrap();
        hq_db::bridge::add_peer(&db, "laptop-id", "laptop", &[], &[]).unwrap();
        rows::record(&db, &row("laptop", rows::ROLE_MACHINE, true), None).unwrap();
        let cfg = AgentHostConfig::default();
        assert_eq!(
            mcp_url_in(&db, &cfg, "laptop").as_deref(),
            Some("http://127.0.0.1:47420/mcp")
        );
        hq_db::bridge::revoke(&db, "laptop-id").unwrap();
        assert_eq!(mcp_url_in(&db, &cfg, "laptop"), None);
    }
}
