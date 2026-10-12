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
fn the_gate_asks_again_after_a_spent_or_expired_approval_and_holds_a_decline() {
    use ValueState::*;
    assert_eq!(Gate::from_latest(None), Gate::Ask);
    let used = Some(value_items::ENGAGEMENT_USED.to_string());
    assert_eq!(Gate::from_latest(Some((Engaged, used))), Gate::Ask);
    assert_eq!(Gate::from_latest(Some((Expired, None))), Gate::Ask);
    assert_eq!(Gate::from_latest(Some((Dismissed, None))), Gate::Declined);
    for waiting in [Pending, Routed, Delivered] {
        assert_eq!(Gate::from_latest(Some((waiting, None))), Gate::Wait);
    }
    assert_eq!(
        Gate::from_latest(Some((Engaged, Some("approved".into())))),
        Gate::Wait,
        "approved but not spendable here (unsigned, or taken by a concurrent call)"
    );
}
