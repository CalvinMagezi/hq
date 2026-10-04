//! Conformance tests for the Cortex-style middleware kernel.

use hq_core::middleware::{
    RuntimeErrorKind, default_timeout_for_interval, exponential_delay_floor,
    harness_attempt_may_retry, resolve_api_key_candidate, runtime_kind_from_error_message,
    unstructured_llm_error_looks_transient,
};
use std::time::Duration;

#[test]
fn unstructured_detects_transient_markers() {
    assert!(unstructured_llm_error_looks_transient("HTTP 429"));
    assert!(unstructured_llm_error_looks_transient("503 unavailable"));
    assert!(!unstructured_llm_error_looks_transient("unknown tool: foo"));
}

#[test]
fn runtime_kind_auth_and_capacity() {
    assert_eq!(
        runtime_kind_from_error_message("401 unauthorized"),
        RuntimeErrorKind::AuthN
    );
    assert_eq!(
        runtime_kind_from_error_message("429 too many requests"),
        RuntimeErrorKind::Capacity
    );
}

#[test]
fn exponential_delay_respects_floor() {
    let base = Duration::from_millis(100);
    let floor = Duration::from_secs(2);
    let d = exponential_delay_floor(base, 0, Some(floor));
    assert_eq!(d, floor);
}

#[test]
fn harness_retry_heuristic() {
    assert!(harness_attempt_may_retry("connection reset"));
    assert!(!harness_attempt_may_retry("syntax error in prompt"));
}

#[test]
fn api_key_candidate_resolution() {
    assert_eq!(resolve_api_key_candidate("a", "b"), "a");
    assert_eq!(resolve_api_key_candidate("", "b"), "b");
}

#[test]
fn daemon_timeout_tiers_match_documentation() {
    assert_eq!(
        default_timeout_for_interval(&Duration::from_secs(60)),
        Duration::from_secs(30)
    );
    assert_eq!(
        default_timeout_for_interval(&Duration::from_secs(300)),
        Duration::from_secs(120)
    );
}
