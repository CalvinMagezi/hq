# hq-tools re-exports the four modules that moved to hq-tool-api

**Reason skipped:** the extraction (C1) was a pure move. Re-exporting keeps every `hq_tools::registry::...`, `hq_tools::util::...` and in-crate `crate::registry::...` path working, so no other crate had to change in the same pull request.
**Date:** 2026-11-18
**Where:** `crates/hq-tools/src/lib.rs` (`pub use hq_tool_api::{family_confirm, family_guest, registry, util};`)
**Remove when:** the modules inside `hq-tools` name `hq_tool_api::registry` instead of `crate::registry`. Every other consumer (`hq-mcp`, `hq-agent`, `hq-web`, `hq-cli`, `hq-daemon`) already imports `hq_tool_api::...` directly and depends on `hq-tool-api`, and `hq-daemon` no longer depends on `hq-tools` at all.

A crate that depends on `hq-tools` only for the `HqTool` trait still rebuilds whenever any tool changes. Depending on `hq-tool-api` directly is what turns the extraction into a real selective rebuild.
