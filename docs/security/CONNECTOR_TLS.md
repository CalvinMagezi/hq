# The connector's own TLS

`docs/CLAUDE_CONNECTOR.md` used to need an ingress (Caddy, Tailscale Funnel, a tunnel) that ends
TLS for HQ. This is the other way: HQ ends TLS itself, for the connector's public name, on a
byte stream a public forwarder hands it. The forwarder (`hq edge`, built separately) reads the
TLS server name to choose a device and passes the bytes on. It cannot read them, and no key
is ever on it. Code: `crates/hq-web/src/tls/`. Off by default.

**Status.** The door is a library surface: nothing in `hq start` builds it or feeds it streams yet.
That is the connector-on-the-bridge work, once the forwarder and the pairing bridge exist.
`mcp_connector.tls` is parsed with the rest of the config and checked by `ConnectorTls::for_state`,
but nothing calls that at startup, so enabling it changes nothing today; the tests drive the door
directly.

```
claude.ai ──TLS──▶ forwarder (SNI only) ──one stream per connection──▶ device
                                                                         │
                          ConnectorTls::serve(stream): rustls, then the gate, then the router
```

## Turn it on

```yaml
mcp_connector:
  enabled: true
  public_url: https://phone.edge.example.com   # the name the certificate is for
  authorize_url: https://hq.example.ts.net     # required: the consent page is never on this name
  tls:
    enabled: true
    agree_to_ca_terms: true                    # required: see below
    acme_directory: letsencrypt                # letsencrypt | letsencrypt-staging | https URL
    contact_email: null                        # optional; the CA may write to it about the certificate
```

- `tls.enabled` needs `mcp_connector.enabled` and a `public_url` that is a domain name on the
  default https port (not an address, not a single label, no trailing dot, because the CA
  validates on port 443 and a client's server name never ends in a dot). The certificate name is
  that host; there is no second setting.
- `authorize_url` must name a different origin. `/oauth/authorize` answers only on the consent
  origin, and with no `authorize_url` that origin is the public name itself. `for_state` refuses
  that configuration.
- `agree_to_ca_terms: true` is required because registering an ACME account accepts the CA's
  subscriber agreement for the owner (Let's Encrypt's: <https://letsencrypt.org/repository/>).
  HQ registers no account until it is set.
- Refused under `profile: lite`, like the connector. `ConnectorTls::for_state` returns the
  reason and the door is not built.
- State is in `<vault>/_data/connector_tls/`, all `0600` in a `0700` directory, created with that
  mode from the first byte and replaced by rename: `account-<tag>.json` (the ACME account key),
  `<name>-<tag>.pem` (the certificate key and chain) and `<name>-<tag>.retry.json` (the failure
  count and the wait before the next order, not secret). `<tag>` is a hash of the ACME directory,
  so a staging certificate is never found after a switch to production. Neither secret file is
  ever logged or shown: the types that hold them print as nothing, and the status carries only the
  name, the expiry and a one-line reason.
- The module itself opens no listener: whoever owns the stream source calls
  `ConnectorTls::serve(stream)` once per connection and spawns `ConnectorTls::run()` once. `run()`
  orders a certificate when there is none and again two thirds of the way through each one, and
  needs `serve` taking connections: the CA's validation arrives on it. The server does this in one
  place, `create_router` (so `hq start` and `hq web` behave the same): when `tls` is on and valid
  it listens on `tls.door_listen` and calls `serve` for each connection (`tls/listener.rs`).
- That listener is a unix socket in a directory only this user can enter: the default is
  `<vault>/_data/connector_door/door.sock`, the directory is created `0700`, must belong to the
  user and not be writable by others (never a symlink), the socket is `0600`, a stale socket from a
  crash is replaced and anything else at the path is left alone. The process that holds the
  forwarder's iroh streams (`hq bridge serve` or `run`, a separate process on a phone) is the only
  thing meant to connect. A TCP port is not the default because a port can be bound by anyone on
  the machine: whoever wins that race receives the forwarded bytes, including the CA's validation
  (`acme-tls/1` passes through the edge), could order a certificate for the device's name and end
  TLS in its place. A loopback `ip:port` with a fixed port is still possible as an explicit opt-in
  (`door_listen`) for a machine with one user. A bind that fails is retried every few seconds and
  logged, never left silent. The door starts with the router, before the main port is bound, so a
  server whose main bind fails still has its door; and started before the bridge it orders its
  first certificate while the forwarder cannot reach it yet, which fails once and waits (15
  minutes) before the next order: start the bridge first, or restart the server after registering.
- Whoever can connect to the socket speaks TLS to the same door and meets the same gate and
  OAuth as the internet would; every request is marked as coming through the door, so a local
  peer is never mistaken for a trusted local caller. It is on whenever TLS is, not only while a
  connection window is armed: calls and refreshes for an existing grant must work at any time, and
  arming guards `/oauth/authorize`, which this door does not serve at all. A local process that
  can reach the socket shares the door's connection pools with Claude's calls and can crowd them;
  the socket's mode keeps that to the owner's own processes.

## What the forwarder must hand over

- The raw bytes of the client's TCP connection, from the first byte of the ClientHello, with
  nothing prepended (no PROXY header) and nothing stripped. (The W1 forwarder's interface note
  says the same: one iroh stream per TCP connection, no preamble, no framing.)
- `acme-tls/1` connections unchanged. The CA validates the device's name by connecting to it on
  port 443 offering only that ALPN; a forwarder that routes on SNI alone passes it through.
- One stream per client connection, closed when either side closes.

The device never learns the client's address. Abuse limits per client address therefore belong
on the forwarder; the device bounds itself (below), but a flood of silent connections can still
fill those bounds for as long as it lasts. The CA's validation is the part that matters: it has
slots of its own and a short deadline, but it still has to get through the hello stage, so a flood
of silent connections can delay an order. A 90 or 45 day certificate is renewed from two thirds of
its life, leaving weeks of retries; the first certificate and a 6 day one leave far less.

## What the door does with a connection

| Step | Rule |
|---|---|
| ClientHello | At most 256 connections are in this stage at once, 5 s each. The name is read before any certificate is chosen. A server name that is not the connector's, or none, is dropped. A hello with no certificate installed yet is dropped. |
| `acme-tls/1` | Offered alone, for the connector's name, with a challenge pending: the challenge certificate (self-signed, critical `acmeIdentifier` extension), handshake, then the stream is shut down without reading a byte of HTTP. These connections have 8 slots of their own and 3 s for the handshake, so a full web door can still renew; they still pass through the hello stage above. With no challenge pending, dropped. Offered beside `h2` or `http/1.1`, it is a web client and gets the issued certificate. |
| Handshake | TLS 1.2 and 1.3 on rustls with the aws-lc provider, named explicitly because the process links two. ALPN `h2`, `http/1.1`, and the protocol served is the one ALPN settled on, never sniffed from the first bytes. 10 s for the handshake itself. At most 64 web connections at once; the 65th is dropped, not queued. |
| Connection | A connection with no request in flight for 60 s is wound down, whatever the protocol (a session that says nothing, an HTTP/2 session that only answers pings), and one that sends no first request within 10 s likewise. HTTP/1 request line and headers within 10 s. A connection lives at most 5 minutes, then is wound down gracefully (an answer in flight may take up to 2 more minutes). A wind-down for idleness that does not finish in 10 s drops the connection, so an HTTP/2 peer that never sent its preface holds a slot for about 20 s, not 60. At most 32 HTTP/2 streams. |
| Request | The body is read first, within 10 s and no larger than 1 MiB (a 413), counted as it arrives against 16 MiB held across all connections (a 503 past that): a body that is still coming costs what it has sent, and a slow one holds nothing else. Then one of 16 answering slots (a 17th is a 503). Then the credential check, which reads the database, off the async workers. The handler gets 120 s (a 408). |
| Gate | Before the router sees anything (see below). A refusal is a plain `404 Not found` whichever check failed. |

### The gate

Everything Caddy or a tunnel used to do in front of HQ happens here, on every request:

1. **Path allow-list**, byte for byte, no decoding or normalising: `/mcp`, `/oauth/token`,
   `/oauth/revoke`, `/.well-known/oauth-protected-resource`,
   `/.well-known/oauth-protected-resource/mcp`, `/.well-known/oauth-authorization-server`. This is
   exactly `deploy/Caddyfile.connector`'s list. (The sprint brief says "two `.well-known` paths":
   they are two documents; the protected-resource document is also served at its `/mcp` suffix,
   which clients try first.) The consent side (`/oauth/authorize`, `/connect`,
   `/api/oauth/requests/*`) is not served on this name, so the arming window cannot be probed or
   filled from the internet. The public name serves no consent page at all (see "Not covered").
   `/mcp/`, `//mcp`, `/%6Dcp`, `/.well-known/../api/tree` and everything else are 404s.
2. **Host.** `Host` and, for HTTP/2, `:authority` must each be exactly the connector's name.
   Two `Host` headers, or a disagreement, is a 404. The door then sets `Host` to the canonical
   value, which also gives HTTP/2 requests the header the handlers read. A client that names a
   different origin through the forwarder therefore reaches nothing that trusts `Host`.
3. **Credentials.** Only a live connector grant's access token goes on, as a single
   `Authorization` value. Anything else a client presents (the owner's web token, a device token,
   an `AGENTHQ_*` API key, an agent's session token, in `Authorization` or `x-api-key`, or a
   second `Authorization` header) is replaced by a token that matches nothing, so the handler
   answers `401 invalid_token` and Claude refreshes or starts over as it would anyway. Without
   this the door would accept a full-access API key from the internet.
4. **Not local.** Proxy and tunnel identity headers are removed (`x-forwarded-*`, `x-real-ip`,
   `tailscale-*`, `x-hq-session-id`) and `Forwarded` is set on every request. `mcp_http` opens
   `/mcp` for `HQ_MCP_DEV_NO_AUTH=1` only when no such header is present, so a mistaken
   development switch cannot open the connector to the internet.

## Certificates

**Library.** `instant-acme` 0.8.5 (Apache-2.0, compatible with this MIT project; `deny.toml`
allows it). It is maintained by a rustls maintainer, the most downloaded Rust ACME client
(3.5 million), last released 2026-02-24, and speaks RFC 8555 with TLS-ALPN-01 and the
Retry-After-aware error type we need. Alternatives: `rustls-acme` (Apache-2.0 or MIT) bundles an
acceptor and a renewal stream, which would put the acceptor, the challenge handling, the cache
layout and the error model in a library we do not control and cannot test the way we test this;
`acme-lib` is unmaintained since 2024 and blocking. `instant-acme` does only the protocol, which
leaves the acceptor, the gate, the storage and the schedule here, where they are tested. Two
settings matter: `default-features = false`, so it brings no HTTP client or platform verifier of
its own (its `hyper-rustls` default pulls the platform verifier, which needs a JNI context on
Android); and its `HttpClient` trait is implemented over the workspace's `reqwest`, so ACME uses
the same TLS stack and roots as the rest of HQ. `rcgen` (MIT or Apache-2.0) makes the CSR and the
challenge certificate; `x509-parser` (MIT or Apache-2.0) reads the validity window. All three are
pinned exactly in `crates/hq-web/Cargo.toml`.

**Flow.** Account (one per ACME directory; the store is checked to be writable before one is
registered, and the account is kept in memory once loaded or made, so a store that fails later
does not lead to a second registration in the same run; a stored account that cannot be read is an
error, never a reason to register another; no account is registered without
`agree_to_ca_terms`) → order for the one name → TLS-ALPN-01 challenge armed on the acceptor
→ `set_ready` → poll until ready → ECDSA P-256 key and CSR → finalize → chain → the leaf is checked
to name the connector and to be for that key → saved, then swapped into the acceptor without a
restart. Renewal starts two thirds of the way through the certificate's life, so the rule holds for
90, 45 and 6 day certificates; a new certificate that is already due (a device clock far off) waits
36 hours before the next order, a little more than the 34 hours in which the CA gives back one
certificate for the same name. A restart serves the saved certificate at once, drops one that has
expired, and remembers the failure count (even after the wait is over) and the wait. A wait more
than seven days ahead of the clock was set by a clock that has been corrected, or by a damaged
file, and is ignored. ARI (RFC 9773) is not used yet
(`TECHDEBT.md`).

**Tests.** `cargo test -p hq-web --lib tls::` runs the whole flow against an in-process ACME
server (`tests/test_ca.rs`). It is not Pebble: it is a test double that checks every request's
JWS signature, nonce and URL, validates TLS-ALPN-01 by dialing the acceptor and reading the
`acmeIdentifier` extension, and signs the client's CSR with a CA whose root the test client
trusts. The same order against Pebble, Let's Encrypt's own test server, is
`scripts/test-connector-tls-pebble.sh` (Docker, pinned image digests; `#[ignore]`d in the suite).

## The CA's limits

Source: <https://letsencrypt.org/docs/rate-limits/>, page last updated 2026-08-05, read
2026-10-22. Numbers change; read it again before relying on them.

| Limit | Value | What it means here |
|---|---|---|
| Certificates per registered domain | 50 per 7 days, refilling 1 per 202 minutes; global, all accounts | Every `<device>.<edge domain>` name shares the edge domain's bucket. A household is far inside it; an open service would not be. |
| Certificates per exact set of names | 5 per 7 days, refilling 1 per 34 hours | A device that loses its state and re-orders daily is stopped on day 6. Keep `_data/connector_tls/`. |
| New orders per account | 300 per 3 hours | Not reachable by one device. |
| New accounts per IPv4 address | 10 per 3 hours, no overrides | A fleet behind one address creating accounts. The account is made once and stored. |
| Failed validations per name per account | 5 per hour, refilling 1 per 12 minutes | The retry wait starts at 15 minutes (below) and, with the failure count, survives a restart, so a process that is restarted over and over does not order each time. |
| Consecutive failed validations | 1,152, refilling 1 a day | Not reachable by the retry waits. |

Renewals are designed to rarely hit a limit. The edge operator has one structural fix for the
shared bucket: put the edge domain on the Public Suffix List, as hosting providers do. The CA
then treats `<device>.<edge domain>` as its own registered domain, one bucket per device.

**What the app does when rate limited.** A rate limit (`urn:ietf:params:acme:error:rateLimited`,
or HTTP 429) is not a failure of the certificate in use:

- The current certificate keeps serving; nothing is torn down.
- The next order waits for the CA's `Retry-After` (kept from the response), at least a minute and
  at most seven days; with none, six hours. Other failures wait 15, 30 and 60 minutes, then 2, 4
  and 6 hours, so the first retry is after the failed-validation limit has refilled.
- No new account is created to get round a limit: the account is made once and read from disk
  after a restart. (A CA that has forgotten the account, as staging CAs do after a reset, leaves
  the door in `failed` with a message that names the file to delete; it does not register another
  by itself, and the next try after deleting it needs no restart.)
- `ConnectorTls::status()` reports `state` (`no_certificate`, `valid`, `renewing`, `rate_limited`,
  `failed`), `not_after`, `next_attempt_at` and a one-line `last_error`. The status surfaces
  (`hq connector status`, the app's Connect Claude screen) read it; wiring them is the
  connector-on-the-bridge work. A first certificate that is rate limited leaves the connector
  without TLS until the wait ends: the app should say when, and offer
  `acme_directory: letsencrypt-staging` only for testing (its certificates are not trusted by
  Claude).
- For development use `letsencrypt-staging`: far higher limits.

## The two doors

`docs/plans/zero-setup-pairing.md` keeps the own-devices door (iroh, paired keys) and the
connector door (public HTTPS, OAuth) apart. Neither depends on the other. The tests in
`tests/doors.rs` hold the four promises. Each was broken on purpose and failed, as recorded here.
The doors share no state, so the assertions that a door is unchanged when the other is stopped or
revoked can only fail if code couples them; the ones with teeth are named in the last column.

| Promise | Test | Mutation that makes it fail |
|---|---|---|
| The own-devices listener never serves connector paths | `the_own_devices_door_never_serves_connector_paths` | `refuse_connector_paths` returns the router unchanged |
| The connector door never accepts own-devices credentials | `the_connector_door_never_accepts_the_credentials_of_the_own_devices_door`, and in `gate.rs` `no_configured_api_key_gets_a_caller_an_identity_through_the_gate`, which runs the real `resolve_identity` with all four `AGENTHQ_*` keys set | The gate leaves `Authorization` and `x-api-key` as they came (an agent session token then works on `/mcp`) |
| Turning one off leaves the other unchanged | `turning_one_door_off_leaves_the_other_unchanged` | `ConnectorTls::for_state` ignores that the connector is off |
| Revoking one leaves the other alone | `revoking_one_leaves_the_other_alone` | Revoking a grant also signs out device tokens |

Two properties are covered by pure-function tests, not through the door, because the first needs a
process-wide environment variable that would race the other tests: that the development switch
(`HQ_MCP_DEV_NO_AUTH`) never opens the door, and that no `AGENTHQ_*` key resolves an identity. Both
call the real `mcp_dev_open` and `resolve_identity` on the headers the gate leaves behind.

The own-devices listener does not exist in this crate yet. `refuse_connector_paths(router)` is the
guard it must wrap its router in, and `is_connector_territory(path)` is the predicate behind it
(`/mcp`, `/oauth` and `/.well-known`, with or without a deeper path). The tests stand in a plain
TCP listener for it; the pairing work should point the same assertions at the real one.

## Not covered

- **No per-client limits on the device.** It cannot see client addresses. The forwarder must cap
  connections and bytes per device; the device caps itself (64 served connections, 256 in the
  hello, 16 requests in flight, 1 MiB bodies). The existing 120 calls a minute per grant still
  applies. A flood of silent connections can fill the hello and web slots for as long as it
  lasts; only the forwarder can tell the flood from clients.
- **A finalized order whose download times out** makes a new order and key next time, spending
  one of the five certificates a week (`TECHDEBT.md`).
- **The consent page on the public name.** Not served here. `authorize_url` may be plain http on
  `127.0.0.1`, `[::1]` or `localhost` (no other http origin parses), for a browser on the same
  device; the request's `Host` must equal that authority exactly, so the public name, another
  port and a DNS-rebound name never reach `/oauth/authorize`, and the request cookie then drops
  `Secure` (it keeps `HttpOnly`, `SameSite=Strict` and its path). A request carrying a
  reverse-proxy header is refused on that origin, because a proxy on the same machine can rewrite
  `Host` to the loopback authority; prefer `127.0.0.1` over `localhost`, which can resolve to
  `::1` where another local user may listen. Whether claude.ai sends a
  browser to an `authorization_endpoint` private to the device is the open test
  (`docs/plans/native-bridge/w6-claude-run.md`). If it refuses, the public-name consent page has
  to be added to the allow-list together with its own arming-window protections, a separate
  design and review.
- **The forwarder.** It sees who connects, when and how much, never the content. Its own limits
  and registration are its own document.
- **Certificate Transparency.** Every certificate for the name is public in CT logs, so the name
  of every device is public. The forwarder derives the label from the device's key id, which is
  not secret but is not a random label either.
- **Revocation of a certificate** (compromise of the device's key) is not wired. Delete the
  `.pem` and `.retry.json` files for the name in the state directory and restart: the next order
  issues a new key, and the running process keeps serving the old one until it restarts. Keep the
  account file for the directory in use, or the CA's account limit applies again. The old
  certificate expires on its own.
