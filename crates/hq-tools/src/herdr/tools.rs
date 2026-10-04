//! MCP tools over Herdr hosts. They see every agent on a host, including ones
//! a person started by hand. All are read-only except `herdr_send`, the one
//! way to prompt such an agent; sessions HQ launched go through
//! `harness_session_*`.

use super::{AgentStatus, HerdrHost, all_hosts, blocking, host};
use crate::registry::HqTool;
use crate::util::{arg_str, arg_str_list};
use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_db::Database;
use serde_json::{Value, json};
use std::sync::Arc;

const DEFAULT_READ_LINES: usize = 60;
const MAX_READ_LINES: usize = 1000;
/// Screen lines returned when `herdr_send` refuses text for a blocked agent.
const BLOCKED_SCREEN_LINES: usize = 40;

pub struct HerdrHostsTool;

#[async_trait]
impl HqTool for HerdrHostsTool {
    fn name(&self) -> &str {
        "herdr_hosts"
    }
    fn description(&self) -> &str {
        "List the machines HQ can run coding agents on through Herdr (this machine plus any configured remote such as a laptop) and whether each is reachable right now."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn category(&self) -> &str {
        "harness"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, _args: Value) -> Result<Value> {
        let hosts = all_hosts()?;
        let rows = blocking(move || hosts.iter().map(host_status).collect::<Vec<_>>()).await?;
        Ok(json!({ "hosts": rows }))
    }
}

fn host_status(h: &HerdrHost) -> Value {
    match h.version() {
        Ok(version) => json!({ "host": h.name(), "reachable": true, "herdr_version": version }),
        Err(e) => json!({ "host": h.name(), "reachable": false, "error": e.to_string() }),
    }
}

pub struct HerdrAgentsTool {
    db: Arc<Database>,
}

#[async_trait]
impl HqTool for HerdrAgentsTool {
    fn name(&self) -> &str {
        "herdr_agents"
    }
    fn description(&self) -> &str {
        "List coding agents Herdr sees on a host (default: every host), with live status (idle, working, blocked, done), working directory and pane title. Includes agents a person started by hand; `hq_session_id` is set only for sessions HQ launched. Use herdr_read to look at one."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "host": { "type": "string", "description": "Host name from herdr_hosts. Omit for all hosts." }
            }
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let hosts = match arg_str(&args, "host").as_str() {
            "" => all_hosts()?,
            name => vec![host(Some(name))?],
        };
        let managed = managed_names(&self.db)?;
        let rows = blocking(move || {
            hosts
                .iter()
                .map(|h| agents_on(h, &managed))
                .collect::<Vec<_>>()
        })
        .await?;
        Ok(json!({ "hosts": rows }))
    }
}

/// (host, agent name) -> session id for every running HQ session.
fn managed_names(db: &Database) -> Result<Vec<(String, String, String)>> {
    let rows = db.with_conn(|c| hq_db::harness_sessions_registry::list(c, Some("running"), 200))?;
    Ok(rows
        .into_iter()
        .map(|r| (r.host, r.agent_name, r.id))
        .collect())
}

fn agents_on(h: &HerdrHost, managed: &[(String, String, String)]) -> Value {
    let agents = match h.agents() {
        Ok(a) => a,
        Err(e) => return json!({ "host": h.name(), "reachable": false, "error": e.to_string() }),
    };
    let rows: Vec<Value> = agents
        .iter()
        .map(|a| {
            let session = managed
                .iter()
                .find(|(host, name, _)| host == h.name() && a.name.as_deref() == Some(name))
                .map(|(_, _, id)| id.clone());
            let mut row = serde_json::to_value(a).unwrap_or(Value::Null);
            row["hq_session_id"] = json!(session);
            row
        })
        .collect();
    json!({ "host": h.name(), "reachable": true, "agents": rows })
}

pub struct HerdrReadTool;

#[async_trait]
impl HqTool for HerdrReadTool {
    fn name(&self) -> &str {
        "herdr_read"
    }
    fn description(&self) -> &str {
        "Read recent terminal output of any agent Herdr shows on a host, by agent name or pane id from herdr_agents. Read-only; does not disturb the agent."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "host": { "type": "string", "description": "Host name (default: the configured default host)" },
                "target": { "type": "string", "description": "Agent name or pane id, e.g. 'w2:p1'" },
                "lines": { "type": "integer", "default": DEFAULT_READ_LINES }
            },
            "required": ["target"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let host_arg = arg_str(&args, "host");
        let target = arg_str(&args, "target");
        let lines = args
            .get("lines")
            .and_then(Value::as_u64)
            .map_or(DEFAULT_READ_LINES, |n| n as usize)
            .clamp(1, MAX_READ_LINES);
        let h = host(Some(host_arg.as_str()).filter(|s| !s.is_empty()))?;
        let name = h.name().to_string();
        let t = target.clone();
        let text = blocking(move || h.read(&t, lines)).await??;
        Ok(json!({ "host": name, "target": target, "output": text }))
    }
}

pub struct HerdrSendTool;

#[async_trait]
impl HqTool for HerdrSendTool {
    fn name(&self) -> &str {
        "herdr_send"
    }
    fn description(&self) -> &str {
        "Prompt or steer an existing agent on a Herdr host, including one a person started by hand (pane id or agent name from herdr_agents). Pass `text` to submit a prompt or `keys` to press logical keys (enter, esc, down, ctrl+c); exactly one. Text is refused while the agent is blocked at a dialog; the screen comes back so you can answer it with keys. For sessions HQ launched, prefer harness_session_send."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "host": { "type": "string", "description": "Host name from herdr_hosts" },
                "target": { "type": "string", "description": "Pane id (e.g. 'w37:p4') or agent name" },
                "text": { "type": "string", "description": "Prompt to submit" },
                "keys": { "type": "array", "items": { "type": "string" }, "description": "Logical keys to press instead of text, e.g. [\"down\", \"enter\"]" }
            },
            "required": ["host", "target"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    // It can type into panes a person started by hand, so only a live chat
    // turn may use it; a watch or scheduled firing never should.
    fn requires_live_user_turn(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let host_arg = arg_str(&args, "host");
        let target = arg_str(&args, "target");
        let text = arg_str(&args, "text");
        let keys = arg_str_list(&args, "keys");
        crate::harness_session::spawned_may_target(&args, &target)?;
        if host_arg.is_empty() || target.is_empty() {
            bail!("`host` and `target` are required; herdr_hosts and herdr_agents list them");
        }
        if text.is_empty() == keys.is_empty() {
            bail!("pass exactly one of `text` (a prompt) or `keys` (logical keys)");
        }
        let h = host(Some(&host_arg))?;
        blocking(move || send_to(&h, &target, &text, &keys)).await?
    }
}

/// Checks the target first so a stale pane id or a sleeping host sends nothing.
pub(super) fn send_to(h: &HerdrHost, target: &str, text: &str, keys: &[String]) -> Result<Value> {
    let agent = match h.agent(target) {
        Err(e) if e.is_unreachable() => bail!("host {} unreachable, nothing sent", h.name()),
        Err(e) => return Err(e.into()),
        Ok(None) => bail!(
            "unknown or stale target '{target}' on host {}, nothing sent",
            h.name()
        ),
        Ok(Some(agent)) => agent,
    };
    if agent.status == AgentStatus::Blocked && keys.is_empty() {
        let screen = h.read(target, BLOCKED_SCREEN_LINES).unwrap_or_default();
        bail!(
            "'{target}' is blocked at a dialog, so the text was not sent. Answer it with `keys`. Screen:\n{}",
            screen.trim_end()
        );
    }
    let note = if keys.is_empty() {
        crate::harness_session::prompt_note(h.submit(target, text)?)
    } else {
        h.send_keys(target, keys)?;
        None
    };
    Ok(json!({
        "host": h.name(),
        "pane_id": agent.pane_id,
        "workspace_id": agent.workspace_id,
        "title": agent.title,
        "cwd": agent.cwd,
        "status_before": agent.status,
        "note": note,
    }))
}

pub fn create_herdr_tools(db: Arc<Database>) -> Vec<Box<dyn HqTool>> {
    vec![
        Box::new(HerdrHostsTool),
        Box::new(HerdrAgentsTool { db }),
        Box::new(HerdrReadTool),
        Box::new(HerdrSendTool),
    ]
}
