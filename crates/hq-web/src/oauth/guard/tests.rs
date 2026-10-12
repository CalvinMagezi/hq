use super::*;

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
    let nested_a = canonical(&json!({"x": {"b": 1, "a": [ {"d": 1, "c": 2} ]}}));
    let nested_b = canonical(&json!({"x": {"a": [ {"c": 2, "d": 1} ], "b": 1}}));
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
