//! [`AgentService`] execution modes: single, parallel, race, and graph.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::warn;

use super::service::{AgentService, RunCtx};
use super::types::{ChildOutcome, ChildPlan, ChildRequest, ChildStatus};

impl AgentService {
    // ─── Mode: Single ──────────────────────────────────────────

    pub(super) async fn run_single(&self, plan: ChildPlan, run_ctx: RunCtx) -> Vec<ChildOutcome> {
        let Some(req) = plan.children.into_iter().next() else {
            return Vec::new();
        };
        let cancel = Arc::new(AtomicBool::new(false));
        vec![self.run_one_child(req, run_ctx, cancel).await]
    }

    // ─── Mode: Parallel ────────────────────────────────────────

    pub(super) async fn run_parallel(&self, plan: ChildPlan, run_ctx: RunCtx) -> Vec<ChildOutcome> {
        let order: Vec<String> = plan.children.iter().map(|c| c.id.clone()).collect();
        let semaphore = Arc::new(Semaphore::new(plan.max_concurrent.max(1)));
        let mut set: JoinSet<ChildOutcome> = JoinSet::new();

        for req in plan.children {
            let svc = self.clone();
            let run_ctx = run_ctx.clone();
            let semaphore = semaphore.clone();
            let id = req.id.clone();
            set.spawn(async move {
                // Acquire a slot; a closed semaphore should never happen here.
                let _permit = match semaphore.acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => {
                        return ChildOutcome::rejected(&id, "scheduler semaphore closed");
                    }
                };
                let cancel = Arc::new(AtomicBool::new(false));
                // Isolated: one child's panic/failure never cancels siblings.
                svc.run_one_child(req, run_ctx, cancel).await
            });
        }

        let mut by_id: HashMap<String, ChildOutcome> = HashMap::new();
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok(outcome) => {
                    by_id.insert(outcome.id.clone(), outcome);
                }
                Err(e) => {
                    warn!(error = %e, "agent-service: parallel child task panicked");
                    // The originating request id is lost with a bare JoinError,
                    // but every remaining id in `order` without a matching
                    // outcome is filled in below so the panic is surfaced as a
                    // failure rather than silently vanishing from the result set.
                }
            }
        }
        for id in &order {
            by_id
                .entry(id.clone())
                .or_insert_with(|| ChildOutcome::panicked(id, "child task panicked or was lost"));
        }
        order_outcomes(order, by_id)
    }

    // ─── Mode: Race ────────────────────────────────────────────

    pub(super) async fn run_race(&self, plan: ChildPlan, run_ctx: RunCtx) -> Vec<ChildOutcome> {
        let order: Vec<String> = plan.children.iter().map(|c| c.id.clone()).collect();
        let mut cancels: HashMap<String, Arc<AtomicBool>> = HashMap::new();
        let mut set: JoinSet<ChildOutcome> = JoinSet::new();

        for req in plan.children {
            let cancel = Arc::new(AtomicBool::new(false));
            cancels.insert(req.id.clone(), cancel.clone());
            let svc = self.clone();
            let run_ctx = run_ctx.clone();
            set.spawn(async move { svc.run_one_child(req, run_ctx, cancel).await });
        }

        let mut finished: HashMap<String, ChildOutcome> = HashMap::new();
        let mut winner: Option<String> = None;

        while let Some(joined) = set.join_next().await {
            match joined {
                Ok(outcome) => {
                    let success = outcome.status.is_success();
                    let id = outcome.id.clone();
                    finished.insert(id.clone(), outcome);
                    if success {
                        winner = Some(id);
                        break;
                    }
                    // A losing failure: keep waiting for another to succeed.
                }
                Err(e) => {
                    warn!(error = %e, "agent-service: race child task panicked");
                }
            }
        }

        if winner.is_some() {
            // Cancel every child that has not yet produced an outcome. Set the
            // cooperative cancel flag *and* abort the task so a mid-flight
            // backend stream is actually dropped, not merely ignored.
            for (id, cancel) in &cancels {
                if !finished.contains_key(id) {
                    cancel.store(true, Ordering::SeqCst);
                }
            }
            set.abort_all();
            // Drain aborted tasks (results discarded).
            while set.join_next().await.is_some() {}
        }

        // Any child that never produced an outcome either lost the race to a
        // winner, or panicked without ever finishing (no winner at all) —
        // report each case distinctly rather than mislabeling a panic as a
        // race loss.
        for id in &order {
            if finished.contains_key(id) {
                continue;
            }
            let outcome = if winner.is_some() {
                ChildOutcome {
                    id: id.clone(),
                    status: ChildStatus::Cancelled,
                    output: "cancelled: another child won the race".to_string(),
                    resolved_backend: String::new(),
                    fallback_used: false,
                    duration_ms: 0,
                    error: None,
                    run_id: None,
                    effective_model: None,
                    evidence_check: None,
                    run: None,
                }
            } else {
                ChildOutcome::panicked(id, "child task panicked or was lost")
            };
            finished.insert(id.clone(), outcome);
        }
        order_outcomes(order, finished)
    }

    // ─── Mode: Graph ───────────────────────────────────────────

    pub(super) async fn run_graph(&self, plan: ChildPlan, run_ctx: RunCtx) -> Vec<ChildOutcome> {
        let specs = plan.children;
        let order: Vec<String> = specs.iter().map(|c| c.id.clone()).collect();
        let by_id: HashMap<String, ChildRequest> =
            specs.iter().map(|s| (s.id.clone(), s.clone())).collect();
        let total = specs.len();
        let max_concurrent = plan.max_concurrent.max(1);

        let mut completed: HashSet<String> = HashSet::new();
        let mut failed: HashSet<String> = HashSet::new();
        let mut outcomes: HashMap<String, ChildOutcome> = HashMap::new();

        loop {
            // Ready = not yet run, every dependency resolved (completed or failed).
            let ready: Vec<&ChildRequest> = specs
                .iter()
                .filter(|s| !outcomes.contains_key(&s.id))
                .filter(|s| {
                    s.depends_on
                        .iter()
                        .all(|d| completed.contains(d) || failed.contains(d))
                })
                .collect();

            if ready.is_empty() {
                if outcomes.len() < total {
                    // Deadlock: remaining tasks depend on missing/cyclic ids.
                    for s in &specs {
                        if !outcomes.contains_key(&s.id) {
                            let missing: Vec<&str> = s
                                .depends_on
                                .iter()
                                .filter(|d| {
                                    !completed.contains(*d)
                                        && !failed.contains(*d)
                                        && !by_id.contains_key(*d)
                                })
                                .map(|d| d.as_str())
                                .collect();
                            let reason = if missing.is_empty() {
                                "blocked by an unresolvable dependency cycle".to_string()
                            } else {
                                format!("blocked by missing dependency: {}", missing.join(", "))
                            };
                            outcomes.insert(s.id.clone(), ChildOutcome::blocked(&s.id, reason));
                            failed.insert(s.id.clone());
                        }
                    }
                }
                break;
            }

            // A failed dependency blocks its dependents with a clear reason —
            // never a silent skip.
            let mut batch: Vec<ChildRequest> = Vec::new();
            for s in ready.into_iter().take(max_concurrent) {
                let failed_deps: Vec<&str> = s
                    .depends_on
                    .iter()
                    .filter(|d| failed.contains(*d))
                    .map(|d| d.as_str())
                    .collect();
                if !failed_deps.is_empty() {
                    outcomes.insert(
                        s.id.clone(),
                        ChildOutcome::blocked(
                            &s.id,
                            format!("blocked by failed dependency: {}", failed_deps.join(", ")),
                        ),
                    );
                    failed.insert(s.id.clone());
                } else {
                    batch.push(s.clone());
                }
            }

            if batch.is_empty() {
                continue;
            }

            // Run the batch concurrently (already bounded by max_concurrent).
            let batch_ids: Vec<String> = batch.iter().map(|r| r.id.clone()).collect();
            let mut set: JoinSet<ChildOutcome> = JoinSet::new();
            for req in batch {
                let svc = self.clone();
                let run_ctx = run_ctx.clone();
                set.spawn(async move {
                    let cancel = Arc::new(AtomicBool::new(false));
                    svc.run_one_child(req, run_ctx, cancel).await
                });
            }
            while let Some(joined) = set.join_next().await {
                match joined {
                    Ok(outcome) => {
                        if outcome.status.is_success() {
                            completed.insert(outcome.id.clone());
                        } else {
                            failed.insert(outcome.id.clone());
                        }
                        outcomes.insert(outcome.id.clone(), outcome);
                    }
                    Err(e) => warn!(error = %e, "agent-service: graph child task panicked"),
                }
            }
            // A `JoinError` carries no request id, so reconcile against the
            // batch we just submitted: any id from this round still missing
            // an outcome panicked and must be marked failed here. Otherwise it
            // would remain "not yet run" forever and `ready` would resubmit it
            // on every following iteration, spinning indefinitely.
            for id in &batch_ids {
                if !outcomes.contains_key(id) {
                    outcomes.insert(
                        id.clone(),
                        ChildOutcome::panicked(id, "child task panicked or was lost"),
                    );
                    failed.insert(id.clone());
                }
            }
        }

        order_outcomes(order, outcomes)
    }
}

/// Reorder a keyed outcome map back into request order.
fn order_outcomes(
    order: Vec<String>,
    mut by_id: HashMap<String, ChildOutcome>,
) -> Vec<ChildOutcome> {
    order
        .into_iter()
        .filter_map(|id| by_id.remove(&id))
        .collect()
}
