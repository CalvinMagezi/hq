# Research behind the private network decision

Gathered 2026-10-18 for [private-network.md](private-network.md). Three independent reads of
primary sources. Anything marked UNCONFIRMED could not be confirmed from a primary source and is
not relied on. A model summarised some fetched pages, and some pages were truncated, so treat
exact figures as leads to re-check before they go into a release note.

## A. Making a web UI reachable only by admitted devices

Findings that carried the decision:

- WireGuard answers nothing to a packet from an unknown key (wireguard.com/protocol). It has no TCP
  mode, so a network that blocks UDP needs an outside wrapper (wireguard.com/known-limitations).
  Keepalive is off by default (wireguard.com/quickstart). Official clients: Android is Apache-2.0
  (github.com/WireGuard/wireguard-android), iOS and macOS are MIT (github.com/WireGuard/wireguard-apple).
  wireguard-go is MIT. boringtun says it is "undergoing restructuring", so prefer the kernel module or
  wireguard-go. No core CVE found for 2023 to 2026; the one kernel entry found (CVE-2024-26950) is on
  an aggregator only.
- NetBird self-hosted needs a domain, public TCP 80 and 443 and UDP 3478, with an embedded login
  (docs.netbird.io/selfhosted/selfhosted-quickstart). Its relay races QUIC and a WebSocket on 443, so
  blocked UDP is covered (docs.netbird.io/about-netbird/understanding-nat-and-connectivity). Server
  AGPL, mobile apps GPL-3.0 (github.com/netbirdio/netbird, /android-client). Advisories: a client
  privilege escalation GHSA-qcpp-8vwj-hhwr in July 2026, and CVE-2025-10678 for an install-script
  default admin password.
- Headscale needs no account and has an embedded relay over HTTPS (headscale.net/stable/ref/derp/),
  but clients are Tailscale's apps; Android is BSD-3, the iOS GUI is closed (github.com/tailscale/tailscale).
- Nebula's lighthouse needs a stable public UDP 4242 (github.com/slackhq/nebula); CVE-2025-62820 and
  CVE-2026-25793; no audit found.
- Tor v3 onion services with client authorisation have no inbound port, but an Android Tor Browser
  report from October 2024 (forum.torproject.org/t/15024) shows client-authorised services failing with
  no fix found; iPhone needs Orbot (MIT) plus a browser app; six denial-of-service CVEs landed in May 2026.
- Caddy client certificates (caddyserver.com/docs/caddyfile/directives/tls): scanners complete TLS and
  see the certificate-logged name; Safari picks keychain identities; behaviour of an installed PWA
  with client certificates on iOS and Android is UNCONFIRMED.
- Cloudflare Tunnel with Access is closed SaaS, needs an account, and ends TLS at the vendor.
- OpenZiti example router configs listen on public 443 and its console had CVE-2025-27501; Yggdrasil
  makes every node routable (yggdrasil-network.github.io/faq.html); I2P and SSH-tunnel phone paths
  are UNCONFIRMED or need extra apps.

Trusted HTTPS for a private address, without Tailscale's names:

- Let's Encrypt DNS-01 needs no public web server, allows wildcards, and cannot cover IP addresses
  (letsencrypt.org/docs/challenge-types/). Pointing a public name at a private address: no rule found
  against it; the discouragement is for loopback names in shipped apps
  (letsencrypt.org/docs/certificates-for-localhost/). The CA/Browser Forum position is UNCONFIRMED.
- The costs: a public A record exposes the private address, and certificate logs expose names
  (tailscale.com/kb/1153/enabling-https describes the same trade-off). A wildcard certificate or
  split-horizon DNS avoids the name leak.
- Delegating `_acme-challenge` by CNAME keeps the server's DNS credential narrow
  (letsencrypt.org/docs/challenge-types/). Caddy has DNS plugins.
- sslip.io and nip.io do not work: the TXT record cannot be set, HTTP-01 cannot reach a private
  address, and neither suffix is on the Public Suffix List, so users share one 50-per-7-days limit
  (letsencrypt.org/docs/rate-limits/, publicsuffix.org/list/public_suffix_list.dat).
- A private CA is poor for strangers: iOS needs a profile and manual trust
  (support.apple.com/en-us/102390); Android shows a persistent warning.
- `localhost`, 127.0.0.0/8 and `*.localhost` are secure contexts, so a laptop helper needs no
  certificate (developer.mozilla.org/en-US/docs/Web/Security/Secure_Contexts).
- Web Push does not ride the tunnel; iOS 16.4 and later needs a Home Screen web app
  (webkit.org/blog/13878/web-push-for-web-apps-on-ios-and-ipados/).

## B. Reticulum

Verdict: borrow ideas only.

- Model: a transport-agnostic encrypted mesh; destinations are 16-byte hashes; the initiator of a link
  is anonymous by design, so a listener cannot refuse an unpaired peer at the handshake. The only
  pre-link gate is IFAC, one shared network name and passphrase per interface
  (reticulum.network/manual/understanding.html, /networks.html). Per-device checks happen after the
  link exists. A path request for a local destination makes the node announce it
  (RNS/Transport.py).
- Implementations: the Python reference (rns 1.5.7, 5 October 2026) is the only endorsed full stack;
  Rust, Go and C++ ports exist with different licences and uneven compatibility. Clients: Sideband
  (Android, Linux, macOS, Windows), Columba (Android); no first-party iOS client found.
- Licence: commit e7dacee on 2025-04-15 replaced the MIT text with the Reticulum License, adding a ban
  on use in systems with the ability to purposefully harm humans and a ban on use in creating,
  training or developing AI or machine-learning models. It is not on the OSI list, and OSD criterion 6
  bars field-of-use limits (opensource.org/osd). The clauses travel with every copy, so bundling or
  depending on it undercuts an unrestricted MIT promise. This is a reading, not legal advice.
- Cryptography: Ed25519, X25519, HKDF and Fernet-style tokens (AES-256-CBC with HMAC-SHA256), no
  AEAD. No external audit or published cryptanalysis found. A severe rnsh flaw fixed in 1.3.9 had its
  details withheld.
- Fit: the maintainer discourages tunnelling TCP (github.com/markqvist/Reticulum/discussions/290);
  MTU 500 bytes; no maintained HTTP or WebSocket forwarder found; a browser needs a JavaScript stack
  plus a bridge, which is another public listener.

## C. Pairing and iroh (for the deferred optional transport)

- Corrections to the question as asked: RFC 9497 is an OPRF, not CPace (CPace is still an IRTF draft,
  draft-irtf-cfrg-cpace); SPAKE2 (RFC 9382) and OPAQUE (RFC 9807) are Informational. No RUSTSEC
  entries exist for the iroh crates; the advisories are GitHub ones.
- Pairing design that came out of it, kept for later: a pairing string with the server's endpoint id
  and a 16-byte single-use secret, ten minute lifetime, a pairing ALPN registered only while a window
  is open, five failed proofs close the window, a proof binding both ids and the TLS exporter, a
  six-digit comparison on both screens and a fingerprint prompt that defaults to No. TOFU is wrong
  for a channel that gives code execution.
- iroh facts: three GitHub advisories on 2026-10-05, fixed in 1.1.0 (a relay frame that spins a
  core, a panic in `CustomAddr` deserialisation, and ULA addresses treated as mapped). The QUIC stack
  noq forked Quinn and inherits Quinn advisories (RUSTSEC-2026-0037, -0185, and six filed on
  2026-10-12) that `cargo audit` cannot see because they are filed against `quinn-proto`. TLS 1.3 with
  Ed25519 raw public keys; 0-RTT is accepted server-side but a client only sends it if the application
  asks, and early data is replayable. `incoming_filter` runs before TLS but there is no default
  filter. A relay sees identities, timing and volume, not content.
- Key storage: the Secure Enclave does not hold Ed25519 keys; Keychain items are bound to
  code-signing access groups, which is another reason stable signing matters; on Linux the kernel
  keyring is per user and a 0600 file is readable by any same-user process.

## D. Phone and store facts (Spike D)

Android 93.09 percent and iOS 6.86 percent of mobile in Uganda (StatCounter, September 2026); small
data bundles about UGX 2,500 to 3,250 per GB (news reports, August and September 2026, not an
operator page). Apple Developer Program $99 a year; Google Play $25 once.
