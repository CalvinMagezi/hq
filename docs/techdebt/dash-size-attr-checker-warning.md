# The `size` attribute of dashboard tiles and plots is not in the checker's list, so it warns

**Reason skipped:** The web layout change ships in `apps/hq-web` only, and the known attributes live in Rust (`allowed_attrs` in `crates/suite/hq-suite-md/src/block.rs`), which that change did not touch.
**Date:** 2026-11-20
**Where:** `crates/suite/hq-suite-md/src/block.rs` (`allowed_attrs` for `BlockType::Tile` and `BlockType::Plot`), `docs/suite/HQMD.md` ("Dashboard layout")
**Remove when:** `size` is added to the allowed attributes of `tile` and `plot` (and validated as `small`, `wide` or `full`, with a test); then the note in HQMD.md can go.

A dashboard the web app has arranged carries `size=small|wide|full` on some fences. The parser keeps the attribute byte for byte, so nothing is lost, but the checker reports `block.unknown_attr` (a warning, not an error) for each one.
