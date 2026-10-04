//! Cross-cutting error classification (Cortex-aligned).
//!
//! Maps cleanly to protocol and retry layers: transients retry, auth terminates,
//! capacity maps to rate limits and budget exhaustion.

/// Stable error kind for middleware and transport mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeErrorKind {
    /// Retryable provider/network/server pressure.
    Transient,
    /// Non-retryable application/client mistakes.
    Permanent,
    /// Deadline exceeded (caller or callee).
    Timeout,
    /// Authentication failure (invalid credentials).
    AuthN,
    /// Authorization failure (forbidden action).
    AuthZ,
    /// Schema / validation failures.
    Validation,
    /// Budget, rate limits, bulkheads, context caps.
    Capacity,
    /// Model/tool orchestration mistakes (bad tool name, loops); recovery differs from Transient.
    Cognitive,
}

impl RuntimeErrorKind {
    #[inline]
    pub fn is_retryable(self) -> bool {
        matches!(self, Self::Transient)
    }
}
