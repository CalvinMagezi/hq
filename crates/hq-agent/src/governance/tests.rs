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

struct SearchStub(&'static str);

#[async_trait::async_trait]
impl crate::tools::AgentTool for SearchStub {
    fn name(&self) -> &str {
        self.0
    }
    fn description(&self) -> &str {
        "stub"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({})
    }
    async fn execute(&self, _id: &str, _args: Value) -> Result<ToolResult> {
        panic!("the guardian let a search leave the confined folder")
    }
}

#[tokio::test]
async fn a_confined_search_cannot_leave_its_folder() {
    let root = "/tmp/hq-confined-checkout";
    let cases = [
        ("grep", serde_json::json!({"pattern": "token"})),
        ("find_files", serde_json::json!({"pattern": "*.pem"})),
        ("list_dir", serde_json::json!({})),
        (
            "find_files",
            serde_json::json!({"pattern": "/etc/*", "path": root}),
        ),
        (
            "find_files",
            serde_json::json!({"pattern": "../../etc/*", "path": root}),
        ),
    ];
    for (tool, args) in cases {
        let guardian = ToolGuardian::confined(
            vec![PathBuf::from(root)],
            SecurityProfile::Guarded,
            PermissionMode::BypassPermissions,
        );
        let registry =
            guardian.build_registry(vec![Box::new(SearchStub(tool))], LiveUserTurn::unattended());
        let wrapped = registry.get(tool).expect("the tool is registered");
        let result = wrapped
            .execute("1", args)
            .await
            .expect("a denial, not an error");
        assert!(format!("{result:?}").contains("Access denied"), "{tool}");
    }
}

#[test]
#[should_panic(expected = "requires at least one allowed path")]
fn test_guardian_rejects_empty_paths() {
    ToolGuardian::new(vec![], SecurityProfile::Guarded, PermissionMode::Default);
}

#[test]
fn a_confined_guardian_does_not_add_home() {
    let home = dirs::home_dir().expect("a home directory");
    let folder = PathBuf::from("/tmp/hq-confined-checkout");
    let open = ToolGuardian::new(
        vec![folder.clone()],
        SecurityProfile::Guarded,
        PermissionMode::Default,
    );
    assert!(
        open.allowed_paths.contains(&home),
        "the usual guardian includes it"
    );
    let confined = ToolGuardian::confined(
        vec![folder],
        SecurityProfile::Guarded,
        PermissionMode::Default,
    );
    assert!(!confined.allowed_paths.contains(&home));
}

#[test]
#[should_panic(expected = "requires at least one allowed path")]
fn a_confined_guardian_refuses_an_empty_list() {
    ToolGuardian::confined(vec![], SecurityProfile::Guarded, PermissionMode::Default);
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
        vault_root: None,
        permission_mode: PermissionMode::Default,
        denial_tracker: DenialTracker::new(),
        repeat_call_tracker: RepeatCallTracker::new(),
        denial_notifier: None,
        owner_approval: None,
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
        vault_root: None,
        permission_mode: PermissionMode::BypassPermissions,
        denial_tracker: DenialTracker::new(),
        repeat_call_tracker: RepeatCallTracker::new(),
        denial_notifier: None,
        owner_approval: None,
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
        vault_root: None,
        permission_mode: PermissionMode::DontAsk,
        denial_tracker: DenialTracker::new(),
        repeat_call_tracker: RepeatCallTracker::new(),
        denial_notifier: None,
        owner_approval: None,
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
        vault_root: None,
        permission_mode: PermissionMode::Default,
        denial_tracker: DenialTracker::new(),
        repeat_call_tracker: RepeatCallTracker::new(),
        denial_notifier: None,
        owner_approval: None,
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
        vault_root: None,
        permission_mode: PermissionMode::Default,
        denial_tracker: DenialTracker::new(),
        repeat_call_tracker: RepeatCallTracker::new(),
        denial_notifier: None,
        owner_approval: None,
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
        vault_root: None,
        permission_mode: PermissionMode::DontAsk,
        denial_tracker: DenialTracker::new(),
        repeat_call_tracker: RepeatCallTracker::new(),
        denial_notifier: Some(notifier),
        owner_approval: None,
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
        vault_root: None,
        permission_mode: PermissionMode::BypassPermissions,
        denial_tracker: DenialTracker::new(),
        repeat_call_tracker: RepeatCallTracker::new(),
        denial_notifier: None,
        owner_approval: None,
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

struct BashStub;

#[async_trait]
impl AgentTool for BashStub {
    fn name(&self) -> &str {
        "bash"
    }
    fn description(&self) -> &str {
        "stub"
    }
    fn parameters(&self) -> Value {
        Value::Object(Default::default())
    }
    async fn execute(&self, _id: &str, _args: Value) -> Result<ToolResult> {
        Ok(crate::coding::text_result("ran"))
    }
}

async fn run_bash(mode: PermissionMode, asks: bool) -> String {
    let mut guardian =
        ToolGuardian::new(vec![std::env::temp_dir()], SecurityProfile::Guarded, mode);
    if asks {
        let db = Arc::new(hq_db::Database::open_memory().unwrap());
        guardian.set_owner_approval(Some(OwnerApproval::new(db, "web:t1")));
    }
    let tool = guardian.govern(Box::new(BashStub));
    let result = tool
        .execute("1", serde_json::json!({"command": "make"}))
        .await
        .unwrap();
    result.content[0].text.clone()
}

#[tokio::test]
async fn default_mode_waits_for_the_owner_only_where_a_chat_can_ask() {
    assert!(
        run_bash(PermissionMode::Default, true)
            .await
            .contains("Waiting for the owner")
    );
    assert_eq!(run_bash(PermissionMode::Default, false).await, "ran");
    assert_eq!(run_bash(PermissionMode::AcceptEdits, true).await, "ran");
}

/// Changing or deleting a calendar event asks the owner every time, in a clean chat too, in every
/// mode but bypass, and does not run where nobody can be asked.
#[tokio::test]
async fn a_calendar_change_always_asks_the_owner_and_never_runs_unasked() {
    let args = serde_json::json!({"open_link": "hq://calendar/event/1?begin=1&end=2",
        "title": "Dentist", "start_local": "2026-02-10T15:00"});
    for tool in ["calendar_delete_event", "calendar_update_event"] {
        for mode in [PermissionMode::Default, PermissionMode::AcceptEdits] {
            let mut guardian = ToolGuardian::new(
                vec![std::env::temp_dir()],
                SecurityProfile::Guarded,
                mode.clone(),
            );
            let db = Arc::new(hq_db::Database::open_memory().unwrap());
            guardian.set_owner_approval(Some(OwnerApproval::new(db, "web:t1")));
            let governed = guardian.govern(Box::new(NamedTool(tool)));
            let out = governed.execute("1", args.clone()).await.unwrap();
            let text = out
                .content
                .first()
                .map(|c| c.text.clone())
                .unwrap_or_default();
            assert!(
                text.contains("Waiting for the owner"),
                "{tool} {mode:?}: {text}"
            );
        }
        for mode in [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::BypassPermissions,
        ] {
            let guardian = ToolGuardian::new(
                vec![std::env::temp_dir()],
                SecurityProfile::Guarded,
                mode.clone(),
            );
            let governed = guardian.govern(Box::new(NamedTool(tool)));
            let out = governed.execute("1", args.clone()).await.unwrap();
            let text = out
                .content
                .first()
                .map(|c| c.text.clone())
                .unwrap_or_default();
            assert!(
                text.contains("nobody is here to approve"),
                "{tool} {mode:?}: {text}"
            );
        }
    }
}

/// A do-nothing tool under a real tool's name, so governance applies that tool's rules.
struct NamedTool(&'static str);

#[async_trait]
impl AgentTool for NamedTool {
    fn name(&self) -> &str {
        self.0
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

fn governed_named(name: &'static str, allowed: PathBuf, mode: PermissionMode) -> GovernedTool {
    GovernedTool {
        inner: Box::new(NamedTool(name)),
        allowed_paths: vec![allowed],
        denied_paths: vec![],
        vault_root: None,
        permission_mode: mode,
        denial_tracker: DenialTracker::new(),
        repeat_call_tracker: RepeatCallTracker::new(),
        denial_notifier: None,
        owner_approval: None,
        notified_denials: Arc::new(Mutex::new(HashSet::new())),
        plan_file_path: None,
        files_read: Arc::new(Mutex::new(HashSet::new())),
        allow_defer: true,
        taint: TaintTracker::new(),
    }
}

fn denied(result: &ToolResult) -> bool {
    result
        .content
        .first()
        .is_some_and(|c| c.text.contains("outside allowed directories"))
}

/// `edit_file` was refused outside the workspace while `file_edit_batch`, `rollback_file` and
/// `grep` reached the same files, because the allowlist only knew the first tool's argument.
#[tokio::test]
async fn every_file_tool_is_held_to_the_allowed_directories() {
    let work = tempfile::tempdir().unwrap();
    let root = work.path().canonicalize().unwrap();
    let inside = root.join("notes.md");
    let outside = "/etc/hostname";
    for mode in [
        PermissionMode::AcceptEdits,
        PermissionMode::BypassPermissions,
    ] {
        let batch = governed_named("file_edit_batch", root.clone(), mode.clone());
        let edit = |path: &str| {
            serde_json::json!({"edits": [
                {"file_path": inside.to_str().unwrap(), "old_string": "a", "new_string": "b"},
                {"file_path": path, "old_string": "a", "new_string": "b"},
            ]})
        };
        assert!(
            denied(&batch.execute("t", edit(outside)).await.unwrap()),
            "{mode:?}"
        );
        assert!(!denied(
            &batch
                .execute("t", edit(inside.to_str().unwrap()))
                .await
                .unwrap()
        ));

        let rollback = governed_named("rollback_file", root.clone(), mode.clone());
        let args = serde_json::json!({"file_path": outside});
        assert!(
            denied(&rollback.execute("t", args).await.unwrap()),
            "{mode:?}"
        );

        let grep = governed_named("grep", root.clone(), mode.clone());
        let args = serde_json::json!({"pattern": "x", "path": "/etc"});
        assert!(denied(&grep.execute("t", args).await.unwrap()), "{mode:?}");
        let args = serde_json::json!({"pattern": "x", "path": root.to_str().unwrap()});
        assert!(!denied(&grep.execute("t", args).await.unwrap()));
    }
}

fn governed_in_vault(name: &'static str, vault: &std::path::Path) -> GovernedTool {
    let mut tool = governed_named(name, vault.to_path_buf(), PermissionMode::AcceptEdits);
    tool.vault_root = Some(vault.canonicalize().unwrap());
    tool
}

fn refused(result: &ToolResult) -> bool {
    result
        .content
        .first()
        .is_some_and(|c| c.text.starts_with("Access denied"))
}

/// The database every credential is checked against, and the keys beside it, are out of reach of
/// every tool that names a file; the folders HQ loads are closed to the file-writing tools.
#[tokio::test]
async fn the_vaults_machinery_is_closed_to_raw_file_tools() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().canonicalize().unwrap();
    for d in ["_data", "_system", "Agents", "skills/x", "Notebooks"] {
        std::fs::create_dir_all(vault.join(d)).unwrap();
    }
    std::fs::write(vault.join("_data/vault.db"), "db").unwrap();
    let v = |rel: &str| vault.join(rel).to_string_lossy().into_owned();

    for tool in ["read_file", "write_file", "convert_to_markdown"] {
        let t = governed_in_vault(tool, &vault);
        for args in [
            serde_json::json!({"file_path": v("_data/vault.db")}),
            serde_json::json!({"input_path": v("_data/web_push_vapid.p8")}),
            serde_json::json!({"file_path": v("Notebooks/../_data/vault.db")}),
        ] {
            assert!(
                refused(&t.execute("t", args.clone()).await.unwrap()),
                "{tool} {args}"
            );
        }
    }

    let write = governed_in_vault("write_file", &vault);
    for rel in [
        "_system/SOUL.md",
        "SKILLS/x/SKILL.md",
        "skills/new/SKILL.md",
        "agents/coder.md",
        "_commands/go.md",
        "_mailboxes/relay/m.md",
    ] {
        let args = serde_json::json!({"file_path": v(rel), "content": "x"});
        assert!(refused(&write.execute("t", args).await.unwrap()), "{rel}");
    }
    let batch = governed_in_vault("file_edit_batch", &vault);
    let args = serde_json::json!({"edits": [{"file_path": v("Agents/a.md"), "old_string": "a", "new_string": "b"}]});
    assert!(refused(&batch.execute("t", args).await.unwrap()));

    // Reading a skill and writing an ordinary note stay open.
    let read = governed_in_vault("read_file", &vault);
    let args = serde_json::json!({"file_path": v("skills/x/SKILL.md")});
    assert!(!refused(&read.execute("t", args).await.unwrap()));
    let args = serde_json::json!({"file_path": v("Notebooks/n.md"), "content": "x"});
    assert!(!refused(&write.execute("t", args).await.unwrap()));
}

/// A file that does not exist yet used to be checked by its raw spelling, so `..` and a linked
/// parent reached a denied root.
#[cfg(unix)]
#[test]
fn a_path_that_does_not_exist_yet_is_checked_where_it_really_lands() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("secret")).unwrap();
    std::os::unix::fs::symlink(root.join("secret"), root.join("link")).unwrap();
    let mut tool = governed_named("write_file", root.clone(), PermissionMode::AcceptEdits);
    tool.denied_paths = vec![root.join("secret")];
    for raw in [
        root.join("missing/../secret/new.txt"),
        root.join("link/new.txt"),
        root.join("link/deeper/new.txt"),
    ] {
        assert!(
            !tool.is_path_allowed(raw.to_str().unwrap()),
            "{}",
            raw.display()
        );
    }
    assert!(tool.is_path_allowed(root.join("missing/ok.txt").to_str().unwrap()));
    assert!(tool.is_path_allowed(root.join("a/../b/ok.txt").to_str().unwrap()));
}
