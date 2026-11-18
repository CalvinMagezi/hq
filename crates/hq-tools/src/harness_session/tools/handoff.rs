//! The handoff tool: start a session and hand it a task in one call.

use anyhow::Result;
use async_trait::async_trait;
use hq_db::Database;
use serde_json::{Value, json};
use std::sync::Arc;

use super::spawn::{FolderAsker, choose_start_dir};
use crate::harness_session::WatchingChat;
use crate::registry::HqTool;
use crate::util::arg_str;

pub struct HarnessSessionHandoffTool {
    pub(super) db: Arc<Database>,
    pub(super) chat: Option<WatchingChat>,
    pub(super) family_guest: Option<crate::family_guest::FamilyGuestContext>,
}

#[async_trait]
impl HqTool for HarnessSessionHandoffTool {
    fn name(&self) -> &str {
        "harness_session_handoff"
    }
    fn description(&self) -> &str {
        "Hand a piece of work to an agent in one call: files an HQ task (or reuses one), starts a session linked to it with the goal and acceptance criteria, and opens a web chat thread that owns the session so its finished turns, blocks and Drive show up there instead of the relay. Returns the task id and display id, session id, thread id and web links (/chat?thread=<id>, /tasks?task=<id>). Idempotent: the same external_id returns the same task, and a task that already has a live session gets no second one. If the host is unreachable or the agent stops at a dialog the result says so (blocked_at_dialog, prompt not typed) and nothing is claimed as running."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Task title. Required unless task_id names an existing task." },
                "description": { "type": "string", "description": "What the work is, for the task and the agent" },
                "acceptance": { "type": "string", "description": "Observable conditions that show the work is done (also accepted as done_criteria). HQ drives the session only when these are specific." },
                "done_criteria": { "type": "string", "description": "Alias of acceptance" },
                "harness": { "type": "string", "description": format!("One of: {}. A general-purpose agent ({}) refuses code work; give it purpose research, review or general.", crate::harness_session::spec::known_harnesses().join(", "), crate::harness_session::spec::names_in(crate::harness_session::HarnessCategory::GeneralPurpose).join(", ")) },
                "purpose": { "type": "string", "enum": ["code", "monitor", "research", "review", "general"], "description": "What the session is for. Default: code for a coding agent, general for a general-purpose agent." },
                "worktree": { "type": "object", "properties": { "branch": { "type": "string" }, "from": { "type": "string" }, "detached": { "type": "string" } }, "description": "Start in a git worktree of its own, under the host's HQ folder, so it never shares a working tree with another agent: {\"branch\": \"<name>\"} for a writer (made from the folder's current commit if it does not exist; refused while another worktree has it checked out), {\"detached\": \"<commit or ref>\"} for a reviewer. {\"branch\": \"<name>\", \"from\": \"<ref>\"} starts a new branch at that ref instead (ignored when the branch exists). A new worktree gets the gitignored files the repository's .worktreeinclude names (such as .env, node_modules), copied as copy-on-write clones, listed in the reply's worktree.copied. Removed at stop or exit when nothing is uncommitted; the branch keeps every commit." },
                "owns": { "type": "array", "items": { "type": "string" }, "description": "Paths, relative to cwd, this session writes to. A coding session in a folder another coding session already writes in is refused unless both own paths that do not overlap (no owns means the whole folder). Use a worktree instead when they would overlap." },
                "policy": { "type": "string", "description": "The command policy it runs under: builder or reviewer, one the owner configured under agent_host.command_policies, or none (every dialog goes to the owner). Default: builder for a coding session, reviewer for a review. A policy answers the dialogs it decides; see docs/AGENT_SESSIONS.md." },
                "team": { "type": "string", "description": "A root session id: the new session joins that session's team as a worker under it, with the team's roster, messages between members, stop-with-root and limits. For the owner, a web chat and a claude.ai connection (on a team it started); an agent inside a session uses agent_delegate. A reviewer joining a team observes its running coding sessions unless observe names others." },
                "model": { "type": "string", "description": "Model to start the agent on, passed with its CLI's --model flag (for example sonnet or gpt-5.5 for claude-code, cursor or codex). Omit for the agent's own default. Refused, before anything starts, for an agent with no model flag HQ knows. The result's `model` reports what was passed." },
                "cwd": { "type": "string", "description": "Project directory on that host. Optional: when omitted, or when the folder does not exist there, the session starts in a folder inside that machine's own HQ folder (Documents/HQ), created if needed. A home or root directory is refused, as is anything matching agent_host.spawn_cwd_deny. A claude.ai connector (and the handoff key while agent_host.handoff_cwd_allow lists anything) may use only the host's HQ folder, that list and folders the owner approved. Asking for any other folder sends the owner an approval request and this call waits up to 30 seconds for their answer: it starts the session if they approve, and otherwise fails with BLOCKED and starts nothing. Tell the user you are waiting, then call again with the same arguments. Never start elsewhere and cd into it." },
                "host": { "type": "string", "description": "host from host_list (default: the configured default host, normally local)" },
                "external_id": { "type": "string", "description": "Idempotency key for the task, unique per space. Not combinable with task_id." },
                "task_id": { "type": "string", "description": "Work on this existing HQ task (id or display id) instead of filing one. Must not be complete." },
                "prompt": { "type": "string", "description": "First prompt typed into the session. Default: the task title, description and acceptance criteria." },
                "space_id": { "type": "string", "description": "Space slug to file a new task in (default personal)" },
                "initiative": { "type": "string", "description": "Initiative name to file a new task under (default Inbox)" },
                "drive": { "type": "boolean", "description": "false starts the thread watching without HQ driving the session. Omit to follow agent_host.drive_new_watches." }
            },
            "required": ["harness"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        crate::harness_session::refuse_if_spawned(&args)?;
        if self.chat.as_ref().is_some_and(|c| c.driver_turn) {
            anyhow::bail!("a driver turn cannot start sessions; tell the user what you need");
        }
        if let Some(guest) = &self.family_guest {
            anyhow::bail!(
                "{} cannot hand work to coding agents; only the owner can",
                guest.name
            );
        }
        let origin = crate::harness_session::call_origin(self.chat.as_ref(), &args);
        crate::harness_session::check_origin_cap(&self.db, origin)?;
        let cfg = hq_core::config::HqConfig::load()?;
        let host_name = arg_str(&args, "host");
        let host = crate::agent_host::host(Some(host_name.as_str()).filter(|h| !h.is_empty()))?;
        let requested = args
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|c| !c.is_empty());
        let (label, harness) = (arg_str(&args, "title"), arg_str(&args, "harness"));
        let model = arg_str(&args, "model");
        // Before a start folder is chosen or made, so a refused model leaves nothing behind.
        crate::harness_session::spec::resolve(&harness)?
            .with_model(Some(model.trim()).filter(|m| !m.is_empty()))?;
        let bound = crate::harness_session::CwdBound::of(&args);
        let asker = FolderAsker::of(&self.db, &args);
        let start = choose_start_dir(
            &host,
            requested,
            &cfg,
            &host_name,
            bound,
            (&label, &harness),
            &asker,
        )
        .await?;
        let _joining = match crate::harness_session::team::parse(&args) {
            Some(_) => Some(crate::session_policy::DELEGATION_LOCK.lock().await),
            None => None,
        };
        let team = match crate::harness_session::team::parse(&args) {
            Some(root) => Some(crate::harness_session::team::join(&self.db, &args, &root)?),
            None => None,
        };
        let owned = |key: &str| arg_str(&args, key);
        let acceptance = [owned("acceptance"), owned("done_criteria")]
            .into_iter()
            .find(|a| !a.trim().is_empty())
            .unwrap_or_default();
        let report = crate::harness_session::handoff::handoff(
            &self.db,
            host,
            crate::harness_session::handoff::HandoffRequest {
                title: owned("title"),
                description: owned("description"),
                acceptance,
                harness: owned("harness"),
                cwd: start.path.to_string_lossy().into_owned(),
                external_id: owned("external_id"),
                task_id: owned("task_id"),
                prompt: owned("prompt"),
                space_id: owned("space_id"),
                initiative: owned("initiative"),
                // A handoff from an hq_ask reply or over the handoff key never starts out driving,
                // so a handoff-key holder cannot fill the driven-session cap and starve the owner.
                drive_new: handoff_drives_new(
                    cfg.agent_host.drive_new_watches,
                    &args,
                    self.chat.as_ref(),
                ),
                drive_opted_out: args.get("drive").and_then(Value::as_bool) == Some(false),
                model: owned("model"),
                purpose: crate::harness_session::Purpose::parse_opt(
                    args.get("purpose").and_then(Value::as_str),
                )?,
                worktree: crate::harness_session::checkout::parse_worktree(&args)?,
                owns: crate::harness_session::checkout::parse_owns(&args)?,
                policy: owned("policy"),
                team: team.clone(),
            },
        )
        .await?;
        let mut report = report;
        if let Some(note) = start.note {
            report["cwd_note"] = json!(note);
        }
        // A handoff that found a session already working started nothing: its origin stays as it was.
        if report["handoff"] != "existing_session" {
            crate::harness_session::tag_origin(&self.db, &report, origin);
        }
        let mut brief = crate::harness_session::connector_brief(
            &args,
            &[
                "task_id",
                "title",
                "description",
                "acceptance",
                "done_criteria",
                "prompt",
            ],
        );
        // The prompt is built from the stored task, which may hold text written earlier.
        if let Some(task_id) = report["task"]["id"].as_str() {
            let stored = self.db.with_conn(|c| hq_db::tasks::get_task(c, task_id));
            if let Ok(Some(task)) = stored {
                brief.push_str(&format!(
                    "\nstored task title: {}\nstored task description: {}",
                    task.title, task.description
                ));
            }
        }
        let cwd = start.path.to_string_lossy().into_owned();
        crate::harness_session::notify_connector_start(&self.db, &args, &report, &brief, &cwd);
        Ok(report)
    }
}

/// Whether a handoff may start its session driven: not from the handoff key (so its holder cannot
/// fill the driven-session cap), not from a claude.ai connector for the same reason, and not from
/// an `hq_ask` reply.
pub(in crate::harness_session) fn handoff_drives_new(
    default_on: bool,
    args: &Value,
    chat: Option<&WatchingChat>,
) -> bool {
    default_on
        && !crate::harness_session::is_handoff_scope(args)
        && crate::harness_session::connector_label(args).is_none()
        && !chat.is_some_and(|c| c.from_ask)
}
