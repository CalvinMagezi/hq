//! Reminders the owner set: when each rings, how a repeat or a nudge advances, and the inbox item
//! each ping raises. No model is involved anywhere here. The rows live in `hq_db::reminders`.
//!
//! `schedule` is pure (every function takes `now`), `fire` is the pass the daemon runs.

pub mod fire;
pub mod schedule;
