# skills re-exports the skill model that moved to skill_model

**Reason skipped:** D2 moved the SKILL.md parser, the validator, skill fixtures, hint matching and the built-in registry down into `skill_model` so `skill_audit` no longer reaches up into `skills`. `skills::...` and `skills::builtin::...` are used across crates, so both keep resolving.
**Date:** 2026-11-18
**Where:** `crates/hq-tools/src/skills/mod.rs` (`pub use crate::skill_model::{parse, validate, fixtures}::...`), `crates/hq-tools/src/skills/builtin.rs` (`pub use crate::skill_model::shipped::*;`), `crates/hq-tools/src/skills/hints.rs` (`pub(super) use ...::word_boundary_match`)
**Remove when:** `hq-agent`, `hq-cli`, `hq-mcp` and the rest import from `hq_tools::skill_model::...` and the code inside `skills/` names `skill_model` directly.
