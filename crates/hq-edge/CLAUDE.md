# The public forwarder

Notes for working under `crates/hq-edge/`, moved out of the root `CLAUDE.md` so an assistant loads them only when it is in this area. The root file has the rules that apply everywhere.

## Entry points

| What | Path |
|------|------|
| The public forwarder (`hq edge run|id|allow|revoke|list|invite`, config key `edge:`, off by default): reads only the TLS server name on one TCP listener, never decrypts, forwards to the registered device over an iroh stream (ALPN `hq/edge/1`); invite-only through the `Registry` trait (an allowlist file for now); optional co-hosted `iroh-relay` that admits only registered devices; per-device and pre-handshake caps | `crates/hq-edge/` (`hello.rs` ClientHello and its byte cap, `registry.rs` the `Registry` trait, `device_label` and the allowlist file checks, `limits.rs` token bucket and pending caps, `pipe.rs` the copy with idle timeout and rate limit, `server.rs` the listener and per-connection decision, `relay.rs` the co-hosted relay and its access control, `config.rs` the `edge:` section, `tests/edge.rs` the device-behind-a-NAT end to end), `crates/hq-cli/src/commands/edge.rs` (not built for Android or Windows), `deploy/edge/` (systemd unit, Dockerfile), `docs/EDGE.md`, `docs/security/EDGE.md`, `docs/plans/native-bridge-sprint.md` |
