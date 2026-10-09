//! System prompt assembly for `SessionBuilder::build`.

use std::path::Path;
use std::sync::Arc;

use tracing::info;

use super::{SessionBuilder, SessionRole};
use crate::session::AgentSession;

/// What prompt assembly needs from the earlier build steps.
pub(super) struct PromptInputs<'a> {
    pub(super) vault_path: &'a Path,
    pub(super) shared_db: Option<Arc<hq_db::Database>>,
    pub(super) preset: crate::tool_policy::Preset,
    pub(super) tool_count: usize,
    pub(super) tool_notes: Option<&'a str>,
    pub(super) weak_catalog: Option<&'a str>,
    pub(super) context_window: usize,
    pub(super) identity: String,
}

impl SessionBuilder {
    /// Set the session's system prompt: the caller's override, or the full
    /// context assembly unless `skip_context` is set.
    pub(super) async fn apply_system_prompt(
        &self,
        session: &mut AgentSession,
        inputs: PromptInputs<'_>,
    ) {
        let vault_path = inputs.vault_path;
        if let Some(prompt) = self.system_prompt.clone() {
            // Explicit override: caller takes full responsibility.
            // Used by sub-agents with custom prompts, relay bots that
            // still use the legacy path, and tests.
            session.set_system_prompt(prompt);
        } else if !self.skip_context {
            // Full 5-layer context assembly.
            // Layers: System (soul) -> UserMessage -> Memory -> Thread -> Injections.
            // Token-budgeted via priority knapsack, with reducer pipeline fallback.
            let sys_ctx = hq_vault::system::get_system_context(vault_path).unwrap_or_default();

            // Same soul as the relays: frontmatter stripped, with a fallback identity.
            let soul = hq_vault::system::load_soul(vault_path);
            let user_model = hq_vault::system::get_user_model(vault_path);
            let mut soul_with_user = if user_model.trim().is_empty() {
                soul
            } else {
                format!("{soul}\n\n---\n{user_model}")
            };
            soul_with_user.push_str(&format!("\n\n{}", inputs.identity));
            if let Some(ref identity) = self.identity
                && self.enable_thread_continuity
            {
                soul_with_user.push_str(&format!(
                    "\n\nThis conversation is happening via {}. Thread lines marked \
                     [via <interface>, HH:MM] happened on other interfaces; they are \
                     part of the same ongoing conversation with the same person.",
                    identity.source.label()
                ));
            }

            if let Some(ref identity) = self.identity
                && matches!(identity.source, hq_core::identity::RequestSource::Web { .. })
            {
                soul_with_user.push_str(WEB_CHART_GUIDANCE);
            }

            // Dynamic memory from MemoryQuerier (merged with static MEMORY.md)
            let memory_text = build_memory_context(&inputs.shared_db, &sys_ctx.memory);

            // Cross-session thread continuity
            let thread = if self.enable_thread_continuity {
                let source = self
                    .identity
                    .as_ref()
                    .map(|i| i.source.label())
                    .unwrap_or("cli");
                let merged = crate::threads::load_merged_thread(
                    vault_path,
                    source,
                    crate::threads::MAX_MERGED_MESSAGES,
                    !self.exclude_own_interface_thread,
                );
                if merged.is_empty() {
                    // Transition path: no JSONL threads written yet, fall back
                    // to the legacy per-session *.json files.
                    load_recent_thread(vault_path)
                } else {
                    merged
                }
            } else {
                vec![]
            };

            // Build harness instructions block
            // Read from the daemon's cache, never probed inline: a prompt
            // build must not block on a subprocess fan-out, and a per-build
            // probe would also make the cached prefix shift mid-session.
            let (machine_block, can_build_self) =
                match hq_core::machine::load_cached(vault_path, MACHINE_PROFILE_MAX_AGE) {
                    Some(block) => {
                        let can_build_self = hq_core::machine::load_cached_profile(vault_path)
                            .map(|p| p.can_build_self)
                            .unwrap_or(false);
                        (Some(block), can_build_self)
                    }
                    None => {
                        // No daemon has written a profile yet (fresh install, daemon
                        // down). A 6-binary existence-only probe keeps the first
                        // session from being blind.
                        tracing::debug!("no cached machine profile; running fast probe");
                        let profile = hq_core::machine::probe_machine_fast(Some(vault_path));
                        let can_build_self = profile.can_build_self;
                        (
                            Some(hq_core::machine::render_markdown(&profile)),
                            can_build_self,
                        )
                    }
                };

            let harness_instructions = build_harness_block(
                self.harness_instructions.as_deref(),
                inputs.preset,
                self.working_dir.as_deref(),
                inputs.tool_count,
                DerivedPromptBlocks {
                    machine_block: machine_block.as_deref(),
                    tool_notes: inputs.tool_notes,
                    tool_catalog: inputs.weak_catalog,
                    can_build_self,
                    role: self.role,
                },
            );

            // Context assembly happens before the caller's next turn is known
            // (see `session::loop::prompt`, where the real message enters later),
            // so there is no current-turn message in scope here. The most recent
            // user turn from thread continuity is the best available proxy for
            // "what the user is talking about" at frame-build time.
            let last_user_message = thread
                .iter()
                .rev()
                .find(|m| m.role == "user")
                .map(|m| m.content.clone())
                .unwrap_or_default();
            let concept_results = hq_vault::VaultClient::new(vault_path.to_path_buf())
                .map(|v| concept_search_results(&v, &last_user_message))
                .unwrap_or_default();

            // Call the ContextEngine.
            // Clone soul_with_user before moving into FrameInput (needed for fallback).
            let soul = soul_with_user.clone();
            let frame = crate::context::layers::FrameInput {
                profile: "standard".to_string(),
                total_tokens: inputs.context_window,
                soul: soul_with_user.clone(),
                harness_instructions,
                user_message: String::new(),
                memory: memory_text,
                private_tags: vec![],
                thread,
                pinned_notes: sys_ctx.pinned_notes,
                search_results: concept_results,
                query_entities: vec![],
            };

            let capabilities = crate::context::cache_strategy::LlmCapabilities {
                exact_context_window: inputs.context_window,
                ..crate::context::cache_strategy::LlmCapabilities::default()
            };

            let engine = crate::context::engine::ContextEngine::new();
            let system_prompt = match engine.build_context(frame, capabilities).await {
                Ok((blocks, manifest)) => {
                    info!(
                        budget_used = manifest.budget_used,
                        total_budget = manifest.total_budget,
                        dropped = manifest.dropped_items_by_id.len(),
                        reducers = manifest.reducers_applied.len(),
                        "ContextEngine: assembled context"
                    );
                    blocks_to_system_prompt(&blocks)
                }
                Err(e) => {
                    tracing::warn!(%e, "ContextEngine failed, falling back to vault soul");
                    soul
                }
            };

            // Enrich with skill hints
            let skills_dir = hq_core::skills_dir(vault_path);
            let skill_index = hq_tools::skills::SkillHintIndex::build(&skills_dir);
            // Catalog only: hint matching needs the user's instruction, which
            // does not exist yet. The session runs the match itself on the
            // first turn (`enrich_with_matching_skills`) — passing an empty
            // instruction here is why no skill had ever auto-loaded.
            let (enriched, _) = hq_tools::skills::enrich_system_prompt(
                &skill_index,
                &system_prompt,
                "",
                None,
                Some(MAX_SKILL_TOKENS),
            );
            session.set_system_prompt(enriched);
            session.set_skill_index(std::sync::Arc::new(skill_index), MAX_SKILL_TOKENS);
        }
    }
}

/// Scans `_graph/` for a concept page whose slug appears as a substring of
/// the user's message (simple substring match today — this is the seam a
/// later LLM-classifier-driven concept lookup would replace, not a final
/// design). Returns at most 3 hits so it never dominates the injections
/// budget layer.
pub(super) fn concept_search_results(
    vault: &hq_vault::VaultClient,
    user_message: &str,
) -> Vec<hq_core::types::SearchResult> {
    let Ok(pages) = vault.list_notes_recursive("_graph") else {
        return vec![];
    };
    let lower_message = user_message.to_lowercase();
    let mut hits = Vec::new();

    for rel_path in pages {
        if rel_path.starts_with("_graph/_archive/") {
            continue;
        }
        let Ok(note) = vault.read_note(&rel_path) else {
            continue;
        };
        let lower_title = note.title.to_lowercase();
        let slug_words: Vec<&str> = lower_title.split_whitespace().collect();
        let matches =
            !slug_words.is_empty() && slug_words.iter().all(|w| lower_message.contains(w));
        if matches {
            hits.push(hq_core::types::SearchResult {
                note_path: rel_path.clone(),
                title: note.title.clone(),
                notebook: "_graph".to_string(),
                snippet: hq_core::text::truncate_chars(&note.content, 400).to_string(),
                tags: vec![],
                relevance: 1.0,
                match_type: hq_core::types::MatchType::Keyword,
            });
        }
        if hits.len() >= 3 {
            break;
        }
    }

    hits
}

/// Build memory context: static MEMORY.md + dynamic MemoryQuerier results.
pub(super) fn build_memory_context(
    db: &Option<Arc<hq_db::Database>>,
    static_memory: &str,
) -> String {
    let mut parts = Vec::new();
    if !static_memory.is_empty() {
        parts.push(static_memory.to_string());
    }
    if let Some(db) = db {
        let mut querier = hq_memory::querier::MemoryQuerier::new(db.as_ref().clone());
        match querier.get_recent_context(Some(8), None) {
            Ok(ctx) if !ctx.formatted.is_empty() => parts.push(ctx.formatted),
            Ok(_) => {}
            Err(e) => tracing::warn!(%e, "MemoryQuerier: failed to retrieve dynamic memories"),
        }
    }
    parts.join("\n\n")
}

/// Load recent conversation thread from `.vault/_threads/`.
pub(super) fn load_recent_thread(
    vault_path: &std::path::Path,
) -> Vec<crate::context::layers::ConversationMessage> {
    let threads_dir = vault_path.join("_threads");
    if !threads_dir.is_dir() {
        return vec![];
    }
    let mut entries: Vec<_> = match std::fs::read_dir(&threads_dir) {
        Ok(iter) => iter.filter_map(|e| e.ok()).collect(),
        Err(_) => return vec![],
    };
    entries.sort_by(|a, b| {
        let ta = a
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let tb = b
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        tb.cmp(&ta)
    });
    let mut messages: Vec<crate::context::layers::ConversationMessage> = Vec::new();
    let max_messages = 20;
    let mut sessions_loaded = 0usize;
    for entry in &entries {
        if messages.len() >= max_messages || sessions_loaded >= 3 {
            break;
        }
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(msgs) = serde_json::from_str::<Vec<ThreadMessage>>(&content) {
                for msg in msgs {
                    if messages.len() >= max_messages {
                        break;
                    }
                    let role = match msg.role.as_str() {
                        "user" => "user",
                        "assistant" => "assistant",
                        _ => continue,
                    };
                    messages.push(crate::context::layers::ConversationMessage {
                        role: role.to_string(),
                        content: truncate_to_chars(&msg.content, 2000),
                    });
                }
                sessions_loaded += 1;
            } else if let Ok(msg) = serde_json::from_str::<ThreadMessage>(&content) {
                let role = match msg.role.as_str() {
                    "user" => "user",
                    "assistant" => "assistant",
                    _ => continue,
                };
                messages.push(crate::context::layers::ConversationMessage {
                    role: role.to_string(),
                    content: truncate_to_chars(&msg.content, 2000),
                });
                sessions_loaded += 1;
            }
        }
    }
    messages.reverse();
    messages
}

#[derive(serde::Deserialize)]
pub(super) struct ThreadMessage {
    role: String,
    content: String,
}

/// How stale a cached machine profile may be before the injected block is
/// flagged. The daemon refreshes every 30 minutes; six hours means a daemon
/// that has been down a while is visibly untrusted rather than silently wrong.
/// The web app only draws charts from this block; ASCII bars and Mermaid are shown as plain text.
const WEB_CHART_GUIDANCE: &str = "\n\nThe web app draws charts. To chart data, put a fenced code block tagged `chart` \
in the reply, whose body is one JSON object: {\"type\": \"line\"|\"bar\"|\"pie\"|\"donut\", \"title\": string, \
\"labels\": [string], \"series\": [{\"name\": string, \"values\": [number]}]} with one value per label. \
Use it instead of ASCII bars or Mermaid, and still state the key numbers in the text.";

pub(super) const MACHINE_PROFILE_MAX_AGE: std::time::Duration =
    std::time::Duration::from_secs(6 * 3600);

/// Longest per-tool behavioral note admitted into the prompt.
pub(super) const MAX_BEHAVIORAL_NOTE_CHARS: usize = 280;

/// Ceiling on auto-loaded skill content injected into the system prompt.
pub(crate) const MAX_SKILL_TOKENS: usize = 3000;

/// Prompt sections derived from the host and the assembled tool registry,
/// rather than from the caller.
#[derive(Default)]
pub(super) struct DerivedPromptBlocks<'a> {
    /// Cached `_system/MACHINE.md` — what is installed on this host.
    pub(super) machine_block: Option<&'a str>,
    /// Per-tool usage guidance for the tools that carry it.
    pub(super) tool_notes: Option<&'a str>,
    /// Name-and-hint enumeration. Weak sessions only.
    pub(super) tool_catalog: Option<&'a str>,
    /// Whether this host has a reachable agent-hq checkout + cargo/git, per
    /// `MachineProfile::can_build_self`. Gates the Self-Management block's
    /// claim that the agent can build/modify its own source here.
    pub(super) can_build_self: bool,
    pub(super) role: SessionRole,
}

/// Replaces the Self-Management and coding Tool Usage blocks for an orchestrator.
const ORCHESTRATOR_ROLE_BLOCK: &str = "## Role\n\nYou are the HQ orchestrator. You plan, delegate, monitor and report. \
    You do not edit repository files or run state-changing commands yourself. Code and file changes go to a child \
    agent or a coding-agent session started with a task ID, a goal and done criteria. Before answering from files, \
    search, read the relevant lines and cite them.\n";

/// The single injection point for every surface. Thirteen call sites reach
/// this through `.harness_instructions()` — the CLI, hq-web, Discord, and the
/// Telegram relay via `native_hq::configure_native_hq_builder` — against only
/// two genuine `.system_prompt()` overrides. Anything added here reaches all
/// of them, which is why the relay's hand-maintained tool list could be
/// deleted instead of duplicated.
pub(super) fn build_harness_block(
    caller_instructions: Option<&str>,
    preset: crate::tool_policy::Preset,
    working_dir: Option<&std::path::Path>,
    tool_count: usize,
    derived: DerivedPromptBlocks<'_>,
) -> String {
    let DerivedPromptBlocks {
        machine_block,
        tool_notes,
        tool_catalog,
        can_build_self,
        role,
    } = derived;
    let mut parts = Vec::new();
    parts.push(format!(
        "## Environment\n\n- Platform: {}\n- Date: {}\n- Tools: {} available\n- Model routing: HQ harness with multi-provider LLM router\n",
        std::env::consts::OS, chrono::Local::now().format("%Y-%m-%d"), tool_count,
    ));
    if let Some(wd) = working_dir {
        parts.push(format!("- Working directory: {}\n", wd.display()));
    }
    // What is actually installed on this host. Without it the agent guesses,
    // and it guessed wrong — denying GitHub access with `gh` authenticated.
    if let Some(machine) = machine_block {
        parts.push(machine.to_string());
    }
    if role == SessionRole::Orchestrator {
        parts.push(ORCHESTRATOR_ROLE_BLOCK.to_string());
    } else {
        parts.push(if can_build_self {
            "## Self-Management\n\nYou run on the Agent-HQ binary built from \
             the agent-hq repository. Your vault is your working memory and you may \
             reorganize it on your own judgment. Development work on your own source \
             is yours to do: use your file and shell tools, keep changes on git \
             branches, and verify with cargo before shipping.\n"
                .to_string()
        } else {
            "## Self-Management\n\nYour vault is your working memory and you may \
             reorganize it on your own judgment. This host cannot build or modify \
             its own source: no reachable agent-hq checkout with cargo/git available. \
             Source changes to Agent-HQ happen on the host that has the checkout — \
             don't claim you can branch, edit, or `cargo build` this repo from here.\n"
                .to_string()
        });
        parts.push(
            "## Tool Usage\n\nYou are the primary coding agent. Use your tools proactively.\n\
             Always verify by reading actual code before making claims.\n\
             **MANDATORY: Search -> Read -> Answer.**\n\
             1. Use `grep` to find relevant code.\n\
             2. Use `read_file` with specific line ranges before editing.\n\
             3. Answer ONLY with evidence: file paths, line numbers, quoted code.\n\
             **Key tools:** `grep`, `read_file`, `edit_file`.\n"
                .to_string(),
        );
    }
    let delegation = match preset {
        crate::tool_policy::Preset::LocalGemma => {
            "**Delegation:** Use `call_code_reasoner`, `call_planner_strong`, `call_verifier_cheap`, `call_web_researcher`.\n"
        }
        crate::tool_policy::Preset::Cloud => {
            "**Delegation:** `spawn_subagents` for parallel research.\n"
        }
        crate::tool_policy::Preset::Subagent => "",
        // Named agents use their own profile; no delegation hint added.
        crate::tool_policy::Preset::Named(_) => "",
    };
    if !delegation.is_empty() {
        parts.push(delegation.to_string());
    }
    // Weak sessions only. Standard sessions already ship every tool's full
    // JSON schema each turn, so a name+hint enumeration would be duplication;
    // and with no universal dispatcher on this path, listing tools the session
    // cannot call just invites hallucinated calls.
    if let Some(catalog) = tool_catalog {
        parts.push(catalog.to_string());
    }
    if let Some(notes) = tool_notes {
        parts.push(notes.to_string());
    }
    if let Some(instr) = caller_instructions {
        parts.push(instr.to_string());
    }
    parts.join("\n")
}

pub(super) fn blocks_to_system_prompt(blocks: &[crate::context::block::ContextBlock]) -> String {
    blocks
        .iter()
        .filter(|b| matches!(b.role, hq_core::types::MessageRole::System))
        .map(|b| b.content.as_ref())
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(super) fn truncate_to_chars(s: &str, max_chars: usize) -> String {
    hq_core::text::truncate_chars_with(s, max_chars, "...")
}
