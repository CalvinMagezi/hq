use super::*;
use crate::backend::{BackendCapabilities, BackendError, BackendRequest};
use async_trait::async_trait;

/// A no-op backend: `authorize_and_resolve` never calls `start()`, so this
/// only needs to exist and be named — exactly what registering a fleet
/// harness for capability-negotiation tests requires.
struct StubBackend {
    name: String,
}

#[async_trait]
impl SessionBackend for StubBackend {
    fn name(&self) -> &str {
        &self.name
    }
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::buffered_cli()
    }
    async fn start(
        &self,
        _request: &BackendRequest,
    ) -> Result<crate::backend::BackendEventStream, BackendError> {
        Err(BackendError::Unavailable(
            "StubBackend::start should not be called in these tests".to_string(),
        ))
    }
}

struct DummyProvider;

#[async_trait]
impl LlmProvider for DummyProvider {
    fn name(&self) -> &str {
        "dummy"
    }
    async fn chat(
        &self,
        _request: &hq_llm::provider::ChatRequest,
    ) -> anyhow::Result<hq_llm::provider::ChatResponse> {
        anyhow::bail!("DummyProvider::chat should not be called in these tests")
    }
    async fn chat_stream(
        &self,
        _request: &hq_llm::provider::ChatRequest,
    ) -> anyhow::Result<
        std::pin::Pin<
            Box<
                dyn tokio_stream::Stream<Item = anyhow::Result<hq_llm::provider::StreamChunk>>
                    + Send,
            >,
        >,
    > {
        anyhow::bail!("DummyProvider::chat_stream should not be called in these tests")
    }
}

/// A service with a registry containing one known leaf/text-only harness
/// ("groq") and one known agentic harness ("claude-code").
fn service_with_fleet() -> AgentService {
    let entries: Vec<(String, Arc<dyn SessionBackend>)> = vec![
        (
            "groq".to_string(),
            Arc::new(StubBackend {
                name: "groq".to_string(),
            }) as Arc<dyn SessionBackend>,
        ),
        (
            "claude-code".to_string(),
            Arc::new(StubBackend {
                name: "claude-code".to_string(),
            }) as Arc<dyn SessionBackend>,
        ),
    ];
    let registry = Arc::new(BackendRegistry::from_backends(entries, "claude-code"));
    AgentService::new(
        Arc::new(DummyProvider),
        PathBuf::from("/tmp"),
        vec![PathBuf::from("/tmp")],
        SessionConfig::default(),
        SecurityProfile::Guarded,
        0,
        3,
        Duration::from_secs(60),
    )
    .with_backend_registry(Some(registry))
}

#[test]
fn file_editing_request_against_leaf_harness_is_rejected() {
    let service = service_with_fleet();
    let mut req = ChildRequest::new("t1", "fix the bug");
    req.backend = Some("groq".to_string());
    req.files_in_scope = vec!["src/main.rs".to_string()];

    let err = service
        .authorize_and_resolve(&req)
        .expect_err("a leaf harness should refuse file-editing work loudly");
    assert!(err.contains("groq"));
    assert!(err.contains("text-only"));
    assert!(err.contains("src/main.rs"));
}

#[test]
fn file_editing_request_against_agentic_harness_resolves() {
    let service = service_with_fleet();
    let mut req = ChildRequest::new("t1", "fix the bug");
    req.backend = Some("claude-code".to_string());
    req.files_in_scope = vec!["src/main.rs".to_string()];

    let resolution = service
        .authorize_and_resolve(&req)
        .expect("a tool-capable harness should be able to take file-editing work");
    assert_eq!(resolution.resolved_backend, "claude-code");
}

#[test]
fn pure_reasoning_request_against_leaf_harness_still_resolves() {
    // No files_in_scope named — nothing implies this child needs tool
    // access, so the leaf harness is a legitimate choice.
    let service = service_with_fleet();
    let mut req = ChildRequest::new("t1", "summarize this");
    req.backend = Some("groq".to_string());

    let resolution = service
        .authorize_and_resolve(&req)
        .expect("a pure-reasoning request should not trip the capability check");
    assert_eq!(resolution.resolved_backend, "groq");
}

#[test]
fn file_editing_request_against_unknown_harness_resolves_or_fails_on_registry_only() {
    // Unknown harness names default to "agentic" (is_agentic_harness's own
    // documented safe default), so the capability check must not be what
    // rejects this — only "not in the registry" should.
    let service = service_with_fleet();
    let mut req = ChildRequest::new("t1", "fix the bug");
    req.backend = Some("some-new-harness".to_string());
    req.files_in_scope = vec!["src/main.rs".to_string()];

    let err = service
        .authorize_and_resolve(&req)
        .expect_err("harness isn't registered, so resolution should fail");
    assert!(err.contains("not available in the registry"));
}

/// FR-056: a subagent inherits the parent's session config verbatim aside
/// from the deliberate model/budget routing in `build_child_session` — there
/// is no `ChildRequest` field, tool argument, or per-role override that can
/// impose a turn-count ceiling on a child (`SessionConfig` has no such field
/// to set).
#[test]
fn subagent_child_session_has_no_turn_limit_to_impose() {
    let service = service_with_fleet();
    let req = ChildRequest::new("t1", "unbounded task");
    let resolution = service.authorize_and_resolve(&req).unwrap();
    let run_ctx = RunCtx {
        parent_run_id: "parent-1".to_string(),
        sink: None,
        parent_messages: None,
        seq: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        ledger: crate::agents::ledger::Ledger::open(
            None,
            &crate::agents::ChildPlan::single(req.clone()),
            &crate::agents::ChildExecContext::default(),
            false,
        ),
    };
    let child = service.build_child_session(&req, &resolution, &run_ctx, "child-1");
    // The parent used `SessionConfig::default()`; the child keeps its
    // wall-clock cap unchanged since nothing in `ChildRequest` can touch it.
    assert_eq!(
        child.config().max_duration_secs,
        SessionConfig::default().max_duration_secs
    );
}

fn child_for_vault(vault: &std::path::Path) -> AgentSession {
    let service = AgentService::new(
        Arc::new(DummyProvider),
        vault.to_path_buf(),
        vec![vault.to_path_buf()],
        SessionConfig::default(),
        SecurityProfile::Guarded,
        0,
        3,
        Duration::from_secs(60),
    );
    let req = ChildRequest::new("t1", "deploy the thing");
    let resolution = service.authorize_and_resolve(&req).unwrap();
    let run_ctx = RunCtx {
        parent_run_id: "parent-1".to_string(),
        sink: None,
        parent_messages: None,
        seq: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        ledger: crate::agents::ledger::Ledger::open(
            None,
            &crate::agents::ChildPlan::single(req.clone()),
            &crate::agents::ChildExecContext::default(),
            false,
        ),
    };
    service.build_child_session(&req, &resolution, &run_ctx, "child-1")
}

#[test]
fn child_session_gets_the_skill_catalog_and_load_skill_when_the_vault_has_skills() {
    let empty = tempfile::tempdir().unwrap();
    let without = child_for_vault(empty.path());
    assert!(
        !without
            .system_prompt()
            .unwrap_or_default()
            .contains("Available HQ Skills")
    );

    let vault = tempfile::tempdir().unwrap();
    let skill = hq_core::skills_dir(vault.path()).join("deploy-pwa");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\ndescription: \"Deploy the PWA\"\nhints:\n  - caddy\n---\nbody",
    )
    .unwrap();
    let with = child_for_vault(vault.path());
    assert!(with.system_prompt().unwrap().contains("deploy-pwa"));
    assert_eq!(with.tool_count(), without.tool_count() + 1);
}

/// A read-only parent must not be able to hand its work to a child that may write.
#[test]
fn children_run_under_the_parents_permission_mode_and_nested_ones_too() {
    use hq_core::types::PermissionMode;

    let service = service_with_fleet();
    let paths = vec![PathBuf::from("/tmp")];
    assert!(matches!(
        service.child_guardian(paths.clone()).permission_mode(),
        PermissionMode::Default
    ));

    let read_only = service.with_permission_mode(PermissionMode::DontAsk);
    assert!(matches!(
        read_only.child_guardian(paths.clone()).permission_mode(),
        PermissionMode::DontAsk
    ));
    assert!(matches!(
        read_only
            .child_service()
            .child_guardian(paths)
            .permission_mode(),
        PermissionMode::DontAsk
    ));
}

/// An orchestrator's children read, search and report, but have no file or git writers.
#[test]
fn orchestrator_children_have_no_file_writers() {
    use crate::builder::{ORCHESTRATOR_REMOVED_TOOLS, SessionRole};

    let provider: Arc<dyn LlmProvider> = Arc::new(DummyProvider);
    let config = SessionConfig::default();
    let names = |service: AgentService| -> Vec<String> {
        service
            .build_inprocess_tools(&config, &provider)
            .iter()
            .map(|t| t.name().to_string())
            .collect()
    };
    let implementor = names(service_with_fleet());
    let orchestrator = names(service_with_fleet().with_role(SessionRole::Orchestrator));
    assert!(implementor.iter().any(|n| n == "write_file"), "premise: today's child can write");
    for removed in ORCHESTRATOR_REMOVED_TOOLS {
        assert!(!orchestrator.iter().any(|n| n == removed), "{removed} reached an orchestrator child");
    }
    for kept in ["read_file", "grep", "find_files", "bash"] {
        assert!(orchestrator.iter().any(|n| n == kept), "{kept} missing");
    }
}

/// An orchestrator's child cannot be sent to a harness that edits files, by name or by default.
#[test]
fn orchestrator_children_cannot_use_a_file_editing_external_backend() {
    use crate::builder::SessionRole;

    let service = service_with_fleet().with_role(SessionRole::Orchestrator);
    let mut named = ChildRequest::new("t1", "fix the bug");
    named.backend = Some("claude-code".to_string());
    assert!(service.authorize_and_resolve(&named).is_err());

    let mut leaf = ChildRequest::new("t2", "summarize this");
    leaf.backend = Some("groq".to_string());
    assert!(service.authorize_and_resolve(&leaf).is_ok(), "text-only backends stay available");

    let auto = ChildRequest::new("t3", "think about this");
    let resolved = service.authorize_and_resolve(&auto).expect("auto resolves");
    assert_eq!(resolved.resolved_backend, INPROCESS_BACKEND, "auto policy stays in-process");
}
