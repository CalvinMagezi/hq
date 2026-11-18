# harness_session still calls a2a::delegate_from_spawn, one layer up

**Reason skipped:** D1 moved the four self-contained checks out of `a2a`, which took `harness_session -> a2a` from 12 references to 1. The last one, `delegate_from_spawn`, calls the whole delegation executor (`run_delegation`, `spawn_as_delegation`, `authorize_delegate`), which in turn uses `harness_session`. It cannot move down without dragging that executor along, and cutting the edge means inverting the dependency, which is a design change rather than a move.
**Date:** 2026-11-18
**Where:** `crates/hq-tools/src/harness_session/tools.rs` (the `harness_session_spawn` tool), `crates/hq-tools/src/a2a.rs`, `crates/hq-tools/layers.toml` (`harness_session -> a2a` in `allowed_back_edges`)
**Remove when:** the spawn tool is given its delegator when the tools are assembled (a small trait in a lower module, implemented in `a2a` and passed in at the two call sites that build the harness tools), and the `harness_session -> a2a` line is deleted from `allowed_back_edges`.
