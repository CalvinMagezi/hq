//! Composable middleware kernel (Cortex-inspired).
//!
//! Provides typed [`RuntimeErrorKind`], HTTP auth helpers and retry backoff utilities.
//! Async wrappers belong at call sites (`hq-agent`, `hq-web`, …).

mod daemon_intervals;
mod http_auth;
mod retry_policy;
mod runtime_error;

pub use daemon_intervals::default_timeout_for_interval;
pub use http_auth::resolve_api_key_candidate;
pub use retry_policy::{
    exponential_delay_floor, harness_attempt_may_retry, jittered_duration,
    runtime_kind_from_error_message, unstructured_llm_error_looks_transient,
};
pub use runtime_error::RuntimeErrorKind;
