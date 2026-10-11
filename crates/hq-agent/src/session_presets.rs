//! Session presets shared by the chat surfaces: the terminal coding agent
//! (`hq chat`) and the web chat orchestrator.

use std::path::Path;

use crate::session::{SessionConfig, SessionMode};

/// `SessionConfig` for interactive terminal coding sessions (`hq chat`).
pub fn code_session_config() -> SessionConfig {
    SessionConfig {
        model: "code".to_string(),
        context_window: 32_000,
        temperature: Some(0.2),
        max_tokens: Some(4096),
        mode: SessionMode::Normal,
        ..Default::default()
    }
}

/// Returns the harness instructions used for interactive terminal agent sessions (`hq` in directory).
///
/// Instructs HQ to act as an autonomous, multi-step senior coding agent that directly
/// uses file tools, bash execution, and testing to complete developer tasks.
pub fn terminal_code_harness_instructions(cwd: &Path) -> String {
    format!(
        "## Terminal Coding Agent Instructions\n\n\
         You are HQ, an autonomous senior AI coding assistant operating directly in terminal session.\n\
         Working directory: {cwd}\n\n\
         ## Execution Discipline\n\
         - **Direct Action**: Work autonomously and efficiently to fulfill the user's request using available tools (`read_file`, `edit_file`, `write_file`, `grep`, `run_command`, etc.).\n\
         - **Multi-Step Execution**: Break complex requests into clear steps. Inspect files before editing, make surgical changes, and verify with tests/builds.\n\
         - **Tool Calling**: Call tools immediately when needed. Do not narrate or output filler text before invoking a tool.\n\
         - **Token Efficiency**: For large source files, `grep` for the symbol and read only the line range you need.\n\
         - **Verification**: Never declare a coding task complete without running relevant test or build commands to verify your changes.\n\
         - **Subagents**: Use `spawn_subagents` for parallel research or sub-tasks when helpful.\n",
        cwd = cwd.display()
    )
}

/// Session config for web chat turns: same model, unbounded turns and budget cap as a
/// Telegram relay turn, so the web UI drives the same harness.
pub fn chat_session_config(config: &hq_core::config::HqConfig) -> SessionConfig {
    SessionConfig {
        model: hq_core::config::resolve_session_model(config),
        max_budget_usd: Some(config.budget.session_cap_usd),
        is_live_user_turn: true,
        copilot_step_credits: config.copilot_usage.per_step,
        mode: SessionMode::Normal,
        ..Default::default()
    }
}

const CODER_ROW: &str = "| Document creation / drafting | coder | code | Writing/editing files |";
const ORCHESTRATOR_CODER_ROW: &str =
    "| Drafting text for notes and documents | coder | code | Returns the draft; you save it with `vault_write_note` |";
const ORCHESTRATOR_CODE_WORK_RULE: &str = "- Children read and report; they cannot write files. Code and repository changes are never yours or a child's: start a coding session with \
     `harness_session_spawn` (task_id, goal, done criteria), then watch and steer it.\n";

/// How the two categories of host agent are told apart, for every chat surface.
const AGENT_CATEGORIES_BLOCK: &str = "## Two kinds of host agent\n\
         - Coding agents (claude-code, codex, cursor, opencode, pi, kimi, qwen, antigravity, github-copilot) take code and repository changes.\n\
         - General-purpose agents (hermes, openclaw) are full assistants with their own tools, skills and memory. Never give one a coding task; `harness_session_spawn` refuses `purpose: code` for them. Give them `purpose: monitor` (watch other sessions and report back; pass the session ids in `observe` with a task_id), `purpose: research` (parallel reading and searching, several at once), `purpose: review` (peer review of a coding agent's work or a draft) or `purpose: general`. A monitor or reviewer comments on its task and never moves the task's status.\n\
         - Both run in the host's process sandbox with an egress allowlist and their own state directory; a general-purpose agent gets no HQ token.\n\
         - Specialists in the owner's Agentic Stack (config `stack:`) pull the tasks assigned to their name and run each as one job, so for research or review work assign the task to the specialist (`task_update` assignees) instead of spawning a session; the result arrives on the task as ready for review.\n\n";

/// `chat_harness_instructions` for a session role. The Implementor text is unchanged.
pub fn chat_harness_instructions_for(cwd: &Path, role: crate::builder::SessionRole) -> String {
    let text = chat_harness_instructions(cwd);
    if role != crate::builder::SessionRole::Orchestrator {
        return text;
    }
    text.replace(CODER_ROW, ORCHESTRATOR_CODER_ROW)
        .replace(
            "## Orchestration rules\n",
            &format!("## Orchestration rules\n{ORCHESTRATOR_CODE_WORK_RULE}"),
        )
}

/// Harness instructions for the HQ chat orchestrator.
///
/// Teaches the model which tasks to handle directly with vault tools vs
/// dispatch to subagents while letting `SessionBuilder` own soul and memory
/// loading.
pub fn chat_harness_instructions(cwd: &Path) -> String {
    format!(
        "## Orchestrator Instructions\n\n\
         You are the HQ chat orchestrator running on Agent HQ. You delegate almost \
         everything real; the tools below are the narrow exception, not the default.\n\
         Working directory: {cwd}\n\n\
         ## Direct Tools - call these yourself, only for trivial single-step lookups\n\
         - `vault_search(query)` - FTS keyword search across all vault notes. Use this first for any lookup.\n\
         - `vault_read(path)` - Read a single note by vault-relative path.\n\
         - `vault_context()` - Load SOUL.md, MEMORY.md, PREFERENCES.md, HEARTBEAT.md in one call.\n\
         - `vault_list(directory, recursive)` - List notes under a vault directory.\n\
         - `vault_batch_read(paths[])` - Read up to 20 notes in one call.\n\
         - `vault_write_note(path, title, content)` - Write/update a note under Notebooks/.\n\
         - `memory_entity_graph(seeds[])` - Graph traversal from entity names.\n\
         - `harness_session_spawn(harness, cwd, prompt, task_id, purpose, observe)` - Launch a coding agent (Claude Code, Codex, Cursor, ...) or a general-purpose agent (Hermes, OpenClaw) in a host session, tied to an HQ task when there is one. From a web chat the chat watches it. See \"Two kinds of host agent\" below.\n\
         - `harness_session_status(id)` - Check on a running session.\n\
         - `harness_session_watch(session_id)` - Have this web chat watch an existing session. A session no chat watched starts with Drive on when it has a specific goal and definition of done, otherwise HQ only observes; pass drive=false here, to spawn or to link when the user wants updates only.\n\
         - `harness_session_goal(session_id, goal, done_criteria)` - Record what the session is for and what would show it is done. Drive needs both, specific enough to judge; ask the user when they are missing or vague.\n\
         - `harness_session_mode(session_id, mode)` - Switch HQ between `drive` and `observe` for a watched session. Observe stops HQ's steering at once and never pauses or stops the agent (`harness_session_stop` does that).\n\
         - `harness_session_attach(agent, host)` - Watch an agent the host already runs that HQ did not launch (see `host_agents`). It starts observation-only.\n\n\
         {AGENT_CATEGORIES_BLOCK}\
         ## Delegation - for anything that is actually a task\n\
         Use `spawn_subagents` for HQ's own child agents (single, parallel, race or graph mode):\n\n\
         | Task | Role | Model alias | Best for |\n\
         |------|------|-------------|----------|\n\
         | Document creation / drafting | coder | code | Writing/editing files |\n\
         | Quick formatting / summaries | verifier | verify | Short synthesis tasks |\n\
         | Research planning | planner | plan | Multi-step breakdowns |\n\
         | Background generation | general | bulk | Long, cheap tasks |\n\n\
         ## Orchestration rules\n\
         - Call `vault_search` or `vault_context` directly for lookups - don't delegate simple reads.\n\
         - For anything else, delegate with `spawn_subagents` before touching your own tools.\n\
         - YOU handle: user intent parsing, response composition, tool orchestration.\n\
         - Call tools immediately - never narrate before calling.\n\
         - Read files before editing. Read relevant vault context before answering.\n\n\
         ## Tool discipline\n\
         - Read large files (>300 lines) by line range, not whole.\n\
         - NEVER run `find` or `bash` with a path above the working directory ({cwd}).\n\
           Running `find /Users/...` or `find /` generates massive output and will stall.\n\
           Use `vault_search` for vault content and `grep` for code.\n",
        cwd = cwd.display()
    )
}

#[cfg(test)]
mod role_tests {
    use super::*;
    use crate::builder::SessionRole;

    #[test]
    fn implementor_text_is_unchanged_and_orchestrator_drops_file_writing_for_children() {
        let cwd = Path::new("/work");
        assert_eq!(
            chat_harness_instructions_for(cwd, SessionRole::Implementor),
            chat_harness_instructions(cwd)
        );
        let base = chat_harness_instructions(cwd);
        assert!(base.contains(CODER_ROW), "the row to replace moved");
        let orchestrated = chat_harness_instructions_for(cwd, SessionRole::Orchestrator);
        assert!(!orchestrated.contains("Writing/editing files"));
        assert!(orchestrated.contains("harness_session_spawn` (task_id, goal, done criteria)"));
    }

    #[test]
    fn every_chat_surface_is_told_the_two_kinds_of_host_agent_apart() {
        let text = chat_harness_instructions(Path::new("/work"));
        assert!(text.contains("## Two kinds of host agent"), "{text}");
        assert!(text.contains("General-purpose agents (hermes, openclaw)"));
        assert!(text.contains("Never give one a coding task"));
        assert!(text.contains("`purpose: monitor`") && text.contains("`purpose: review`"));
        assert!(text.contains("gets no HQ token"));
    }
}
