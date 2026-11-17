use super::*;
use hq_core::types::ValueState;

#[test]
fn the_digest_ignores_key_order_and_caller_markers() {
    let a = Call::of(&json!({"params": {"name": "hq_call", "arguments":
        {"tool": "vault_delete", "args": {"path": "a.md", "dry_run": false}}}}));
    let b = Call::of(&json!({"params": {"name": "hq_call", "arguments":
        {"tool": "vault_delete", "args": {"dry_run": false, "path": "a.md", "_hq_connector": "x"}}}}));
    assert_eq!(a.tool, "vault_delete");
    assert_eq!(a.digest, b.digest);
    let c = Call::of(&json!({"params": {"name": "hq_call", "arguments":
        {"tool": "vault_delete", "args": {"path": "b.md", "dry_run": false}}}}));
    assert_ne!(
        a.digest, c.digest,
        "different arguments need a different approval"
    );
    let nested_a =
        hq_core::approval_key::canonical_json(&json!({"x": {"b": 1, "a": [ {"d": 1, "c": 2} ]}}));
    let nested_b =
        hq_core::approval_key::canonical_json(&json!({"x": {"a": [ {"c": 2, "d": 1} ], "b": 1}}));
    assert_eq!(nested_a, nested_b);
}

#[test]
fn a_direct_gateway_call_is_named_by_its_own_tool() {
    let call = Call::of(&json!({"params": {"name": "hq_discover", "arguments": {"query": "x"}}}));
    assert_eq!(call.tool, "hq_discover");
    let empty = Call::of(&json!({"params": {"name": "hq_call", "arguments": {"tool": "t"}}}));
    assert_eq!(empty.args, json!({}));
}

#[test]
fn the_gate_asks_again_after_a_spent_or_expired_request_and_holds_a_live_decline() {
    use ValueState::*;
    let now = chrono::Utc::now();
    let row = |state, engagement: Option<&str>, hours: i64| {
        Some(value_items::LatestEngagement {
            state,
            engagement: engagement.map(str::to_string),
            expires_at: Some(now + chrono::Duration::hours(hours)),
        })
    };
    assert_eq!(Gate::from_latest(None, now), Gate::Ask);
    assert_eq!(
        Gate::from_latest(row(Engaged, Some(value_items::ENGAGEMENT_USED), 1), now),
        Gate::Ask
    );
    assert_eq!(
        Gate::from_latest(row(Dismissed, None, 1), now),
        Gate::Declined
    );
    assert_eq!(
        Gate::from_latest(row(Dismissed, None, -1), now),
        Gate::Ask,
        "a decline ends with its request"
    );
    assert_eq!(
        Gate::from_latest(row(Delivered, None, -1), now),
        Gate::Ask,
        "an unanswered request is asked again"
    );
    for waiting in [Pending, Routed, Delivered] {
        assert_eq!(Gate::from_latest(row(waiting, None, 1), now), Gate::Wait);
    }
    assert_eq!(
        Gate::from_latest(row(Engaged, Some("approved"), 1), now),
        Gate::Wait,
        "approved but not spendable here (unsigned, or taken by a concurrent call)"
    );
}

#[test]
fn only_hq_call_is_gated_as_a_registry_tool() {
    let direct = Call::of(&json!({"params": {"name": "vault_delete", "arguments": {}}}));
    assert!(!direct.via_hq_call);
    let wrapped =
        Call::of(&json!({"params": {"name": "hq_call", "arguments": {"tool": "vault_delete"}}}));
    assert!(wrapped.via_hq_call);
}

fn session_key(id: &str) -> String {
    format!("{SESSION_LOG_PREFIX}{id}")
}

#[test]
fn a_session_call_is_logged_with_a_digest_and_never_its_arguments() {
    let db = hq_db::Database::open_memory().unwrap();
    let call = Call::of(&json!({"params": {"name": "hq_call", "arguments":
        {"tool": "task_get", "args": {"id": "secret-body-text"}}}}));
    log(&db, &session_key("s1"), &call, "ok");
    let rows = db
        .with_conn(|c| store::recent_calls(c, Some(&session_key("s1")), 5))
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].tool, "task_get");
    assert_eq!(rows[0].args_sha256, call.digest);
    assert!(!format!("{:?}", rows[0]).contains("secret-body-text"));
}

#[test]
fn a_session_limit_refuses_recovers_after_the_window_and_is_per_session() {
    let db = hq_db::Database::open_memory().unwrap();
    let call = Call::of(&json!({"params": {"name": "task_get", "arguments": {}}}));
    let now = super::super::now();
    for _ in 0..3 {
        log(&db, &session_key("busy"), &call, "ok");
    }
    assert!(over_limit(&db, &session_key("busy"), 3, now));
    assert!(!over_limit(&db, &session_key("busy"), 4, now));
    assert!(
        !over_limit(&db, &session_key("quiet"), 3, now),
        "another session is unaffected"
    );
    assert!(
        !over_limit(&db, &session_key("busy"), 3, now + RATE_WINDOW_SECS + 5),
        "the limit lifts once the window passes"
    );
}
