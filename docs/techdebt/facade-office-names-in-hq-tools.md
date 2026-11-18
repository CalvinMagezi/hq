# hq-tools re-exports the office_file name and its safe actions from hq-tool-api

**Reason skipped:** P2-5 moved the `office_file` tool into `hq-tools-office` and put the three dependency-free items the approval gates need (`OFFICE_TOOL`, `SAFE_ACTIONS`, `moves_original`) in `hq-tool-api::office`. `hq-mcp/src/gateway.rs` and `hq-agent/src/governance/owner_approval.rs` still name them as `hq_tools::office::...`, which keeps resolving through a re-export, so neither had to change in the same pull request.
**Date:** 2026-11-18
**Where:** `crates/hq-tools/src/office.rs` (the `pub use hq_tool_api::office::{...}` line)
**Remove when:** `hq-mcp` and `hq-agent` import `hq_tool_api::office::...` directly (with `hq-tool-api` in their own `Cargo.toml`), then delete `crates/hq-tools/src/office.rs`, its `pub mod office;` in `lib.rs` and its entry in `crates/hq-tools/layers.toml`.
