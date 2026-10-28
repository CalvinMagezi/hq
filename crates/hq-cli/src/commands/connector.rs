//! `hq connector`: the owner's controls for the claude.ai connector
//! (`docs/plans/claude-connector.md`), the same ones Settings shows.

use anyhow::{Result, bail};
use clap::Subcommand;
use hq_core::config::HqConfig;
use hq_db::mcp_connector as store;
use std::io::Write;

const DEFAULT_ARM_MINUTES: i64 = 10;
const MAX_ARM_MINUTES: i64 = 60;

#[derive(Subcommand)]
pub enum ConnectorAction {
    /// Whether the connector is on, where Claude reaches it, and who is connected
    Status,
    /// Allow one new connection for a few minutes; then press Connect in Claude
    Arm {
        #[arg(long, default_value_t = DEFAULT_ARM_MINUTES)]
        minutes: i64,
    },
    /// Close the window and drop requests waiting in it
    Disarm,
    /// List connected clients, with revoked ones when asked
    Grants {
        #[arg(long)]
        all: bool,
    },
    /// Disconnect one client by grant id
    Revoke { id: String },
    /// Disconnect every client and close the window
    Off,
    /// The newest calls connected clients made: tool, outcome and time, never the arguments
    Log {
        #[arg(long)]
        grant: Option<String>,
        #[arg(long, default_value_t = 30)]
        limit: i64,
    },
    /// Clients HQ issues credentials to, for Gemini Apps (`docs/GEMINI_CONNECTOR.md`)
    Client {
        #[command(subcommand)]
        action: ClientAction,
    },
}

#[derive(Subcommand)]
pub enum ClientAction {
    /// Create a Gemini client and print its id and secret once
    Add {
        /// A name for the consent page and the grants list
        label: String,
    },
    /// List clients, revoked ones included; secrets are never shown again
    List,
    /// Revoke a client: its secret stops working and its connections end
    Revoke { id: String },
}

const MAX_CLIENT_LABEL_CHARS: usize = 60;

pub fn run(config: &HqConfig, action: ConnectorAction) -> Result<()> {
    let db = hq_db::Database::open(&config.db_path())?;
    run_with_output(
        config,
        &db,
        action,
        chrono::Utc::now().timestamp(),
        &mut std::io::stdout(),
    )
}

fn run_with_output(
    config: &HqConfig,
    db: &hq_db::Database,
    action: ConnectorAction,
    now: i64,
    out: &mut dyn Write,
) -> Result<()> {
    match action {
        ConnectorAction::Status => status(config, db, now, out),
        ConnectorAction::Arm { minutes } => {
            // Same checks the server runs at startup: arming a connector it will not serve helps nobody.
            if config.profile.is_lite() {
                bail!("the connector is not available under profile: lite");
            }
            match config
                .mcp_connector
                .settings(config.web_auth_token.as_deref())
            {
                Ok(Some(_)) => {}
                Ok(None) => bail!(
                    "the connector is off: set mcp_connector.enabled in the config and restart HQ"
                ),
                Err(why) => bail!("the connector cannot start: {why}"),
            }
            let until = now + minutes.clamp(1, MAX_ARM_MINUTES) * 60;
            db.with_conn(|c| store::arm(c, until))?;
            writeln!(
                out,
                "Open until {}. Press Connect in Claude now.",
                clock(until)
            )?;
            Ok(())
        }
        ConnectorAction::Disarm => {
            db.with_conn(store::close_window)?;
            writeln!(out, "Closed. No new client can connect.")?;
            Ok(())
        }
        ConnectorAction::Grants { all } => grants(db, all, now, out),
        ConnectorAction::Revoke { id } => {
            let revoked =
                db.with_conn(|c| store::revoke_grant(c, &id, "revoked by the owner", now))?;
            if !revoked {
                bail!("no live grant {id}; `hq connector grants` lists them");
            }
            writeln!(out, "Revoked {id}.")?;
            Ok(())
        }
        ConnectorAction::Log { grant, limit } => {
            let calls = db.with_conn(|c| store::recent_calls(c, grant.as_deref(), limit))?;
            if calls.is_empty() {
                writeln!(out, "No calls logged.")?;
            }
            for c in calls {
                writeln!(
                    out,
                    "{}  {}  {}  {}",
                    clock(c.at),
                    c.grant_id,
                    c.tool,
                    c.outcome
                )?;
            }
            Ok(())
        }
        ConnectorAction::Client { action } => client(config, db, action, now, out),
        ConnectorAction::Off => {
            let n =
                db.with_conn(|c| store::revoke_all(c, "owner turned the connector off", now))?;
            writeln!(out, "Revoked {n} grant(s) and closed the window.")?;
            Ok(())
        }
    }
}

fn client(
    config: &HqConfig,
    db: &hq_db::Database,
    action: ClientAction,
    now: i64,
    out: &mut dyn Write,
) -> Result<()> {
    match action {
        ClientAction::Add { label } => {
            let label: String = label
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_CLIENT_LABEL_CHARS)
                .collect();
            if label.trim().is_empty() {
                bail!("give the client a name");
            }
            let created = db.with_conn(|c| {
                store::create_client(c, label.trim(), store::Profile::Gemini, now)
            })?;
            writeln!(out, "Client ID:     {}", created.id)?;
            writeln!(out, "Client secret: {}", created.secret)?;
            writeln!(
                out,
                "The secret is shown only now. Enter both in Gemini's custom app settings \
                 (Additional settings), never in a chat or a ticket."
            )?;
            if !config.mcp_connector.gemini.enabled {
                writeln!(
                    out,
                    "The Gemini profile is off: set mcp_connector.gemini.enabled and restart HQ."
                )?;
            }
            Ok(())
        }
        ClientAction::List => {
            let clients = db.with_conn(store::list_clients)?;
            if clients.is_empty() {
                writeln!(out, "No clients.")?;
            }
            for c in clients {
                let state = c
                    .revoked_at
                    .map(|at| format!("revoked {}", clock(at)))
                    .unwrap_or_else(|| "live".into());
                writeln!(
                    out,
                    "{}  {}  {}  created {}  {state}",
                    c.id,
                    c.label,
                    c.profile,
                    clock(c.created_at)
                )?;
            }
            Ok(())
        }
        ClientAction::Revoke { id } => {
            if !db.with_conn(|c| store::revoke_client(c, &id, now))? {
                bail!("no live client {id}; `hq connector client list` lists them");
            }
            writeln!(out, "Revoked {id} and every connection it held.")?;
            Ok(())
        }
    }
}

fn status(config: &HqConfig, db: &hq_db::Database, now: i64, out: &mut dyn Write) -> Result<()> {
    let settings = config
        .mcp_connector
        .settings(config.web_auth_token.as_deref());
    match settings {
        Ok(None) => writeln!(out, "Off (mcp_connector.enabled is false).")?,
        Err(why) => writeln!(out, "Off: {why}")?,
        Ok(Some(s)) => {
            writeln!(out, "On. Connector URL for Claude: {}", s.resource())?;
            if s.gemini {
                writeln!(
                    out,
                    "Gemini profile on (read-only). MCP server URL for Gemini: {}",
                    s.gemini_resource()
                )?;
            }
            writeln!(out, "Consent page origin: {}", s.consent)?;
            writeln!(
                out,
                "Scope preselected on the consent page: {}",
                s.default_scope
            )?;
        }
    }
    match db.with_conn(|c| store::armed_until(c, now))? {
        Some(until) => writeln!(out, "Accepting a new connection until {}.", clock(until))?,
        None => writeln!(out, "Not accepting new connections (`hq connector arm`).")?,
    }
    grants(db, false, now, out)
}

fn grants(db: &hq_db::Database, all: bool, now: i64, out: &mut dyn Write) -> Result<()> {
    let grants = db.with_conn(|c| store::list_grants(c, all, now))?;
    if grants.is_empty() {
        writeln!(out, "No connected clients.")?;
        return Ok(());
    }
    for g in grants {
        let used = g.last_used_at.map(clock).unwrap_or_else(|| "never".into());
        let state = match (&g.revoked_at, &g.revoked_reason) {
            (Some(at), reason) => format!(
                "revoked {} ({})",
                clock(*at),
                reason.as_deref().unwrap_or("no reason")
            ),
            (None, _) => format!("expires {}", clock(g.expires_at)),
        };
        writeln!(
            out,
            "{}  {}  {}  {}  last used {used}  {state}",
            g.id, g.label, g.client_name, g.grant_scope
        )?;
    }
    Ok(())
}

fn clock(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| unix.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn config(enabled: bool) -> HqConfig {
        let mut c = HqConfig::default();
        c.mcp_connector.enabled = enabled;
        c.mcp_connector.public_url = Some("https://hq.example.com".into());
        c.web_auth_token = Some("owner".into());
        c
    }

    fn run(config: &HqConfig, db: &hq_db::Database, action: ConnectorAction) -> Result<String> {
        let mut out = Vec::new();
        run_with_output(config, db, action, NOW, &mut out)?;
        Ok(String::from_utf8(out).unwrap())
    }

    #[test]
    fn status_says_why_the_connector_is_off_or_where_claude_reaches_it() {
        let db = hq_db::Database::open_memory().unwrap();
        assert!(
            run(&config(false), &db, ConnectorAction::Status)
                .unwrap()
                .contains("Off (")
        );
        let mut no_token = config(true);
        no_token.web_auth_token = None;
        assert!(
            run(&no_token, &db, ConnectorAction::Status)
                .unwrap()
                .contains("web_auth_token")
        );
        let on = run(&config(true), &db, ConnectorAction::Status).unwrap();
        assert!(on.contains("https://hq.example.com/mcp"), "{on}");
        assert!(on.contains("Not accepting"), "{on}");
    }

    #[test]
    fn arming_needs_the_connector_on_and_is_capped() {
        let db = hq_db::Database::open_memory().unwrap();
        assert!(run(&config(false), &db, ConnectorAction::Arm { minutes: 10 }).is_err());
        let mut no_token = config(true);
        no_token.web_auth_token = None;
        let refused = run(&no_token, &db, ConnectorAction::Arm { minutes: 10 }).unwrap_err();
        assert!(refused.to_string().contains("web_auth_token"), "{refused}");
        run(&config(true), &db, ConnectorAction::Arm { minutes: 600 }).unwrap();
        let until = db.with_conn(|c| store::armed_until(c, NOW)).unwrap();
        assert_eq!(until, Some(NOW + MAX_ARM_MINUTES * 60));
        run(&config(true), &db, ConnectorAction::Disarm).unwrap();
        assert_eq!(db.with_conn(|c| store::armed_until(c, NOW)).unwrap(), None);
    }

    #[test]
    fn revoking_an_unknown_grant_is_an_error_and_off_reports_the_count() {
        let db = hq_db::Database::open_memory().unwrap();
        let cfg = config(true);
        assert!(
            run(
                &cfg,
                &db,
                ConnectorAction::Revoke {
                    id: "grant-x".into()
                }
            )
            .is_err()
        );
        assert!(
            run(&cfg, &db, ConnectorAction::Off)
                .unwrap()
                .contains("Revoked 0")
        );
        assert!(
            run(&cfg, &db, ConnectorAction::Grants { all: true })
                .unwrap()
                .contains("No connected")
        );
        db.with_conn(|c| store::record_call(c, "grant-a", "vault_search", "ok", "h", NOW))
            .unwrap();
        let log = run(
            &cfg,
            &db,
            ConnectorAction::Log {
                grant: None,
                limit: 5,
            },
        )
        .unwrap();
        assert!(
            log.contains("grant-a") && log.contains("vault_search"),
            "{log}"
        );
    }

    #[test]
    fn a_client_is_created_once_with_its_secret_listed_without_it_and_revoked() {
        let db = hq_db::Database::open_memory().unwrap();
        let mut cfg = config(true);
        cfg.mcp_connector.gemini.enabled = true;
        let add = |label: &str| {
            run(
                &cfg,
                &db,
                ConnectorAction::Client {
                    action: ClientAction::Add {
                        label: label.into(),
                    },
                },
            )
        };
        assert!(add("  ").is_err());
        let shown = add("Pixel").unwrap();
        let id = shown
            .lines()
            .find_map(|l| l.strip_prefix("Client ID:"))
            .unwrap()
            .trim()
            .to_string();
        let secret = shown
            .lines()
            .find_map(|l| l.strip_prefix("Client secret:"))
            .unwrap()
            .trim()
            .to_string();
        assert!(id.starts_with(store::CLIENT_PREFIX));
        let list = run(
            &cfg,
            &db,
            ConnectorAction::Client {
                action: ClientAction::List,
            },
        )
        .unwrap();
        assert!(list.contains(&id) && list.contains("live"), "{list}");
        assert!(!list.contains(&secret), "the secret is never shown again");
        let revoke = || {
            run(
                &cfg,
                &db,
                ConnectorAction::Client {
                    action: ClientAction::Revoke { id: id.clone() },
                },
            )
        };
        assert!(revoke().is_ok());
        assert!(revoke().is_err(), "already revoked");
        let status = run(&cfg, &db, ConnectorAction::Status).unwrap();
        assert!(
            status.contains("https://hq.example.com/mcp/gemini/v1"),
            "{status}"
        );
    }
}
