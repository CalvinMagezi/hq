//! Global shared HTTP client for all LLM providers.
//!
//! A single `reqwest::Client` preserves TCP connection pools and TLS sessions
//! across all providers, eliminating per-request handshake overhead.

use std::sync::LazyLock;
use std::time::Duration;

/// Global HTTP client shared by all LLM providers.
///
/// Configuration:
/// - 10s connect timeout (fail fast on unreachable hosts)
/// - 300s total timeout — local Ollama inference can be slow (large models,
///   first-token latency). Cloud providers are typically fast; the generous
///   timeout here is dominated by the local case.
/// - Connection pooling enabled (reqwest default)
/// - TLS session reuse enabled (reqwest default)
pub static SHARED_HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .pool_max_idle_per_host(10)
        .build()
        .expect("failed to build global HTTP client")
});
