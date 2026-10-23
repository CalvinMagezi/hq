# Native Bridge sprint: the phone as ground zero, claude.ai included

Goal: download the app and it works, with the phone as the first HQ. Scanning a QR code adds another
device, and as long as the phone is online Claude can reach it. No WireGuard, no Tailscale, no account.
This plan answers the open decisions from `docs/plans/zero-setup-pairing.md` and bundles the claude.ai
connector into the same cutover.

## The one thing that cannot be wished away

Claude reaches a connector from Anthropic's servers, never from the device, and the help article says
so for every client including the mobile apps. A phone on mobile data holds only a carrier address
(measured: private `10.x` addresses on its cellular interfaces), so nothing on the internet can open a
connection to it, and Anthropic's own tunnel option is an Enterprise research preview. "Claude connects
to my phone without bouncing anywhere" is therefore not possible; Anthropic is always one hop. What is
possible: no server the user must run or pay for, TLS ending on the phone, and in between one small
public forwarder that passes encrypted bytes and cannot read them. The iroh relay cannot be that
forwarder, because it only carries iroh traffic. The forwarder is a new component, `hq edge`.

## Decisions

| Question | Answer | Whose call |
|---|---|---|
| Relay default | Direct first, relay URL one config value. Measured: every network but UDP-blocked went direct. n0's own docs now say the public relays are rate limited and not for production, so they are the development default only and replacing them is a release gate. The edge hosts an `iroh-relay` too, so a household runs its own | Owner: who pays for or runs the relay at release |
| Who reviews pairing | The owner signs off the sprint. Gate before anyone outside the household installs: a second independent review of the written-up protocol and implementation, plus one outside human security reviewer for pairing and the connector | Owner names the human reviewer |
| Connector scope | Bundled in this sprint (the owner's call), through `hq edge` and TLS on the device, reusing the merged OAuth server (`crates/hq-web/src/oauth/`) unchanged. The Mobile profile already allows it; only Lite refuses | Decided |
| First platforms | Android as the hub, macOS desktop app (Tauri) as the first desktop, the Linux `hq` binary for servers (it can also be the edge). iOS later: it gives no background inbound connections, so an iPhone cannot be a hub, and iroh on iOS loses network-change detection. Windows through WSL2 as today | Decided |
| Who runs the forwarder | Decided (2026-10-22): the owner runs `hq edge` on a public machine they control, invite-only (a device registers by scanning the edge's QR), off by default in the app. A project-run open endpoint stays rejected: it is the "R4" option under a new name (abuse, cost and a hosted default). Third parties (Tailscale Funnel, Cloudflare Tunnel) stay guided options for people with an account | Decided |

## Shape

```
claude.ai (Anthropic servers)
   │ HTTPS, SNI <device>.<edge domain>
   ▼
hq edge (public machine): reads the TLS server name only, never decrypts
   │ one iroh stream per connection, to the registered device
   ▼
device (the phone): rustls ends TLS, hq-web serves /mcp, the OAuth endpoints and two .well-known paths only
```

- Door 1 (own devices) stays iroh, dial by key, one-time QR pairing, as built in Spike F. Door 2 (the
  connector) uses the same iroh link only as a carrier from the edge to the device. The doors keep
  separate credentials and the regression tests from the pairing plan stand: the bridge never serves
  connector paths, the connector never accepts bridge credentials, turning one off leaves the other.
- The device gets a certificate for its own name through ACME, answered through the same passthrough,
  so no key ever sits on the edge. Let's Encrypt's per-domain limits cap how many devices one edge
  domain can enroll per week; fine for a household, a ceiling for an open service.
- Consent: the existing armed window and request binding. The consent page is served by the device on
  the public name while armed. Whether claude.ai accepts an `authorization_endpoint` that is private
  to the phone (loopback or an in-app page) is unverified; slice 0 proved only a private https name.
  Test it first (W6). If refused, the public-name consent stands, with the documented caveat that a
  stranger can watch for the armed window.
- Registration with an edge uses the pairing protocol (a second ALPN), so there is one mechanism to
  review, not two.

## The risk to settle first

A phone as the only hub must answer Anthropic's discovery calls (about 10 s) while it sleeps. The
sleep row measured so far proved little: the phone was charging and the process ran as the `adb` shell
user. W4 measured the real app process as a foreground service
(`docs/plans/native-bridge/w4-measurements.md`, section 4).

**Result (2026-10-22, partly settled).** Doze does not cut off a foreground service: in forced deep
idle the system's own view of the app was `effective=NONE` where other apps read `DOZE`, and a cold dial
from a second machine connected in 0.17 to 0.24 s at +10 s, +5 min and +10 min, against a budget of
about 10 s. No battery exemption was needed. What this does **not** show is the processor suspending:
the phone was on a USB cable and Samsung's ambient display held a wake lock, so it never slept. The
unplugged, dark-display, hour-and-night row is left to W8 ("in-app Doze"), with the steps written in the
measurements file. Until it is run, the plan does not promise "phone only".

**The fallback, specified.** If an unplugged phone cannot answer in time, an always-on paired machine
carries the connector and the phone stays ground zero for everything else:

- The edge registers the always-on machine (a desktop with the Native Bridge or the Linux `hq`
  binary) as the device behind the connector name, instead of the phone. Registration is the same
  exchange, scanned on that machine.
- `hq-web` on that machine serves the connector's `/mcp` and OAuth paths as the phone would have, so
  nothing in W2 changes. It reaches the phone's notes and tasks over the bridge when the phone is awake and
  answers from its own vault copy when it is not.
- The Connect Claude screen on the phone then shows the address of the machine, and arming happens on
  the machine's consent page (or the phone's, over the bridge: a W6 decision).
- Cost: the connector is only as available as that machine, and the vault copy has to be kept in step
  (a sync question outside this sprint, so the first version would serve what that machine already holds).

## Done means (the fresh-install run)

1. Install the app on a phone with no other app on it. HQ is running on the phone.
2. Show a QR on the macOS app or any second machine's `hq`, scan it with the stock camera (it opens
   the app, because the payload is a link), type the six digits. Paired, direct or by relay.
3. Register the phone with an edge by scanning the edge's QR.
4. Press "Connect Claude" in the app. It shows a connector URL; add it in claude.ai, approve in the
   app. Claude lists the tools and runs `hq_discover` with the phone as the only host.
5. Switch the phone from wifi to mobile data: Claude's next call works within the time budget.
6. Revoke the device: the connection is cut.
Steps 2 to 6 run with no WireGuard and no Tailscale installed. The WireGuard path (`hq net`,
`hq-tunnel`) stays documented as the power-user option, not the default. The live split tunnel on the
test phone is left alone until step 1 to 6 pass on a fresh install; removing it is the cutover.

## Workstreams

| # | Work | Depends on |
|---|---|---|
| W1 | `hq edge`: SNI passthrough on 443, iroh stream to the registered device, invite-only registration, co-hosted `iroh-relay`, per-device caps, systemd and Docker, `docs/security/` entry, off by default | none |
| W2 | In-process TLS and ACME on the device for the connector paths, path filter in `hq-web`, two-doors regression tests | W1 interface (a fake edge is enough to start) |
| W3 | Pairing as a product: spike into a crate, throwaway key per ticket, per-source limits, paired list in the database with pushed revocation, `hqpair://` link payload, protocol write-up with attack notes, review round 2 | none |
| W4 | Android: iroh in the `hq` child process (fix the `ndk-context` panic), QR scan, link handling, report network changes and retry, foreground service and wake policy, Doze measurement, "Connect Claude" screen | W3 |
| W5 | macOS app (Tauri): show QR, permissions, host lifecycle, loopback proxy, "Connect Claude" | W3 |
| W6 | Connector on the bridge: `public_url` from the edge name, consent in the app, the private-`authorization_endpoint` test against a real claude.ai account, end to end run | W1, W2, W4 |
| W7 | Cutover: defaults, rewrite `JOIN_A_MACHINE.md` and `CLAUDE_CONNECTOR.md`, demote `PRIVATE_NETWORK.md`, the fresh-install run on phone, Mac and an edge | all |
| W8 | Measurements still open: carrier hotspot and public wifi rows, in-app Doze, edge load | W4 |

Critical path: W1, W2, W6, W7. W3 starts at once and feeds W4 and W5.

## Not in this sprint

iOS, Windows native, an open project-run edge, DCR in the connector, removing `hq-net` and `hq-tunnel`.

## What is verified and what is not

Verified: Anthropic calls connectors from its cloud for every client; MCP tunnels are an Enterprise
research preview; n0's public relays are not for production; the OAuth server is merged and the Mobile
profile allows it; pairing and reconnects measured on a real phone (Spike F README). Not verified: that
claude.ai accepts a phone-private authorization endpoint; that a sleeping phone answers in time;
ACME through the passthrough end to end; the edge under load.
