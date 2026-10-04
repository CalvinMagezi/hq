use super::*;

/// An `HqTool` that overrides `is_destructive()` against what its
/// `is_read_only()` would otherwise default to — proves `HqToolAdapter`
/// forwards the tool's real answer instead of re-deriving one.
struct ExplicitDestructiveHqTool;
#[async_trait::async_trait]
impl hq_tools::HqTool for ExplicitDestructiveHqTool {
    fn name(&self) -> &str {
        "explicit_destructive_hq_tool"
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
    fn is_destructive(&self) -> bool {
        // Deliberately disagrees with the `!is_read_only()` default, so
        // the test can tell "forwarded" apart from "re-derived."
        true
    }
    async fn execute(&self, _: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::json!("ok"))
    }
}

#[test]
fn hq_tool_adapter_forwards_is_destructive_instead_of_rederiving_it() {
    let adapter = HqToolAdapter {
        inner: Box::new(ExplicitDestructiveHqTool),
    };
    assert!(AgentTool::is_destructive(&adapter));
}

#[test]
fn permission_preset_sets_both_security_profile_and_permission_mode() {
    let config = HqConfig::default();
    let builder =
        SessionBuilder::from_config(&config).permission_preset(PermissionPreset::WorkspaceWrite);
    assert!(matches!(
        builder.test_security_profile(),
        SecurityProfile::Guarded
    ));
    assert!(matches!(
        builder.test_permission_mode(),
        PermissionMode::AcceptEdits
    ));
}

#[test]
fn permission_mode_setter_is_independent_of_security_profile() {
    let config = HqConfig::default();
    let builder = SessionBuilder::from_config(&config)
        .security_profile(SecurityProfile::Admin)
        .permission_mode(PermissionMode::DontAsk);
    assert!(matches!(
        builder.test_security_profile(),
        SecurityProfile::Admin
    ));
    assert!(matches!(
        builder.test_permission_mode(),
        PermissionMode::DontAsk
    ));
}

/// A named no-op provider: proves which handle drives a session's turns
/// without ever touching the network (`chat`/`chat_stream` are unused).
struct NamedMockProvider(&'static str);

#[async_trait::async_trait]
impl LlmProvider for NamedMockProvider {
    fn name(&self) -> &str {
        self.0
    }
    async fn chat(
        &self,
        _request: &hq_llm::provider::ChatRequest,
    ) -> Result<hq_llm::provider::ChatResponse> {
        anyhow::bail!("NamedMockProvider::chat should not be called in this test")
    }
    async fn chat_stream(
        &self,
        _request: &hq_llm::provider::ChatRequest,
    ) -> Result<
        std::pin::Pin<
            Box<dyn tokio_stream::Stream<Item = Result<hq_llm::provider::StreamChunk>> + Send>,
        >,
    > {
        anyhow::bail!("NamedMockProvider::chat_stream should not be called in this test")
    }
}

/// Build a session whose config declares a CLI-primary `backends` chain,
/// optionally with an explicit `.provider(...)` override. No network call:
/// the CLI harness backend is always constructible and the mock provider is
/// never invoked during `build`.
async fn build_session_with_backends(provider: Option<Arc<dyn LlmProvider>>) -> AgentSession {
    use hq_core::config::{BackendEntry, BackendKind, BackendsConfig};

    let dir = std::env::temp_dir().join(format!(
        "hq-builder-provider-override-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    let mut config = HqConfig {
        openrouter_api_key: Some("test-key-no-network".to_string()),
        vault_path: dir.clone(),
        local_only: false,
        ..Default::default()
    };
    config.backends = BackendsConfig {
        primary: "copilot".to_string(),
        fallbacks: vec![],
        backends: vec![BackendEntry {
            name: "copilot".to_string(),
            kind: BackendKind::GithubCopilotCli,
            endpoint: None,
            credential_env: None,
            model: None,
            effort: None,
            wire: Default::default(),
            enabled: true,
        }],
        ..Default::default()
    };

    let mut builder = SessionBuilder::from_config(&config).working_dir(dir);
    if let Some(provider) = provider {
        builder = builder.provider(provider);
    }
    builder
        .build()
        .await
        .expect("session should build without any network call")
}

#[tokio::test]
async fn configured_chain_drives_turns_without_a_provider_override() {
    // Baseline: with a `backends` chain and no override, the chain drives turns.
    let session = build_session_with_backends(None).await;
    assert_eq!(session.backend_name(), "provider-chain");
}

#[tokio::test]
async fn explicit_provider_override_takes_over_turns_even_with_backends_config() {
    // `.provider(...)` must override turn execution as documented: turns run
    // through the single ApiBackend adapter over the override (named after the
    // provider), not the configured chain.
    let provider: Arc<dyn LlmProvider> = Arc::new(NamedMockProvider("override-provider"));
    let session = build_session_with_backends(Some(provider)).await;
    assert_eq!(
        session.backend_name(),
        "override-provider",
        "explicit .provider(...) must drive turns, not the configured chain"
    );
}

#[test]
fn concept_search_results_populated_when_user_message_names_a_known_concept() {
    let dir = std::env::temp_dir().join(format!(
        "hq-builder-concept-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let vault = hq_vault::VaultClient::new(dir).unwrap();
    hq_memory::concept_pages::upsert_concept_page(
        &vault,
        "Fleet Dispatcher",
        "concept",
        &[],
        "test",
    )
    .unwrap();

    let results = concept_search_results(&vault, "how does the fleet dispatcher pick a harness?");

    assert_eq!(results.len(), 1);
    assert!(results[0].note_path.contains("fleet-dispatcher"));
}

/// Builds a session exactly the way `hq_agent::native_hq::run_native_hq` does
/// (same config shape, same "relay" model alias used by the live Telegram
/// bridge) so tests measure the real tool catalog a Telegram message would
/// get, with no network call — `build_provider_from_config` only constructs
/// lazy HTTP clients.
async fn build_measurement_session(profile: SessionProfile) -> AgentSession {
    let dir = std::env::temp_dir().join(format!(
        "hq-builder-measure-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    let mut config = HqConfig {
        openrouter_api_key: Some("test-key-no-network".to_string()),
        vault_path: dir.clone(),
        local_only: false,
        ..Default::default()
    };
    config.relay.model = Some("relay".to_string());

    let session_config = SessionConfig {
        model: "relay".to_string(),
        ..SessionConfig::default()
    };

    SessionBuilder::from_config(&config)
        .working_dir(dir)
        .session_config(session_config)
        .session_profile(profile)
        .build()
        .await
        .expect("session should build without any network call")
}

struct AutonomyRestrictedStub;

#[async_trait::async_trait]
impl AgentTool for AutonomyRestrictedStub {
    fn name(&self) -> &str {
        "acme_call_stub"
    }
    fn description(&self) -> &str {
        "stub"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({})
    }
    fn requires_live_user_turn(&self) -> bool {
        true
    }
    async fn execute(
        &self,
        _id: &str,
        _args: serde_json::Value,
    ) -> Result<hq_core::types::ToolResult> {
        unreachable!("stub is never actually invoked in this test")
    }
}

/// Proves the autonomy gate cannot be bypassed via `AgentProfile.allow_tools`:
/// even an explicit, maximally-permissive vault profile for this exact agent
/// name allow-listing the stub tool by name must not resurrect it, because
/// `ToolGuardian::build_registry` excludes it before `tool_policy::filter`
/// ever runs.
#[tokio::test]
async fn autonomy_restricted_tool_excluded_even_with_allow_tools_override() {
    let dir = std::env::temp_dir().join(format!(
        "hq-builder-autonomy-gate-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join("_system")).unwrap();
    std::fs::write(
        dir.join("_system").join("AGENT_PROFILES.yaml"),
        "mission-test:\n  allow_tools: [\"acme_call_stub\"]\n",
    )
    .unwrap();

    let config = HqConfig {
        openrouter_api_key: Some("test-key-no-network".to_string()),
        vault_path: dir.clone(),
        ..Default::default()
    };

    let session_config = SessionConfig {
        agent_name: "mission-test".to_string(),
        is_live_user_turn: false, // explicit, though it's already the default
        ..SessionConfig::default()
    };

    let session = SessionBuilder::from_config(&config)
        .working_dir(dir)
        .session_config(session_config)
        .add_tool(Box::new(AutonomyRestrictedStub))
        .build()
        .await
        .expect("session should build without any network call");

    let names = session.tool_names().await;
    assert!(!names.contains(&"acme_call_stub".to_string()));
}

/// Companion to the above: the same tool, same allow-list, but a live user
/// turn — the tool must be present. Guards against the gate over-blocking.
#[tokio::test]
async fn autonomy_restricted_tool_present_for_a_live_session() {
    let dir = std::env::temp_dir().join(format!(
        "hq-builder-autonomy-gate-live-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    let config = HqConfig {
        openrouter_api_key: Some("test-key-no-network".to_string()),
        vault_path: dir.clone(),
        ..Default::default()
    };

    let session_config = SessionConfig {
        agent_name: "hq".to_string(),
        is_live_user_turn: true,
        ..SessionConfig::default()
    };

    let session = SessionBuilder::from_config(&config)
        .working_dir(dir)
        .session_config(session_config)
        .add_tool(Box::new(AutonomyRestrictedStub))
        .build()
        .await
        .expect("session should build without any network call");

    let names = session.tool_names().await;
    assert!(names.contains(&"acme_call_stub".to_string()));
}

/// Regression test for the Task 6 incident: a Telegram "Remember the code word
/// plum42." message (classified `TaskType::Vault`) produced a ~23,074-token
/// request that Groq's 12,000 TPM rate limit rejected. Measurement (see
/// task-6-report.md) found `SessionBuilder` registers 71 tools as full,
/// non-deferred function-calling schemas — 35,825 bytes of serialized JSON
/// schema, ~8,956 estimated tokens — regardless of task type. Switching to
/// `SessionProfile::Weak` (the 9 tools already tagged `ToolPolicy::Weak`)
/// drops that to 9 tools / 3,306 bytes / ~826 tokens, an ~8,130-token
/// reduction. `configure_native_hq_builder` now applies `SessionProfile::Weak`
/// for `TaskType::Vault` prompts. This asserts that gating holds: the
/// Weak-profile tool set is a small, fixed fraction of the Standard set.
#[tokio::test]
async fn weak_session_profile_has_far_fewer_tools_than_standard() {
    let standard = build_measurement_session(SessionProfile::Standard).await;
    let weak = build_measurement_session(SessionProfile::Weak).await;

    assert!(
        standard.tool_count() >= 60,
        "expected the Standard-profile baseline to be dozens of tools (regardless \
         of task type) — got {}; if this fails because the catalog shrank, that's \
         fine, but re-check the ~9000-token estimate in task-6-report.md still holds",
        standard.tool_count()
    );
    assert!(
        weak.tool_count() <= 16,
        "SessionProfile::Weak should keep only the small vault/dev shortcut \
         tool set (~9 tools tagged ToolPolicy::Weak, plus system_info and the \
         5-tool CAPABILITY_FLOOR) — got {}",
        weak.tool_count()
    );
    assert!(
        weak.tool_count() < standard.tool_count() / 3,
        "Weak profile ({}) should be a small fraction of Standard ({}) — the whole \
         point of the gate is to stop registering the full catalog for simple, \
         non-coding messages",
        weak.tool_count(),
        standard.tool_count()
    );
}

#[test]
fn harness_block_carries_machine_profile_and_tool_notes() {
    let block = build_harness_block(
        Some("caller instructions here"),
        crate::tool_policy::Preset::Cloud,
        None,
        42,
        DerivedPromptBlocks {
            machine_block: Some(
                "# Machine Profile\n- **GitHub**: gh 2.62.0 — authenticated as someone\n",
            ),
            tool_notes: Some("## Tool Usage Notes\n\n- **bash**: prefer the dedicated tools.\n"),
            tool_catalog: None,
            can_build_self: true,
        },
    );
    assert!(block.contains("## Environment"), "{block}");
    assert!(block.contains("authenticated as someone"), "{block}");
    assert!(block.contains("## Tool Usage Notes"), "{block}");
    assert!(block.contains("prefer the dedicated tools"), "{block}");
    // Caller instructions stay last so the stable prefix keeps growing.
    assert!(
        block.trim_end().ends_with("caller instructions here"),
        "{block}"
    );
    assert!(
        block.len() < 8_000,
        "harness block grew to {} bytes; it ships every turn",
        block.len()
    );
}

/// Standard sessions already send every tool's full JSON schema each turn,
/// so enumerating names again would be duplication — and would advertise
/// tools on a path with no universal dispatcher.
#[test]
fn harness_block_omits_the_catalog_when_not_weak() {
    let block = build_harness_block(
        None,
        crate::tool_policy::Preset::Cloud,
        None,
        42,
        DerivedPromptBlocks::default(),
    );
    assert!(!block.contains("## Your Tools"), "{block}");

    let weak = build_harness_block(
        None,
        crate::tool_policy::Preset::LocalGemma,
        None,
        14,
        DerivedPromptBlocks {
            tool_catalog: Some(
                "## Your Tools (14 available)\n\n**vault**\n- `vault_find` — search\n",
            ),
            ..Default::default()
        },
    );
    assert!(weak.contains("## Your Tools (14 available)"), "{weak}");
}

/// The Weak tier is a token optimization, not a lobotomy. Before the
/// floor, a Telegram message containing the bare word "remember" produced
/// a session with no shell and no file access at all.
#[tokio::test]
async fn weak_profile_keeps_capability_floor() {
    let weak = build_measurement_session(SessionProfile::Weak).await;
    let names = weak.tool_names().await;
    for required in CAPABILITY_FLOOR {
        assert!(
            names.iter().any(|n| n == required),
            "SessionProfile::Weak dropped {required}; have {names:?}"
        );
    }
}

/// Git was MCP-only, which is why HQ denied having GitHub access while
/// `gh` sat authenticated on PATH.
#[tokio::test]
async fn standard_profile_registers_git_and_system_tools() {
    let standard = build_measurement_session(SessionProfile::Standard).await;
    let names = standard.tool_names().await;
    for required in ["git_status", "git_diff", "git_log", "git_commit", "git_pr"] {
        assert!(names.iter().any(|n| n == required), "missing {required}");
    }
    assert!(
        names.iter().any(|n| n == "system_info"),
        "missing system_info"
    );
}

/// `system_info` is `ToolPolicy::Weak` precisely so a relay turn can still
/// answer "do I have gh?".
#[tokio::test]
async fn weak_profile_keeps_system_info() {
    let weak = build_measurement_session(SessionProfile::Weak).await;
    assert!(weak.tool_names().await.iter().any(|n| n == "system_info"));
}

/// Native tasks are HQ's task system; the session must register them
/// directly instead of leaving them reachable only through the gateway.
#[tokio::test]
async fn standard_profile_registers_task_tools() {
    let standard = build_measurement_session(SessionProfile::Standard).await;
    let names = standard.tool_names().await;
    for required in ["task_create", "task_list", "task_get", "task_update"] {
        assert!(names.iter().any(|n| n == required), "missing {required}");
    }
}

/// Governance wraps every tool, so a metadata method it fails to forward
/// collapses to the trait default for the entire registry. `should_defer`
/// was permanently false, which made `deferred_catalog()` always empty and
/// the `tool_search` meta-tool unreachable — while the LSP tools that opt
/// into deferral shipped full schemas every turn anyway.
#[tokio::test]
async fn governed_tools_preserve_deferral_and_category() {
    let standard = build_measurement_session(SessionProfile::Standard).await;
    let deferred = standard.deferred_tool_catalog().await;
    assert!(
        !deferred.is_empty(),
        "no tools deferred after governance — should_defer is being dropped again"
    );
    assert!(
        standard.active_tool_definitions().await.len() < standard.tool_count(),
        "active definitions should exclude the deferred tools"
    );
    // Sorted output keeps the cached prompt prefix byte-stable across restarts.
    let mut sorted = deferred.clone();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(deferred, sorted, "deferred_catalog must be sorted by name");
}

/// End-to-end proof that `configure_native_hq_builder` — the actual function
/// `run_native_hq` calls for both the Telegram relay and the HTTP proxy —
/// applies the Weak profile for the exact failing message from the incident,
/// and leaves calendar/quick-reasoning/coding messages on the full tool set
/// (so the fix doesn't silently break non-vault Telegram requests).
#[tokio::test]
async fn configure_native_hq_builder_gates_tool_count_by_task_type() {
    let dir = std::env::temp_dir().join(format!(
        "hq-native-hq-gate-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let config = HqConfig {
        openrouter_api_key: Some("test-key-no-network".to_string()),
        vault_path: dir.clone(),
        local_only: false,
        ..Default::default()
    };

    let build_for_prompt = |prompt: &'static str| {
        let config = config.clone();
        let dir = dir.clone();
        async move {
            let builder = SessionBuilder::from_config(&config).working_dir(dir);
            let builder = crate::native_hq::configure_native_hq_builder(
                builder,
                &config,
                prompt,
                "base instructions".to_string(),
            );
            builder.build().await.expect("session should build")
        }
    };

    let vault_session = build_for_prompt("Remember the code word plum42.").await;
    let calendar_session = build_for_prompt("What's on my calendar tomorrow?").await;

    assert!(
        vault_session.tool_count() <= 15,
        "TaskType::Vault prompt should get the Weak tool tier — got {} tools",
        vault_session.tool_count()
    );
    assert!(
        calendar_session.tool_count() > vault_session.tool_count() * 2,
        "a non-vault message (calendar lookup) must keep the full tool set so \
         this fix doesn't regress everyday Telegram functionality — vault={} \
         calendar={}",
        vault_session.tool_count(),
        calendar_session.tool_count()
    );
}

/// Task 6 review finding 2: `task_classifier::classify_task` checks
/// `TaskType::Vault` triggers (bare "note", "notebook", "remember") before
/// `TaskType::CodeEditing` triggers, so a real coding prompt like "Add a note
/// explaining the recursion in fibonacci.rs" misclassifies as Vault. Before
/// this fix, that misclassification would apply `SessionProfile::Weak` and
/// strip `bash`/`edit`/`read`/`write` entirely — a hard failure for a coding
/// task, not a degraded one. `configure_native_hq_builder` must skip the Weak
/// downgrade when the prompt still carries an obvious code signal.
#[tokio::test]
async fn configure_native_hq_builder_keeps_full_tools_for_vault_misclassified_code_prompt() {
    let dir = std::env::temp_dir().join(format!(
        "hq-native-hq-vault-miscls-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let config = HqConfig {
        openrouter_api_key: Some("test-key-no-network".to_string()),
        vault_path: dir.clone(),
        local_only: false,
        ..Default::default()
    };

    let prompt = "Add a note explaining the recursion in fibonacci.rs";
    assert_eq!(
        hq_tools::task_classifier::classify_task(prompt),
        hq_tools::task_classifier::TaskType::Vault,
        "prompt should misclassify as Vault via the bare word 'note' — this test \
         is only meaningful if the primary classifier actually mis-fires here"
    );

    let builder = SessionBuilder::from_config(&config).working_dir(dir);
    let builder = crate::native_hq::configure_native_hq_builder(
        builder,
        &config,
        prompt,
        "base instructions".to_string(),
    );
    let session = builder.build().await.expect("session should build");
    assert!(
        session.tool_count() > 20,
        "a Vault-misclassified prompt with an obvious code signal (fibonacci.rs) \
         must keep the full tool catalog so bash/edit/read/write stay available — \
         got {} tools",
        session.tool_count()
    );
}

/// The skill catalog in every prompt says "use load_skill"; a session built
/// without it can never load a skill, which is how zero loads were ever logged.
#[tokio::test]
async fn skill_tools_survive_the_profile_filters() {
    let standard = build_measurement_session(SessionProfile::Standard)
        .await
        .tool_names()
        .await;
    let weak = build_measurement_session(SessionProfile::Weak)
        .await
        .tool_names()
        .await;
    assert!(standard.iter().any(|n| n == "load_skill"), "{standard:?}");
    assert!(standard.iter().any(|n| n == "skill_manage"), "{standard:?}");
    assert!(weak.iter().any(|n| n == "load_skill"), "{weak:?}");
}
