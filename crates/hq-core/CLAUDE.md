# Types, configuration and editions

Notes for working under `crates/hq-core/`, moved out of the root `CLAUDE.md` so an assistant loads them only when it is in this area. The root file has the rules that apply everywhere.

## Entry points

| What | Path |
|------|------|
| Editions (`profile: full`, `lite`, `mobile`) and the Android build | `crates/hq-core/src/config/profile.rs` (`Profile`, `MobileConfig`, `embeds_notes`), `effective_profile` in `crates/hq-core/src/config/mod.rs` (Windows is always Lite, Android defaults to Mobile), `Profile::drops_tool_category` (used by `crates/hq-mcp/src/registry.rs` and `crates/hq-agent/src/builder/tools.rs`), `scheduled_tasks` in `crates/hq-cli/src/commands/start/daemon/mod.rs`, `.github/workflows/android.yml`, `scripts/check-android-alignment.sh`, `docs/HQ_MOBILE.md`, `docs/HQ_LITE.md` |
