//! MCP tool registry setup: instantiates all built-in tools.

use hq_core::config::HqConfig;
use hq_db::Database;
use hq_tools::registry::{HqTool, ToolRegistry};
use hq_tools::{
    a2a, agent_comm, agents, ask, background_turns, brand, coding, convert, harness_session, agent_host,
    imagegen, prose_lint, remote_mcp, self_update, session_search, shortcuts, skill_manage_tool,
    budget_status, github, skills, slash_commands, subagent_runs, system_info, tasks, vault, web,
};

#[cfg(feature = "gws")]
use hq_tools::gws;
use hq_vault::VaultClient;
use std::path::PathBuf;
use std::sync::Arc;

/// Create the default tool registry with all built-in tools.
///
/// `skills_dir` is `hq_core::skills_dir` of the vault and `agents_dir` is its `Agents/`.
pub fn create_default_registry(
    vault_client: Arc<VaultClient>,
    db: Arc<Database>,
    skills_dir: PathBuf,
    agents_dir: PathBuf,
    config: Option<&HqConfig>,
) -> ToolRegistry {
    let vault_path = vault_client.vault_path().to_path_buf();
    let mut tools: Vec<Box<dyn HqTool>> = Vec::new();

    tools.extend(vault::create_vault_tools(
        vault_path.clone(),
        Some(db.clone()),
    ));
    if let Some(cfg) = config.filter(|c| c.self_update.enabled) {
        tools.extend(self_update::create_self_update_tools(cfg, db.clone()));
    }
    tools.extend(harness_session::tools::create_harness_session_tools(
        vault_path.clone(),
        db.clone(),
        None,
        None,
    ));
    tools.extend(agent_host::tools::create_host_tools(db.clone()));
    tools.extend(background_turns::create_background_turn_tools(db.clone()));
    tools.extend(subagent_runs::create_subagent_run_tools(db.clone(), None));
    tools.extend(tasks::create_task_tools(
        vault_path.clone(),
        vault_client.clone(),
        db.clone(),
    ));

    let skills_write_approval = config.is_some_and(|c| c.governance.skills_write_approval);
    tools.push(Box::new(skills::ListSkillsTool::new(skills_dir.clone())));
    // The MCP server is process-wide rather than per-session, so load_skill
    // telemetry gets one id for its lifetime, like tool_usage's "mcp-external".
    tools.push(Box::new(skills::LoadSkillTool::with_telemetry(
        skills_dir.clone(),
        db.clone(),
        format!("mcp-{}", uuid::Uuid::new_v4()),
    )));
    tools.push(Box::new(skill_manage_tool::SkillManageTool::new(
        skills_dir,
        skills_write_approval,
    )));
    tools.push(Box::new(slash_commands::SlashCommandManageTool::new(
        vault_path.clone(),
    )));
    tools.push(Box::new(session_search::SessionSearchTool::new(db.clone())));
    tools.push(Box::new(agents::ListAgentsTool::new(agents_dir.clone())));
    tools.push(Box::new(agents::LoadAgentTool::new(agents_dir)));
    #[cfg(feature = "gws")]
    tools.push(Box::new(gws::GoogleWorkspaceTool::new()));

    let openrouter_key = config.and_then(|c| c.openrouter_api_key.clone());
    tools.push(Box::new(imagegen::ImageGenTool::new(
        vault_path.clone(),
        openrouter_key,
    )));
    tools.extend(agent_comm::create_agent_comm_tools(vault_path.clone()));
    tools.extend(a2a::create_a2a_tools(vault_path.clone(), db.clone()));
    tools.extend(ask::create_ask_tools(db.clone(), None));

    // web_search: SearxNG when configured, then the built-in keyless engines, then Brave.
    let (searxng_url, brave_api_key, native) = config
        .map(|c| (c.searxng_url.clone(), c.brave_api_key.clone(), c.web_search_native))
        .unwrap_or((None, None, true));
    web::set_search_peer(config.and_then(|c| c.web_search_peer_server()).cloned());
    tools.push(Box::new(web::WebSearchHqTool::new(
        searxng_url,
        brave_api_key,
        native,
    )));
    tools.push(Box::new(web::WebFetchHqTool));

    tools.extend(coding::create_coding_tools());
    tools.extend(github::create_github_tools());
    tools.extend(system_info::create_system_info_tools(vault_path.clone()));
    tools.extend(budget_status::create_budget_status_tools(db.clone()));
    tools.extend(convert::create_convert_tools(vault_path.clone()));
    tools.extend(prose_lint::create_prose_lint_tools());
    tools.extend(brand::create_brand_tools(vault_path.clone()));
    tools.extend(shortcuts::create_shortcut_tools(
        vault_path.clone(),
        db.clone(),
    ));
    if let Some(cfg) = config {
        tools.extend(remote_mcp::create_remote_mcp_tools(&cfg.remote_mcp, None));
    }

    let mut registry = ToolRegistry::new();
    for tool in tools {
        registry.register(tool);
    }
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    fn real_registry(vault: &tempfile::TempDir) -> ToolRegistry {
        let vault_path = vault.path().to_path_buf();
        create_default_registry(
            Arc::new(VaultClient::new(vault_path.clone()).unwrap()),
            Arc::new(Database::open_memory().unwrap()),
            vault_path.join("skills"),
            vault_path.join("agents"),
            None,
        )
    }

    /// The tools were dead code for months while the server prompt named them,
    /// because nothing ever asserted the wiring existed.
    #[test]
    fn agent_comm_tools_are_registered() {
        let vault = tempfile::TempDir::new().unwrap();
        let registry = real_registry(&vault);

        for name in ["agent_send_message", "agent_read_inbox"] {
            assert!(
                registry.get(name).is_some(),
                "{name} is missing from the MCP registry"
            );
        }
        assert!(registry.categories().contains(&"agent-comm".to_string()));
    }

    /// A read-only chat turn denies every tool that does not say it only reads, so a
    /// vault read tool missing the flag would leave that turn unable to look anything up.
    #[test]
    fn vault_reads_are_marked_read_only_and_vault_writes_are_not() {
        let vault = tempfile::TempDir::new().unwrap();
        let registry = real_registry(&vault);
        for name in [
            "vault_read",
            "vault_outline",
            "vault_read_section",
            "vault_context",
            "vault_list",
            "vault_batch_read",
            "vault_search",
            "vault_find_similar",
            "vault_backlinks",
            "vault_links",
            "vault_tags",
            "memory_entity_graph",
            "context_packet",
            "list_agents",
            "load_agent",
        ] {
            let tool = registry
                .get(name)
                .unwrap_or_else(|| panic!("{name} is not registered"));
            assert!(tool.is_read_only(), "{name} only reads and must say so");
        }
        for name in [
            "vault_write_note",
            "vault_append_note",
            "vault_patch_section",
            "vault_reindex",
            "hq_ask",
        ] {
            let tool = registry
                .get(name)
                .unwrap_or_else(|| panic!("{name} is not registered"));
            assert!(
                !tool.is_read_only(),
                "{name} changes state and must not be read-only"
            );
        }
    }

    /// An allowlist entry that names no tool is a scope that silently grants less
    /// than documented, and a typo can hide a tool that was meant to stay out.
    #[test]
    fn every_scoped_allowlist_name_is_a_registered_tool() {
        let vault = tempfile::TempDir::new().unwrap();
        let registry = real_registry(&vault);

        for (scope, list) in [
            ("spark", crate::gateway::SPARK_READONLY_ALLOWLIST),
            ("handoff", crate::gateway::HANDOFF_ALLOWLIST),
            ("tasks", crate::gateway::TASKS_ALLOWLIST),
        ] {
            for name in list {
                assert!(
                    registry.get(name).is_some(),
                    "{scope} allows unknown tool {name}"
                );
            }
        }
    }
    /// The end-to-end check for the tasks scope against the real tools: filing a task with
    /// routing tags on that scope must not put anything in an agent's or the relay's mailbox,
    /// and the vault tools are out of reach entirely.
    #[tokio::test]
    async fn the_tasks_scope_files_tasks_without_touching_mailboxes_or_the_vault() {
        let vault = tempfile::TempDir::new().unwrap();
        for tag in ["relay", "agent-worker", "claude-code"] {
            std::fs::create_dir_all(vault.path().join("_mailboxes").join(tag)).unwrap();
        }
        std::fs::write(vault.path().join("secret.md"), "private note").unwrap();
        let registry = real_registry(&vault);
        let db = Database::open_memory().unwrap();
        let tasks = Some(crate::gateway::TASKS_ALLOWLIST);

        let call = |tool: &str, args: serde_json::Value| {
            serde_json::json!({"tool": tool, "args": args}).as_object().unwrap().clone()
        };

        let filed = crate::gateway::handle_call(
            &registry,
            Some(&call(
                "task_create",
                serde_json::json!({"title": "from an editor agent", "tags": ["relay", "agent-worker", "claude-code"]}),
            )),
            &db,
            tasks,
        )
        .await
        .unwrap();
        let text = format!("{filed:?}");
        assert!(text.contains("mcp:tasks"), "attributed to the scope, not a name the caller chose: {text}");

        let mailbox_entries = |tag: &str| {
            std::fs::read_dir(vault.path().join("_mailboxes").join(tag)).unwrap().count()
        };
        for tag in ["relay", "agent-worker", "claude-code"] {
            assert_eq!(mailbox_entries(tag), 0, "{tag} received something from the tasks scope");
        }

        for (tool, args) in [
            ("vault_read", serde_json::json!({"path": "secret.md"})),
            ("vault_search", serde_json::json!({"query": "private"})),
            ("harness_session_spawn", serde_json::json!({"harness": "claude-code", "cwd": "/tmp"})),
        ] {
            let denied = crate::gateway::handle_call(&registry, Some(&call(tool, args)), &db, tasks).await;
            assert!(denied.is_err(), "{tool} must be refused on the tasks scope");
        }

        // Control: the unscoped call with the same tags does deliver, so the assertion above
        // is not passing because nothing could ever be delivered.
        crate::gateway::handle_call(
            &registry,
            Some(&call("task_create", serde_json::json!({"title": "owner", "tags": ["relay"]}))),
            &db,
            None,
        )
        .await
        .unwrap();
        assert!(mailbox_entries("relay") >= 1, "an owner call delivers to a tagged mailbox");
    }
}
