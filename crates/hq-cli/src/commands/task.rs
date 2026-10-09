//! `hq task`: the task system from a terminal, for agents with a shell but no MCP.
//!
//! Every verb goes through the same gateway the tasks-scoped MCP key uses, so a terminal agent
//! gets exactly that scope: task and space tools only, writes attributed to `mcp:tasks`, no
//! routing tags, no notifications, no vault, no sessions. Output is the tool's JSON.

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
        /// The comment text; with none given it is read from standard input
        #[arg(allow_hyphen_values = true)]
        body: Vec<String>,
    },
    /// List a task's comments (JSON)
    Comments { id: String },
    /// List spaces (JSON)
    Spaces,
}

/// The gateway call a verb makes: tool name and its arguments.
fn call_for(cmd: &TaskCmd, stdin: &mut dyn FnMut() -> Result<String>) -> Result<(&'static str, Value)> {
    fn put(m: &mut Map<String, Value>, k: &str, v: &Option<String>) {
        if let Some(v) = v {
            m.insert(k.into(), json!(v));
        }
    }
    Ok(match cmd {
        TaskCmd::List { status, space, initiative } => {
            let mut a = Map::new();
            put(&mut a, "status", status);
            put(&mut a, "space_id", space);
            put(&mut a, "initiative_id", initiative);
            ("task_list", Value::Object(a))
        }
        TaskCmd::Get { id } => ("task_get", json!({ "id": id })),
        TaskCmd::Create { title, description, description_stdin, space, initiative, priority, due } => {
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
        TaskCmd::Update { id, status, title, priority, due } => {
            let mut a = Map::new();
            a.insert("id".into(), json!(id));
            put(&mut a, "status", status);
            put(&mut a, "title", title);
            put(&mut a, "priority", priority);
            put(&mut a, "due_date", due);
            if a.len() == 1 {
                bail!("nothing to change: pass --status, --title, --priority or --due");
            }
            ("task_update", Value::Object(a))
        }
        TaskCmd::Comment { id, body } => {
            let text = if body.is_empty() { stdin()? } else { body.join(" ") };
            if text.trim().is_empty() {
                bail!("the comment is empty");
            }
            ("task_comment_add", json!({ "task_id": id, "body": text.trim() }))
        }
        TaskCmd::Comments { id } => ("task_comment_list", json!({ "task_id": id })),
        TaskCmd::Spaces => ("space_list", json!({})),
    })
}

pub async fn run(config: &HqConfig, cmd: TaskCmd) -> Result<()> {
    let mut read_stdin = || -> Result<String> {
        use std::io::Read;
        let mut s = String::new();
        use std::io::IsTerminal;
        if std::io::stdin().is_terminal() {
            bail!("nothing piped in: give the text as an argument, or pipe it in");
        }
        const MAX: u64 = 64 * 1024;
        std::io::stdin().take(MAX + 1).read_to_string(&mut s).context("standard input is not valid text")?;
        if s.len() as u64 > MAX {
            bail!("the text on standard input is over 64 KiB");
        }
        Ok(s)
    };
    let (tool, args) = call_for(&cmd, &mut read_stdin)?;

    let vault = Arc::new(VaultClient::new(config.vault_path.clone()).context("failed to open vault")?);
    let db = Arc::new(Database::open(&config.db_path()).context("failed to open database")?);
    let registry = hq_mcp::registry::create_default_registry(
        vault,
        db.clone(),
        hq_core::skills_dir(&config.vault_path),
        config.vault_path.join("Agents"),
        Some(config),
    );
    let value = hq_mcp::gateway::call_tool_whole(
        &registry,
        tool,
        args,
        Some(hq_mcp::gateway::TASKS_ALLOWLIST),
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
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
        assert!(args.get("tags").is_none(), "the CLI never sets routing tags");
        for cmd in [
            TaskCmd::Get { id: "x".into() },
            TaskCmd::Comments { id: "x".into() },
            TaskCmd::Spaces,
            TaskCmd::List { status: None, space: None, initiative: None },
        ] {
            let (tool, _) = call_for(&cmd, &mut no_stdin).unwrap();
            assert!(hq_mcp::gateway::TASKS_ALLOWLIST.contains(&tool), "{tool}");
        }
    }

    #[test]
    fn an_update_with_no_change_and_an_empty_comment_are_refused() {
        let upd = TaskCmd::Update { id: "x".into(), status: None, title: None, priority: None, due: None };
        assert!(call_for(&upd, &mut no_stdin).is_err());
        let c = TaskCmd::Comment { id: "x".into(), body: vec![] };
        assert!(call_for(&c, &mut || Ok("  \n".into())).is_err());
        let c = TaskCmd::Comment { id: "x".into(), body: vec!["hello".into(), "there".into()] };
        let (_, a) = call_for(&c, &mut no_stdin).unwrap();
        assert_eq!(a["body"], "hello there");
    }
}
