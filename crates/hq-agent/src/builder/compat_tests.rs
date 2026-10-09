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
    tool_names_for(SessionRole::Implementor).await
}

pub(super) async fn tool_names_for(role: SessionRole) -> Vec<String> {
    let vault = tempfile::TempDir::new().unwrap();
    let config = HqConfig {
        openrouter_api_key: Some("test-key-no-network".to_string()),
        vault_path: vault.path().to_path_buf(),
        ..HqConfig::default()
    };
    let cwd = vault.path().to_path_buf();
    let session = SessionBuilder::from_config(&config)
        .role(role)
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
    for writer in [
        "edit_file",
        "write_file",
        "file_edit_batch",
        "rollback_file",
        "git_commit",
        "git_pr",
        "bash",
    ] {
        assert!(
            names.iter().any(|n| n == writer),
            "{writer} missing: {names:?}"
        );
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

#[test]
fn orchestrator_prompt_block_has_no_coding_framing() {
    let derived = DerivedPromptBlocks {
        can_build_self: true,
        role: SessionRole::Orchestrator,
        ..Default::default()
    };
    let block = build_harness_block(
        Some("CALLER INSTRUCTIONS"),
        crate::tool_policy::Preset::Cloud,
        None,
        100,
        derived,
    );
    assert!(block.contains("You are the HQ orchestrator."));
    for banned in [
        "primary coding agent",
        "Self-Management",
        "Development work on your own source",
    ] {
        assert!(!block.contains(banned), "{banned} leaked into: {block}");
    }
    assert!(block.contains("**Delegation:** `spawn_subagents`"));
    assert!(block.trim_end().ends_with("CALLER INSTRUCTIONS"));
}

/// The role takes away exactly the named writers (and the shell only when no sandbox can contain it).
#[tokio::test]
async fn orchestrator_catalog_is_today_minus_the_removal_set() {
    let today = web_chat_tool_names().await;
    let orchestrator = tool_names_for(SessionRole::Orchestrator).await;
    let mut expected_gone: Vec<&str> = ORCHESTRATOR_REMOVED_TOOLS
        .iter()
        .copied()
        .filter(|t| today.iter().any(|n| n == t))
        .collect();
    if crate::bash_sandbox::available_backend_for(false).is_none() {
        expected_gone.push("bash");
    }
    let mut gone: Vec<&str> = today
        .iter()
        .filter(|n| !orchestrator.contains(n))
        .map(String::as_str)
        .collect();
    gone.sort_unstable();
    expected_gone.sort_unstable();
    assert_eq!(gone, expected_gone);
    let added: Vec<&String> = orchestrator.iter().filter(|n| !today.contains(n)).collect();
    assert!(added.is_empty(), "the role must not add tools: {added:?}");
}

#[tokio::test]
async fn orchestrator_keeps_spawn_steer_and_planning_tools() {
    let names = tool_names_for(SessionRole::Orchestrator).await;
    for required in SPAWN_AND_STEER_TOOLS.iter().chain(PLANNING_AND_READ_TOOLS) {
        assert!(
            names.iter().any(|n| n == required),
            "orchestrator lost {required}: {names:?}"
        );
    }
}

async fn orchestrator_session(vault: &std::path::Path) -> crate::session::AgentSession {
    let config = HqConfig {
        openrouter_api_key: Some("test-key-no-network".to_string()),
        vault_path: vault.to_path_buf(),
        ..HqConfig::default()
    };
    SessionBuilder::from_config(&config)
        .role(SessionRole::Orchestrator)
        .working_dir(vault.to_path_buf())
        .session_config(SessionConfig {
            model: CLOUD_MODEL.to_string(),
            ..SessionConfig::default()
        })
        .build()
        .await
        .unwrap()
}

fn result_text(result: &hq_core::types::ToolResult) -> String {
    result.content.iter().map(|c| c.text.as_str()).collect()
}

#[tokio::test]
async fn an_orchestrator_calling_a_removed_tool_is_routed_to_delegation() {
    let vault = tempfile::TempDir::new().unwrap();
    let session = orchestrator_session(vault.path()).await;
    let reply = session
        .call_tool_for_test(
            "edit_file",
            serde_json::json!({"file_path": "x", "old_string": "a", "new_string": "b"}),
        )
        .await
        .unwrap();
    let text = result_text(&reply);
    assert!(
        text.contains("not available in the orchestrator role"),
        "{text}"
    );
    assert!(text.contains("harness_session_spawn"), "{text}");
    let unknown = session
        .call_tool_for_test("no_such_tool", serde_json::json!({}))
        .await;
    assert!(unknown.is_err(), "other unknown tools still fail as before");
}

/// Needs a sandbox backend; skipped where none exists (the shell is absent there anyway).
#[tokio::test]
async fn an_orchestrators_shell_cannot_write_and_says_where_to_go() {
    if crate::bash_sandbox::available_backend_for(false).is_none() {
        eprintln!("skipped: no sandbox backend on this host");
        return;
    }
    let vault = tempfile::TempDir::new().unwrap();
    let session = orchestrator_session(vault.path()).await;
    let probe = format!("hq-orch-probe-{}", std::process::id());
    let home_probe = dirs::home_dir().unwrap().join(&probe);
    let command = format!("touch ./{probe} '{}'", home_probe.display());
    let reply = session
        .call_tool_for_test("bash", serde_json::json!({"command": command}))
        .await
        .unwrap();
    let leaked = std::env::current_dir().unwrap().join(&probe).exists() || home_probe.exists();
    let _ = std::fs::remove_file(std::env::current_dir().unwrap().join(&probe));
    let _ = std::fs::remove_file(&home_probe);
    assert!(!leaked, "orchestrator bash wrote to disk");
    let text = result_text(&reply);
    assert!(
        text.contains("harness_session_spawn"),
        "no routing hint: {text}"
    );
}

#[tokio::test]
async fn an_orchestrator_cannot_export_a_document_outside_notebooks() {
    let vault = tempfile::TempDir::new().unwrap();
    // The temp directory is an allowed root, so the refused target lives in the home directory.
    let outside = dirs::home_dir()
        .unwrap()
        .join(format!("hq-orch-probe-{}", std::process::id()));
    let session = orchestrator_session(vault.path()).await;
    let target = outside.join("x.html");
    let reply = session
        .call_tool_for_test(
            "convert_from_markdown",
            serde_json::json!({"content": "hi", "format": "html", "output": target.display().to_string()}),
        )
        .await
        .unwrap();
    assert!(
        result_text(&reply).contains("may only write under"),
        "{}",
        result_text(&reply)
    );
    let written = outside.exists();
    let _ = std::fs::remove_dir_all(&outside);
    assert!(
        !written,
        "the refused export still created {}",
        outside.display()
    );
}

#[tokio::test]
async fn an_orchestrator_cannot_point_a_note_export_outside_notebooks() {
    let vault = tempfile::TempDir::new().unwrap();
    let session = orchestrator_session(vault.path()).await;
    let outside = dirs::home_dir()
        .unwrap()
        .join(format!("hq-orch-export-{}", std::process::id()));
    for tool in ["vault_export", "vault_export_pdf"] {
        let reply = session
            .call_tool_for_test(
                tool,
                serde_json::json!({"path": "Notebooks/a.md", "format": "md", "output": outside.join("x.md").display().to_string()}),
            )
            .await
            .unwrap();
        assert!(
            result_text(&reply).contains("may only write under"),
            "{tool}: {}",
            result_text(&reply)
        );
    }
    assert!(!outside.exists());
}
