//! Evidence for verifying a task's completion (docs/plans/task-verification.md, "Evidence").
//!
//! Everything here produces observations: facts HQ recorded itself, or read from an independent
//! system, before any model sees the task. A verifier may only cite an observation that exists,
//! and a source that cannot be read becomes an `unavailable` observation, never a pass.
//!
//! - [`observation`]: the record of facts, and what counts as supporting one.
//! - [`remote`]: the read-only GitHub client, behind a trait so collectors test on fixtures.
//! - [`artifact`]: finding what a task produced, from its links and then its lease's branch.
//! - [`collect`]: pull request facts, check runs, required checks, a note's hash.
//! - [`probe`]: a guarded probe of a URL the task says it deployed.
//! - [`tamper`]: a deterministic scan of a diff for edits that weaken the checks themselves.
//! - [`contract`]: the acceptance criteria a task states, and their hash.
//! - [`pipeline`]: gather, decide, review and apply, for one task.
//! - [`dispatch`]: the queue loop behind the `verify-dispatch` daemon task.

pub mod artifact;
pub mod collect;
pub mod contract;
pub mod dispatch;
pub mod observation;
pub mod pipeline;
#[cfg(test)]
mod pipeline_tests;
pub mod probe;
pub mod remote;
pub mod reviewer;
pub mod tamper;

pub use artifact::{Artifact, Discovery, TaskFacts, discover, repo_of_checkout, repos_for_cwds};
pub use collect::{
    ChecksSummary, PrFacts, collect_checks, collect_diff, collect_note, collect_pr,
    record_diff_excerpts, touches_tier2_path,
};
pub use contract::{Contract, contract_of};
pub use dispatch::{Backoff, DispatchReport, dispatch};
pub use observation::{Observation, ObservationLog};
pub use pipeline::{
    Env, Evidence, Finished, Gate, Resolution, WaitingFor, finish, gate, gather, review,
    verify_task,
};
pub use probe::{ProbePolicy, probe_url};
pub use remote::{Diff, GhCli, GitHubRead};
pub use reviewer::{LlmReviewer, NoReviewer, ReviewReply, ReviewRequest, Reviewer};
pub use tamper::{TamperFlag, TamperReport, scan_diff};
