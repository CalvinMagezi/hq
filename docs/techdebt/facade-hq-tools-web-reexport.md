# hq-tools re-exports web, remote_mcp and imagegen from hq-tools-web

**Reason skipped:** C2 moved the three modules into their own crate as a pure move. Re-exporting keeps every `hq_tools::web::...` path in other crates and every `crate::web::...` path inside `hq-tools` working, so nothing else had to change in the same pull request.
**Date:** 2026-11-18
**Where:** `crates/hq-tools/src/lib.rs` (`pub use hq_tools_web::{imagegen, remote_mcp, web};`)
**Remove when:** every consumer that uses the web tools (`hq-mcp`, `hq-agent`, `hq-web`, `hq-cli`, `hq-daemon`) imports `hq_tools_web::...` directly and lists `hq-tools-web` in its own `Cargo.toml`.

Until then a crate that only wants the web tools still depends on all of `hq-tools`, so a change to any other tool rebuilds it.
