# Pairing machines

Notes for working under `crates/hq-bridge/`, moved out of the root `CLAUDE.md` so an assistant loads them only when it is in this area. The root file has the rules that apply everywhere.

## Entry points

| What | Path |
|------|------|
| Pairing machines with a QR code and reconnecting by key (`hq bridge`, with `register`, `edge` and `forget-edge` for a device's `hq edge`; cargo feature `bridge`, on by default except on Windows; the library does nothing until a caller starts it) | `crates/hq-bridge/` (`ticket.rs` the `hqpair://` link and protocol versions, `code.rs` the one-time code, commitment and six digits, `limits.rs` per-source limits, `host.rs` the ticket and its throwaway endpoint, `join.rs`, `bridge.rs` the long-term endpoint, the paired-only gate and pushed revocation, `door.rs` the `hq/edge/1` protocol a device accepts only from the edge it registered with and pipes to the connector door on loopback, `wire.rs`), `crates/hq-db/src/bridge.rs` and `sql/098_bridge_pairing.sql`, `sql/099_bridge_edge.sql` (the `bridge_peers` list and the one `bridge_edge`), `crates/hq-cli/src/commands/bridge.rs`, `docs/PAIRING.md`, `docs/security/PAIRING.md`, `docs/plans/native-bridge-sprint.md`, `docs/plans/native-bridge/spike-f/` (the historical spike) |
| The phone app as a pairing hub (`hq bridge run`: JSON lines on stdin and stdout, driven by the Android app; the Android build sets a null `ndk-context` and gives every bridge endpoint its own resolver) | `crates/hq-cli/src/commands/bridge_control.rs` (control loop, tested with two machines), `crates/hq-cli/src/android_runtime.rs`, `crates/hq-bridge/src/dns.rs`, `apps/android/app/src/main/kotlin/example/hq/spike/bridge/` (supervisor, wire format, network watcher), `ui/pairing/` (screens, QR scan, `hqpair://` door), `ui/connect/` (Connect Claude), `docs/HQ_MOBILE.md` ("The bridge on the phone"), `docs/plans/native-bridge/w4-measurements.md`, `docs/security/PAIRING.md` section 12 |
