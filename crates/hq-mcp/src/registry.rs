//! MCP tool registry setup: instantiates all built-in tools.

use hq_core::config::HqConfig;
use hq_db::Database;
use hq_tools::registry::{HqTool, ToolRegistry};
use hq_tools::{
    a2a, agent_comm, agent_host, agents, ask, background_turns, brand, budget_status, coding,
    convert, github, harness_session, imagegen, notify, prose_lint, remote_mcp, self_update,
    session_search, shortcuts, skill_manage_tool, skills, slash_commands, subagent_runs,
    system_info, tasks, vault, web,
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
        .map(|c| {
            (
                c.searxng_url.clone(),
                c.brave_api_key.clone(),
                c.web_search_native,
            )
        })
        .unwrap_or((None, None, true));
    let remotes = config.map(hq_db::bridge_setup::resolve_remote_mcp);
    web::set_search_peer(
        config
            .zip(remotes.as_deref())
            .and_then(|(c, r)| c.web_search_peer_in(r))
            .cloned(),
    );
    tools.push(Box::new(web::WebSearchHqTool::new(
        searxng_url,
        brave_api_key,
        native,
    )));
    tools.push(Box::new(web::WebFetchHqTool));

    tools.extend(coding::create_coding_tools());
    let write_repos = config
        .map(|c| c.github_write.repos.clone())
        .unwrap_or_default();
    tools.extend(github::create_github_tools(&write_repos));
    tools.extend(system_info::create_system_info_tools(vault_path.clone()));
    tools.extend(budget_status::create_budget_status_tools(db.clone()));
    tools.extend(notify::create_notify_tools(db.clone(), config));
    tools.extend(convert::create_convert_tools(vault_path.clone()));
    tools.extend(prose_lint::create_prose_lint_tools());
    tools.extend(brand::create_brand_tools(vault_path.clone()));
    tools.extend(shortcuts::create_shortcut_tools(
        vault_path.clone(),
        db.clone(),
    ));
    if let Some(remotes) = &remotes {
        tools.extend(remote_mcp::create_remote_mcp_tools(remotes, None));
    }

    let mut registry = ToolRegistry::new();
    for tool in tools {
        registry.register(tool);
    }
    if let Some(profile) = config.map(|c| c.profile) {
        if profile.is_lite() {
            registry.retain(|t| LITE_CATEGORIES.contains(&t.category()));
        } else {
            // Mobile drops git and the self-updater; the other editions drop nothing here. The
            // chat agent's own builder asks the same question (`Profile::drops_tool_category`).
            registry.retain(|t| !profile.drops_tool_category(t.category()));
        }
    }
    registry
}

/// The tool categories `profile: lite` keeps: tasks only. The note tools are out too, since they
/// could read `_system` and the other hidden folders. Everything else (the
/// shell and file tools, coding-agent sessions, hosts, web search and fetch, image generation,
/// email, GitHub, remote MCP servers, sub-agents and the chat agent's own tools) is not
/// registered, so no transport can reach it.
pub const LITE_CATEGORIES: &[&str] = &["tasks"];

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

    /// The self-updater's tools exist only when `self_update.enabled` is set, so the profile tests
    /// turn it on; otherwise "mobile drops them" would hold for the wrong reason.
    fn registry_for(vault: &tempfile::TempDir, profile: hq_core::config::Profile) -> ToolRegistry {
        let vault_path = vault.path().to_path_buf();
        let config = HqConfig {
            profile,
            self_update: hq_core::config::SelfUpdateConfig {
                enabled: true,
                ..hq_core::config::SelfUpdateConfig::default()
            },
            ..HqConfig::default()
        };
        create_default_registry(
            Arc::new(VaultClient::new(vault_path.clone()).unwrap()),
            Arc::new(Database::open_memory().unwrap()),
            vault_path.join("skills"),
            vault_path.join("agents"),
            Some(&config),
        )
    }

    /// A phone has no `git` and no binary self-updater, so neither is reachable through any
    /// transport; the chat agent, vault, tasks and web tools stay, and so do the coding-agent
    /// tools, because a phone instance drives sessions on hosts elsewhere.
    #[test]
    fn the_mobile_registry_drops_git_and_the_self_updater_and_keeps_the_rest() {
        use hq_core::config::Profile;
        let vault = tempfile::TempDir::new().unwrap();
        let full = registry_for(&vault, Profile::Full).categories();
        let mobile = registry_for(&vault, Profile::Mobile).categories();
        for removed in hq_core::config::MOBILE_REMOVED_TOOL_CATEGORIES {
            assert!(
                full.iter().any(|c| c == removed),
                "{removed} missing from full, so mobile dropping it proves nothing"
            );
            assert!(
                !mobile.iter().any(|c| c == removed),
                "{removed} still on mobile"
            );
        }
        for kept in ["tasks", "vault", "web", "harness"] {
            assert!(mobile.iter().any(|c| c == kept), "{kept} missing on mobile");
        }
        assert!(mobile.len() < full.len());
    }

    /// Lite stays narrower than mobile, and a mobile instance is never made narrower than Lite.
    #[test]
    fn mobile_keeps_more_than_lite() {
        use hq_core::config::Profile;
        let vault = tempfile::TempDir::new().unwrap();
        let lite = registry_for(&vault, Profile::Lite).list().len();
        let mobile = registry_for(&vault, Profile::Mobile).list().len();
        assert!(mobile > lite, "mobile {mobile} vs lite {lite}");
    }

    /// A typo or a renamed tool would silently drop a confirmation, so every entry must be a
    /// real tool. Two are registered only with config or a build feature.
    #[test]
    fn every_connector_confirm_tool_exists() {
        const OPTIONAL: &[&str] = &[
            "google_workspace",
            "self_update_install",
            "self_update_rollback",
        ];
        let vault = tempfile::TempDir::new().unwrap();
        let registry = real_registry(&vault);
        let missing: Vec<_> = crate::gateway::CONNECTOR_CONFIRM_TOOLS
            .iter()
            .filter(|name| registry.get(name).is_none() && !OPTIONAL.contains(name))
            .collect();
        assert!(missing.is_empty(), "not registered tools: {missing:?}");
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

    /// Agents reaching HQ over MCP can notify the owner.
    #[test]
    fn notify_owner_is_registered_and_not_read_only() {
        let vault = tempfile::TempDir::new().unwrap();
        let registry = real_registry(&vault);
        let tool = registry
            .get("notify_owner")
            .expect("notify_owner is registered");
        assert!(!tool.is_read_only());
        assert_eq!(tool.category(), "notifications");
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
            "vault_inventory",
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
            "vault_import",
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
            serde_json::json!({"tool": tool, "args": args})
                .as_object()
                .unwrap()
                .clone()
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
        assert!(
            text.contains("mcp:tasks"),
            "attributed to the scope, not a name the caller chose: {text}"
        );

        let mailbox_entries = |tag: &str| {
            std::fs::read_dir(vault.path().join("_mailboxes").join(tag))
                .unwrap()
                .count()
        };
        for tag in ["relay", "agent-worker", "claude-code"] {
            assert_eq!(
                mailbox_entries(tag),
                0,
                "{tag} received something from the tasks scope"
            );
        }

        for (tool, args) in [
            ("vault_read", serde_json::json!({"path": "secret.md"})),
            ("vault_search", serde_json::json!({"query": "private"})),
            (
                "harness_session_spawn",
                serde_json::json!({"harness": "claude-code", "cwd": "/tmp"}),
            ),
        ] {
            let denied =
                crate::gateway::handle_call(&registry, Some(&call(tool, args)), &db, tasks).await;
            assert!(denied.is_err(), "{tool} must be refused on the tasks scope");
        }

        // task_update and task_comment_add go through the same gateway marker.
        // The created task's id, read out of the tool result (Debug-escaped JSON text).
        let id = text
            .split("\\\"id\\\": \\\"")
            .nth(1)
            .and_then(|rest| rest.split('\\').next())
            .unwrap_or("")
            .to_string();
        assert!(id.len() > 5, "found the filed task's id in {text}");
        let moved = crate::gateway::handle_call(
            &registry,
            Some(&call(
                "task_update",
                serde_json::json!({"id": id, "status": "in_progress", "tags": ["relay"]}),
            )),
            &db,
            tasks,
        )
        .await
        .unwrap();
        assert!(
            !format!("{moved:?}").contains("isError: Some(true)"),
            "{moved:?}"
        );
        let commented = crate::gateway::handle_call(
            &registry,
            Some(&call(
                "task_comment_add",
                serde_json::json!({"task_id": id, "body": "hello", "author": "the owner"}),
            )),
            &db,
            tasks,
        )
        .await
        .unwrap();
        assert!(
            format!("{commented:?}").contains("mcp:tasks"),
            "{commented:?}"
        );
        for tag in ["relay", "agent-worker", "claude-code"] {
            assert_eq!(
                mailbox_entries(tag),
                0,
                "{tag} received something after update and comment"
            );
        }

        // Control: the unscoped call with the same tags does deliver, so the assertion above
        // is not passing because nothing could ever be delivered.
        crate::gateway::handle_call(
            &registry,
            Some(&call(
                "task_create",
                serde_json::json!({"title": "owner", "tags": ["relay"]}),
            )),
            &db,
            None,
        )
        .await
        .unwrap();
        assert!(
            mailbox_entries("relay") >= 1,
            "an owner call delivers to a tagged mailbox"
        );
    }
    #[test]
    fn the_lite_profile_registers_only_tasks() {
        let vault = tempfile::TempDir::new().unwrap();
        let vault_path = vault.path().to_path_buf();
        let build = |lite: bool| {
            let cfg = HqConfig {
                profile: if lite {
                    hq_core::config::Profile::Lite
                } else {
                    hq_core::config::Profile::Full
                },
                ..HqConfig::default()
            };
            create_default_registry(
                Arc::new(VaultClient::new(vault_path.clone()).unwrap()),
                Arc::new(Database::open_memory().unwrap()),
                vault_path.join("skills"),
                vault_path.join("agents"),
                Some(&cfg),
            )
        };
        let (full, lite) = (build(false), build(true));
        assert!(full.len() > lite.len() && lite.len() >= 2);
        assert_eq!(lite.categories(), ["tasks"]);
        assert!(lite.get("task_create").is_some());
        for gone in [
            // The note tools would read `_system` and the other hidden folders.
            "vault_read",
            "vault_batch_read",
            "vault_list",
            "vault_outline",
            "vault_search",
            "vault_context",
            "bash",
            "harness_session_spawn",
            "host_send",
            "web_fetch",
            "web_search",
            "imagegen",
            "hq_ask",
            "agent_message_send",
            "config_manage",
            "gws",
        ] {
            assert!(
                lite.get(gone).is_none(),
                "{gone} must not be registered in Lite"
            );
        }
        // The full profile still has them, so the assertion above is not an empty registry.
        assert!(full.get("harness_session_spawn").is_some() && full.get("web_fetch").is_some());
    }
}
