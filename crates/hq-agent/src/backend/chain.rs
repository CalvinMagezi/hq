//! [`ProviderChain`] — explicit primary + ordered fallback over [`SessionBackend`]s.
//!
//! The chain is itself a [`SessionBackend`], so callers treat "one backend" and
//! "a primary with fallbacks" identically. It enforces two rules:
//!
//! 1. **Capability-gated selection.** A backend that cannot satisfy the request
//!    (e.g. a buffered CLI when tools are required) is skipped *before* it is
//!    ever called.
//! 2. **Fail over only before any output.** The chain owns the event stream: it
//!    buffers pre-output lifecycle events (`Progress`/`Usage`/`ModelInfo`) and,
//!    on a failoverable error that arrives *before* any committed output, moves
//!    to the next backend (discarding those buffered events). Once any output
//!    event is produced, the chain commits — every later event, including
//!    errors, flows straight through and the fallbacks are never touched.
//!
//! Selection is fully explicit (declared primary + ordered fallbacks). There is
//! no adaptive scoring here — that remains only on the legacy router paths.

use std::sync::Arc;

use async_trait::async_trait;
use tokio_stream::StreamExt;

use super::{
    BackendCapabilities, BackendError, BackendEvent, BackendEventStream, BackendRequest,
    SessionBackend,
};

/// An explicit primary + ordered-fallback chain of backends.
pub struct ProviderChain {
    label: String,
    /// Primary first, then fallbacks in declared order.
    backends: Vec<Arc<dyn SessionBackend>>,
}

impl std::fmt::Debug for ProviderChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderChain")
            .field("label", &self.label)
            .field(
                "backends",
                &self.backends.iter().map(|b| b.name()).collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Outcome of inspecting a backend's stream for a commit-or-fail-over decision.
enum CommitOutcome {
    /// The chain commits to this backend's (possibly buffered-then-chained) stream.
    Committed(BackendEventStream),
    /// The backend failed before any output; try the next backend.
    Failover(BackendError),
}

impl ProviderChain {
    /// Build a chain from an ordered list of backends (primary first).
    pub fn new(label: impl Into<String>, backends: Vec<Arc<dyn SessionBackend>>) -> Self {
        Self {
            label: label.into(),
            backends,
        }
    }

    /// Number of backends in the chain.
    pub fn len(&self) -> usize {
        self.backends.len()
    }

    /// Whether the chain has no backends.
    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }

    /// The ordered backends (primary first).
    pub fn backends(&self) -> &[Arc<dyn SessionBackend>] {
        &self.backends
    }

    /// Inspect a backend's stream, buffering pre-output events. Commit on the
    /// first output event (or clean completion), or signal fail-over on a
    /// failoverable error that arrives before any output.
    async fn commit_or_failover(
        mut stream: BackendEventStream,
        allow_failover: bool,
    ) -> CommitOutcome {
        let mut prelude: Vec<Result<BackendEvent, BackendError>> = Vec::new();
        loop {
            match stream.next().await {
                Some(Ok(event)) => {
                    let is_output = event.is_output();
                    let is_done = event.is_done();
                    prelude.push(Ok(event));
                    if is_output {
                        // Committed: replay buffered prelude, then the live tail.
                        let committed = tokio_stream::iter(prelude).chain(stream);
                        return CommitOutcome::Committed(Box::pin(committed));
                    }
                    if is_done {
                        // Completed with no output (empty turn) — nothing to fail
                        // over to that would help; deliver what we buffered.
                        return CommitOutcome::Committed(Box::pin(tokio_stream::iter(prelude)));
                    }
                    // Otherwise a pre-output lifecycle event: keep buffering.
                }
                Some(Err(err)) => {
                    if err.is_failoverable() && allow_failover {
                        // Discard this backend's buffered prelude and try the next.
                        return CommitOutcome::Failover(err);
                    }
                    // Non-failoverable, or nothing left to fail over to: deliver
                    // the buffered prelude followed by the terminal error.
                    prelude.push(Err(err));
                    return CommitOutcome::Committed(Box::pin(tokio_stream::iter(prelude)));
                }
                None => {
                    // Stream ended without an explicit Done — deliver what we have.
                    return CommitOutcome::Committed(Box::pin(tokio_stream::iter(prelude)));
                }
            }
        }
    }
}

#[async_trait]
impl SessionBackend for ProviderChain {
    fn name(&self) -> &str {
        &self.label
    }

    fn capabilities(&self) -> BackendCapabilities {
        self.backends
            .iter()
            .map(|b| b.capabilities())
            .reduce(BackendCapabilities::union)
            // An empty chain advertises the buffered-CLI floor (least capable).
            .unwrap_or_else(BackendCapabilities::buffered_cli)
    }

    fn root_capabilities(&self) -> BackendCapabilities {
        // Explicit root selection: the primary (first) backend decides how the
        // session drives this chain — not the capability union. A CLI primary
        // therefore yields tool-free, buffered turns even with an API fallback.
        self.backends
            .first()
            .map(|b| b.root_capabilities())
            .unwrap_or_else(BackendCapabilities::buffered_cli)
    }

    fn aborts_on_drop(&self) -> bool {
        // The chain owns whichever backend committed; it is drop-safe iff every
        // constituent backend is.
        self.backends.iter().all(|b| b.aborts_on_drop())
    }

    fn utility_provider(&self) -> Option<std::sync::Arc<dyn hq_llm::provider::LlmProvider>> {
        // Compaction and background calls fail over across every API backend,
        // like turns do. A pure-CLI chain returns `None`; the caller then routes
        // compaction through the backend itself.
        let mut providers: Vec<_> = self
            .backends
            .iter()
            .filter_map(|b| Some((b.name().to_string(), b.utility_provider()?)))
            .collect();
        if providers.len() <= 1 {
            return providers.pop().map(|(_, p)| p);
        }
        Some(Arc::new(
            hq_llm::backend_chain::ChainProvider::from_providers(providers),
        ))
    }

    fn owns_failover(&self) -> bool {
        // The chain fails over across its backends before any output, so the
        // session must not wrap it in additional legacy retry/backoff.
        true
    }

    async fn start(&self, request: &BackendRequest) -> Result<BackendEventStream, BackendError> {
        // The only eager failure: an empty chain has nothing to poll. Everything
        // else — capability gating, per-backend startup, pre-output failover, and
        // the terminal "no backend served this" error — happens lazily inside the
        // returned stream (below), so `start` returns immediately and never blocks
        // until first output. This lets `AgentSession` enter its consume loop at
        // once and race every poll against cancellation; dropping the returned
        // stream drops whichever backend stream is in scope, aborting in-flight
        // CLI work (`kill_on_drop`).
        if self.backends.is_empty() {
            return Err(BackendError::NoBackendAvailable(format!(
                "chain '{}' has no backends",
                self.label
            )));
        }

        let backends = self.backends.clone();
        let label = self.label.clone();
        let request = request.clone();

        let stream = async_stream::stream! {
            let total = backends.len();
            let mut last_err: Option<BackendError> = None;

            for (idx, backend) in backends.iter().enumerate() {
                let is_last = idx + 1 == total;

                // Capability gate: skip backends that cannot satisfy the request.
                if !backend.capabilities().satisfies(&request) {
                    last_err = Some(BackendError::Incompatible(format!(
                        "backend '{}' cannot satisfy request (tools/streaming requirements)",
                        backend.name()
                    )));
                    continue;
                }

                match backend.start(&request).await {
                    Ok(stream) => match Self::commit_or_failover(stream, !is_last).await {
                        CommitOutcome::Committed(mut committed) => {
                            // Announce which backend actually committed (may be a
                            // fallback), before any committed output, so the session
                            // tags backend-origin events with the selected identity.
                            yield Ok(BackendEvent::BackendSelected(backend.name().to_string()));
                            // Forward the committed tail. `committed` owns the
                            // backend's live stream and lives in this generator's
                            // scope, so dropping the returned stream cancels it.
                            while let Some(event) = committed.next().await {
                                yield event;
                            }
                            return;
                        }
                        CommitOutcome::Failover(err) => {
                            tracing::warn!(
                                chain = %label,
                                backend = %backend.name(),
                                error = %err,
                                "provider chain: backend errored before any output, failing over"
                            );
                            // Surfaces to `AgentSession` as `TurnOutput.is_fallback`
                            // (session/loop.rs), which the relay layer already
                            // consumes to annotate the reply and (via the fallback
                            // notification) ping Telegram — this was the one
                            // missing emit in an otherwise fully-wired pipeline.
                            yield Ok(BackendEvent::Failover(backend.name().to_string()));
                            last_err = Some(err);
                            continue;
                        }
                    },
                    Err(err) => {
                        if err.is_failoverable() && !is_last {
                            tracing::warn!(
                                chain = %label,
                                backend = %backend.name(),
                                error = %err,
                                "provider chain: backend failed to start, failing over"
                            );
                            yield Ok(BackendEvent::Failover(backend.name().to_string()));
                            last_err = Some(err);
                            continue;
                        }
                        yield Err(err);
                        return;
                    }
                }
            }

            yield Err(last_err.unwrap_or_else(|| {
                BackendError::NoBackendAvailable(format!("chain '{label}' has no backends"))
            }));
        };

        Ok(Box::pin(stream))
    }
}

#[cfg(test)]
#[path = "chain_tests.rs"]
mod tests;
