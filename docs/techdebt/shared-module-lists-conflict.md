# Adding a module edits a hand-kept alphabetical `mod` list, so parallel pull requests conflict

**Reason skipped:** Found in a parallel-work pilot, not in a feature change. Resolving the conflict is mechanical (keep both lines, keep the order), while a macro or build script that generates the list would cost more than it saves today and would hide the module list from readers and tools.
**Date:** 2026-11-19
**Where:** `crates/hq-core/src/lib.rs`, `crates/hq-web/src/lib.rs`, `crates/hq-tools/src/lib.rs`, `crates/hq-db/src/lib.rs`
**Remove when:** two pull requests that each add a module to the same `lib.rs` conflict on it more than about once a month for a quarter, or a crate's list passes 80 entries. Then group modules into sub-modules by concern (so each pull request edits its own group's `mod.rs`), or generate the list, and delete this item.

**Symptom.** `crates/hq-core/src/lib.rs` is a hand-kept, alphabetical list of `pub mod` lines (32 modules in 103 lines). A new module is added at its sorted position, so two pull requests that add neighbouring names touch adjacent lines and Git reports a conflict, even though both changes are correct and independent.

**Why it is left.** The resolution is trivial: keep both lines in alphabetical order, nothing else. A macro or `build.rs` that generates the list would add build machinery, hide the public surface from grep and IDEs, and cost more than the occasional resolution does today.

**Other lists of the same shape** (hand-kept `mod` declarations in a crate's `lib.rs`, counted by `mod name;` lines, 40 or more):

| File | `mod` lines | File lines |
|------|-------------|------------|
| `crates/hq-web/src/lib.rs` | 61 | 320 |
| `crates/hq-tools/src/lib.rs` | 60 | 76 |
| `crates/hq-db/src/lib.rs` | 59 | 68 |

`hq-tools` and `hq-db` are almost pure module lists, so they are the likeliest to collide. `hq-web` mixes modules with other code. The next largest are `hq-host` (31), `hq-llm` (28) and `hq-agent` (28), below the threshold; `hq-core` itself has 32. No crate under `crates/suite/` is near it.
