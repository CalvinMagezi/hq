//! `hq task`: the task system from a terminal, for agents with a shell but no MCP.
//!
//! Every verb goes through the same gateway the tasks-scoped MCP key uses, so a terminal agent
//! gets exactly that scope: task and space tools only, writes attributed to `mcp:tasks`, no
//! routing tags, no notifications, no vault, no sessions. Output is the tool's JSON. The
//! exceptions run locally: `time`, a text report read from this machine's database
//! (`commands::tasks`), and `install-skill` (`commands::agent_skill`).

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use hq_core::config::HqConfig;
use hq_db::Database;
use hq_vault::VaultClient;
use serde_json::{Map, Value, json};

#[derive(Subcommand, Debug)]
pub enum TaskCmd {
    /// List tasks (JSON)
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        space: Option<String>,
        #[arg(long)]
        initiative: Option<String>,
    },
    /// Show one task by id or display id (JSON)
    Get { id: String },
    /// Create a task
    Create {
        title: String,
        #[arg(long)]
        description: Option<String>,
        /// Read the description from standard input
        #[arg(long, conflicts_with = "description")]
        description_stdin: bool,
        #[arg(long)]
        space: Option<String>,
        #[arg(long)]
        initiative: Option<String>,
        #[arg(long)]
        priority: Option<String>,
        /// YYYY-MM-DD
        #[arg(long)]
        due: Option<String>,
    },
    /// Change a task's status, title, priority or due date
    Update {
        id: String,
        /// Your lease from `claim` or `next`, so the change is recorded as yours
        #[arg(long)]
        lease: Option<String>,
        /// Why the task is blocked (needed with --status blocked)
        #[arg(long)]
        blocked_reason: Option<String>,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        priority: Option<String>,
        #[arg(long)]
        due: Option<String>,
    },
    /// Add a comment to a task
    Comment {
        id: String,
        /// Your lease from `claim` or `next`, so the comment is recorded as yours
        #[arg(long)]
        lease: Option<String>,
        /// The comment text; with none given it is read from standard input
        #[arg(allow_hyphen_values = true)]
        body: Vec<String>,
    },
    /// List a task's comments (JSON)
    Comments { id: String },
    /// List spaces (JSON)
    Spaces,
    /// Claim the most urgent open task assigned to you and start it (JSON, with a lease)
    Next {
        /// Your name: the assignee whose queue to take from
        #[arg(long = "as")]
        actor: String,
        /// Also take tasks nobody is assigned to
        #[arg(long)]
        include_unassigned: bool,
    },
    /// Claim a task before you work on it (JSON, with a lease to pass on later calls)
    Claim {
        id: String,
        /// Your name, shown on the task
        #[arg(long = "as")]
        actor: String,
    },
    /// Keep your lease alive while you work; optionally leave a resume point
    Heartbeat {
        lease: String,
        /// Where the work stands
        #[arg(long)]
        summary: Option<String>,
        /// The single next thing to do
        #[arg(long)]
        next_step: Option<String>,
    },
    /// Stop working on a task and say where it stands
    Release {
        lease: String,
        /// ready_for_review when done, blocked when stuck, to_do to hand it back
        #[arg(long)]
        status: Option<String>,
        /// What you did and what is next (the reason, when blocked)
        #[arg(long)]
        summary: Option<String>,
    },
    /// Leased hours, cycle time and estimate accuracy by initiative and agent, read from this
    /// machine's database (text)
    Time {
        /// Window in days (default 30)
        days: Option<i64>,
    },
    /// Install the hq-tasks skill into your coding agents (claude, codex, cursor or all;
    /// default: the agents found on this machine)
    InstallSkill {
        agents: Vec<String>,
        /// Show what would be written and write nothing
        #[arg(long)]
        dry_run: bool,
        /// Replace a copy that is not HQ's
        #[arg(long)]
        force: bool,
    },
}

/// The gateway call a verb makes: tool name and its arguments.
fn call_for(
    cmd: &TaskCmd,
    stdin: &mut dyn FnMut() -> Result<String>,
) -> Result<(&'static str, Value)> {
    fn put(m: &mut Map<String, Value>, k: &str, v: &Option<String>) {
        if let Some(v) = v {
            m.insert(k.into(), json!(v));
        }
    }
    Ok(match cmd {
        TaskCmd::List {
            status,
            space,
            initiative,
        } => {
            let mut a = Map::new();
            put(&mut a, "status", status);
            put(&mut a, "space_id", space);
            put(&mut a, "initiative_id", initiative);
            ("task_list", Value::Object(a))
        }
        TaskCmd::Get { id } => ("task_get", json!({ "id": id })),
        TaskCmd::Create {
            title,
            description,
            description_stdin,
            space,
            initiative,
            priority,
            due,
        } => {
            let mut a = Map::new();
            a.insert("title".into(), json!(title));
            if *description_stdin {
                a.insert("description".into(), json!(stdin()?));
            } else {
                put(&mut a, "description", description);
            }
            put(&mut a, "space_id", space);
            put(&mut a, "initiative", initiative);
            put(&mut a, "priority", priority);
            put(&mut a, "due_date", due);
            ("task_create", Value::Object(a))
        }
        TaskCmd::Update {
            id,
            lease,
            blocked_reason,
            status,
            title,
            priority,
            due,
        } => {
            let mut a = Map::new();
            a.insert("id".into(), json!(id));
            put(&mut a, "status", status);
            put(&mut a, "title", title);
            put(&mut a, "priority", priority);
            put(&mut a, "due_date", due);
            put(&mut a, "blocked_reason", blocked_reason);
            if a.len() == 1 {
                bail!("nothing to change: pass --status, --title, --priority or --due");
            }
            put(&mut a, "lease", lease);
            ("task_update", Value::Object(a))
        }
        TaskCmd::Comment { id, lease, body } => {
            let text = if body.is_empty() {
                stdin()?
            } else {
                body.join(" ")
            };
            if text.trim().is_empty() {
                bail!("the comment is empty");
            }
            let mut a = Map::new();
            a.insert("task_id".into(), json!(id));
            a.insert("body".into(), json!(text.trim()));
            put(&mut a, "lease", lease);
            ("task_comment_add", Value::Object(a))
        }
        TaskCmd::Comments { id } => ("task_comment_list", json!({ "task_id": id })),
        TaskCmd::Spaces => ("space_list", json!({})),
        TaskCmd::Next {
            actor,
            include_unassigned,
        } => (
            "task_next",
            json!({ "actor": actor, "harness": "terminal", "include_unassigned": include_unassigned }),
        ),
        TaskCmd::Claim { id, actor } => (
            "task_claim",
            json!({ "task_id": id, "actor": actor, "harness": "terminal" }),
        ),
        TaskCmd::Heartbeat {
            lease,
            summary,
            next_step,
        } => {
            let mut a = Map::new();
            a.insert("lease".into(), json!(lease));
            if summary.is_some() || next_step.is_some() {
                let mut cp = Map::new();
                put(&mut cp, "summary", summary);
                put(&mut cp, "next_step", next_step);
                a.insert("checkpoint".into(), Value::Object(cp));
            }
            ("task_heartbeat", Value::Object(a))
        }
        TaskCmd::Release {
            lease,
            status,
            summary,
        } => {
            let mut a = Map::new();
            a.insert("lease".into(), json!(lease));
            put(&mut a, "status", status);
            put(&mut a, "summary", summary);
            ("task_release", Value::Object(a))
        }
        TaskCmd::Time { .. } | TaskCmd::InstallSkill { .. } => {
            bail!("`time` and `install-skill` run locally, not through the gateway")
        }
    })
}

pub async fn run(config: &HqConfig, cmd: TaskCmd) -> Result<()> {
    match cmd {
        TaskCmd::Time { days } => return super::tasks::run(config, "time", days).await,
        TaskCmd::InstallSkill {
            agents,
            dry_run,
            force,
        } => {
            return super::agent_skill::run(&agents, dry_run, force, &mut std::io::stdout());
        }
        _ => {}
    }
    let mut read_stdin = || -> Result<String> {
        use std::io::Read;
        let mut s = String::new();
        use std::io::IsTerminal;
        if std::io::stdin().is_terminal() {
            bail!("nothing piped in: give the text as an argument, or pipe it in");
        }
        const MAX: u64 = 64 * 1024;
        std::io::stdin()
            .take(MAX + 1)
            .read_to_string(&mut s)
            .context("standard input is not valid text")?;
        if s.len() as u64 > MAX {
            bail!("the text on standard input is over 64 KiB");
        }
        Ok(s)
    };
    let setups = hq_db::bridge_setup::setups_of(config);
    let remotes = config.remote_mcp_with(hq_db::bridge_setup::hub_remotes(&setups));
    let peer = match config.task_peer_in(&remotes).map_err(anyhow::Error::msg)? {
        Some(peer) => Some(peer.clone()),
        None => task_peer_from_setup(&setups).and_then(|hub| {
            // A configured entry of the same name wins over the one pairing made.
            remotes.iter().find(|s| s.name == hub.name).cloned()
        }),
    };
    let (tool, args) = call_for(&cmd, &mut read_stdin)?;
    let value = match &peer {
        Some(peer) => call_peer(peer, tool, args).await?,
        None => call_local(config, tool, args).await?,
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

/// With no `task_peer` configured, a machine paired through the bridge with exactly one hub sends
/// its tasks there, through its forward to the hub and with the key the hub minted for it.
fn task_peer_from_setup(
    setups: &[hq_db::bridge_setup::Setup],
) -> Option<hq_core::config::RemoteMcpServer> {
    let mut hubs = hq_db::bridge_setup::hub_remotes(setups);
    (hubs.len() == 1).then(|| hubs.remove(0))
}

/// A peer that accepts the connection and never answers would otherwise hang the terminal.
const PEER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The verb on the HQ that `task_peer` names, through its MCP gateway. The tasks allowlist is
/// checked here too, because the peer's key may be wider than the tasks-only scope.
async fn call_peer(
    peer: &hq_core::config::RemoteMcpServer,
    tool: &str,
    args: Value,
) -> Result<Value> {
    if !hq_mcp::gateway::TASKS_ALLOWLIST.contains(&tool) {
        bail!("`{tool}` is not a task tool");
    }
    let call = hq_tools::remote_mcp::call_named_server(
        peer,
        "hq_call",
        json!({ "tool": tool, "args": args }),
    );
    tokio::time::timeout(PEER_TIMEOUT, call)
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "task peer `{}` did not answer within {PEER_TIMEOUT:?}",
                peer.name
            )
        })?
        .with_context(|| format!("task peer `{}`", peer.name))
}

async fn call_local(config: &HqConfig, tool: &str, args: Value) -> Result<Value> {
    let vault =
        Arc::new(VaultClient::new(config.vault_path.clone()).context("failed to open vault")?);
    let db = Arc::new(Database::open(&config.db_path()).context("failed to open database")?);
    let registry = hq_mcp::registry::create_default_registry(
        vault,
        db.clone(),
        hq_core::skills_dir(&config.vault_path),
        config.vault_path.join("Agents"),
        Some(config),
    );
    hq_mcp::gateway::call_tool_whole(
        &registry,
        tool,
        args,
        Some(hq_mcp::gateway::TASKS_ALLOWLIST),
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_stdin() -> Result<String> {
        bail!("stdin was read")
    }

    #[test]
    fn verbs_map_to_task_tools_only() {
        let (tool, args) = call_for(
            &TaskCmd::Create {
                title: "t".into(),
                description: Some("d".into()),
                description_stdin: false,
                space: Some("personal".into()),
                initiative: None,
                priority: Some("high".into()),
                due: Some("2030-01-02".into()),
            },
            &mut no_stdin,
        )
        .unwrap();
        assert_eq!(tool, "task_create");
        assert_eq!(args["due_date"], "2030-01-02");
        assert_eq!(args["space_id"], "personal");
        assert!(
            args.get("tags").is_none(),
            "the CLI never sets routing tags"
        );
        for cmd in [
            TaskCmd::Get { id: "x".into() },
            TaskCmd::Comments { id: "x".into() },
            TaskCmd::Spaces,
            TaskCmd::List {
                status: None,
                space: None,
                initiative: None,
            },
            TaskCmd::Next {
                actor: "me".into(),
                include_unassigned: false,
            },
            TaskCmd::Claim {
                id: "x".into(),
                actor: "me".into(),
            },
            TaskCmd::Heartbeat {
                lease: "l".into(),
                summary: None,
                next_step: None,
            },
            TaskCmd::Release {
                lease: "l".into(),
                status: None,
                summary: None,
            },
        ] {
            let (tool, _) = call_for(&cmd, &mut no_stdin).unwrap();
            assert!(hq_mcp::gateway::TASKS_ALLOWLIST.contains(&tool), "{tool}");
        }
    }

    #[test]
    fn an_update_with_no_change_and_an_empty_comment_are_refused() {
        let upd = TaskCmd::Update {
            id: "x".into(),
            lease: Some("l".into()),
            blocked_reason: None,
            status: None,
            title: None,
            priority: None,
            due: None,
        };
        assert!(call_for(&upd, &mut no_stdin).is_err());
        let c = TaskCmd::Comment {
            id: "x".into(),
            lease: None,
            body: vec![],
        };
        assert!(call_for(&c, &mut || Ok("  \n".into())).is_err());
        let c = TaskCmd::Comment {
            id: "x".into(),
            lease: None,
            body: vec!["hello".into(), "there".into()],
        };
        let (_, a) = call_for(&c, &mut no_stdin).unwrap();
        assert_eq!(a["body"], "hello there");
    }

    fn peer_config(uri: &str, peer: &str) -> HqConfig {
        let mut config = HqConfig::default();
        config.remote_mcp.push(hq_core::config::RemoteMcpServer {
            name: "server".into(),
            url: format!("{uri}/mcp"),
            api_key: Some("tasks-key".into()),
            live_user_turn_only: true,
        });
        config.task_peer = Some(peer.into());
        config
    }

    #[tokio::test]
    async fn a_task_peer_gets_the_verb_through_its_gateway() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/mcp"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0", "id": 1,
                "result": {"content": [{"type": "text", "text": "{\"ok\":true}"}]},
            })))
            .mount(&server)
            .await;
        let config = peer_config(&server.uri(), "server");
        let peer = config.task_peer_server().unwrap().unwrap();
        let value = call_peer(
            peer,
            "task_comment_add",
            json!({"task_id": "X-1", "body": "hi"}),
        )
        .await
        .unwrap();
        assert_eq!(value["ok"], true);
        let req = &server.received_requests().await.unwrap()[0];
        assert_eq!(
            req.headers.get("authorization").unwrap(),
            "Bearer tasks-key"
        );
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(body["params"]["name"], "hq_call");
        assert_eq!(body["params"]["arguments"]["tool"], "task_comment_add");
        assert_eq!(body["params"]["arguments"]["args"]["task_id"], "X-1");
    }

    #[tokio::test]
    async fn a_task_peer_refuses_a_tool_outside_the_tasks_allowlist() {
        let config = peer_config("http://127.0.0.1:9", "server");
        let peer = config.task_peer_server().unwrap().unwrap();
        let err = call_peer(peer, "vault_write_note", json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not a task tool"));
    }

    #[test]
    fn a_machine_paired_with_one_hub_through_the_bridge_sends_its_tasks_there() {
        use hq_db::bridge_setup::{ROLE_HUB, ROLE_MACHINE, Setup};
        let row = |id: &str, role: &str, key: Option<&str>| Setup {
            peer_id: id.into(),
            peer_role: role.into(),
            peer_name: format!("{id}-name"),
            local_port: 47_450,
            peer_offers: true,
            peer_port: None,
            mcp_key: key.map(Into::into),
            offered: true,
        };
        let phone = row("phone", ROLE_HUB, Some("hqd_k"));
        let peer = task_peer_from_setup(std::slice::from_ref(&phone)).unwrap();
        assert_eq!(peer.url, "http://127.0.0.1:47450/mcp");
        assert_eq!(peer.api_key.as_deref(), Some("hqd_k"));
        assert_eq!(peer.name, "phone-name");
        let laptop = row("laptop", ROLE_MACHINE, None);
        assert!(task_peer_from_setup(&[phone.clone(), laptop]).is_some());
        let two = [phone.clone(), row("vps", ROLE_HUB, Some("hqd_j"))];
        assert!(task_peer_from_setup(&two).is_none(), "two hubs: no guess");
        assert!(task_peer_from_setup(&[row("phone", ROLE_HUB, None)]).is_none());
        assert!(task_peer_from_setup(&[]).is_none());
    }

    #[test]
    fn a_task_peer_with_no_matching_entry_is_an_error_not_a_local_fallback() {
        let config = peer_config("http://127.0.0.1:9", "missing");
        assert!(config.task_peer_server().is_err());
        assert!(HqConfig::default().task_peer_server().unwrap().is_none());
    }
}
