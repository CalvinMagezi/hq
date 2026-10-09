//! Tool registry — trait definition, registry struct, discovery.

use anyhow::Result;
use async_trait::async_trait;
use hq_core::types::ValidationResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashMap;

/// Controls which session profiles can see a tool.
///
/// Tools at Weak level appear in all sessions (simple shortcuts, defaults).
/// Tools at Standard level are hidden from weak sessions (local/small models).
/// Tools at Full level are only visible in full sessions (direct use, Claude Code).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ToolPolicy {
    Weak,
    #[default]
    Standard,
    Full,
}

/// Every tool in the HQ system implements this trait.
///
/// The execution pipeline is:
/// 1. `validate(args)` — cheap structural checks before any I/O
/// 2. `execute(args)` — actual work (only called when validation passes)
/// 3. Result token estimation and optional truncation
///
/// This separation (distilled from claude-code's Tool.ts) keeps permission
/// checking and error reporting orthogonal to tool logic.
#[async_trait]
pub trait HqTool: Send + Sync {
    /// Machine-readable name (e.g. `vault_search`).
    fn name(&self) -> &str;

    /// Human-readable description shown during discovery.
    fn description(&self) -> &str;

    /// JSON Schema for the tool's input parameters.
    fn parameters(&self) -> Value;

    /// Validate arguments before execution. Run cheap checks here:
    /// required fields present, paths within allowed directories, file not
    /// modified since last read, no-op diffs, etc.
    ///
    /// Default implementation always succeeds — tools opt in to validation
    /// by overriding this method.
    ///
    /// Distilled from claude-code's `Tool.validateInput()` pattern.
    async fn validate(&self, _args: &Value) -> ValidationResult {
        ValidationResult::ok()
    }

    /// Execute the tool with the given arguments, returning a JSON result.
    /// Only called when `validate` returns `ValidationResult::Ok`.
    async fn execute(&self, args: Value) -> Result<Value>;

    /// Optional category tag used for filtering in `discover`.
    fn category(&self) -> &str {
        "general"
    }

    /// Short capability phrase (3-10 words) for tool search hints.
    /// Shown in compact discovery output when the LLM uses ToolSearch.
    /// Distilled from claude-code's `Tool.searchHint` field.
    ///
    /// `Some` means hand-written. Prefer [`HqTool::effective_hint`] when you
    /// need a hint for every tool — it derives one from `description()` for
    /// the majority that do not override this.
    fn search_hint(&self) -> Option<&str> {
        None
    }

    /// The hint to display for this tool, hand-written if available and
    /// mechanically derived from `description()` otherwise.
    fn effective_hint(&self) -> Cow<'_, str> {
        match self.search_hint() {
            Some(h) => Cow::Borrowed(h),
            None => Cow::Owned(derive_search_hint(self.description())),
        }
    }

    /// Whether this tool performs only read operations (no side effects).
    /// Used by permission modes to auto-approve safe tools.
    /// Distilled from claude-code's read-only tool classification.
    fn is_read_only(&self) -> bool {
        false
    }

    /// Whether this tool performs destructive/irreversible operations.
    /// Destructive tools get extra confirmation in governed agent sessions
    /// (`hq_agent::builder::HqToolAdapter` forwards this to `AgentTool`,
    /// which `GovernedTool` checks under `SecurityProfile::Guarded`).
    /// Default `!is_read_only()`: an imperfect proxy, but strictly better
    /// than a blanket `false` and matches every known genuinely-destructive
    /// tool (batch file edits, crypto transfers). Override when a
    /// non-read-only tool is not actually destructive, or vice versa.
    fn is_destructive(&self) -> bool {
        !self.is_read_only()
    }

    /// Excluded from any session not directly driven by a live user turn
    /// (daemon periodic tasks, mission-engine steps, any future background
    /// trigger) regardless of `AgentProfile` allow-lists. Enforced
    /// authoritatively by `hq_agent::governance::ToolGuardian::build_registry`,
    /// the sole constructor of a session's tool registry.
    ///
    /// A tool that proxies a real external account/platform should override
    /// this from its own config flag rather than a hardcoded literal, so a
    /// future integration opts in without touching shared gating code.
    /// Default `false`: almost every tool is fine to run unattended.
    fn requires_live_user_turn(&self) -> bool {
        false
    }

    /// Session profile gate. Tools at `Weak` appear in all sessions.
    /// Tools at `Standard` are hidden from weak (relay/sub-agent) sessions.
    /// Tools at `Full` are only shown in full sessions (direct use, Claude Code).
    /// Default: `Standard`.
    fn tool_policy(&self) -> ToolPolicy {
        ToolPolicy::Standard
    }

    /// Per-call deadline in milliseconds, forwarded to agent sessions whose
    /// `GovernedTool` enforces it. Default `None`, no deadline. Declare one
    /// only when `execute` stops its work on cancellation.
    fn timeout_ms(&self) -> Option<u64> {
        None
    }

    /// Behavioral prompt injected into the system prompt to guide the LLM
    /// on *how* to use this tool well (constraints, best practices, gotchas).
    /// Distilled from claude-code's per-tool `prompt.ts` pattern.
    ///
    /// Default is `None` — only tools with non-obvious usage rules need this.
    fn behavioral_prompt(&self) -> Option<&str> {
        None
    }

    /// Test-only introspection hook: exposes the address of a tool's internal
    /// shared state (e.g. an `Arc<dyn SecretStore>`'s data pointer) so tests
    /// can prove two tool instances share one allocation through a real
    /// factory function (`Box<dyn HqTool>` offers no downcasting otherwise).
    /// Default `None`; only tools that need this for a DI-sharing test override it.
    #[cfg(test)]
    fn test_shared_state_ptr(&self) -> Option<*const ()> {
        None
    }
}

/// Run `validate`, then `execute` only when it passes. Every caller of an
/// `HqTool` goes through here, so a tool's validation is never skipped.
pub async fn validate_and_execute(tool: &dyn HqTool, args: Value) -> Result<Value> {
    if let ValidationResult::Err { message, .. } = tool.validate(&args).await {
        anyhow::bail!("{message}");
    }
    tool.execute(args).await
}

const HINT_MAX_WORDS: usize = 9;
const HINT_MAX_BYTES: usize = 55;

/// Lowercase prefixes that carry no information once a description is
/// reduced to a capability phrase. Ordered longest-first so the longer
/// match wins.
const HINT_BOILERPLATE: &[&str] = &[
    "use this tool to ",
    "this tool is used to ",
    "a tool that lets you ",
    "use this tool for ",
    "a tool that ",
    "use this to ",
    "this tool ",
    "tool that ",
    "used to ",
    "used for ",
    "tool to ",
    "tool for ",
];

/// Words that read as truncation artifacts when a phrase ends on them.
const HINT_DANGLING: &[&str] = &[
    "and", "or", "to", "the", "for", "with", "a", "an", "of", "in", "on", "by", "from", "at", "as",
    "that", "into", "via",
];

/// Truncate at the largest char boundary at or below `max_bytes`.
fn truncate_on_char_boundary(s: &str, max_bytes: usize) -> &str {
    &s[..s.floor_char_boundary(max_bytes)]
}

/// Truncate a behavioral note without cutting a word in half.
///
/// These are instructions the model follows, so a note ending mid-word reads
/// as a corrupted directive. Backs up to the last word boundary and marks the
/// cut so it is visibly incomplete rather than subtly wrong.
pub fn truncate_note(note: &str, max_chars: usize) -> String {
    let note = note.trim();
    if note.len() <= max_chars {
        return note.to_string();
    }
    let clipped = truncate_on_char_boundary(note, max_chars);
    let cut = clipped.rfind(char::is_whitespace).unwrap_or(clipped.len());
    format!(
        "{}…",
        clipped[..cut]
            .trim_end_matches([',', ';', '—', '-'])
            .trim_end()
    )
}

/// Reduce a tool description to a short capability phrase.
///
/// Deterministic and content-preserving — it only cuts and strips, never
/// invents. Every tool gets a hint this way, so `catalog_block` stops
/// emitting bare names for the ~180 tools with no hand-written hint.
pub fn derive_search_hint(description: &str) -> String {
    // First clause only: a hint is a phrase, not a sentence.
    let first_clause = description
        .find(['.', '!', '\n'])
        .or_else(|| description.find(" ("))
        .map(|i| &description[..i])
        .unwrap_or(description)
        .trim();

    let lowered = first_clause.to_lowercase();
    let body = HINT_BOILERPLATE
        .iter()
        .find(|p| lowered.starts_with(**p))
        .map(|p| first_clause[p.len()..].trim())
        .unwrap_or(first_clause);

    let mut words: Vec<&str> = Vec::new();
    let mut bytes = 0usize;
    for word in body.split_whitespace() {
        let added = if words.is_empty() {
            word.len()
        } else {
            word.len() + 1
        };
        if words.len() >= HINT_MAX_WORDS || bytes + added > HINT_MAX_BYTES {
            break;
        }
        bytes += added;
        words.push(word);
    }

    // A single word over the byte cap would otherwise yield nothing.
    if words.is_empty()
        && let Some(first) = body.split_whitespace().next()
    {
        words.push(truncate_on_char_boundary(first, HINT_MAX_BYTES));
    }

    while words
        .last()
        .is_some_and(|w| HINT_DANGLING.contains(&w.to_lowercase().trim_matches(',')))
    {
        words.pop();
    }

    let phrase = words.join(" ");
    let phrase = phrase.trim_end_matches([',', ';', ':', '-']).trim();

    if phrase.is_empty() {
        return String::new();
    }

    // Lead lowercase so hints read as phrases, unless the first token is an
    // acronym or identifier the caller capitalized on purpose.
    let first_word = phrase.split_whitespace().next().unwrap_or("");
    let is_shouty = first_word.len() > 1
        && first_word
            .chars()
            .all(|c| c.is_uppercase() || !c.is_alphabetic());
    if is_shouty {
        return phrase.to_string();
    }

    let mut chars = phrase.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Compact summary returned by `list` and `discover`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSummary {
    pub name: String,
    pub description: String,
    pub category: String,
    pub parameters: Value,
    /// Short capability phrase (3-10 words) for compact tool search output.
    /// `None` if the tool does not provide a hint.
    /// Distilled from claude-code's `Tool.searchHint` field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_hint: Option<String>,
    /// Guidance on how to use the tool well. Carried here so consumers that
    /// only hold summaries (the MCP server, the capabilities API) can reach
    /// it without the trait object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavioral_prompt: Option<String>,
}

impl ToolSummary {
    fn from_tool(t: &dyn HqTool) -> Self {
        Self {
            name: t.name().to_string(),
            description: t.description().to_string(),
            category: t.category().to_string(),
            parameters: t.parameters(),
            search_hint: t.search_hint().map(|s| s.to_string()),
            behavioral_prompt: t.behavioral_prompt().map(|s| s.to_string()),
        }
    }
}

/// Central registry of all available tools.
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn HqTool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool. Overwrites any existing tool with the same name.
    pub fn register(&mut self, tool: Box<dyn HqTool>) {
        let name = tool.name().to_string();
        self.tools.insert(name, tool);
    }

    /// Look up a tool by name.
    pub fn get(&self, name: &str) -> Option<&dyn HqTool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    /// List summaries of every registered tool.
    pub fn list(&self) -> Vec<ToolSummary> {
        let mut out: Vec<ToolSummary> = self
            .tools
            .values()
            .map(|t| ToolSummary::from_tool(t.as_ref()))
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Return a sorted list of unique category names across all registered tools.
    pub fn categories(&self) -> Vec<String> {
        let mut cats: Vec<String> = self
            .tools
            .values()
            .map(|t| t.category().to_string())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        cats.sort();
        cats
    }

    /// Drop every tool `keep` rejects.
    pub fn retain(&mut self, keep: impl Fn(&dyn HqTool) -> bool) {
        self.tools.retain(|_, t| keep(t.as_ref()));
    }

    /// Return the total number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Discover tools matching an optional category and/or free-text query.
    ///
    /// - If `category` is `Some`, only tools whose category matches are returned.
    /// - If `query` is `Some`, the name and description are searched (case-insensitive substring).
    pub fn discover(&self, category: Option<&str>, query: Option<&str>) -> Vec<ToolSummary> {
        let mut out: Vec<ToolSummary> = self
            .tools
            .values()
            .filter(|t| {
                if let Some(cat) = category
                    && t.category() != cat
                {
                    return false;
                }
                if let Some(q) = query {
                    let q_lower = q.to_lowercase();
                    let in_name = t.name().to_lowercase().contains(&q_lower);
                    let in_desc = t.description().to_lowercase().contains(&q_lower);
                    if !in_name && !in_desc {
                        return false;
                    }
                }
                true
            })
            .map(|t| ToolSummary::from_tool(t.as_ref()))
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Return tool summaries visible at or below `max_policy`.
    ///
    /// Used by `SessionBuilder` to filter the tool list for weak sessions
    /// (local/small models, relay bots) so only simplified shortcuts are exposed.
    pub fn tools_by_policy(&self, max_policy: ToolPolicy) -> Vec<ToolSummary> {
        let mut out: Vec<ToolSummary> = self
            .tools
            .values()
            .filter(|t| t.tool_policy() <= max_policy)
            .map(|t| ToolSummary::from_tool(t.as_ref()))
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        if out.is_empty() {
            tracing::warn!(
                "tools_by_policy({max_policy:?}) returned no tools — shortcuts not registered yet?"
            );
        }
        out
    }

    /// Generate a compact tool catalog block for system prompt injection.
    ///
    /// Groups tools by category, one line per tool: name + description.
    /// Designed to be appended to system prompts so agents know what's
    /// available without calling `hq_discover`. ~50 bytes per tool.
    pub fn catalog_block(&self) -> String {
        self.catalog_block_filtered(ToolPolicy::Full)
    }

    /// Same as [`ToolRegistry::catalog_block`] but limited to tools visible at
    /// or below `max_policy`. Used for weak sessions, where enumerating the
    /// full catalog would advertise tools the session cannot call.
    ///
    /// Output is sorted by category then name so the block is byte-identical
    /// across process restarts — the enclosing system prompt is a cached
    /// prefix, and `HashMap` iteration order would invalidate it.
    pub fn catalog_block_filtered(&self, max_policy: ToolPolicy) -> String {
        let mut visible: Vec<&dyn HqTool> = self
            .tools
            .values()
            .map(|t| t.as_ref())
            .filter(|t| t.tool_policy() <= max_policy)
            .collect();
        if visible.is_empty() {
            return String::new();
        }
        visible.sort_by(|a, b| (a.category(), a.name()).cmp(&(b.category(), b.name())));

        let mut out = String::with_capacity(visible.len() * 60 + 256);
        out.push_str(&format!(
            "# Available HQ Tools ({} tools)\n\n\
             Use `hq_call(tool, args)` to invoke any tool directly.\n\n",
            visible.len()
        ));

        let mut current_category: Option<&str> = None;
        for tool in &visible {
            if current_category != Some(tool.category()) {
                current_category = Some(tool.category());
                let count = visible
                    .iter()
                    .filter(|t| t.category() == tool.category())
                    .count();
                out.push_str(&format!("\n**{}** ({}):\n", tool.category(), count));
            }
            out.push_str(&format!(
                "  - **{}**: {}\n",
                tool.name(),
                tool.effective_hint()
            ));
        }

        out
    }

    /// Per-tool guidance for the tools that provide it, as a system-prompt
    /// section. Separate from the catalog because sessions want the notes
    /// without the enumeration — their tool schemas already ship in full.
    ///
    /// Notes longer than `max_chars_per_note` are truncated on a char boundary.
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
                truncate_note(note, max_chars_per_note)
            ));
        }
        out
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct WeakTool;
    #[async_trait::async_trait]
    impl HqTool for WeakTool {
        fn name(&self) -> &str {
            "weak_tool"
        }
        fn description(&self) -> &str {
            "test"
        }
        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        fn tool_policy(&self) -> ToolPolicy {
            ToolPolicy::Weak
        }
        async fn execute(&self, _: serde_json::Value) -> anyhow::Result<serde_json::Value> {
            Ok(serde_json::json!("ok"))
        }
    }

    struct StandardTool;
    #[async_trait::async_trait]
    impl HqTool for StandardTool {
        fn name(&self) -> &str {
            "standard_tool"
        }
        fn description(&self) -> &str {
            "test"
        }
        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn execute(&self, _: serde_json::Value) -> anyhow::Result<serde_json::Value> {
            Ok(serde_json::json!("ok"))
        }
    }

    struct ReadOnlyTool;
    #[async_trait::async_trait]
    impl HqTool for ReadOnlyTool {
        fn name(&self) -> &str {
            "read_only_tool"
        }
        fn description(&self) -> &str {
            "test"
        }
        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        fn is_read_only(&self) -> bool {
            true
        }
        async fn execute(&self, _: serde_json::Value) -> anyhow::Result<serde_json::Value> {
            Ok(serde_json::json!("ok"))
        }
    }

    /// Not read-only, but explicitly not destructive either (e.g. a crypto
    /// price lookup that writes a cache file but isn't a fund transfer).
    struct ExplicitlyNonDestructiveTool;
    #[async_trait::async_trait]
    impl HqTool for ExplicitlyNonDestructiveTool {
        fn name(&self) -> &str {
            "explicitly_non_destructive_tool"
        }
        fn description(&self) -> &str {
            "test"
        }
        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        fn is_destructive(&self) -> bool {
            false
        }
        async fn execute(&self, _: serde_json::Value) -> anyhow::Result<serde_json::Value> {
            Ok(serde_json::json!("ok"))
        }
    }

    #[test]
    fn is_destructive_defaults_to_not_read_only() {
        assert!(!ReadOnlyTool.is_destructive());
        assert!(StandardTool.is_destructive());
    }

    #[test]
    fn is_destructive_override_wins_over_the_default() {
        assert!(!ExplicitlyNonDestructiveTool.is_destructive());
    }

    #[test]
    fn policy_ord_is_weak_lt_standard_lt_full() {
        assert!(ToolPolicy::Weak < ToolPolicy::Standard);
        assert!(ToolPolicy::Standard < ToolPolicy::Full);
        assert!(ToolPolicy::Weak < ToolPolicy::Full);
    }

    #[test]
    fn policy_filtering_excludes_standard_in_weak_session() {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(WeakTool));
        reg.register(Box::new(StandardTool));
        let weak_visible = reg.tools_by_policy(ToolPolicy::Weak);
        assert_eq!(weak_visible.len(), 1);
        assert!(weak_visible.iter().any(|s| s.name == "weak_tool"));
        let standard_visible = reg.tools_by_policy(ToolPolicy::Standard);
        assert_eq!(standard_visible.len(), 2);
    }

    /// A tool whose metadata the test controls, so catalog assertions don't
    /// depend on whatever the real registry happens to contain.
    struct FakeTool {
        name: String,
        description: String,
        category: String,
        behavioral: Option<String>,
    }

    impl FakeTool {
        fn new(name: &str, description: &str, category: &str) -> Self {
            Self {
                name: name.into(),
                description: description.into(),
                category: category.into(),
                behavioral: None,
            }
        }
        fn with_behavioral(mut self, note: &str) -> Self {
            self.behavioral = Some(note.into());
            self
        }
    }

    #[async_trait::async_trait]
    impl HqTool for FakeTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            &self.description
        }
        fn category(&self) -> &str {
            &self.category
        }
        fn behavioral_prompt(&self) -> Option<&str> {
            self.behavioral.as_deref()
        }
        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn execute(&self, _: serde_json::Value) -> anyhow::Result<serde_json::Value> {
            Ok(serde_json::json!("ok"))
        }
    }

    #[test]
    fn derive_search_hint_cuts_to_first_clause() {
        assert_eq!(
            derive_search_hint("Search the vault. Returns ranked notes with excerpts."),
            "search the vault"
        );
        assert_eq!(
            derive_search_hint("Read a file (with line numbers and offsets)"),
            "read a file"
        );
        assert_eq!(
            derive_search_hint("Start a recording\nSecond line is ignored"),
            "start a recording"
        );
    }

    #[test]
    fn derive_search_hint_strips_boilerplate_prefixes() {
        assert_eq!(
            derive_search_hint("Use this tool to commit staged changes"),
            "commit staged changes"
        );
        assert_eq!(
            derive_search_hint("A tool that renders diagrams"),
            "renders diagrams"
        );
        assert_eq!(
            derive_search_hint("Tool for querying the code graph"),
            "querying the code graph"
        );
    }

    #[test]
    fn derive_search_hint_respects_word_and_byte_caps() {
        let hint = derive_search_hint(
            "Alpha bravo charlie delta echo foxtrot golf hotel india juliett kilo lima",
        );
        assert!(hint.split_whitespace().count() <= HINT_MAX_WORDS, "{hint}");
        assert!(hint.len() <= HINT_MAX_BYTES, "{hint}");
    }

    #[test]
    fn derive_search_hint_drops_dangling_connectives() {
        // The byte cap would otherwise cut mid-phrase and end on "and".
        let hint = derive_search_hint(
            "Reconcile every outstanding transaction and settle the remainder later",
        );
        assert!(!hint.ends_with(" and"), "{hint}");
        assert!(!hint.ends_with(" the"), "{hint}");
    }

    #[test]
    fn derive_search_hint_preserves_leading_acronyms() {
        assert_eq!(
            derive_search_hint("AIDC project status"),
            "AIDC project status"
        );
        assert_eq!(derive_search_hint("Get the agenda"), "get the agenda");
    }

    #[test]
    fn derive_search_hint_handles_empty_and_multibyte_input() {
        assert_eq!(derive_search_hint(""), "");
        assert_eq!(derive_search_hint("   "), "");
        // Must not panic slicing a multibyte word longer than the byte cap.
        let long_multibyte = "\u{1F600}".repeat(40);
        let hint = derive_search_hint(&long_multibyte);
        assert!(hint.len() <= HINT_MAX_BYTES, "{}", hint.len());
        let mixed = derive_search_hint("Résumé análisis über naïve façade coöperate piñata");
        assert!(mixed.len() <= HINT_MAX_BYTES);
    }

    /// hq-agent applies the derivation to strings that may already be hints,
    /// so a second pass must be a no-op.
    #[test]
    fn derive_search_hint_is_idempotent() {
        let cases = [
            "Search the vault. Returns ranked notes.",
            "Use this tool to commit staged changes",
            "Alpha bravo charlie delta echo foxtrot golf hotel india juliett",
            "AIDC project status",
            "",
        ];
        for case in cases {
            let once = derive_search_hint(case);
            let twice = derive_search_hint(&once);
            assert_eq!(once, twice, "not idempotent for {case:?}");
        }
    }

    #[test]
    fn effective_hint_prefers_handwritten_over_derived() {
        struct Hinted;
        #[async_trait::async_trait]
        impl HqTool for Hinted {
            fn name(&self) -> &str {
                "hinted"
            }
            fn description(&self) -> &str {
                "A long description that would derive something else entirely"
            }
            fn search_hint(&self) -> Option<&str> {
                Some("hand written hint")
            }
            fn parameters(&self) -> serde_json::Value {
                serde_json::json!({})
            }
            async fn execute(&self, _: serde_json::Value) -> anyhow::Result<serde_json::Value> {
                Ok(serde_json::json!("ok"))
            }
        }
        assert_eq!(Hinted.effective_hint(), "hand written hint");
        assert_eq!(
            FakeTool::new("t", "Query the graph", "codegraph").effective_hint(),
            "query the graph"
        );
    }

    #[test]
    fn catalog_block_lists_every_tool_exactly_once_with_a_hint() {
        let mut reg = ToolRegistry::new();
        for (name, cat) in [("a_one", "alpha"), ("a_two", "alpha"), ("b_one", "beta")] {
            reg.register(Box::new(FakeTool::new(name, "Does a thing well", cat)));
        }
        let block = reg.catalog_block();
        for name in ["a_one", "a_two", "b_one"] {
            assert_eq!(block.matches(&format!("**{name}**")).count(), 1, "{block}");
        }
        // No bare names: every entry carries a hint after the colon.
        assert_eq!(block.matches("does a thing well").count(), 3, "{block}");
        assert!(block.contains("**alpha** (2)"), "{block}");
        assert!(block.contains("**beta** (1)"), "{block}");
    }

    /// The catalog lands in a cached system-prompt prefix. `HashMap` iteration
    /// order is randomized per process, so unsorted output would silently
    /// invalidate that cache on every restart.
    #[test]
    fn catalog_block_is_deterministic() {
        let build = || {
            let mut reg = ToolRegistry::new();
            for i in 0..60 {
                reg.register(Box::new(FakeTool::new(
                    &format!("tool_{i:03}"),
                    "Performs a representative operation on the input",
                    &format!("cat_{}", i % 7),
                )));
            }
            reg.catalog_block()
        };
        assert_eq!(build(), build());
    }

    #[test]
    fn catalog_block_stays_within_token_budget() {
        let mut reg = ToolRegistry::new();
        for i in 0..210 {
            reg.register(Box::new(FakeTool::new(
                &format!("tool_number_{i:03}"),
                "Performs a representative operation against the configured backend and returns it",
                &format!("category_{}", i % 43),
            )));
        }
        let block = reg.catalog_block();
        assert!(
            block.len() < 20_000,
            "catalog grew to {} bytes for 210 tools",
            block.len()
        );
    }

    #[test]
    fn catalog_block_filtered_hides_tools_above_policy() {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(WeakTool));
        reg.register(Box::new(StandardTool));
        let weak = reg.catalog_block_filtered(ToolPolicy::Weak);
        assert!(weak.contains("weak_tool"));
        assert!(!weak.contains("standard_tool"));
        assert!(reg.catalog_block().contains("standard_tool"));
    }

    #[test]
    fn behavioral_block_collects_only_annotated_tools_and_truncates() {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(FakeTool::new("plain", "Does a thing", "misc")));
        reg.register(Box::new(
            FakeTool::new("noted", "Does a thing", "misc").with_behavioral(&"x".repeat(400)),
        ));
        let block = reg.behavioral_block(180);
        assert!(block.contains("## Tool Usage Notes"));
        assert!(block.contains("**noted**"));
        assert!(!block.contains("**plain**"));
        assert!(block.contains(&"x".repeat(180)));
        assert!(!block.contains(&"x".repeat(181)));
    }

    #[test]
    fn behavioral_block_is_empty_without_annotated_tools() {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(FakeTool::new("plain", "Does a thing", "misc")));
        assert!(reg.behavioral_block(180).is_empty());
    }
}
