# The hq-bridge e2e tests need an IPv6 loopback and cannot run where there is none

**Reason skipped:** the tests bind `[::1]` on purpose, as a second source address that is not `127.0.0.1`, and one machine binds IPv4 and IPv6 together. A container with no IPv6 (`AF_INET6` unsupported) fails them all at the bind, which hides what the tests check.
**Date:** 2026-11-21
**Where:** `crates/hq-bridge/tests/e2e.rs` (`V6`, `Machine::new("desk", &[V4, V6], ...)`)
**Remove when:** the tests choose a second loopback address that exists on the host (for example `127.0.0.2` when there is no IPv6, with the desk bound to IPv4 alone), so they run in an IPv4-only container.

Checked once by hand: with `V6` set to `127.0.0.2:0` and the desk bound to `V4` only, all 31 tests pass in an IPv4-only container, so the failures there are the missing address family, not a fault in the pairing code. That change was not committed.
