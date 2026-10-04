//! Message routing, tool dispatch, and parallel execution.

use anyhow::Result;
use hq_core::types::{SessionEvent, ToolCall, ToolResultContent};
use tracing::{debug, warn};

use super::AgentSession;

impl AgentSession {
    /// Run one tool through the full dispatch path, for tests outside this module.
    #[cfg(test)]
    pub(crate) async fn call_tool_for_test(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<hq_core::types::ToolResult> {
        let call = ToolCall {
            id: "test-call".to_string(),
            name: name.to_string(),
            arguments,
        };
        self.execute_tool(&call).await
    }

    /// Execute a single tool call with harness policy enforcement.
    pub(super) async fn execute_tool(&self, tc: &ToolCall) -> Result<hq_core::types::ToolResult> {
        // Backstop, redundant by design: `ToolGuardian::build_registry` is the
        // authoritative gate and already excludes autonomy-restricted tools
        // from an unattended session's registry entirely, so this branch
        // should be unreachable. Kept in case a future change to
        // `GovernedRegistry`'s privacy or a new construction path bypasses it.
        if !self.config.is_live_user_turn {
            let requires_live = {
                let tools = self.tools.lock().await;
                tools
                    .get(&tc.name)
                    .map(|t| t.requires_live_user_turn())
                    .unwrap_or(false)
            };
            if requires_live {
                warn!(
                    tool = %tc.name,
                    agent_name = %self.config.agent_name,
                    "blocked autonomy-restricted tool in a non-live session (backstop)"
                );
                return Ok(hq_core::types::ToolResult {
                    content: vec![ToolResultContent {
                        r#type: "text".to_string(),
                        text: format!(
                            "[BLOCKED] '{}' requires a live user turn and cannot run in an autonomous session.",
                            tc.name
                        ),
                    }],
                    details: None,
                    context_modifier: None,
                });
            }
        }

        if tc.name == "tool_search" {
            return self.handle_tool_search(&tc.arguments).await;
        }

        // Plan mode enforcement: block non-read-only tools at the session level.
        if self.config.mode == super::SessionMode::Plan && tc.name != "tool_search" {
            let tools = self.tools.lock().await;
            if let Some(tool) = tools.get(&tc.name)
                && !tool.is_read_only()
            {
                return Ok(hq_core::types::ToolResult {
                    content: vec![ToolResultContent {
                        r#type: "text".to_string(),
                        text: format!(
                            "[PLAN MODE] Tool '{}' is not read-only and cannot be used in plan mode. \
                                 Only read-only tools are allowed. Produce a plan instead of executing changes.",
                            tc.name
                        ),
                    }],
                    details: None,
                    context_modifier: None,
                });
            }
        }

        let tool = {
            // Take a cloneable execution handle, then drop the registry lock
            // *before* awaiting `execute`. Holding the global mutex across the
            // await would serialize otherwise-parallel read-only tools. Governance
            // (call counters, denial tracking) is unaffected: the handle points at
            // the same governed instance and its state is internally shared.
            let tools = self.tools.lock().await;
            tools
                .get_shared(&tc.name)
                .ok_or_else(|| anyhow::anyhow!("unknown tool: {}", tc.name))?
        };

        let result = crate::middleware_runtime::tap_tool_elapsed(
            &tc.name,
            tool.execute(&tc.id, tc.arguments.clone()),
        )
        .await;

        // Record tool usage telemetry (non-fatal, fire-and-forget).
        if let Some(db) = self.telemetry_db.clone() {
            let agent_name = self.config.agent_name.clone();
            let session_id = self.session_id.clone();
            let tool_name = tc.name.clone();
            let success = result.is_ok();
            let error_msg = result.as_ref().err().map(|e| e.to_string());
            tokio::task::spawn_blocking(move || {
                let _ = db.with_conn(|conn| {
                    hq_db::tool_usage::record_tool_call(
                        conn,
                        &tool_name,
                        &agent_name,
                        &session_id,
                        success,
                        error_msg.as_deref(),
                    )
                });
            });
        }

        result
    }

    /// Handle the built-in `tool_search` meta-tool.
    ///
    /// Searches the deferred tool catalog by keyword and returns full definitions
    /// for matching tools, enabling the agent to discover tools not in the initial
    /// prompt (saving ~4500 tokens/turn).
    pub(super) async fn handle_tool_search(
        &self,
        args: &serde_json::Value,
    ) -> Result<hq_core::types::ToolResult> {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;

        let tools = self.tools.lock().await;
        let catalog = tools.deferred_catalog();

        let query_lower = query.to_lowercase();
        let matches: Vec<_> = catalog
            .iter()
            .filter(|(name, hint)| {
                name.to_lowercase().contains(&query_lower)
                    || hint.to_lowercase().contains(&query_lower)
            })
            .take(max_results)
            .collect();

        if matches.is_empty() {
            return Ok(hq_core::types::ToolResult {
                content: vec![ToolResultContent {
                    r#type: "text".to_string(),
                    text: format!(
                        "No deferred tools match query \"{}\". Available deferred tools: {}",
                        query,
                        catalog
                            .iter()
                            .map(|(n, _)| n.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                }],
                details: None,
                context_modifier: None,
            });
        }

        let mut output = String::new();
        for (name, _) in &matches {
            if let Some(def) = tools.definition_for(name) {
                output.push_str(&format!(
                    "## {}\n{}\n\nParameters:\n```json\n{}\n```\n\n",
                    def.name,
                    def.description,
                    serde_json::to_string_pretty(&def.parameters).unwrap_or_default()
                ));
            }
        }

        Ok(hq_core::types::ToolResult {
            content: vec![ToolResultContent {
                r#type: "text".to_string(),
                text: output,
            }],
            details: None,
            context_modifier: None,
        })
    }

    /// Execute tool calls, parallelizing **contiguous** runs of read-only
    /// (concurrent-safe) tools while preserving overall call order.
    ///
    /// The model's tool-call order is a contract: a `read` issued after a `write`
    /// must observe the write, and two writes must not be reordered. So the batch
    /// is split at every mutating call — each maximal run of adjacent read-only
    /// calls runs together via `join_all`, and each mutating call runs alone, in
    /// place. This keeps read-read concurrency (the common, safe case) without
    /// ever floating a read across a neighbouring write. Results are returned in
    /// the original call order.
    pub(super) async fn execute_tools_parallel(
        &self,
        tool_calls: &[ToolCall],
    ) -> Vec<(String, Option<String>)> {
        if tool_calls.len() <= 1 {
            let mut results = Vec::with_capacity(tool_calls.len());
            for tc in tool_calls {
                self.emit(SessionEvent::ToolStart {
                    tool_name: tc.name.clone(),
                    tool_call_id: tc.id.clone(),
                    arguments: tc.arguments.clone(),
                });
                if let Some(preview) = tool_arg_preview(&tc.name, &tc.arguments) {
                    self.emit(SessionEvent::ToolProgress {
                        tool_name: tc.name.clone(),
                        tool_call_id: tc.id.clone(),
                        message: preview,
                    });
                }
                let result = self.execute_tool(tc).await;
                results.push(Self::format_tool_result(&result));
            }
            return results;
        }

        let mut is_readonly = Vec::with_capacity(tool_calls.len());
        {
            let tools = self.tools.lock().await;
            for tc in tool_calls {
                let readonly = tools
                    .get(&tc.name)
                    .map(|t| t.is_read_only())
                    .unwrap_or(false);
                is_readonly.push(readonly);
            }
        }

        for tc in tool_calls {
            self.emit(SessionEvent::ToolStart {
                tool_name: tc.name.clone(),
                tool_call_id: tc.id.clone(),
                arguments: tc.arguments.clone(),
            });
            if let Some(preview) = tool_arg_preview(&tc.name, &tc.arguments) {
                self.emit(SessionEvent::ToolProgress {
                    tool_name: tc.name.clone(),
                    tool_call_id: tc.id.clone(),
                    message: preview,
                });
            }
        }

        debug!(
            count = tool_calls.len(),
            readonly = is_readonly.iter().filter(|&&r| r).count(),
            mutating = is_readonly.iter().filter(|&&r| !r).count(),
            "executing tool batch: contiguous read-only runs in parallel, order preserved"
        );

        let mut results: Vec<Option<(String, Option<String>)>> = vec![None; tool_calls.len()];
        let mut i = 0;
        while i < tool_calls.len() {
            if is_readonly[i] {
                // Consume the maximal contiguous run of read-only calls at `i` and
                // run them together — safe because no mutating call sits between
                // them to reorder around.
                let start = i;
                while i < tool_calls.len() && is_readonly[i] {
                    i += 1;
                }
                if i - start == 1 {
                    let result = self.execute_tool(&tool_calls[start]).await;
                    results[start] = Some(Self::format_tool_result(&result));
                } else {
                    let futures: Vec<_> = tool_calls[start..i]
                        .iter()
                        .map(|tc| self.execute_tool(tc))
                        .collect();
                    let group = futures::future::join_all(futures).await;
                    for (offset, result) in group.iter().enumerate() {
                        results[start + offset] = Some(Self::format_tool_result(result));
                    }
                }
            } else {
                // A mutating call is a barrier: run it alone, in place, so reads on
                // either side never float across it.
                let result = self.execute_tool(&tool_calls[i]).await;
                results[i] = Some(Self::format_tool_result(&result));
                i += 1;
            }
        }

        results.into_iter().map(|r| r.unwrap()).collect()
    }

    /// Format a tool result into a (text, context_modifier) pair.
    pub(super) fn format_tool_result(
        result: &Result<hq_core::types::ToolResult>,
    ) -> (String, Option<String>) {
        match result {
            Ok(tr) => {
                let text = tr
                    .content
                    .iter()
                    .map(|c| c.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                (text, tr.context_modifier.clone())
            }
            Err(e) => (format!("Error: {}", e), None),
        }
    }
}

/// Build a short, human-readable preview of a tool call's salient argument, used
/// for live progress visibility (Telegram activity feed, web UI). Returns None when
/// there's nothing useful to show. Kept generic so new tools surface automatically.
pub(crate) fn tool_arg_preview(name: &str, args: &serde_json::Value) -> Option<String> {
    let get = |k: &str| args.get(k).and_then(|v| v.as_str());
    let n = name.to_lowercase();

    let raw = if n == "bash" || n.ends_with("shell") {
        get("command").map(|c| format!("$ {c}"))
    } else if n.contains("search") || n == "grep" {
        get("query")
            .or_else(|| get("pattern"))
            .or_else(|| get("q"))
            .map(|q| format!("\u{201C}{q}\u{201D}"))
    } else if n.contains("subagent") || n.contains("spawn") || n == "coordinator" {
        get("agent_type")
            .or_else(|| get("agent"))
            .or_else(|| get("role"))
            .map(|a| {
                format!(
                    "\u{2192} {a}: {}",
                    get("task").or_else(|| get("instruction")).unwrap_or("")
                )
            })
    } else if n.contains("web_fetch") || n.contains("fetch") {
        get("url").map(|u| u.to_string())
    } else if n.contains("write") || n.contains("note") {
        get("path")
            .or_else(|| get("file_path"))
            .or_else(|| get("title"))
            .or_else(|| get("folder"))
            .map(|p| p.to_string())
    } else {
        // read / edit / generic: prefer a path-like arg, else first string.
        get("path")
            .or_else(|| get("file_path"))
            .or_else(|| get("file"))
            .or_else(|| get("files"))
            .map(|p| p.to_string())
            .or_else(|| {
                args.as_object()
                    .and_then(|o| o.values().find_map(|v| v.as_str()).map(|s| s.to_string()))
            })
    }?;

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let preview: String = trimmed.chars().take(90).collect();
    Some(if trimmed.chars().count() > 90 {
        format!("{preview}\u{2026}")
    } else {
        preview
    })
}

#[cfg(test)]
mod arg_preview_tests {
    use super::tool_arg_preview;
    use serde_json::json;

    #[test]
    fn bash_shows_command() {
        let p = tool_arg_preview("bash", &json!({"command": "cargo test -p hq-web"})).unwrap();
        assert!(p.starts_with("$ cargo test"), "{p}");
    }

    #[test]
    fn search_shows_query() {
        let p = tool_arg_preview("vault_search", &json!({"query": "Value Bus"})).unwrap();
        assert!(p.contains("Value Bus"), "{p}");
    }

    #[test]
    fn read_shows_path() {
        let p = tool_arg_preview("read_file", &json!({"path": "crates/hq-web/src/ws.rs"})).unwrap();
        assert_eq!(p, "crates/hq-web/src/ws.rs");
    }

    #[test]
    fn subagent_shows_target() {
        let p = tool_arg_preview(
            "spawn_subagent",
            &json!({"agent_type": "researcher", "task": "find X"}),
        )
        .unwrap();
        assert!(p.contains("researcher"), "{p}");
    }

    #[test]
    fn empty_args_is_none() {
        assert!(tool_arg_preview("noop", &json!({})).is_none());
    }
}
