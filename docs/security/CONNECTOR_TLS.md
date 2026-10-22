# The connector's own TLS

`docs/CLAUDE_CONNECTOR.md` used to need an ingress (Caddy, Tailscale Funnel, a tunnel) that ends
TLS for HQ. This is the other way: HQ ends TLS itself, for the connector's public name, on a
byte stream a public forwarder hands it. The forwarder (`hq edge`, built separately) reads the
TLS server name to choose a device and passes the bytes on. It cannot read them, and no key
is ever on it. Code: `crates/hq-web/src/tls/`. Off by default.

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
  tls:
    enabled: true
    acme_directory: letsencrypt                # letsencrypt | letsencrypt-staging | https URL
    contact_email: null                        # optional; the CA may write to it about the certificate
```

- `tls.enabled` needs `mcp_connector.enabled` and a `public_url` that is a domain name (not an
  address, not a single label). The certificate name is that host; there is no second setting.
- Refused under `profile: lite`, like the connector. `ConnectorTls::for_state` returns the
  reason and the door is not built.
- State is in `<vault>/_data/connector_tls/`: `account-<hash of the directory>.json` (the ACME
  account key) and `<name>.pem` (the certificate key and chain). The directory is `0700` and the
  files `0600`, created with that mode from the first byte and replaced by rename. Neither is
  ever logged or shown: the types that hold them print as nothing, and the status carries only
  the name, the expiry and a one-line reason.
- The module opens no listener. Whoever owns the stream source (the edge bridge) calls
  `ConnectorTls::serve(stream)` once per connection and spawns `ConnectorTls::run()` once.
  `run()` orders a certificate when there is none and again two thirds of the way through each
  one, and needs `serve` taking connections: the CA's validation arrives on it.

## What the forwarder must hand over

- The raw bytes of the client's TCP connection, from the first byte of the ClientHello, with
  nothing prepended (no PROXY header) and nothing stripped.
- `acme-tls/1` connections unchanged. The CA validates the device's name by connecting to it on
  port 443 offering only that ALPN; a forwarder that routes on SNI alone passes it through.
- One stream per client connection, closed when either side closes.

The device never learns the client's address. Abuse limits per client address therefore belong
on the forwarder; the device bounds itself (below).

## What the door does with a connection

| Step | Rule |
|---|---|
| ClientHello | Read before any certificate is chosen. A server name that is not the connector's, or none, is dropped. A hello with no certificate installed yet is dropped. |
| `acme-tls/1` | Offered alone, for the connector's name, with a challenge pending: the challenge certificate (self-signed, critical `acmeIdentifier` extension), handshake, then the stream is shut down without reading a byte of HTTP. With no challenge pending, dropped. Offered beside `h2` or `http/1.1`, it is a web client and gets the issued certificate. |
| Handshake | TLS 1.2 and 1.3 on rustls with the aws-lc provider, named explicitly because the process links two. ALPN `h2`, `http/1.1`. Hello and handshake share a 10 s limit; a peer that opens a stream and says nothing is closed. |
| Request | Request line and headers within 10 s; a request, body and handler together, within 120 s; a connection lives at most 15 minutes; HTTP/2 keep-alive pings, at most 32 streams. At most 64 connections at once; the 65th is dropped, not queued. |
| Gate | Before the router sees anything (see below). A refusal is a plain `404 Not found` whichever check failed. |

### The gate

Everything Caddy or a tunnel used to do in front of HQ happens here, on every request:

1. **Path allow-list**, byte for byte, no decoding or normalising: `/mcp`, `/oauth/authorize`,
   `/oauth/token`, `/oauth/revoke`, `/.well-known/oauth-protected-resource`,
   `/.well-known/oauth-protected-resource/mcp`, `/.well-known/oauth-authorization-server`. This is
   `deploy/Caddyfile.connector`'s list plus `/oauth/authorize`. (The sprint brief says "two
   `.well-known` paths": they are two documents; the protected-resource document is also served
   at its `/mcp` suffix, which clients try first.) `/oauth/authorize` is listed only so that the
   handler's own check can refuse it: it answers on the consent origin alone. The consent page
   (`/connect`, `/api/oauth/requests/*`) is not served here. Whether a public name serves it is
   decided with the connector-on-the-bridge work.
   `/mcp/`, `//mcp`, `/%6Dcp`, `/.well-known/../api/tree` and everything else are 404s.
2. **Host.** `Host` and, for HTTP/2, `:authority` must each be exactly the connector's name (with
   its port if `public_url` names one). Two `Host` headers, or a disagreement, is a 404. Without
   this a client could send the private consent origin's name through the forwarder and reach
   `/oauth/authorize`, whose own check trusts `Host`. The door also sets `Host` for HTTP/2
   requests that have none, because the handlers read that header.
3. **Credentials.** Only a live connector grant's access token goes on. Anything else a client
   presents (the owner's web token, a device token, an `AGENTHQ_*` API key, an agent's session
   token, in `Authorization` or `x-api-key`) is replaced by a token that matches nothing, so the
   handler answers `401 invalid_token` and Claude refreshes or starts over as it would anyway.
   Without this the door would accept a full-access API key from the internet.
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

**Flow.** Account (stored, one per ACME directory; a stored account that cannot be read is an
error, never a reason to register another) → order for the one name → TLS-ALPN-01 challenge armed
on the acceptor → `set_ready` → poll until ready → ECDSA P-256 key and CSR → finalize → chain →
the leaf is checked to be for that key → saved, then swapped into the acceptor without a restart.
Renewal starts two thirds of the way through the certificate's life, so the rule holds for 90, 45
and 6 day certificates. A restart serves the saved certificate at once. ARI (RFC 9773) is not used
yet (`TECHDEBT.md`).

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
| Failed validations per name per account | 5 per hour, refilling 1 per 12 minutes | The retry wait starts at 15 minutes (below). |
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
- No new account is created to get round a limit.
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

| Promise | Test | Mutation that makes it fail |
|---|---|---|
| The own-devices listener never serves connector paths | `the_own_devices_door_never_serves_connector_paths` | `refuse_connector_paths` returns the router unchanged |
| The connector door never accepts own-devices credentials | `the_connector_door_never_accepts_the_credentials_of_the_own_devices_door` | The gate leaves `Authorization` and `x-api-key` as they came |
| Turning one off leaves the other unchanged | `turning_one_door_off_leaves_the_other_unchanged` | `ConnectorTls::for_state` ignores that the connector is off |
| Revoking one leaves the other alone | `revoking_one_leaves_the_other_alone` | Revoking a grant also signs out device tokens |

The own-devices listener does not exist in this crate yet. `refuse_connector_paths(router)` is the
guard it must wrap its router in, and `is_connector_territory(path)` is the predicate behind it
(`/mcp`, `/oauth` and `/.well-known`, with or without a deeper path). The tests stand in a plain
TCP listener for it; the pairing work should point the same assertions at the real one.

## Not covered

- **No per-client limits on the device.** It cannot see client addresses. The forwarder must cap
  connections and bytes per device; the device caps itself at 64 connections. The existing 120
  calls a minute per grant still applies.
- **The consent page on the public name.** Not served here. If claude.ai refuses an
  `authorization_endpoint` private to the phone, the public-name consent page has to be added to
  the allow-list together with its own arming-window protections.
- **The forwarder.** It sees who connects, when and how much, never the content. Its own limits
  and registration are its own document.
- **Certificate Transparency.** Every certificate for the name is public in CT logs, so the name
  of every device is public. Name devices with random labels.
- **Revocation of a certificate** (compromise of the device's key) is not wired; delete the state
  directory and the next order issues a new key. The old certificate expires on its own.
