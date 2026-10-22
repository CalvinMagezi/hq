# W4 measurements: the phone app as a pairing hub

Workstream W4 of `docs/plans/native-bridge-sprint.md`: `hq bridge` in the Android build, driven by the
app (`docs/HQ_MOBILE.md`, "The bridge on the phone"). Same format as the Spike F results
(`spike-f/README.md`): what was run, a table per question, then what the rows say and what is still
open. Dates are 2026-10-22. Hosts are placeholders; the ids are not recorded.

Setup. **Emulator**: Android 16 arm64 (`google_apis`), headless, behind the emulator's address
translation, with an interface for Wi-Fi (`wlan0`) and one for mobile data (`eth0`), both translated by
the Mac. **Phone**: a Samsung SM-A325F on Android 13, on a home Wi-Fi with LTE as the second network, USB
attached, the owner's live HQ app untouched (the test build installs beside it as `example.hq.spike.w4`
with its own data, built with `-Phq.appId`, and was uninstalled afterwards). **Peer**: `hq bridge serve`
from a debug build on a Mac, n0's public relays (`--relay default`, development only). The real app
process runs the bridge as a child of its foreground service, so these are not `adb shell` processes.

## 1. The `ndk-context` abort in the child process (PERSONAL-INBOX-099)

| Check | Result |
|---|---|
| `hq bridge pair --relay default` from the 2026-10-22 `release-small` Android build (no fix), emulator, run as a plain process | Aborted with SIGABRT (exit 134, a tombstone, no message on the terminal) right after the database opened, before any endpoint existed |
| Same command, build with the fix | Ran to the QR code, relay registered (`home is now relay ...`) |
| `hq bridge run` as a child of the app, on the emulator and on the phone, through the rest of the session (pairing, keeping a connection, switching networks, forced idle) | No abort |

What it says:

- **The spike's "prints a panic and carries on" was an artifact of its build.** The spike unwound on
  panic; `hq`'s release profiles set `panic = "abort"`. `ndk_context::android_context()` panics when
  nothing initialised it, `netdev` and iroh's DNS reader both call it, and both catch the panic with
  `catch_unwind`, which does nothing under abort. The first endpoint bind ended the process. (`netdev`
  hides its message with a silent panic hook, which is why the abort was silent.)
- **There is no JVM to give it.** A child process started with `ProcessBuilder` has none, so the context
  cannot be initialised for real. The fix has two parts: `hq` sets a null context on Android
  (`netdev` checks it and skips the JVM-only extras), and each bridge endpoint is built with a resolver
  that asks `getaddrinfo` and then iroh's public resolvers, so iroh's own JNI read is never made (with a
  null JVM it would abort too: `jni` 0.22 asserts the pointer). `getaddrinfo` also follows the
  network's DNS, private DNS and a VPN, which the public resolvers do not.

## 2. Pairing through the app

| Row | Result |
|---|---|
| Emulator shows a code, a Mac `hq bridge join` scans its text | Paired. The request reached the app 4 s after the Mac started joining; digits typed in the app; both sides listed each other |
| Mac `hq bridge pair` shows a ticket, link opened on the emulator with a VIEW intent (what the stock camera sends) | The confirmation screen opened. Pair, six digits on the phone, typed on the Mac: paired |
| Same on the phone (real app process, relay) | Confirmation screen, Pair tap to digits on screen in about 3 s (the bridge starting included), digits typed on the Mac, "Paired with" the Mac. The phone recorded the Mac and the Mac listed the phone |
| Wrong digits on the Mac | Declined, nothing paired, the phone showed "the other device did not accept the digits" |
| Ticket QR rendered by the app, read back by macOS CoreImage | Decoded to the same link. A 248 character link (relay, `--no-addrs`) is about half the size of the 375 and 540 character ones with addresses |
| Camera: CameraX preview and the ZXing analyzer on the emulator's virtual scene | The preview and analysis ran (no crash, frames flowing). **No QR code was in the virtual scene's view**, so a decode from camera frames was not seen. The decoder is covered by unit tests on generated codes with camera-style row padding |

What these say:

- **The link door is real.** A VIEW intent for `hqpair://ticket/...` opens the app on a screen that asks first.
  It did not dial anything until Pair was pressed.
- **Still to try by hand:** the stock Camera app handing a QR code to the app, and the in-app scanner on
  a real camera. Both need a person with the phone and a code on a second screen.

## 3. Network changes (PERSONAL-INBOX-097)

The peer holds one connection from the phone and echoes once a second (`keep`, a diagnostic that writes
`keep ts=... rtt=... path=...` to `files/bridge.log`). At t=0 Wi-Fi is switched off, so the default network
moves to mobile data; Wi-Fi comes back 40 s later. "Before" is the same build with the app told not to
report network changes (`--ez watch false`): no `network_change`, no probe. "After" is the app as shipped.
Silence is the longest gap between two successful echoes.

| Run | Wi-Fi to mobile data | Mobile data to Wi-Fi |
|---|---|---|
| Before, run 1 (emulator) | 17.1 s of silence, one echo took 16.1 s | 1.4 s |
| Before, run 2 (emulator) | 21.6 s of silence, one echo took 20.6 s | 1.5 s |
| After, run 1 (emulator) | 4.0 s: Android reported the new network 2.1 s after the switch, the probe failed, the connection was dropped, a new one was up (direct) 1 s later | 1.2 s |
| After, run 2 (emulator) | 4.0 s, the same sequence | 1.0 s |
| Earlier, `network_change` alone, on a memory-starved headed emulator | One echo took 7.5 s, 6.0 s and 9.7 s | 0.3 s |

What these say:

- **`Endpoint::network_change()` alone did not shorten the stall.** The three runs of it (on a slow
  emulator, so only a hint) stalled for 6 to 10 s, and the clean "before" runs, which never call it,
  stalled for 17 to 22 s; the clean runs do not isolate it. Treat it as harmless and cheap, not as the fix.
- **What cut the stall was dropping the old connection.** On a change that arrives while a call is
  waiting, `hq bridge run` sends a fresh probe on the same connection; if it is not answered within 1.5 s
  the connection is closed and dialed again. A new connection took about a second here. The transport's own
  recovery of the old one takes the 17 to 22 s.
- **Roughly 2 s of the 4 s is Android**: it reports the new default network that long after Wi-Fi is switched
  off. The app cannot act before it hears.
- **What this does not cover.** It is a client keeping a connection (phone to another device). When the
  phone is the one dialed (the edge, workstream W1, dials the phone by key), the edge's connection goes
  stale on its side and the phone has no connection to drop and redial. The phone can still close
  connections it holds when its network changes, which makes the other side dial again: not built, since it
  would cut a request in flight. Whether the edge should hold the connection the other way round (the
  phone dials the edge, which opens its streams on it) is a question for W1.

### 3a. On the phone

The same procedure on the Samsung (Wi-Fi switched off with `svc wifi disable`, the phone falls back to LTE),
with the keep connection running from the real app process to the Mac. This phone carries the owner's
VPN (a WireGuard split tunnel), so its default network is the VPN.

| Run | App build | Wi-Fi to mobile data | Mobile data to Wi-Fi |
|---|---|---|---|
| Before 1 | Network reports off | 17.6 s of silence | 1.4 s |
| Before 2 | Network reports off | 17.4 s | 1.4 s |
| First build 1 | Default-network callback | 17.4 s, no loss reported | 1.4 s |
| First build 2 | Default-network callback | 18.9 s, no loss reported | 1.4 s |
| After 1 | Physical-network callback, probe and redial | 4.6 s (probe failed 2.5 s after the switch, back on a direct path 0.5 s later) | 1.7 s |
| After 2 | Physical-network callback, probe and redial | 4.3 s (the same sequence) | 2.6 s |
| After 3 (final build) | Physical-network callback, probe and redial | 5.9 s (the connection was dropped twice, 2.4 s and 8.2 s after the switch: a second network update arrived while the new connection was settling) | 1.3 s |

What these say:

- **The spike's 18 s is reproduced on the phone, with and without the first version of the fix.** The
  stall was 17 to 19 s in all four runs that did not have the final build.
- **The first version never fired on this phone, and that is how a bug was found.** With a VPN up, Android's
  default network is the VPN, and a switch underneath it does not change it: `registerDefaultNetworkCallback`
  stayed silent (the app log showed one "network changed" at start and none at the switch). The emulator
  has no VPN, so it had passed. The callback now watches every internet network that is not a VPN, and fires
  on one coming, going or changing its addresses.
- **With it, the silence fell from about 17.5 s to between 4.3 and 5.9 s.** About 2.4 s of that is Android reporting
  the change, 1.5 s is the probe's limit, and the rest is the new connection.
- **The way back costs a little.** Mobile data to Wi-Fi went from 1.4 s to 1.7 and 2.6 s: the change also
  triggers a probe there, and a connection that was fine and slow for a moment gets dropped and dialed again.
  That is the price of not waiting 17 s the other way.

## 4. A sleeping phone: Doze and the foreground service

The sprint's first risk: a phone that is the only hub must answer Anthropic's discovery calls (about 10 s)
while it sleeps. Every row dials the phone cold from the Mac (a new endpoint each time) and reports the
connect time of the first `hq bridge ping` and the median round trip of three. The phone's screen was off
for rows E2 to E5. The app is the foreground service build: `example.hq.spike.w4`, no battery exemption.

| Row | State | Connect | Median round trip |
|---|---|---|---|
| E0 | Awake, app in front | 64 ms | 31 ms |
| E1 | App in the background, screen on, 30 s | 127 ms | 20 ms |
| E2 | Screen off 60 s, on the charger | 204 ms | 37 ms |
| E3 | `dumpsys battery unplug`, `dumpsys deviceidle force-idle` (deep idle, `mState=IDLE`), +10 s | 221 ms | 11 ms |
| E4 | Same, +5 min | 237 ms | 47 ms |
| E5 | Same, +10 min | 171 ms | 11 ms |

The state was put back afterwards (`deviceidle unforce`, `battery reset`, screen woken); Wi-Fi and mobile
data were left as found.

What these say:

- **Doze does not cut off a foreground service.** In deep idle the system's own view of the app
  (`dumpsys netpolicy`) was `procState=FGS, blocked=DOZE, allowed=FOREGROUND, effective=NONE`: Doze applies,
  and the foreground state lifts it. Other apps on the same phone read `effective=DOZE`. No battery
  exemption was needed for this.
- **Not measured: the CPU actually suspending.** Two things kept this phone awake: the USB cable, and a
  wake lock Samsung's ambient display held for as long as the screen was "dozing"
  (`DOZE_WAKE_LOCK` by `DreamManagerService`, visible in `dumpsys power`). The 11 ms round trips of E3 and
  E5 say the processor was running. A phone that has suspended wakes on an incoming packet from the radio
  and takes longer; how much longer, and whether a relay's idle connection survives hours of it, is
  not known.
- **Not measured: hours, other makers.** Ten minutes, one phone. Samsung's "sleeping apps" and similar
  vendor rules, and Android's app standby buckets over a night, are what the next row should cover.

**The state of the risk.** What was measured does not call for the fallback: Doze itself does not
silence the foreground service, and a cold dial in forced deep idle took under a quarter of a second against
a budget of about 10 s. It does not settle it either, because the case that matters (unplugged, display fully
off, an hour or a night) was out of reach with the phone on a cable. The fallback stays written down in
`docs/plans/native-bridge-sprint.md` until that row is run; it is W8's "in-app Doze" row.

To run it: unplug the cable (use `adb` over Wi-Fi or watch `files/bridge.log` afterwards), turn the ambient
display off for the test, leave the phone untouched, and from the Mac run `hq bridge ping <phone id>` at
+1 min, +10 min, +1 h and +8 h; record connect time and whether it answers at all. Compare with and without
"Unrestricted" battery use (the notice on the pairing screen opens the page).

## 5. Left open

- The stock Camera app and a real camera decoding a QR code in the app.
- The Doze row above, unplugged, with a fully dark display, for an hour and a night, on this phone and one
  other make.
- A carrier hotspot and public Wi-Fi (W8, unchanged).
- The probe-and-redial applied to connections the phone is dialed on (needs W1's stream first).
