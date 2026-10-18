# Native bridge: HQ without a Tailscale requirement

Status: validation only. Nothing in the product changed. The code under
`native-bridge/spike/` is a throwaway proof, not a start on the implementation.

Evidence labels used below: **[spike]** measured by running the spike,
**[repo]** read in this repository, **[web]** from search summaries (the
sandbox this was written in could not reach the primary docs for iroh, Tailscale
or Tauri, so treat these as leads), **[inference]** reasoning, not tested.

## The three questions

1. Does it make sense to reduce HQ's dependence on Tailscale?
2. Can HQ run its own bridge between a server and a laptop that only paired
   devices can use, and let HQ start sessions on the laptop?
3. What do a Tauri desktop app and a mobile app add, given the web app exists?

## Short answers

1. **Yes, as "Tailscale becomes optional", not "HQ rebuilds Tailscale".** The
   case is onboarding, coupling and blast radius. It is not that Tailscale is
   insecure, and a custom bridge is not automatically safer than WireGuard.
2. **Yes at the protocol level, shown on real HQ code [spike].** The existing
   host gate runs unchanged over an iroh connection, in both directions. What is
   still unproven: real NATs and relays, macOS, Tauri, iOS and Android.
3. **The desktop app earns its place on pairing, permissions and host
   lifecycle, not on the session UI, which already exists.** The mobile app is
   the most expensive piece and the least necessary. Do it last, after the
   spike for it below.

## What Tailscale does for HQ today [repo]

| Job | Where | Replaced by |
|---|---|---|
| Reachability between server, laptop and phone | HQ reaches a laptop with `ssh` to its tailnet address (`agent_host/transport.rs`); the join code holds that address (`agent_host/pairing.rs`, `tailscale_ip`) | The bridge |
| HTTPS front door for the web app | `tailscale serve --https=8443` in `deploy/hetzner/hq-join.sh`; `web_allowed_origins` | A loopback proxy inside the desktop app. Browsers keep needing Tailscale or a public origin |
| Address book and access list for a paired machine | Join code, `from=` pin in `authorized_keys` | Pairing by endpoint key |

What it costs a new owner (`docs/plans/owner-onboarding.md`, dead ends 1, 12, 14
and the Stage 0 checklist): a second account with MagicDNS and HTTPS
certificates switched on, a VPN app on every device, node keys that expire after
180 days, and a failure mode HQ cannot explain when the VPN is off. For a
laptop there is one more: `openssh-server` has to run (`docs/WINDOWS.md`), and on
macOS that means Remote Login, which no HQ doc mentions.

Points in Tailscale's favour, so the decision is made with them in view:
the free tier is not a cost problem [web: reported as 6 users and unlimited
devices since April 2026], and WireGuard is far better tested than anything HQ
will write. The security argument for the bridge is **a smaller blast radius**.
A tailnet address gives network reach to everything listening on the laptop
(within ACLs), and the paired ssh key is "shell as you" (`hq-host/src/gate.rs`
says so). The bridge exposes the gate allowlist, 23 methods with `host.stop`,
`agent.report` and unlock left out, to one named key. That is narrower, but the
pairing and key handling become code HQ owns.

## What the desktop app is for

The Sessions page already has a new-agent dialog with a computer picker, a
folder picker, a live terminal, a blocked-agent callout and Stop all
(`apps/hq-web/src/components/sessions/`). So "a visual interface instead of the
terminal" is mostly built. What still needs a terminal:

- pairing a machine: `hq host join`, `hq host add`, `hq host authorize`,
  `hq host check` across two machines;
- installing and restarting the host as a login service;
- macOS folder access and the self-signed signing workaround in
  `docs/AGENT_HOST.md` (a session blocked because macOS denied a folder is a
  known failure);
- seeing and allowing hosts the sandbox's egress proxy denied.

So the app's jobs are: **pair** (code or QR, no terminal), **permissions**
(guide the folder grants, list denied egress hosts with an allow button),
**host lifecycle** (install, status, restart when `binary_stale`),
**notifications** for a blocked agent, and a **window onto the existing web
app**. A Developer ID signature would replace the self-signed keychain trick
[inference: whether macOS then attributes folder approvals to the app for the
agents it spawns needs testing]. It should not re-implement the web UI.

## What was validated

`native-bridge/spike/` starts a real `hq_host::Host` and `Server`, puts the
unmodified `gate_parse_request` and `gate_forward` behind an iroh protocol
handler, and drives it from a second iroh endpoint with its own key. Two
shapes were run on Linux over loopback QUIC.

| # | Claim | Result [spike] |
|---|---|---|
| A1 | HQ dials the laptop by public key, no IP, ssh or Tailscale | Authenticated connection up in about 2 to 3 ms |
| A2 | The existing gate works unchanged over it | `host.status`, `agent.spawn` (a real PTY), `agent.read` (returned the agent's output), `agent.send_text` all worked |
| A3 | One connection carries many calls | A parked `events.poll` did not block a status call; the event arrived |
| A4 | The allowlist holds | `host.stop`, `agent.report`, `host.unlock` denied |
| A5 | An unpaired key cannot call | Closed by the laptop. Note the QUIC handshake itself completes for any key; reject earlier with iroh's incoming filter before this ships |
| B1 | **Inverted:** the laptop dials out, HQ is never told its address, HQ calls down that connection | Worked, same calls |
| B2 to B4 | Allowlist and unknown-device refusal in the inverted shape | Held |
| 0 | Overhead | 47.9 ms per call with no transport at all, 50.3 ms through the bridge: about 2.5 ms |

The 50 ms floor is not the transport. It is `ACCEPT_POLL` (50 ms) in the host's
accept loop, `crates/hq-host/src/server.rs:33`, hit because `gate_forward`
opens a new socket per call. The "about 40 ms for the gate" in
`docs/AGENT_HOST.md` is this. Fixing it (blocking accept, or one held client
connection in the bridge) speeds up every remote call whatever transport
carries it, and is worth doing on its own.

**Not shown:** real NATs, CGNAT, UDP-blocked networks, relay fallback; macOS
(the spike ran on Linux); Tauri; iOS and Android; sleep and wake and network
changes; many agents at once; any security review.

## Facts about iroh

Verified here: latest on the crates index is 1.3.0, minimum Rust 1.91 (this
sandbox has 1.97, check what CI and release builds use); licence
`MIT OR Apache-2.0`; its own QUIC stack (`noq`) with rustls and ring; builds
and runs cleanly. Added to the lockfile: 112 crate names HQ does not have.
`deny.toml` needs one addition, `CDLA-Permissive-2.0` (`webpki-roots`); the one
MPL-2.0 crate (`attohttpc`) is already allowed. Some majors will sit beside
HQ's: `ed25519-dalek` 3 against 2, `curve25519-dalek` 5, `reqwest` 0.13 against
0.12.

Reported, not verified [web]: 1.0 shipped on 2026-06-15 with a wire and API
stability promise; roughly 90 to 95 percent of traffic goes direct; Swift and
Kotlin bindings exist; the public relays are for development and testing and
throttle at 5 MB/s; each self-hosted relay handles about 60,000 connections;
relay support for pre-1.0 clients ends on 2026-09-30.

## Options compared

| Option | What it is | For HQ | Against |
|---|---|---|---|
| A. Keep Tailscale, remove friction | Script `tailscale up`, self-host Headscale for the control plane | Days of work, no new crypto | Still a VPN app and an account on every device; Headscale clients are unofficial [web] |
| B. Embed Tailscale (`tsnet`) in the apps | App-level node, no VPN permission | Keeps Tailscale's NAT traversal | Go in a Rust and Tauri app, still needs a control plane, heavy |
| **C. iroh bridge, hub and spoke** | Dial by key, QUIC streams, optional relay | Rust, fits the crates, proven on the real gate, mobile bindings reported | New pairing and key code to own; browsers cannot use it |
| D. WireGuard in userspace plus own control plane | Rebuild Tailscale | Full IP-level network | Largest effort, most risk, nothing HQ needs |
| E. NetBird or Nebula | Another mesh product | Self-hostable | Separate clients, not native to HQ, licence boundaries to check [web] |
| F. A tunnel service in front of the web app | Third-party edge | Easiest for web access | Public edge, another vendor, opposite of the goal |

## Recommendation

**Option C, as a hub and spoke, with Tailscale kept as a supported path.**

Why a star and not a mesh: every HQ flow is device to HQ. Devices never talk to
each other. The HQ server has a public address, so a device can always dial it,
which takes hole punching off the critical path [inference, to be confirmed on
real networks]. A relay becomes a fallback for networks that block UDP. That is
a much smaller thing than Tailscale, and the part worth building.

Three flows:

1. **Host calls.** The laptop dials HQ and HQ calls down the same connection
   (B1 above). Add a `Transport::Iroh` variant beside `Ssh` in
   `crates/hq-tools/src/agent_host/transport.rs`. That module is synchronous
   and spawns a process per call, so it needs an async facade or a held
   connection.
2. **Agent to HQ's MCP.** A loopback listener on the laptop forwards to HQ's MCP
   port through the bridge, and `agent_host.agent_mcp_url` points at it. The
   host's egress proxy already allows the configured MCP endpoint
   (`crates/hq-host/src/egress.rs`) [inference: confirm in a test].
3. **App to web UI.** The desktop app runs a loopback proxy to HQ's web port and
   the WebView loads that. The bridge must set `X-Forwarded-For` to the device,
   or the code-exchange rate limit sees every caller as loopback
   (`docs/security/WEB_AUTH.md`).

Pairing and keys:

- Reuse the shape of the join code and the device-code rules in `WEB_AUTH.md`:
  one time, ten minutes, constant-time compare, a global attempt cap. The code
  binds the two endpoint keys. The join code never holds a bearer secret.
- Private keys live in the OS keychain, never in `config.yaml`. Revoking a
  device removes its key from the paired set and closes the live connection.
- Keep the existing rules: unlock is local only, identity comes from the
  runtime and never from the caller.

What "private" means here, said plainly: HQ listens on one UDP port, or runs a
relay. Unpaired peers get nothing and are closed at the handshake, but the port
is reachable. A relay-only variant leaves HQ with no inbound port at all, and
trades that for a relay HQ must also host or rent. Tailscale has the same
property (nodes listen, DERP is public). Also, a plain browser cannot use the
bridge, so every device needs an HQ app, which is parity with Tailscale today.

### Phases

| Phase | Scope | Size | Exit criteria (proposed) |
|---|---|---|---|
| 1. Bridge in the CLI | `Transport::Iroh`, pairing, incoming filter, MCP forward, revocation, the `ACCEPT_POLL` fix, the `deny.toml` line | Medium | A Mac pairs with a Hetzner HQ with no sshd and no Tailscale; a session starts from the Sessions page; a 24 h soak survives laptop sleep and wake and a network change; threat model written and reviewed |
| 2. Desktop app (Tauri) | Pair, permissions, host lifecycle, notifications, loopback web proxy, signed and notarized, update channel | Medium to large | A non-terminal user pairs and starts a session in under 5 minutes; permission failures are visible in the app |
| 3. Mobile | Only after spike D | Large, uncertain | See spike D |
| 4. Tailscale decision | Keep, demote to advanced, or retire `hq-join` | Small | Decided from Phase 2 data, not before |

Existing deployments use `tailscale serve` and `hq-join`; none of this should
break them.

## Spikes that need your hardware

- **B. Mac and Tauri.** Build iroh and `hq-host` on macOS, run a pane under
  `sandbox-exec`, wrap the web app in a Tauri shell, sign and notarize, and see
  whether folder approvals stick to the app. Not Mac App Store: the app starts
  arbitrary agents, so it cannot be sandboxed.
- **C. Real networks.** Run the spike's two roles on a home laptop and a phone
  hotspot against a VPS. Record direct versus relayed, round-trip time, and what
  happens with UDP blocked. This is where the 90 to 95 percent claim is tested
  on the networks that matter. [web: no source describes carrier NAT behaviour
  for East African mobile networks, so measure it]
- **D. iOS and Android.** Does iroh build for both targets, can a WebView reach
  the app's loopback proxy (iOS 17 adds proxy configuration for `WKWebView`,
  with reported bugs [web]), and how does a blocked-agent alert arrive when the
  app is not running? iOS will not hold a background connection, so push through
  APNs is required. The installed web app already gets Web Push on iOS, so the
  mobile app must beat that to be worth building.

## What to measure

Time to first remote session; share of pairings finished without a terminal;
"can't reach HQ" reports; and how often each of owner-onboarding dead ends 1, 12
and 14 occurs. Without a baseline for these, a mobile app cannot be justified.

## Open decisions

1. Is public UDP on the HQ server acceptable, or is a relay-only variant
   required?
2. Who hosts the relay, if one is used: the HQ server, or a small separate box
   in a nearer region?
3. Is the Apple Developer Program fee acceptable? Without it there is no stable
   signature and no notarization.
4. Should desktop updates reuse `hq-update`'s signed channel, or Tauri's updater?
5. Who reviews the pairing design before Phase 1 ships?
