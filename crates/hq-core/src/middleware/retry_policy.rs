//! Shared retry and backoff helpers for LLM and stringly-typed failures.

use std::time::Duration;

use rand::Rng;

use super::RuntimeErrorKind;

/// Heuristic for unstructured errors when [`hq_llm::LlmError`] downcast fails.
#[inline]
pub fn unstructured_llm_error_looks_transient(err_str: &str) -> bool {
    let s = err_str.to_ascii_lowercase();
    s.contains("429")
        || s.contains("rate limit")
        || s.contains("overloaded")
        || s.contains("502")
        || s.contains("503")
        || s.contains("529")
        || s.contains("connection reset")
        || s.contains("timed out")
        || s.contains("timeout")
}

/// Whether an outer harness/session retry is worthwhile for relay-style wrappers.
#[inline]
pub fn harness_attempt_may_retry(err_msg: &str) -> bool {
    unstructured_llm_error_looks_transient(err_msg)
        || err_msg.to_ascii_lowercase().contains("busy")
        || err_msg.to_ascii_lowercase().contains("temporarily")
}

/// Exponential backoff: `base * 2^attempt`, optionally floored by `retry_after`.
#[inline]
pub fn exponential_delay_floor(
    base: Duration,
    attempt: u32,
    retry_after_floor: Option<Duration>,
) -> Duration {
    let factor = 2u32.saturating_pow(attempt);
    let mut d = base.checked_mul(factor).unwrap_or(Duration::MAX);
    if let Some(floor) = retry_after_floor {
        d = d.max(floor);
    }
    d
}

/// Jitter multiplier in \[low, high\] applied to `delay`.
#[inline]
pub fn jittered_duration(delay: Duration, rng: &mut impl Rng, low: f64, high: f64) -> Duration {
    let j = rng.gen_range(low..high);
    delay.mul_f64(j)
}

/// Classify opaque errors into [`RuntimeErrorKind`] using substring rules only.
#[inline]
pub fn runtime_kind_from_error_message(message: &str) -> RuntimeErrorKind {
    let m = message.to_ascii_lowercase();
    if m.contains("401") || m.contains("unauthorized") || m.contains("invalid api key") {
        return RuntimeErrorKind::AuthN;
    }
    if m.contains("403") || m.contains("forbidden") {
        return RuntimeErrorKind::AuthZ;
    }
    if m.contains("timed out") || m.contains("timeout") || m.contains("deadline") {
        return RuntimeErrorKind::Timeout;
    }
    if m.contains("validation") || m.contains("invalid") || m.contains("malformed") {
        return RuntimeErrorKind::Validation;
    }
    if m.contains("budget")
        || m.contains("capacity")
        || m.contains("429")
        || m.contains("rate limit")
    {
        return RuntimeErrorKind::Capacity;
    }
    if unstructured_llm_error_looks_transient(&m) {
        return RuntimeErrorKind::Transient;
    }
    RuntimeErrorKind::Permanent
}
