//! Tool registration, profile and preset filtering, and governance for
//! `SessionBuilder::build`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hq_llm::provider::LlmProvider;

use super::{CAPABILITY_FLOOR, HqToolAdapter, SessionBuilder, mailbox_denial_notifier};
use crate::governance::{GovernedRegistry, ToolGuardian};
use crate::session::SessionConfig;
use crate::tools::AgentTool;

impl SessionBuilder {
    /// Paths the session's file tools may touch.
    pub(super) fn allowed_paths(&self, vault_path: &Path) -> Vec<PathBuf> {
        let mut allowed_paths = Vec::new();

        // Vault path is always allowed (already captured above for DB open).
        allowed_paths.push(vault_path.to_path_buf());

        // Working directory (if set)
        if let Some(ref wd) = self.working_dir {
            allowed_paths.push(wd.clone());
        }

        // Extra allowed paths
        allowed_paths.extend(self.extra_allowed_paths.iter().cloned());

        // System temp dir: scratch space bash can already reach unrestricted.
        allowed_paths.extend(crate::governance::system_temp_paths());
        allowed_paths
    }

    /// The native tool set.
    pub(super) fn native_tools(
        &self,
        vault_path: &Path,
        shared_db: Option<Arc<hq_db::Database>>,
    ) -> Vec<Box<dyn AgentTool>> {
        let mut tools = self.coding_tools(vault_path);
        for tool in self.hq_tool_catalog(vault_path, shared_db) {
            tools.push(Box::new(HqToolAdapter { inner: tool }));
        }

        // LSP tools (deferred, read-only)
        let lsp_manager = crate::lsp_tools::shared_lsp_manager();
        crate::lsp_tools::register_lsp_tools(&mut tools, lsp_manager);

        tools
    }

    /// `governance.bash` resolved against this session's writable roots.
    pub(super) fn bash_settings(&self, vault_path: &Path) -> crate::bash_sandbox::BashSettings {
        crate::bash_sandbox::BashSettings::from_config(
            &self.config.governance.bash,
            self.allowed_paths(vault_path),
        )
    }

    /// File, shell, search, todo and web tools, sharing one file-state cache.
    fn coding_tools(&self, vault_path: &Path) -> Vec<Box<dyn AgentTool>> {
        let state_cache = hq_tools::file_edit::FileStateCache::default();
        let history = hq_tools::file_edit::FileHistory::default();

        // Deliberately narrower than hq-mcp's create_default_registry() (the
        // full catalog for external MCP harnesses): this is curated for what
        // native sessions need. Invariant: a tool named in a daemon task's
        // prompt must be registered here too, or the call silently no-ops
        // (see the trello/clickup/remote_mcp/convert/git comments below).
        vec![
            Box::new(crate::coding::BashTool::new(self.bash_settings(vault_path))),
            Box::new(crate::coding::ReadTool::new(state_cache.clone())),
            Box::new(crate::coding::WriteTool::new(
                state_cache.clone(),
                history.clone(),
            )),
            Box::new(crate::coding::EditTool::new(
                state_cache.clone(),
                history.clone(),
            )),
            // Multi-file atomic edit — was previously only exposed to *other*
            // harnesses over MCP (hq_tools::coding::BatchEditTool), not to
            // hq's own session. Shares this session's state_cache/history so
            // read-before-write tracking and rollback stay consistent with
            // edit_file. hq_tools::HqTool -> AgentTool via HqToolAdapter, the
            // same wrapper used for every other hq-tools-sourced tool below.
            Box::new(HqToolAdapter {
                inner: Box::new(hq_tools::coding::BatchEditTool::new(
                    state_cache.clone(),
                    history.clone(),
                )),
            }),
            Box::new(crate::coding::RollbackTool::new(state_cache, history)),
            Box::new(crate::coding::FindTool),
            Box::new(crate::coding::GrepTool),
            Box::new(crate::coding::LsTool),
            // Session-scoped live task tracking (TodoWrite-equivalent) —
            // distinct from hq plan's coarser, vault-persisted phases. Built
            // and registered here; whether it measurably helps coding-task
            // quality is for the Workstream 6 competition harness to decide,
            // not assumed.
            Box::new(HqToolAdapter {
                inner: Box::new(hq_tools::coding::TodoWriteTool::new(
                    hq_tools::coding::TodoStore::new(),
                )),
            }),
            Box::new({
                hq_tools::web::set_search_peer(self.config.web_search_peer_server().cloned());
                crate::web::WebSearchTool::new(
                self.config.searxng_url.clone(),
                self.config.brave_api_key.clone(),
                self.config.web_search_native,
            )
            }),
            Box::new(crate::web::WebFetchTool),
        ]
    }

    /// The hq-tools tools a native session registers, in registration order.
    fn hq_tool_catalog(
        &self,
        vault_path: &Path,
        shared_db: Option<Arc<hq_db::Database>>,
    ) -> Vec<Box<dyn hq_tools::HqTool>> {
        // Vault + memory tools — direct vault access without subagent overhead
        let mut tools = hq_tools::vault::create_vault_tools(vault_path.to_path_buf(), shared_db.clone());
        // Restricted audiences (FR-072) only see vault context labelled for them.
        if let (Some(id), Ok(vault)) = (
            self.identity.as_ref(),
            hq_vault::VaultClient::new(vault_path.to_path_buf()),
        ) {
            tools =
                hq_tools::vault::scope_vault_tools(tools, std::sync::Arc::new(vault), &id.scope);
        }

        // Mailbox messaging (relay and local agent mailboxes).
        tools.extend(hq_tools::agent_comm::create_agent_comm_tools(
            vault_path.to_path_buf(),
        ));

        // Document conversion + OCR (image → text via macOS Vision, on-device,
        // model-agnostic). Was previously MCP-only (hq-mcp/src/registry.rs),
        // so hq's own native session — including the relay/Telegram agent —
        // had no way to read an image's text on demand.
        tools.extend(hq_tools::convert::create_convert_tools(vault_path.to_path_buf()));

        // Mechanical AI-slop check for document/report drafts.
        tools.extend(hq_tools::prose_lint::create_prose_lint_tools());

        // Per-brand asset knowledge base (logos/palette/reference-docs).
        tools.extend(hq_tools::brand::create_brand_tools(vault_path.to_path_buf()));

        if let Some(ref db) = shared_db {
            tools.extend(self.db_backed_tools(vault_path, db));
        }

        // Remote MCP servers from `remote_mcp:`. ToolGuardian::build_registry
        // drops the ones marked live_user_turn_only from unattended sessions.
        tools.extend(hq_tools::remote_mcp::create_remote_mcp_tools(
            &self.config.remote_mcp,
            self.identity
                .as_ref()
                .and_then(hq_tools::family_guest::FamilyGuestContext::from_identity),
        ));

        // Model control (Copilot catalog, primary switch) is for the owner only.
        let owner_scope = self.identity.as_ref().is_none_or(|id| {
            matches!(id.scope, hq_core::privacy::DisclosureScope::Unrestricted)
        });
        if owner_scope {
            tools.extend(hq_tools::model_control::create_model_tools());
        }

        // Git / GitHub — these existed only in the MCP registry, so an agent
        // session had to shell out through `bash` to touch git at all, and
        // reported having no GitHub access despite an authenticated `gh` on
        // PATH.
        tools.push(Box::new(hq_tools::coding::GitStatusTool));
        tools.push(Box::new(hq_tools::coding::GitDiffTool));
        tools.push(Box::new(hq_tools::coding::GitLogTool));
        tools.push(Box::new(hq_tools::coding::GitCommitTool));
        tools.push(Box::new(hq_tools::coding::GitPrTool));

        // Custom slash commands — lets the agent create/list/delete the
        // `_commands/*.md` templates the terminal's `/name` dispatch reads,
        // so "set up a /deploy command" works mid-conversation without a
        // restart. Registered in-process (not just the MCP registry) since
        // this is the tool set `hq chat`'s native session actually uses.
        tools.push(Box::new(
            hq_tools::slash_commands::SlashCommandManageTool::new(vault_path.to_path_buf()),
        ));

        // Live host capability checks, so the agent can confirm what is
        // installed instead of guessing.
        tools.extend(hq_tools::system_info::create_system_info_tools(
            vault_path.to_path_buf(),
        ));

        // Shortcut tools — lightweight wrappers for Weak/relay sessions.
        // Must be registered before the profile filter so Weak sessions have tools.
        if let Some(ref db) = shared_db {
            tools.extend(hq_tools::shortcuts::create_shortcut_tools(
                vault_path.to_path_buf(),
                db.clone(),
            ));
        }
        tools
    }

    /// Self-update, host harness sessions, background turns and native
    /// tasks, which all need the shared database.
    fn db_backed_tools(
        &self,
        vault_path: &Path,
        db: &Arc<hq_db::Database>,
    ) -> Vec<Box<dyn hq_tools::HqTool>> {
        let mut tools = Vec::new();

        // Self-update lifecycle — branch + test + reinstall of HQ's own binary
        if self.config.self_update.enabled {
            tools.extend(hq_tools::self_update::create_self_update_tools(
                &self.config,
                db.clone(),
            ));
        }

        // Harness session manager — spawn/monitor/steer/resume fleet CLIs in the host
        tools.extend(
            hq_tools::harness_session::tools::create_harness_session_tools(
                vault_path.to_path_buf(),
                db.clone(),
                self.identity
                    .as_ref()
                    .and_then(|i| watching_chat(i, &self.config.agent_host, db)),
                self.identity
                    .as_ref()
                    .and_then(hq_tools::family_guest::FamilyGuestContext::from_identity),
            ),
        );
        tools.extend(hq_tools::agent_host::tools::create_host_tools(db.clone()));
        tools.extend(hq_tools::background_turns::create_background_turn_tools(
            db.clone(),
        ));
        // A session with no chat (CLI, proxy) sees only unrouted runs, never
        // another chat's.
        tools.extend(hq_tools::subagent_runs::create_subagent_run_tools(
            db.clone(),
            Some(
                self.run_origin()
                    .unwrap_or_else(hq_db::subagent_runs::Origin::unrouted),
            ),
        ));
        let origin = self
            .identity
            .as_ref()
            .and_then(hq_tools::background_turns::WatchOrigin::from_identity);
        tools.extend(hq_tools::background_turns::create_watch_tools(
            db.clone(),
            origin,
        ));

        // Native task management. External MCP clients already used these;
        // HQ's own session could only reach them through the `hq` gateway.
        if let Ok(vault) = hq_vault::VaultClient::new(vault_path.to_path_buf()) {
            let task_tools =
                hq_tools::tasks::create_task_tools(vault_path.to_path_buf(), Arc::new(vault), db.clone());
            // Restricted audiences (FR-073) only reach their own person-scoped task space.
            tools.extend(match self.identity.as_ref() {
                Some(id) => hq_tools::tasks::scope_task_tools(task_tools, db.clone(), &id.scope),
                None => task_tools,
            });
        }
        tools
    }

    /// The prompt's skill catalog tells the agent to call `load_skill`, so the
    /// session needs it, and `skill_manage` so it can fix a skill it used.
    pub(super) fn skill_tools(
        &self,
        vault_path: &Path,
        shared_db: Option<Arc<hq_db::Database>>,
        session_id: &str,
    ) -> Vec<Box<dyn AgentTool>> {
        let skills_dir = hq_core::skills_dir(vault_path);
        let load: Box<dyn hq_tools::HqTool> = match shared_db {
            Some(db) => Box::new(hq_tools::skills::LoadSkillTool::with_telemetry(
                skills_dir.clone(),
                db,
                session_id,
            )),
            None => Box::new(hq_tools::skills::LoadSkillTool::new(skills_dir.clone())),
        };
        let manage = Box::new(hq_tools::skill_manage_tool::SkillManageTool::new(
            skills_dir,
            self.config.governance.skills_write_approval,
        ));
        vec![
            Box::new(HqToolAdapter { inner: load }),
            Box::new(HqToolAdapter { inner: manage }),
        ]
    }

    /// The chat delegated runs belong to, from this session's identity.
    fn run_origin(&self) -> Option<hq_db::subagent_runs::Origin> {
        use hq_core::identity::RequestSource;
        let identity = self.identity.as_ref()?;
        let (platform, chat_id, thread_id) = match &identity.source {
            RequestSource::Web { thread_id, .. } => {
                ("web", thread_id.clone(), Some(thread_id.clone()))
            }
            RequestSource::Telegram { chat_id } => ("telegram", chat_id.to_string(), None),
            RequestSource::Discord { channel_id } => ("discord", channel_id.to_string(), None),
            RequestSource::ProxyApi { .. } | RequestSource::LocalCli => return None,
        };
        Some(hq_db::subagent_runs::Origin {
            platform: platform.to_string(),
            chat_id,
            thread_id,
            identity: Some(identity.user_id.clone()),
        })
    }

    /// Delegation tools, unless this session is already at max sub-agent depth.
    pub(super) fn subagent_tools(
        &self,
        provider: &Arc<dyn LlmProvider>,
        vault_path: &Path,
        allowed_paths: &[PathBuf],
        session_config: &SessionConfig,
        shared_db: Option<Arc<hq_db::Database>>,
        taint: &crate::governance::TaintTracker,
    ) -> Vec<Box<dyn AgentTool>> {
        let mut tools: Vec<Box<dyn AgentTool>> = Vec::new();
        // Sub-agent tools (if not at max depth)
        let max_depth = self.config.collaboration.max_subagent_depth;
        let timeout_secs = self.config.collaboration.subagent_timeout_secs;
        if self.subagent_depth < max_depth {
            // `spawn_subagents` (the unified AgentService surface) is the single
            // delegation tool registered on the standard build path — it
            // replaces registering the legacy `spawn_subagent` + `coordinate`
            // pair so the model sees exactly one delegation surface. The
            // escalation catalog below now routes through the *same*
            // `AgentService` instance, so every delegation path shares one
            // governed runtime.

            // The unified child-execution runtime, backed by the configured
            // external backends (when any) for backend-selection policy.
            let service_registry = crate::backend::BackendRegistry::from_config(&self.config)
                .ok()
                .map(Arc::new);
            let agent_service = Arc::new(
                crate::agents::AgentService::new(
                    provider.clone(),
                    vault_path.to_path_buf(),
                    allowed_paths.to_vec(),
                    session_config.clone(),
                    self.security_profile.clone(),
                    self.subagent_depth,
                    max_depth,
                    std::time::Duration::from_secs(timeout_secs),
                )
                .with_backend_registry(service_registry)
                .with_db(shared_db.clone())
                .with_bash_settings(self.bash_settings(vault_path))
                .with_permission_mode(self.permission_mode.clone())
                .with_taint(taint.clone()),
            );

            // Forward child session-events to the parent's broadcast channel so
            // sub-agent tool calls stay visible, mirroring the old spawner's
            // event forwarding. Lifecycle-only envelopes carry no SessionEvent
            // and are dropped here; the surface-migration todo threads the full
            // correlated envelope stream into the parent session.
            let child_ctx = if let Some(tx) = self.event_tx.clone() {
                let sink: crate::agents::EnvelopeSink = Arc::new(move |env| {
                    if let Some(ev) = env.as_session_event() {
                        let _ = tx.send(ev.clone());
                    }
                });
                crate::agents::ChildExecContext {
                    parent_run_id: None,
                    envelope_sink: Some(sink),
                    parent_messages: None,
                    parent_turn_id: self.child_parent_turn_id.clone(),
                    completion_sink: self.child_completion_sink.clone(),
                    progress_sink: self.child_progress_sink.clone(),
                    origin: self.run_origin(),
                    auto_followup: self.config.collaboration.supervision_followup,
                }
            } else {
                crate::agents::ChildExecContext {
                    parent_turn_id: self.child_parent_turn_id.clone(),
                    completion_sink: self.child_completion_sink.clone(),
                    progress_sink: self.child_progress_sink.clone(),
                    origin: self.run_origin(),
                    auto_followup: self.config.collaboration.supervision_followup,
                    ..Default::default()
                }
            };
            tools.push(Box::new(
                crate::agents::SpawnSubagentsTool::new(agent_service.clone())
                    .with_context(child_ctx.clone()),
            ));
            tools.push(Box::new(
                crate::agents::ReportProgressTool::new().with_context(child_ctx.clone()),
            ));

            // Pre-bound escalation catalog (call_code_reasoner,
            // ...). Filtered out for cloud models by
            // tool_policy. Now built over the shared `AgentService` so each
            // pre-bound specialist is a single-child plan on the unified path.
            for t in crate::callable_agents::build_catalog(agent_service, child_ctx) {
                tools.push(t);
            }
        }
        tools
    }

    /// Session-profile filter, per-model or named-agent preset, then
    /// governance. Returns the registry and the preset the prompt describes.
    pub(super) fn govern_tools(
        &self,
        tools: Vec<Box<dyn AgentTool>>,
        vault_path: &Path,
        allowed_paths: &[PathBuf],
        session_config: &SessionConfig,
        taint: crate::governance::TaintTracker,
    ) -> (GovernedRegistry, crate::tool_policy::Preset) {
        let mut guardian = ToolGuardian::new(
            allowed_paths.to_vec(),
            self.security_profile.clone(),
            self.permission_mode.clone(),
        );
        guardian.set_denial_notifier(mailbox_denial_notifier(vault_path.to_path_buf()));
        guardian.set_taint(taint);

        // Apply session profile: drop tools whose policy exceeds the profile tier.
        // This ensures Weak sessions see only shortcut tools, Standard sessions see
        // Standard + Weak, and Full sessions see everything.
        let profile_policy = self.session_profile.max_policy();
        let tools = tools
            .into_iter()
            .filter(|t| t.tool_policy() <= profile_policy || CAPABILITY_FLOOR.contains(&t.name()))
            .collect::<Vec<_>>();
        let tools = tools
            .into_iter()
            .filter(|t| {
                !self
                    .tool_deny_prefixes
                    .iter()
                    .any(|p| t.name().starts_with(p.as_str()))
            })
            .collect::<Vec<_>>();
        if tools.is_empty() {
            tracing::warn!(
                profile = ?self.session_profile,
                "session profile filter left 0 tools — shortcuts not registered yet?"
            );
        }

        // Apply per-model tool policy before governance wraps everything.
        // Named agents (depth == 0, not the root "hq" session) get their own
        // capability profile if one is registered, otherwise fall back to the
        // model-based preset.
        let preset = {
            let agent_name = &session_config.agent_name;
            if agent_name != "hq" && self.subagent_depth == 0 {
                let profile = crate::tool_policy::load_vault_profile(vault_path, agent_name)
                    .or_else(|| crate::tool_policy::default_profile_for_agent(agent_name));
                if let Some(p) = profile {
                    crate::tool_policy::Preset::Named(p)
                } else {
                    crate::tool_policy::Preset::for_model(
                        &session_config.model,
                        self.subagent_depth,
                    )
                }
            } else {
                crate::tool_policy::Preset::for_model(&session_config.model, self.subagent_depth)
            }
        };
        let tools = crate::tool_policy::filter(tools, preset.clone());

        let governed_registry = guardian.build_registry(
            tools,
            crate::governance::LiveUserTurn::from_session_config(session_config),
        );
        (governed_registry, preset)
    }
}

/// The chat a harness tool call comes from. A session it starts drives by default only when
/// the user typed the turn in a chat no `hq_ask` started; a lookup that fails counts as an
/// ask thread, the safe side for Drive.
fn watching_chat(
    identity: &hq_core::identity::RequestIdentity,
    agent_host: &hq_core::config::AgentHostConfig,
    db: &Arc<hq_db::Database>,
) -> Option<hq_tools::harness_session::WatchingChat> {
    let thread = identity.web_thread()?.to_string();
    let from_ask = db
        .with_conn(|c| hq_db::ask_requests::thread_is_ask_owned(c, &thread))
        .unwrap_or(true);
    // Sub-agent follow-ups start no driving watch either, but only the session driver is budgeted.
    Some(hq_tools::harness_session::WatchingChat {
        thread,
        drive_new: agent_host.drive_new_watches && !identity.is_web_driver_turn() && !from_ask,
        driver_turn: identity.is_session_driver_turn(),
        from_ask,
    })
}

#[cfg(test)]
mod watching_chat_tests {
    use super::*;
    use hq_core::identity::RequestIdentity;

    fn ask_thread(db: &Arc<hq_db::Database>) -> String {
        db.with_conn(|c| {
            Ok(hq_db::ask_requests::open(
                c,
                &hq_db::ask_requests::NewAsk {
                    thread: hq_db::ask_requests::ThreadTarget::New { title: "q" },
                    external_id: None,
                    scope: "full",
                    mode: "full",
                    caller: "claude-code",
                    fingerprint: "f",
                },
            )?
            .row
            .thread_id)
        })
        .unwrap()
    }

    #[test]
    fn sessions_started_from_a_driver_turn_or_an_ask_thread_never_drive_by_default() {
        let db = Arc::new(hq_db::Database::open_memory().unwrap());
        let host_cfg = hq_core::config::AgentHostConfig::default();
        assert!(host_cfg.drive_new_watches);
        let typed = watching_chat(
            &RequestIdentity::from_web_thread("th-typed", false, false),
            &host_cfg,
            &db,
        )
        .unwrap();
        assert!((typed.drive_new, typed.driver_turn, typed.from_ask) == (true, false, false));
        let driver = watching_chat(
            &RequestIdentity::from_web_thread("th-typed", true, true),
            &host_cfg,
            &db,
        )
        .unwrap();
        assert!((driver.drive_new, driver.driver_turn) == (false, true));
        let thread = ask_thread(&db);
        let asked = watching_chat(
            &RequestIdentity::from_web_thread(&thread, false, false),
            &host_cfg,
            &db,
        )
        .unwrap();
        assert!((asked.drive_new, asked.from_ask) == (false, true));
        assert!(
            watching_chat(&RequestIdentity::local(), &host_cfg, &db).is_none(),
            "no chat, no watch"
        );
    }
}
