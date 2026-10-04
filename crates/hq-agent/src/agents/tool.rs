//! The single native delegation tool: `spawn_subagents`.
//!
//! One [`AgentTool`](crate::tools::AgentTool) over [`AgentService`] that replaces
//! the pair of legacy tools (`spawn_subagent` + `coordinate`). It accepts a
//! single task or a task list, an explicit execution `mode`, per-task
//! dependencies and backend preferences, and returns a structured summary plus
//! per-child detail. The schema stays close to the old `spawn_subagent` /
//! `coordinate` schemas so the surface-migration todo is a mechanical port.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use hq_core::types::{SubagentType, ToolResult, ToolResultContent};
use serde_json::{Value, json};

use crate::tools::AgentTool;

use super::model_select::OnModelUnavailable;
use super::service::AgentService;
use super::types::{
    ChildExecContext, ChildMode, ChildOutcome, ChildPlan, ChildRequest, ChildStatus,
};

/// Native tool wrapping [`AgentService`] as the primary delegation surface.
pub struct SpawnSubagentsTool {
    service: Arc<AgentService>,
    /// Per-execute correlation context (parent run id + envelope sink), set by
    /// the builder so the parent's stream receives child markers.
    ctx: ChildExecContext,
}

impl SpawnSubagentsTool {
    /// Wrap a service with no parent correlation context (top-level use/tests).
    pub fn new(service: Arc<AgentService>) -> Self {
        Self {
            service,
            ctx: ChildExecContext::default(),
        }
    }

    /// Attach the parent correlation context (run id + envelope sink) so child
    /// markers and forwarded events interleave with the parent run.
    pub fn with_context(mut self, ctx: ChildExecContext) -> Self {
        self.ctx = ctx;
        self
    }
}

#[async_trait]
impl AgentTool for SpawnSubagentsTool {
    fn name(&self) -> &str {
        "spawn_subagents"
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Each subagent starts with no memory of this conversation — put everything it needs in its prompt. Use parallel mode only for genuinely independent work; anything with a shared dependency belongs in one subagent or a graph.",
        )
    }

    fn description(&self) -> &str {
        "Delegate work to child agents. Provide one `task` or a list of `tasks`, \
         and a `mode`: 'single' (one child), 'parallel' (independent children run \
         concurrently), 'race' (first success wins, the rest are cancelled), or \
         'graph' (children with `depends_on` scheduled in dependency order). Each \
         task has a role (general/explorer/planner/verifier/coder), optional \
         `backend` preference, `depends_on`, budget and timeout. Returns a summary \
         plus per-child results."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["single", "parallel", "race", "graph"],
                    "description": "Execution mode. Inferred when omitted: 'single' for one task, 'parallel' for a list without dependencies, 'graph' when any task has depends_on."
                },
                "task": {
                    "type": "object",
                    "description": "A single task (shorthand for a one-element `tasks`).",
                    "properties": task_schema()
                },
                "tasks": {
                    "type": "array",
                    "description": "A list of tasks.",
                    "items": { "type": "object", "properties": task_schema() }
                },
                "max_concurrent": {
                    "type": "integer",
                    "default": 3,
                    "description": "Max children running at once (parallel/graph)."
                },
                "blocking": {
                    "type": "boolean",
                    "default": true,
                    "description": "Set blocking=false to run children in the background; results are delivered asynchronously to the chat thread instead of returned inline."
                }
            }
        })
    }

    async fn execute(&self, _id: &str, args: Value) -> Result<ToolResult> {
        // Collect tasks from `task` (single) or `tasks` (list).
        let mut requests: Vec<ChildRequest> = Vec::new();
        if let Some(one) = args.get("task") {
            requests.push(parse_child_request(one, requests.len())?);
        }
        if let Some(Value::Array(items)) = args.get("tasks") {
            for item in items {
                let idx = requests.len();
                requests.push(parse_child_request(item, idx)?);
            }
        }

        if requests.is_empty() {
            return Ok(text_result("No tasks provided.", None));
        }

        let max_concurrent = args
            .get("max_concurrent")
            .and_then(|v| v.as_u64())
            .unwrap_or(3)
            .max(1) as usize;

        // Resolve mode: explicit when given, else inferred.
        let has_deps = requests.iter().any(|r| !r.depends_on.is_empty());
        let mode = match args.get("mode").and_then(|v| v.as_str()) {
            Some("single") => ChildMode::Single,
            Some("parallel") => ChildMode::Parallel,
            Some("race") => ChildMode::Race,
            Some("graph") => ChildMode::Graph,
            _ if requests.len() == 1 => ChildMode::Single,
            _ if has_deps => ChildMode::Graph,
            _ => ChildMode::Parallel,
        };

        if mode == ChildMode::Single && requests.len() > 1 {
            return Ok(text_result(
                &format!(
                    "Rejected: mode \"single\" accepts exactly one task, but {} were provided. \
                     Use \"parallel\", \"race\", or \"graph\" for more than one task.",
                    requests.len()
                ),
                None,
            ));
        }

        let blocking = args
            .get("blocking")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let plan = ChildPlan {
            mode,
            children: requests,
            max_concurrent,
            blocking: if blocking { None } else { Some(false) },
        };
        let dispatched_ids: Vec<String> = plan.children.iter().map(|c| c.id.clone()).collect();

        let (outcomes, run_ids) = self.service.execute_with_runs(plan, self.ctx.clone()).await;

        // Non-blocking dispatch: children are running detached; report the
        // task ids and that results arrive asynchronously. (An empty outcome
        // list from a blocking plan never happens, so this branch is exact.)
        if !blocking && outcomes.is_empty() {
            let runs_note = run_ids
                .iter()
                .map(|(task, run)| format!("{task}={run}"))
                .collect::<Vec<_>>()
                .join(", ");
            let text = format!(
                "Dispatched {} child task(s) in the background ({} mode): {}. Run ids: {}. {}",
                dispatched_ids.len(),
                format!("{mode:?}").to_lowercase(),
                dispatched_ids.join(", "),
                runs_note,
                background_promise(&self.ctx),
            );
            return Ok(text_result(
                &text,
                Some(json!({
                    "mode": format!("{mode:?}").to_lowercase(),
                    "blocking": false,
                    "dispatched": dispatched_ids,
                    "runs": run_ids
                        .iter()
                        .map(|(task, run)| json!({"task": task, "run_id": run}))
                        .collect::<Vec<_>>(),
                })),
            ));
        }

        let summary = ChildRunSummary::from_outcomes(mode, &outcomes);

        Ok(ToolResult {
            content: vec![ToolResultContent {
                r#type: "text".to_string(),
                text: summary.to_string(),
            }],
            details: serde_json::to_value(&summary).ok(),
            context_modifier: None,
        })
    }

    fn category(&self) -> &str {
        "agents"
    }
}

/// Native tool letting the agent volunteer a substantive mid-turn progress
/// note. Fires a [`ProgressEvent`](crate::native_hq::ProgressEvent) with
/// `note: Some(message)` into the [`ProgressSink`](crate::native_hq::ProgressSink)
/// threaded through the context, so a detached/background turn can update the
/// user while it keeps running. No-op safe: with no sink it returns an
/// informational string and never panics.
pub struct ReportProgressTool {
    ctx: ChildExecContext,
}

impl ReportProgressTool {
    /// No progress routing (top-level use/tests): calls are graceful no-ops.
    pub fn new() -> Self {
        Self {
            ctx: ChildExecContext::default(),
        }
    }

    /// Attach the per-turn context carrying the progress sink and turn id
    /// (same threading pattern as `completion_sink` in phase 1).
    pub fn with_context(mut self, ctx: ChildExecContext) -> Self {
        self.ctx = ctx;
        self
    }
}

impl Default for ReportProgressTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentTool for ReportProgressTool {
    fn name(&self) -> &str {
        "report_progress"
    }

    fn description(&self) -> &str {
        "Report a brief progress update to the user while you work. Use this \
         during long-running tasks (multi-step builds, research, sub-agent \
         runs) to say what you've done and what's next. Keep `message` short \
         (one or two sentences). Does not interrupt your work. \
         If you have stopped to wait on an answer from the operator or another \
         agent, set `blocked_on` to who you're waiting on so it reads \
         differently from ordinary progress. If you gave up waiting and \
         proceeded on a guess instead, set `resumed_with_assumption` to what \
         you assumed, so it can be checked and corrected later."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Short progress note for the user (one or two sentences)."
                },
                "blocked_on": {
                    "type": "string",
                    "description": "Who you've stopped to wait on (e.g. 'the operator'), if you have. Omit for ordinary progress."
                },
                "resumed_with_assumption": {
                    "type": "string",
                    "description": "The assumption you made to keep going instead of waiting further, if you did. Omit otherwise."
                }
            },
            "required": ["message"]
        })
    }

    async fn execute(&self, _id: &str, args: Value) -> Result<ToolResult> {
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let Some(message) = message else {
            return Ok(text_result(
                "No `message` provided; nothing reported.",
                None,
            ));
        };

        let trimmed_arg = |key: &str| {
            args.get(key)
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        let blocked_on = trimmed_arg("blocked_on");
        let resumed_with_assumption = trimmed_arg("resumed_with_assumption");

        let Some(sink) = self.ctx.progress_sink.clone() else {
            return Ok(text_result(
                "Progress reporting not available in this context.",
                None,
            ));
        };

        sink(crate::native_hq::ProgressEvent {
            turn_id: self.ctx.parent_turn_id.clone().unwrap_or_default(),
            elapsed_secs: 0,
            note: Some(message),
            blocked_on,
            resumed_with_assumption,
        });

        Ok(text_result("Progress reported.", None))
    }

    fn category(&self) -> &str {
        "agents"
    }
}

/// Shared JSON schema for a task object (used by both `task` and `tasks[]`).
pub(super) fn task_schema() -> Value {
    json!({
        "id": { "type": "string", "description": "Unique task id (referenced by depends_on)." },
        "goal": { "type": "string", "description": "The task instruction/prompt." },
        "prompt": { "type": "string", "description": "Alias for `goal`." },
        "context": { "type": "string", "description": "Extra context for the child's system prompt." },
        "role": {
            "type": "string",
            "enum": ["general", "explorer", "planner", "verifier", "coder"],
            "description": "Child role: coder=implementation, explorer=read-only search, planner=design, verifier=checking, general=default."
        },
        "agent_type": { "type": "string", "enum": ["general", "explorer", "planner", "verifier", "coder"], "description": "Alias for `role`." },
        "backend": { "type": "string", "description": "Backend/harness preference. 'hq' or omitted = in-process; any other name resolves against configured backends." },
        "model": { "type": "string", "description": "Optional model override for this child. With a configured backend chain it must name a declared backend or that backend's model, and the child runs on that backend. Omit to inherit the parent's model (or the role's default). The result reports the effective model and whether it was inherited or overridden." },
        "on_model_unavailable": { "type": "string", "enum": ["reject", "inherit"], "description": "When `model` is unsupported or unavailable: reject (default) fails that child with a clear error; inherit runs it on the parent's model and records the substitution in the result." },
        "depends_on": { "type": "array", "items": { "type": "string" }, "description": "Task ids that must finish before this one (graph mode)." },
        "files_in_scope": {
            "type": "array",
            "items": { "type": "string" },
            "description": "Files this task will edit. If non-empty and `backend` names a known leaf/text-only harness (e.g. groq, cerebras), dispatch is rejected up front with a clear error instead of silently returning an unapplied diff."
        },
        "success_criteria": { "type": "array", "items": { "type": "string" }, "description": "What must be true for the deliverable to count as done. Recorded with the run; a clean exit is still reported as unverified until you review it." },
        "required_tools": { "type": "array", "items": { "type": "string" }, "description": "Tool names the child must have. If it will not have one, the child is blocked before it starts instead of reporting success without the deliverable." },
        "task_id": { "type": "string", "description": "HQ task this child's work belongs to, recorded on its run." },
        "max_budget_usd": { "type": "number" },
        "timeout_secs": { "type": "integer" },
        "context_need": {
            "type": "object",
            "description": "Vault context this child needs, retrieved when it starts and handed over as a bounded, source-linked packet. The child sees nothing else from the vault or from this conversation. Keys: why, refs (note paths), queries, graph_seeds, source_prefixes, time_sensitive, max_age_days, budget_chars, max_sources. The child is told to cite sources as [S1] and end with Sources used and Gaps lists, and its citations are checked."
        }
    })
}

/// Parse a JSON task object into a [`ChildRequest`], tolerating both the new
/// (`goal`/`role`) and legacy (`prompt`/`agent_type`) field names.
pub(super) fn parse_child_request(value: &Value, index: usize) -> Result<ChildRequest> {
    let goal = value
        .get("goal")
        .or_else(|| value.get("prompt"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("task {index}: missing 'goal' (or 'prompt')"))?
        .to_string();

    let id = value
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("task-{}", index + 1));

    let role_str = value
        .get("role")
        .or_else(|| value.get("agent_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("general");
    let agent_type = match role_str {
        "coder" => SubagentType::Coder,
        "explorer" => SubagentType::Explorer,
        "planner" => SubagentType::Planner,
        "verifier" => SubagentType::Verifier,
        _ => SubagentType::General,
    };

    let mut req = ChildRequest::new(id, goal);
    req.agent_type = agent_type;
    req.model = value
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    req.on_model_unavailable = match value.get("on_model_unavailable").and_then(|v| v.as_str()) {
        None | Some("reject") => OnModelUnavailable::Reject,
        Some("inherit") => OnModelUnavailable::Inherit,
        Some(other) => {
            anyhow::bail!(
                "task {index}: on_model_unavailable must be 'reject' or 'inherit', got '{other}'"
            )
        }
    };
    req.context = value
        .get("context")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    req.backend = value
        .get("backend")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    req.depends_on = value
        .get("depends_on")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    req.files_in_scope = value
        .get("files_in_scope")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    req.success_criteria = value
        .get("success_criteria")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    if let Some(need) = value.get("context_need") {
        req.context_need = Some(
            serde_json::from_value(need.clone())
                .map_err(|e| anyhow::anyhow!("task {index}: invalid context_need: {e}"))?,
        );
    }
    req.required_tools = value
        .get("required_tools")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    req.task_id = value
        .get("task_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    req.max_budget_usd = value.get("max_budget_usd").and_then(|v| v.as_f64());
    req.timeout_secs = value.get("timeout_secs").and_then(|v| v.as_u64());

    Ok(req)
}

/// What the dispatch ack may truthfully promise about delivery.
fn background_promise(ctx: &ChildExecContext) -> &'static str {
    if ctx.auto_followup && ctx.origin.is_some() {
        "HQ is set to resume this conversation when each child settles (within a per-chat daily limit, and not for work started inside a follow-up turn more than three levels deep). Do not claim progress beyond what subagent_run_status reports."
    } else if ctx.origin.is_some() || ctx.completion_sink.is_some() {
        "A notice is posted to the chat as each child settles, but HQ does not resume on its own: check subagent_run_list before saying anything about progress."
    } else {
        "No chat route is attached, so results are only recorded: poll subagent_run_list before saying anything about progress."
    }
}

fn text_result(text: &str, details: Option<Value>) -> ToolResult {
    ToolResult {
        content: vec![ToolResultContent {
            r#type: "text".to_string(),
            text: text.to_string(),
        }],
        details,
        context_modifier: None,
    }
}

/// A serializable summary of a `spawn_subagents` run, formatted in the same
/// spirit as `CoordinationSummary` so migration keeps parity.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChildRunSummary {
    pub mode: String,
    pub total: usize,
    pub completed: usize,
    pub failed: usize,
    pub timed_out: usize,
    pub blocked: usize,
    pub rejected: usize,
    pub cancelled: usize,
    pub total_duration_ms: u64,
    pub results: Vec<ChildOutcome>,
}

impl ChildRunSummary {
    fn from_outcomes(mode: ChildMode, outcomes: &[ChildOutcome]) -> Self {
        let count = |s: ChildStatus| outcomes.iter().filter(|o| o.status == s).count();
        Self {
            mode: format!("{mode:?}").to_lowercase(),
            total: outcomes.len(),
            completed: count(ChildStatus::Completed),
            failed: count(ChildStatus::Failed),
            timed_out: count(ChildStatus::TimedOut),
            blocked: count(ChildStatus::Blocked),
            rejected: count(ChildStatus::Rejected),
            cancelled: count(ChildStatus::Cancelled),
            total_duration_ms: outcomes.iter().map(|o| o.duration_ms).sum(),
            results: outcomes.to_vec(),
        }
    }
}

/// One bounded line per child naming its run, verdict, blocker and missing
/// deliverables, so they reach the model even when the preview cuts the output.
fn write_run_line(
    f: &mut std::fmt::Formatter<'_>,
    run: &super::types::RunInfo,
) -> std::fmt::Result {
    write!(f, "    run {} accept={}", run.run_id, run.accept_status)?;
    if let Some(b) = &run.blocker {
        write!(f, " BLOCKER: {b}")?;
    }
    if !run.missing_deliverables.is_empty() {
        write!(f, " MISSING: {}", run.missing_deliverables.join("; "))?;
    }
    writeln!(f, " (full result: subagent_run_result)")
}

/// ` [model X, override]`, plus the rejected request when `inherit` replaced it.
fn model_note(outcome: &ChildOutcome) -> String {
    let Some(m) = &outcome.effective_model else {
        return String::new();
    };
    let source = format!("{:?}", m.source).to_lowercase();
    match &m.fallback_from {
        Some(from) => format!(" [model {}, {source}, fell back from {from}]", m.model),
        None => format!(" [model {}, {source}]", m.model),
    }
}

/// One line HQ reads before it trusts a child's cited evidence.
fn evidence_note(check: &hq_memory::context_packet::CitationReport) -> String {
    let list = |v: &[String]| {
        if v.is_empty() {
            "none".to_string()
        } else {
            v.join(",")
        }
    };
    format!(
        "evidence check: verified {}; changed or gone {}; not in packet {}; sources and gaps listed: {}",
        list(&check.verified),
        list(&check.changed),
        list(&check.unknown),
        if check.lists_sources_and_gaps {
            "yes"
        } else {
            "no"
        }
    )
}

impl std::fmt::Display for ChildRunSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "## Sub-agent Run Summary ({} mode)", self.mode)?;
        writeln!(
            f,
            "Children: {}/{} completed, {} failed, {} timed out, {} blocked, {} rejected, {} cancelled",
            self.completed,
            self.total,
            self.failed,
            self.timed_out,
            self.blocked,
            self.rejected,
            self.cancelled
        )?;
        let accept = |v: &str| {
            self.results
                .iter()
                .filter(|r| r.run.as_ref().is_some_and(|i| i.accept_status == v))
                .count()
        };
        writeln!(
            f,
            "Acceptance: {} unverified, {} partial, {} blocked. A completed child is not an accepted deliverable until you check it.",
            accept("unverified"),
            accept("partial"),
            accept("blocked")
        )?;
        writeln!(
            f,
            "Duration: {:.1}s",
            self.total_duration_ms as f64 / 1000.0
        )?;
        writeln!(f)?;
        for r in &self.results {
            let backend = if r.resolved_backend.is_empty() {
                "-".to_string()
            } else if r.fallback_used {
                format!("{} (fallback)", r.resolved_backend)
            } else {
                r.resolved_backend.clone()
            };
            let preview = if r.output.len() > 200 {
                // Truncate on a char boundary — `r.output` may contain
                // multibyte UTF-8, and a raw byte-offset slice would panic if
                // 200 lands mid-character.
                let end = r
                    .output
                    .char_indices()
                    .map(|(i, _)| i)
                    .take_while(|&i| i <= 200)
                    .last()
                    .unwrap_or(0);
                format!("{}...", &r.output[..end])
            } else {
                r.output.clone()
            };
            writeln!(
                f,
                "[{}] {} <{}> ({:.1}s){}: {}",
                r.status.glyph(),
                r.id,
                backend,
                r.duration_ms as f64 / 1000.0,
                model_note(r),
                preview
            )?;
            if let Some(check) = &r.evidence_check {
                writeln!(f, "    {}", evidence_note(check))?;
            }
            if let Some(run) = &r.run {
                write_run_line(f, run)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FR-056: `spawn_subagents`'s schema no longer advertises `max_turns`,
    /// and even if a caller sends it anyway, `parse_child_request` has no
    /// field to route it into — the argument is silently dropped rather than
    /// capping the child's turns.
    #[test]
    fn max_turns_argument_is_ignored_not_applied() {
        let value = json!({
            "id": "t1",
            "goal": "do the thing",
            "max_turns": 3
        });
        let req = parse_child_request(&value, 0).unwrap();
        assert_eq!(req.id, "t1");
        assert_eq!(req.goal, "do the thing");

        assert!(
            !task_schema().as_object().unwrap().contains_key("max_turns"),
            "max_turns must not be advertised in the tool schema"
        );
    }
}
