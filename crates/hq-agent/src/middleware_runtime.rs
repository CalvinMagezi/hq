//! Async middleware helpers for the agent runtime (Cortex-style taps).

use std::time::Instant;

use tracing::debug;

/// Best-effort latency observation around an awaited tool call.
pub async fn tap_tool_elapsed<Fut>(tool_name: &str, fut: Fut) -> Fut::Output
where
    Fut: std::future::Future,
{
    let start = Instant::now();
    let out = fut.await;
    debug!(
        target: "hq_agent::middleware",
        tool = %tool_name,
        elapsed_ms = start.elapsed().as_millis(),
        "tool_execution"
    );
    out
}
