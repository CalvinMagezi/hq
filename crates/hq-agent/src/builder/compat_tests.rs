//! Contract tests: what a web-chat turn can do today. The orchestrator role
//! work must keep every name here callable for the Implementor role, and keep
//! the spawn and steer tools for the Orchestrator role too.

use super::*;
use crate::session::SessionConfig;
use hq_core::config::HqConfig;

/// Web chat runs a hosted model, which selects the Cloud tool preset.
const CLOUD_MODEL: &str = "claude-sonnet-5-5";

/// Tools a web-chat turn uses to spawn and steer coding sessions and children.
pub(super) const SPAWN_AND_STEER_TOOLS: &[&str] = &[
    "harness_session_spawn",
    "harness_session_handoff",
    "harness_session_attach",
    "harness_session_goal",
    "harness_session_mode",
    "harness_session_watch",
    "harness_session_link",
    "harness_session_send",
    "harness_session_status",
    "harness_session_list",
    "harness_session_logs",
    "harness_session_wait",
    "harness_session_resume",
    "harness_session_stop",
    "host_agents",
    "host_check",
    "host_list",
    "host_read",
    "host_send",
    "spawn_subagents",
    "report_progress",
    "subagent_run_status",
    "subagent_run_result",
    "subagent_run_list",
    "subagent_run_cancel",
    "subagent_run_review",
];

/// Planning, memory, task and reading tools an orchestrator relies on.
pub(super) const PLANNING_AND_READ_TOOLS: &[&str] = &[
    "vault_search",
    "vault_read",
    "vault_write_note",
    "vault_context",
    "vault_list",
    "vault_batch_read",
    "memory_entity_graph",
    "task_create",
    "task_update",
    "task_list",
    "task_get",
    "task_comment_add",
    "watch_create",
    "watch_list",
    "watch_cancel",
    "web_fetch",
    "web_search",
    "read_file",
    "grep",
    "find_files",
    "list_dir",
    "git_status",
    "git_log",
    "git_diff",
    "convert_from_markdown",
    "load_skill",
    "skill_manage",
    "host_add",
];

pub(super) async fn web_chat_tool_names() -> Vec<String> {
    let vault = tempfile::TempDir::new().unwrap();
    let config = HqConfig {
        openrouter_api_key: Some("test-key-no-network".to_string()),
        vault_path: vault.path().to_path_buf(),
        ..HqConfig::default()
    };
    let cwd = vault.path().to_path_buf();
    let session = SessionBuilder::from_config(&config)
        .working_dir(cwd.clone())
        .session_config(SessionConfig {
            model: CLOUD_MODEL.to_string(),
            ..crate::session_presets::chat_session_config(&config)
        })
        .harness_instructions(crate::session_presets::chat_harness_instructions(&cwd))
        .build()
        .await
        .expect("web chat session builds without a network call");
    let mut names = session.tool_names().await;
    names.sort();
    names
}

/// Today's web chat can do all of this. The orchestrator role must not take any of it away.
#[tokio::test]
async fn web_chat_keeps_spawn_steer_and_planning_tools() {
    let names = web_chat_tool_names().await;
    for required in SPAWN_AND_STEER_TOOLS.iter().chain(PLANNING_AND_READ_TOOLS) {
        assert!(
            names.iter().any(|n| n == required),
            "web chat lost {required}: {names:?}"
        );
    }
}

/// The writers the orchestrator role is meant to remove exist today, so the removal is a real change.
#[tokio::test]
async fn web_chat_today_has_the_writers_the_role_will_remove() {
    let names = web_chat_tool_names().await;
    for writer in ["edit_file", "write_file", "file_edit_batch", "rollback_file", "git_commit", "git_pr", "bash"] {
        assert!(names.iter().any(|n| n == writer), "{writer} missing: {names:?}");
    }
    assert!(
        !names.iter().any(|n| n == "config_manage"),
        "config_manage is MCP-only today; if it reached native sessions the role must remove it"
    );
}

fn implementor_block(can_build_self: bool) -> String {
    let derived = DerivedPromptBlocks {
        can_build_self,
        ..Default::default()
    };
    let block = build_harness_block(
        Some("CALLER INSTRUCTIONS"),
        crate::tool_policy::Preset::Cloud,
        Some(std::path::Path::new("/work")),
        100,
        derived,
    );
    // Platform and date vary per machine and day.
    block
        .lines()
        .filter(|l| !l.starts_with("- Platform:"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn implementor_prompt_block_is_pinned() {
    let block = implementor_block(false);
    assert!(block.contains("## Self-Management\n\nYour vault is your working memory"));
    assert!(block.contains("## Tool Usage\n\nYou are the primary coding agent."));
    assert!(block.contains("**Delegation:** `spawn_subagents` for parallel research."));
    assert!(block.trim_end().ends_with("CALLER INSTRUCTIONS"));
    let building = implementor_block(true);
    assert!(building.contains("Development work on your own source"));
}
