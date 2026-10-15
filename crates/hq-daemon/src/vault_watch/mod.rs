//! Notices edits to notes made outside HQ (an editor, git, a coding agent's shell) and keeps the
//! search index in step with them in seconds.
//!
//! The files stay the source of truth and a filesystem event is only a hint that a path may have
//! changed. Consumers read the file again and act on what is there, and the scheduled scan keeps
//! running as the floor, so a missed event costs latency, never correctness.

pub mod normalize;
pub mod pipeline;
pub mod queue;
pub mod watcher;

pub use normalize::{Normalized, WatchEvent, normalize};
pub use pipeline::{PipelineStats, spawn};
pub use watcher::{VaultWatcher, WatchBackend, WatchStats};
