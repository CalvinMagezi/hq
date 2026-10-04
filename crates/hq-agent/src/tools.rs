//! Tool trait and registry for agent tool execution.

use anyhow::Result;
use async_trait::async_trait;
use hq_core::types::{ToolDefinition, ToolResult, ValidationResult};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Trait for tools that an agent can invoke during a session.
///
/// The execution pipeline (distilled from claude-code):
/// 1. `validate(args)` — structural checks before any I/O; cheap and safe to abort
/// 2. `execute(id, args)` — actual work, only called when validation passes
#[async_trait]
pub trait AgentTool: Send + Sync {
    /// Unique name of this tool (e.g., "bash", "read_file").
    fn name(&self) -> &str;

    /// Human-readable description shown to the LLM.
    fn description(&self) -> &str;

    /// JSON Schema for the tool's parameters.
    fn parameters(&self) -> Value;

    /// Validate arguments before any execution. Override to add tool-specific
    /// checks: required fields, path policy, file staleness, etc.
    /// Default always succeeds.
    async fn validate(&self, _args: &Value) -> ValidationResult {
        ValidationResult::ok()
    }

    /// Execute the tool with the given arguments.
    /// `id` is the tool_call_id from the LLM response.
    /// Only called after `validate` returns `ValidationResult::Ok`.
    async fn execute(&self, id: &str, args: Value) -> Result<ToolResult>;

    /// Category tag for capability-profile filtering (e.g. "browser", "audio").
    /// Matches the same category strings used by `HqTool::category()`.
    fn category(&self) -> &str {
        "general"
    }

    /// Whether this tool performs only read operations (no side effects).
    fn is_read_only(&self) -> bool {
        false
    }

    /// Whether this tool performs destructive/irreversible operations.
    /// Destructive tools get extra confirmation in governed modes.
    fn is_destructive(&self) -> bool {
        false
    }

    /// Excluded from any session not directly driven by a live user turn —
    /// see `hq_tools::registry::HqTool::requires_live_user_turn` for the
    /// full rationale; this mirrors it on the native `AgentTool` side so
    /// `hq_agent::governance::ToolGuardian::build_registry` can enforce it
    /// regardless of which trait a tool was originally written against.
    /// Default `false`.
    fn requires_live_user_turn(&self) -> bool {
        false
    }

    /// Short search hint (3-10 words) for deferred tool discovery.
    /// Used by ToolSearch to match user intent without loading full schemas.
    fn search_hint(&self) -> &str {
        self.description()
    }

    /// Whether this tool's full schema should be deferred (not sent every turn).
    /// Override to return true for tools with large parameter schemas.
    fn should_defer(&self) -> bool {
        false
    }

    /// Visibility policy for this tool. Used by `SessionBuilder` to filter the
    /// tool list to those allowed by the active `SessionProfile`.
    /// Defaults to `Standard` so all existing core tools remain visible in
    /// Standard and Full sessions. Override to `Weak` for tools that should
    /// appear even in lightweight relay sessions.
    fn tool_policy(&self) -> hq_tools::registry::ToolPolicy {
        hq_tools::registry::ToolPolicy::Standard
    }

    /// Guidance on *how* to use this tool well, injected into the system
    /// prompt rather than sent per-turn. Mirrors `HqTool::behavioral_prompt`.
    /// Default `None` — only tools with non-obvious usage rules need it.
    fn behavioral_prompt(&self) -> Option<&str> {
        None
    }

    /// Cooperative per-call deadline in milliseconds, enforced once by
    /// `GovernedTool::execute` (distilled from DeepSeek Harness's
    /// `tools/execute` timeout middleware: the tool declares the budget, one
    /// shared wrapper enforces it). Default `None` — no deadline.
    ///
    /// This is advisory to the wrapper, not a hard kill: only tools whose
    /// `execute` genuinely forwards a cancellation signal to the work they
    /// await should declare a value, since a tool that ignores cancellation
    /// keeps running after the wrapper gives up and reports `TOOL_TIMEOUT`.
    fn timeout_ms(&self) -> Option<u64> {
        None
    }
}

/// Registry holding all available tools for a session.
///
/// Tools are stored behind [`Arc`] so an execution handle can be cloned out of
/// the registry (see [`get_shared`](Self::get_shared)) and awaited *without*
/// holding the registry lock across the await — letting concurrent-safe
/// (read-only) tools actually run in parallel while governance state (call
/// counters, denial tracking) stays shared through the same `Arc`.
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn AgentTool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool. Overwrites any existing tool with the same name.
    pub fn register(&mut self, tool: Box<dyn AgentTool>) {
        let name = tool.name().to_string();
        // Box -> Arc is a pointer move (no re-allocation of the trait object).
        self.tools.insert(name, Arc::from(tool));
    }

    /// Look up a tool by name (borrowing handle, tied to the registry lock).
    pub fn get(&self, name: &str) -> Option<&dyn AgentTool> {
        self.tools.get(name).map(|t| &**t)
    }

    /// Obtain a cloneable, `'static` execution handle for a tool by name.
    ///
    /// Unlike [`get`](Self::get), the returned [`Arc`] outlives the registry
    /// borrow, so callers can release the registry lock *before* awaiting
    /// `execute` — the fix that lets read-only tools run concurrently instead of
    /// serializing on the shared registry mutex. Governance counters and deferred
    /// activation are preserved because the handle points at the same governed
    /// tool instance stored in the registry.
    pub fn get_shared(&self, name: &str) -> Option<Arc<dyn AgentTool>> {
        self.tools.get(name).cloned()
    }

    /// Return tool definitions for LLM function calling.
    ///
    /// Sorted by name: definitions ride in the cached request prefix, and
    /// `HashMap` iteration order is randomized per process, so unsorted
    /// output would invalidate that cache on every restart.
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let mut out: Vec<ToolDefinition> = self.tools.values().map(definition_of).collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Iterate over tool names.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.tools.keys().map(|s| s.as_str())
    }

    /// Return compact catalog entries (name + search hint) for deferred tools.
    /// Non-deferred tools are excluded since their full schemas are sent directly.
    /// Sorted by name for the same prompt-cache reason as `definitions()`.
    pub fn deferred_catalog(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .tools
            .values()
            .filter(|t| t.should_defer())
            .map(|t| (t.name().to_string(), t.search_hint().to_string()))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Return the full definition for a single tool by name.
    /// Used by ToolSearch to load schemas on demand.
    pub fn definition_for(&self, name: &str) -> Option<ToolDefinition> {
        self.tools.get(name).map(definition_of)
    }

    /// Return definitions for non-deferred tools only.
    /// Deferred tools should be loaded on demand via `definition_for()`.
    /// Sorted by name for the same prompt-cache reason as `definitions()`.
    pub fn active_definitions(&self) -> Vec<ToolDefinition> {
        let mut out: Vec<ToolDefinition> = self
            .tools
            .values()
            .filter(|t| !t.should_defer())
            .map(definition_of)
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Per-tool usage guidance for the tools that provide it, as a
    /// system-prompt section.
    ///
    /// Sessions get this instead of a name+hint enumeration: their tool
    /// schemas already ship in full every turn, so listing the names again
    /// is pure duplication. Notes are truncated on a char boundary.
    pub fn behavioral_block(&self, max_chars_per_note: usize) -> String {
        let mut notes: Vec<(&str, &str)> = self
            .tools
            .values()
            .filter_map(|t| t.behavioral_prompt().map(|bp| (t.name(), bp)))
            .collect();
        if notes.is_empty() {
            return String::new();
        }
        notes.sort_by(|a, b| a.0.cmp(b.0));

        let mut out = String::with_capacity(notes.len() * (max_chars_per_note + 24) + 64);
        out.push_str("## Tool Usage Notes\n\n");
        for (name, note) in notes {
            out.push_str(&format!(
                "- **{name}**: {}\n",
                hq_tools::registry::truncate_note(note, max_chars_per_note)
            ));
        }
        out
    }

    /// Compact `name: hint` catalog grouped by category.
    ///
    /// Only used for weak sessions, where the tool set is small enough that
    /// enumerating it is cheaper than the schemas it replaces.
    pub fn catalog_block_by_category(&self) -> String {
        let mut visible: Vec<&Arc<dyn AgentTool>> = self.tools.values().collect();
        if visible.is_empty() {
            return String::new();
        }
        visible.sort_by(|a, b| (a.category(), a.name()).cmp(&(b.category(), b.name())));

        let mut out = String::with_capacity(visible.len() * 60 + 128);
        out.push_str(&format!("## Your Tools ({} available)\n", visible.len()));
        let mut current: Option<&str> = None;
        for tool in &visible {
            if current != Some(tool.category()) {
                current = Some(tool.category());
                out.push_str(&format!("\n**{}**\n", tool.category()));
            }
            let hint = hq_tools::registry::derive_search_hint(tool.search_hint());
            out.push_str(&format!("- `{}` — {hint}\n", tool.name()));
        }
        out
    }
}

fn definition_of(tool: &Arc<dyn AgentTool>) -> ToolDefinition {
    ToolDefinition {
        name: tool.name().to_string(),
        description: tool.description().to_string(),
        parameters: tool.parameters(),
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
