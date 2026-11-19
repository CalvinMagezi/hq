# hq-tools re-exports coding and file_edit from hq-tools-coding

**Reason skipped:** C3 moved the coding tools and the file-edit engine into their own crate as a pure move. Re-exporting keeps every `hq_tools::coding::...` path in other crates and every `crate::coding::...` path inside `hq-tools` working, so nothing else had to change in the same pull request.
**Date:** 2026-11-18
**Where:** `crates/hq-tools/src/lib.rs` (`pub use hq_tools_coding::{coding, file_edit};`)
**Remove when:** every consumer that uses the coding tools (`hq-agent` first of all, then `hq-mcp` and `hq-cli`) imports `hq_tools_coding::...` directly and lists `hq-tools-coding` in its own `Cargo.toml`.
