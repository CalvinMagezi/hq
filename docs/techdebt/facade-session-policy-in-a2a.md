# a2a re-exports the session checks that moved to session_policy

**Reason skipped:** D1 moved `check_session_access`, `check_tree_limits`, `RATE_WINDOW_MINUTES` and `DELEGATION_LOCK` down so `harness_session` no longer reaches up into `a2a`. The `a2a::...` paths are public, so they keep resolving.
**Date:** 2026-11-18
**Where:** `crates/hq-tools/src/a2a.rs` (the `pub use crate::session_policy::{...}` and `pub(crate) use ...::DELEGATION_LOCK` lines)
**Remove when:** nothing outside `a2a.rs` names them through `a2a::` (inside the crate that is already true; the old design notes in `docs/plans/agent-teams.md` and `docs/plans/remote-driven-teams.md` still mention `a2a::check_tree_limits`), and no code outside the crate uses the `hq_tools::a2a::` paths.
