# The private network

Notes for working under `crates/hq-net/`, moved out of the root `CLAUDE.md` so an assistant loads them only when it is in this area. The root file has the rules that apply everywhere.

## Entry points

| What | Path |
|------|------|
| Private network (`hq net`, WireGuard hub and spoke), the root applier and its grammar | `crates/hq-net/` (`grammar.rs` is the only language the applier reads, `applier.rs` the Linux-only opener and the `wg` and `nft` caller, `wgconf.rs`, `links.rs` (the set of linked device pairs the applier hands to `nft`; version 2 of the peers file), `store.rs`, `init.rs`, `ops.rs`), `crates/hq-cli/src/commands/net.rs`, `deploy/net/` (systemd units; `deploy/net/test/run.sh` runs them for real in Docker, `deploy/net/tunnel-test/run.sh` runs the userspace tunnel against that hub; both share `deploy/net/test/hub-lib.sh`), `docs/PRIVATE_NETWORK.md`, `docs/plans/private-network.md`, `docs/security/NETWORK.md` |
| The phone as a userspace WireGuard peer without `VpnService`: the `hq-tunnel` crate and `hq net tunnel` (subcommands keygen, import, check, run, status; cargo feature `tunnel`, off by default, on in the Android build) | `crates/hq-tunnel/` (`join.rs` the strict join-text parser, `config.rs` the serve and reach maps, `stack.rs` the smoltcp side and the source fence, `run.rs` the poll loop and its schedule, `status.rs` the status file, `session.rs` boringtun, `store.rs` the tunnel directory and the new-hub rule, `keyin.rs` the key from stdin or a private file), `crates/hq-cli/src/commands/net_tunnel.rs`, `crates/hq-cli/tests/net_tunnel.rs` (`cargo test -p hq-cli --features tunnel --test net_tunnel`), `docs/PRIVATE_NETWORK.md` ("A phone without the WireGuard app"), `docs/security/NETWORK.md` section 9 |
