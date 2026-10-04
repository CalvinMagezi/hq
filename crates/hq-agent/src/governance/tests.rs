use super::*;
use anyhow::Result;
use async_trait::async_trait;
use hq_core::types::PermissionPreset;
use hq_core::types::{ToolResult, ToolResultContent};
use serde_json::Value;
use std::path::PathBuf;

struct RestrictedStub;

#[async_trait::async_trait]
impl crate::tools::AgentTool for RestrictedStub {
    fn name(&self) -> &str {
        "acme_call_stub"
    }
    fn description(&self) -> &str {
        "stub"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({})
    }
    fn requires_live_user_turn(&self) -> bool {
        true
    }
    async fn execute(&self, _id: &str, _args: Value) -> Result<ToolResult> {
        unreachable!("stub is never actually invoked in this test")
    }
}

#[test]
fn autonomy_restricted_tool_excluded_when_unattended() {
    let guardian =
        ToolGuardian::with_default_mode(vec![PathBuf::from(".")], SecurityProfile::Guarded);
    let registry =
        guardian.build_registry(vec![Box::new(RestrictedStub)], LiveUserTurn::unattended());
    assert!(registry.get("acme_call_stub").is_none());
}

#[test]
fn autonomy_restricted_tool_present_for_a_live_session() {
    let guardian =
        ToolGuardian::with_default_mode(vec![PathBuf::from(".")], SecurityProfile::Guarded);
    let config = crate::session::SessionConfig {
        is_live_user_turn: true,
        ..crate::session::SessionConfig::default()
    };
    let registry = guardian.build_registry(
        vec![Box::new(RestrictedStub)],
        LiveUserTurn::from_session_config(&config),
    );
    assert!(registry.get("acme_call_stub").is_some());
}

#[test]
fn test_security_floor() {
    assert!(!meets_security_floor(&SecurityProfile::Minimal));
    assert!(meets_security_floor(&SecurityProfile::Standard));
    assert!(meets_security_floor(&SecurityProfile::Guarded));
    assert!(meets_security_floor(&SecurityProfile::Admin));
}

#[test]
#[should_panic(expected = "below the minimum floor")]
fn test_guardian_rejects_minimal_profile() {
    ToolGuardian::new(
        vec![PathBuf::from("/tmp")],
        SecurityProfile::Minimal,
        PermissionMode::Default,
    );
}

#[test]
#[should_panic(expected = "requires at least one allowed path")]
fn test_guardian_rejects_empty_paths() {
    ToolGuardian::new(vec![], SecurityProfile::Guarded, PermissionMode::Default);
}

#[test]
fn test_guardian_accepts_valid_config() {
    let guardian = ToolGuardian::new(
        vec![PathBuf::from("/tmp")],
        SecurityProfile::Guarded,
        PermissionMode::Default,
    );
    assert_eq!(guardian.profile(), &SecurityProfile::Guarded);
    assert!(matches!(
        guardian.permission_mode(),
        PermissionMode::Default
    ));
}

#[test]
fn test_guardian_with_default_mode_convenience() {
    let guardian =
        ToolGuardian::with_default_mode(vec![PathBuf::from("/tmp")], SecurityProfile::Guarded);
    assert!(matches!(
        guardian.permission_mode(),
        PermissionMode::Default
    ));
}

#[test]
fn test_guardian_from_preset_resolves_expected_profile_and_mode() {
    let read_only =
        ToolGuardian::from_preset(PermissionPreset::ReadOnly, vec![PathBuf::from("/tmp")]);
    assert!(matches!(read_only.profile(), SecurityProfile::Standard));
    assert!(matches!(
        read_only.permission_mode(),
        PermissionMode::DontAsk
    ));

    let workspace_write = ToolGuardian::from_preset(
        PermissionPreset::WorkspaceWrite,
        vec![PathBuf::from("/tmp")],
    );
    assert!(matches!(
        workspace_write.profile(),
        SecurityProfile::Guarded
    ));
    assert!(matches!(
        workspace_write.permission_mode(),
        PermissionMode::AcceptEdits
    ));

    let danger = ToolGuardian::from_preset(
        PermissionPreset::DangerFullAccess,
        vec![PathBuf::from("/tmp")],
    );
    assert!(matches!(danger.profile(), SecurityProfile::Admin));
    assert!(matches!(
        danger.permission_mode(),
        PermissionMode::BypassPermissions
    ));
}

#[test]
fn test_empty_paths_denies_everything() {
    // Directly test GovernedTool with empty paths (defense-in-depth check)
    let tool = GovernedTool {
        inner: Box::new(DummyTool),
        allowed_paths: vec![],
        denied_paths: vec![],
        profile: SecurityProfile::Guarded,
        permission_mode: PermissionMode::Default,
        denial_tracker: DenialTracker::new(),
        repeat_call_tracker: RepeatCallTracker::new(),
        denial_notifier: None,
        notified_denials: Arc::new(Mutex::new(HashSet::new())),
        plan_file_path: None,
        files_read: Arc::new(Mutex::new(HashSet::new())),
        allow_defer: true,
        taint: TaintTracker::new(),
    };
    assert!(!tool.is_path_allowed("/any/path"));
}

#[tokio::test]
async fn test_bypass_permissions_skips_path_check() {
    // BypassPermissions should execute even with empty allowed_paths
    let tool = GovernedTool {
        inner: Box::new(DummyTool),
        allowed_paths: vec![],
        denied_paths: vec![],
        profile: SecurityProfile::Guarded,
        permission_mode: PermissionMode::BypassPermissions,
        denial_tracker: DenialTracker::new(),
        repeat_call_tracker: RepeatCallTracker::new(),
        denial_notifier: None,
        notified_denials: Arc::new(Mutex::new(HashSet::new())),
        plan_file_path: None,
        files_read: Arc::new(Mutex::new(HashSet::new())),
        allow_defer: true,
        taint: TaintTracker::new(),
    };
    let result = tool
        .execute("id", serde_json::json!({"file_path": "/outside/any/path"}))
        .await;
    assert!(result.is_ok());
    // DummyTool returns empty content — no "Access denied" text
    let tr = result.unwrap();
    assert!(tr.content.is_empty() || !tr.content[0].text.contains("Access denied"));
}

#[tokio::test]
async fn test_dont_ask_denies_non_readonly_tool() {
    let tool = GovernedTool {
        inner: Box::new(DummyTool), // is_read_only() returns false
        allowed_paths: vec![PathBuf::from("/tmp")],
        denied_paths: vec![],
        profile: SecurityProfile::Guarded,
        permission_mode: PermissionMode::DontAsk,
        denial_tracker: DenialTracker::new(),
        repeat_call_tracker: RepeatCallTracker::new(),
        denial_notifier: None,
        notified_denials: Arc::new(Mutex::new(HashSet::new())),
        plan_file_path: None,
        files_read: Arc::new(Mutex::new(HashSet::new())),
        allow_defer: true,
        taint: TaintTracker::new(),
    };
    let result = tool.execute("id", serde_json::json!({})).await.unwrap();
    assert!(result.content[0].text.contains("DontAsk"));
}

#[test]
fn test_denial_tracker_consecutive_threshold() {
    let tracker = DenialTracker::new();
    assert!(!tracker.record_denial("tool_a")); // 1
    assert!(!tracker.record_denial("tool_a")); // 2
    assert!(tracker.record_denial("tool_a")); // 3 — threshold hit
}

#[test]
fn test_denial_tracker_reset_on_success() {
    let tracker = DenialTracker::new();
    tracker.record_denial("tool_a");
    tracker.record_denial("tool_a");
    tracker.record_success("tool_a");
    // Consecutive resets; next denial should not immediately hit threshold
    assert!(!tracker.record_denial("tool_a")); // only 1 consecutive
}

#[test]
fn test_denial_tracker_session_saturation() {
    let tracker = DenialTracker::new();
    // 20 denials across tools saturates session total
    for i in 0..20 {
        tracker.record_denial(&format!("tool_{i}"));
    }
    assert!(tracker.is_saturated());
}

#[cfg(unix)]
#[test]
fn test_symlinked_allowed_path_resolves_existing_files() {
    // Regression test for the `.vault` bug: an allowlist entry that is
    // itself a symlink (like `.vault` -> Application Support) must admit
    // reads of files that already exist under it, not just files that
    // don't exist yet.
    let tmp = std::env::temp_dir().join(format!("tg-symlink-test-{}", std::process::id()));
    let real_dir = tmp.join("real");
    let link = tmp.join("link");
    std::fs::create_dir_all(&real_dir).unwrap();
    std::fs::write(real_dir.join("note.md"), "hi").unwrap();
    std::os::unix::fs::symlink(&real_dir, &link).unwrap();

    let tool = GovernedTool {
        inner: Box::new(DummyTool),
        allowed_paths: expand_with_canonical_forms(vec![link.clone()]),
        denied_paths: vec![],
        profile: SecurityProfile::Guarded,
        permission_mode: PermissionMode::Default,
        denial_tracker: DenialTracker::new(),
        repeat_call_tracker: RepeatCallTracker::new(),
        denial_notifier: None,
        notified_denials: Arc::new(Mutex::new(HashSet::new())),
        plan_file_path: None,
        files_read: Arc::new(Mutex::new(HashSet::new())),
        allow_defer: true,
        taint: TaintTracker::new(),
    };

    // Existing file reached through the symlink: canonicalize() resolves
    // it to the real dir, which pre-fix failed starts_with against the
    // raw symlink entry.
    assert!(tool.is_path_allowed(link.join("note.md").to_str().unwrap()));
    // Not-yet-existing file: canonicalize() fails and falls back to the
    // raw path, which already matched pre-fix — must keep working.
    assert!(tool.is_path_allowed(link.join("new.md").to_str().unwrap()));

    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn test_denied_path_wins_over_broader_allowed_root() {
    let tmp = std::env::temp_dir().join(format!("tg-denylist-test-{}", std::process::id()));
    let ssh_dir = tmp.join(".ssh");
    std::fs::create_dir_all(&ssh_dir).unwrap();
    let secret = ssh_dir.join("id_ed25519");
    std::fs::write(&secret, "fake-key").unwrap();
    let sibling = tmp.join("notes.md");
    std::fs::write(&sibling, "ok").unwrap();

    let tool = GovernedTool {
        inner: Box::new(DummyTool),
        allowed_paths: expand_with_canonical_forms(vec![tmp.clone()]),
        denied_paths: expand_with_canonical_forms(vec![ssh_dir]),
        profile: SecurityProfile::Guarded,
        permission_mode: PermissionMode::Default,
        denial_tracker: DenialTracker::new(),
        repeat_call_tracker: RepeatCallTracker::new(),
        denial_notifier: None,
        notified_denials: Arc::new(Mutex::new(HashSet::new())),
        plan_file_path: None,
        files_read: Arc::new(Mutex::new(HashSet::new())),
        allow_defer: true,
        taint: TaintTracker::new(),
    };

    assert!(!tool.is_path_allowed(secret.to_str().unwrap()));
    assert!(tool.is_path_allowed(sibling.to_str().unwrap()));

    std::fs::remove_dir_all(&tmp).ok();
}

#[tokio::test]
async fn test_denial_notifier_fires_once_per_distinct_reason() {
    let calls = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let calls_clone = calls.clone();
    let notifier: DenialNotifier = Arc::new(move |tool, reason| {
        calls_clone
            .lock()
            .unwrap()
            .push((tool.to_string(), reason.to_string()));
    });

    let tool = GovernedTool {
        inner: Box::new(DummyTool), // is_read_only() returns false
        allowed_paths: vec![PathBuf::from("/tmp")],
        denied_paths: vec![],
        profile: SecurityProfile::Guarded,
        permission_mode: PermissionMode::DontAsk,
        denial_tracker: DenialTracker::new(),
        repeat_call_tracker: RepeatCallTracker::new(),
        denial_notifier: Some(notifier),
        notified_denials: Arc::new(Mutex::new(HashSet::new())),
        plan_file_path: None,
        files_read: Arc::new(Mutex::new(HashSet::new())),
        allow_defer: true,
        taint: TaintTracker::new(),
    };

    // Same denial twice: only the first should notify.
    tool.execute("id", serde_json::json!({})).await.unwrap();
    tool.execute("id", serde_json::json!({})).await.unwrap();

    let seen = calls.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "dummy");
    assert!(seen[0].1.contains("DontAsk"));
}

// Minimal tool implementation for tests
struct DummyTool;

#[async_trait]
impl AgentTool for DummyTool {
    fn name(&self) -> &str {
        "dummy"
    }
    fn description(&self) -> &str {
        "test tool"
    }
    fn parameters(&self) -> Value {
        Value::Object(Default::default())
    }
    async fn execute(&self, _id: &str, _args: Value) -> Result<ToolResult> {
        Ok(ToolResult {
            content: vec![],
            details: None,
            context_modifier: None,
        })
    }
}

/// A tool that sleeps for `sleep_ms` before returning, declaring
/// `declared_timeout_ms` (or none) via `timeout_ms()`.
struct SlowTool {
    sleep_ms: u64,
    declared_timeout_ms: Option<u64>,
}

#[async_trait]
impl AgentTool for SlowTool {
    fn name(&self) -> &str {
        "slow"
    }
    fn description(&self) -> &str {
        "test tool that sleeps"
    }
    fn parameters(&self) -> Value {
        Value::Object(Default::default())
    }
    fn timeout_ms(&self) -> Option<u64> {
        self.declared_timeout_ms
    }
    async fn execute(&self, _id: &str, _args: Value) -> Result<ToolResult> {
        tokio::time::sleep(std::time::Duration::from_millis(self.sleep_ms)).await;
        Ok(ToolResult {
            content: vec![ToolResultContent {
                r#type: "text".to_string(),
                text: "finished".to_string(),
            }],
            details: None,
            context_modifier: None,
        })
    }
}

fn governed_slow_tool(sleep_ms: u64, declared_timeout_ms: Option<u64>) -> GovernedTool {
    GovernedTool {
        inner: Box::new(SlowTool {
            sleep_ms,
            declared_timeout_ms,
        }),
        allowed_paths: vec![PathBuf::from("/tmp")],
        denied_paths: vec![],
        profile: SecurityProfile::Guarded,
        permission_mode: PermissionMode::BypassPermissions,
        denial_tracker: DenialTracker::new(),
        repeat_call_tracker: RepeatCallTracker::new(),
        denial_notifier: None,
        notified_denials: Arc::new(Mutex::new(HashSet::new())),
        plan_file_path: None,
        files_read: Arc::new(Mutex::new(HashSet::new())),
        allow_defer: true,
        taint: TaintTracker::new(),
    }
}

#[tokio::test]
async fn test_declared_timeout_elapses_returns_structured_result() {
    let tool = governed_slow_tool(200, Some(20));
    let result = tool.execute("id", serde_json::json!({})).await.unwrap();
    assert!(
        result.content[0].text.contains("timed out after 20ms"),
        "expected timeout text, got: {:?}",
        result.content
    );
}

#[tokio::test]
async fn test_no_declared_timeout_runs_to_completion() {
    // No timeout_ms() override — a slow tool with no declared budget must
    // not be affected by the wrapper even when it sleeps past what would
    // otherwise be a short deadline.
    let tool = governed_slow_tool(50, None);
    let result = tool.execute("id", serde_json::json!({})).await.unwrap();
    assert_eq!(result.content[0].text, "finished");
}

#[tokio::test]
async fn test_declared_timeout_not_hit_returns_normal_result() {
    let tool = governed_slow_tool(10, Some(500));
    let result = tool.execute("id", serde_json::json!({})).await.unwrap();
    assert_eq!(result.content[0].text, "finished");
}

// ─── RepeatCallTracker ──────────────────────────────────────

#[test]
fn test_repeat_call_tracker_fires_once_per_threshold() {
    let tracker = RepeatCallTracker::with_config(&[3, 5], &[]);
    let args = serde_json::json!({"path": "/tmp/a"});

    // Calls 1 and 2: no advisory yet.
    assert!(tracker.record_call("read_file", &args).is_none());
    assert!(tracker.record_call("read_file", &args).is_none());
    // Call 3 crosses the first threshold.
    let advisory = tracker.record_call("read_file", &args);
    assert!(advisory.is_some());
    // Call 4: threshold already fired for this run, no repeat advisory.
    assert!(tracker.record_call("read_file", &args).is_none());
    // Call 5 crosses the second threshold.
    let advisory2 = tracker.record_call("read_file", &args);
    assert!(advisory2.is_some());
    assert_ne!(advisory, advisory2);
}

#[test]
fn test_repeat_call_tracker_resets_on_different_args() {
    let tracker = RepeatCallTracker::with_config(&[3], &[]);
    let args_a = serde_json::json!({"path": "/tmp/a"});
    let args_b = serde_json::json!({"path": "/tmp/b"});

    assert!(tracker.record_call("read_file", &args_a).is_none());
    assert!(tracker.record_call("read_file", &args_a).is_none());
    // Different args reset the consecutive run — this is the 1st call
    // for this key, not the 3rd, so no advisory yet.
    assert!(tracker.record_call("read_file", &args_b).is_none());
    assert!(tracker.record_call("read_file", &args_b).is_none());
    // Now the 3rd consecutive call to args_b crosses the threshold.
    assert!(tracker.record_call("read_file", &args_b).is_some());
}

#[test]
fn test_repeat_call_tracker_resets_on_different_tool() {
    let tracker = RepeatCallTracker::with_config(&[3], &[]);
    let args = serde_json::json!({"path": "/tmp/a"});

    assert!(tracker.record_call("read_file", &args).is_none());
    assert!(tracker.record_call("read_file", &args).is_none());
    // Same args, different tool — resets the run.
    assert!(tracker.record_call("write_file", &args).is_none());
}

#[test]
fn test_repeat_call_tracker_excludes_configured_tools() {
    let tracker = RepeatCallTracker::with_config(&[3], &["todo_write"]);
    let args = serde_json::json!({"todos": []});
    for _ in 0..10 {
        assert!(tracker.record_call("todo_write", &args).is_none());
    }
}

#[test]
fn test_repeat_call_tracker_ignores_key_order() {
    let tracker = RepeatCallTracker::with_config(&[3], &[]);
    let args_1 = serde_json::json!({"a": 1, "b": 2});
    let args_2 = serde_json::json!({"b": 2, "a": 1});

    assert!(tracker.record_call("some_tool", &args_1).is_none());
    assert!(tracker.record_call("some_tool", &args_2).is_none());
    // Same canonicalized args despite different key order — 3rd call
    // crosses the threshold.
    assert!(tracker.record_call("some_tool", &args_1).is_some());
}

#[tokio::test]
async fn test_repeat_guard_sets_context_modifier_via_execute() {
    // sleep_ms=0, no declared timeout — runs instantly so the timeout
    // wrapper is a no-op and only the repeat guard is under test.
    let tool = governed_slow_tool(0, None);
    let args = serde_json::json!({"key": "same"});

    let r1 = tool.execute("id", args.clone()).await.unwrap();
    let r2 = tool.execute("id", args.clone()).await.unwrap();
    assert!(r1.context_modifier.is_none());
    assert!(r2.context_modifier.is_none());

    // Third identical call crosses the default first threshold (3).
    let r3 = tool.execute("id", args).await.unwrap();
    assert!(
        r3.context_modifier
            .as_deref()
            .is_some_and(|m| m.contains("same arguments")),
        "expected repeat-call advisory, got: {:?}",
        r3.context_modifier
    );
}
