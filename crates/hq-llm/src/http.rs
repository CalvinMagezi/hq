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
/// - Connection pooling enabled (reqwest default), except in this crate's
///   unit tests: each `#[tokio::test]` has its own runtime, and a pooled
///   connection left by a test whose runtime has shut down fails the next
///   request with a network error.
/// - TLS session reuse enabled (reqwest default)
pub static SHARED_HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    builder()
        .build()
        .expect("failed to build global HTTP client")
});

fn builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .pool_max_idle_per_host(if cfg!(test) { 0 } else { 10 })
}

/// A client with the shared settings whose every request goes to the unix socket at `path`, for
/// a backend reached through a bridge forward that listens on a socket. It cannot share the pool
/// of [`SHARED_HTTP_CLIENT`], since the socket replaces the network for the whole client.
#[cfg(unix)]
pub fn unix_socket_client(path: &std::path::Path) -> reqwest::Result<reqwest::Client> {
    builder().unix_socket(path).build()
}
