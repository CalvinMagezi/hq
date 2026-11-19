# The macOS desktop app

Notes for working under `crates/hq-desktop/`, moved out of the root `CLAUDE.md` so an assistant loads them only when it is in this area. The root file has the rules that apply everywhere.

## Entry points

| What | Path |
|------|------|
| macOS desktop app (Tauri 2: host start/stop, the pairing QR and typed digits, paired devices and revoke, Connect Claude, a loopback proxy for the web app and the consent page) | `crates/hq-desktop/` (all the logic, no GUI dependency, in the root workspace: `host.rs` the one child it may stop, `pairing.rs` over `hq-bridge`, `proxy.rs`, `connect.rs`, `window.rs` navigation allow-lists, `desktop.rs` wiring, `settings.rs`), `apps/desktop/` (the shell, its own Cargo workspace in `src-tauri`, excluded from the root: `main.rs`, `commands.rs`, `build.rs` and `capabilities/control.json` list the same commands; `ui/` the control window), `crates/hq-desktop/tests/` (`pairing_e2e.rs`, `proxy.rs`, `shell_config.rs` guards the CSP, capability and no-plugin settings, `cli_join.rs` opt-in), `docs/DESKTOP.md`, `docs/security/DESKTOP.md` |
