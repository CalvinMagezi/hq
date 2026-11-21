# An office_edit preview token only works in the process that made it

**Reason skipped:** the token is a keyed hash of the note's revision and the request, with a secret made when the process starts. Keeping it across a restart would mean storing a secret on disk, which is more than the preview needs.
**Date:** 2026-11-20
**Where:** `crates/suite/hq-suite-adapter/src/edit_plan.rs` (`secret`)
**Remove when:** previews must survive a daemon restart, for example a stored per-vault key.

A preview made before a restart cannot be applied and has to be asked for again.
