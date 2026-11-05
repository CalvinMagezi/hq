# Web wave 3 · 02 · HQ Hub: Devices, and the HQ window for your phone's HQ (WB3, Drift desktop 06, 11)

Boards: **AlignedHubDevices** ("pairing a phone or a computer"), **AlignedHubDevices2** ("the HQ window
for your phone's HQ", Devices at 320 px), **AlignedShell2** frame 6, **AlignedChat2** frames 4 to 6,
AlignedReview WB3 and M8, Drift desktop 06 and 11. Follows `../../rules-web.md` sections 3, 5 and 6 and
`../../rules-shared.md` section 4. Pairing words are the phone's (`PairingWords.kt` on the phone). Where
this spec and the rules differ, the rules win.

## 1. Finding and the review items it closes

- **Drift desktop 06.** Outcomes vanish after three seconds: wrong digits and tries left, Accepted,
  Revoked and "Added. Restart Claude to see it." appear only in a toast that clears after 3 s.
- **Drift desktop 11.** Confirm revoke stays armed: after one press Revoke turns into "Confirm revoke" and
  stays that way until the list changes; no cancel, no timeout.
- **WB3** (the Devices side): a forward another program holds says why it matters.
- **M8.** The HQ window is not a full browser: say what works there; key storage by HQ id.
- **D04** (partly closed): the hub cannot learn that the HQ window signed in, so the first-sign-in note
  closes by hand.
- **W38** ("Remove device"), approvals order (Decline left, Pair right).

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/desktop/ui/index.html:39-72` Devices: "Pair a phone or another computer with a QR code. Switching
  this on opens a network port for devices you have paired; nobody else is let in. Nothing listens until
  you do." [Switch on]; [Show pairing code] [Switch off]; [Copy link] [Cancel].
- `apps/desktop/ui/src/main.ts:285-289` outcomes as toasts: "Not the right digits. {n} tries left.",
  "Accepted. It shows in the list once the other device finishes.", "Too many wrong tries: the device was
  declined.", "Type the six digits.", "That device is no longer waiting."; `:306` "{n} tries left. Waiting
  until {time}."; `:312-338` `renderPeers()`: "Revoke" then "Confirm revoke" (324), "Revoked. Its open
  connections were closed." (332), the Claude warning in the error line.
- `crates/hq-desktop/src/pairing.rs:135, 501` `running_elsewhere` per forward; `crates/hq-desktop/src/desktop.rs:260`
  `revoke()` takes Claude's entries out first (`cleanup_peer_outside_bridge`, line 141).
- `crates/hq-desktop/src/desktop.rs:314-326` `main_window_url()`: a paired hub has its own token, so the
  person signs in to it once in the window; this machine's own token goes in the fragment.
- The Claude tab (`index.html:73-80`): "Adds the hub this computer is paired with to Claude Code and
  Claude Desktop. Remove takes out only what was added here." (moves to Helpers, `03-helpers-tab.md`).

## 3. Decision

**Devices pairs with the QR and six digits in the phone's words; every outcome stays on screen until the
next action; Remove asks once with a way out; a forward another program holds says so in the rules'
words. The HQ window says once what works there, and keeps the phone's HQ apart from this computer's.**

Rejected:

- **An armed second press ("Confirm revoke").** A dialog with Keep it paired is clearer and cannot stay
  armed.
- **Undo for Remove.** The other device was told at once; there is nothing to put back.

## 4. Screens and states

Same window and frame as This PC (940 by 700). Status line per `01-this-pc.md`.

- **V1 · Off.** Card "Pair a phone or another computer" / "Off" / "A device you pair can reach this computer
  directly, with no account to make. Switching on opens one network port. Only devices you pair get
  through." [Switch on] (the one primary). Nothing listens until switched on. If the port is taken:
  "Couldn't switch on pairing. Another program uses its port. Close it and try again."
- **V2 · On, nothing paired.** "Pairing is on" / "On" / "Paired devices know this computer as Studio PC ·
  3f9a1c2e. Only devices you pair here get through." [Show a pairing code] (primary) [Switch off]
  (secondary, asks nothing: nothing is lost). "Paired devices" / "No devices paired yet. Show a pairing
  code, then scan it with the other device." "Go-between server · None · Devices must reach each other
  directly, for example on one network." (no relay, n0 or VPN words).
- **V3 · A pairing code.** "Pairing code" / "Works once · until 14:41" / the QR and the link side by side
  / "On the other device, scan this code, or paste the link." / `hqpair://ticket/xxxxxxxxxxxxxxxx`
  [Copy link] [Stop pairing] (was "Cancel", `../../resolutions.md` B18). A spinner line "Waiting for the
  other device". Stop pairing ends the code; a new code replaces the old one. When it runs out, in words.
- **V4 · Six digits, after a wrong try.** Status "Connected · “Pixel 8” wants to pair". Amber Needs your OK
  card: "“Pixel 8” (9c41e0aa) wants to pair" / "Type the six digits it shows. It pairs only if they
  match." / field "Six digits on Pixel 8" / under it, amber: "Not the right digits. 2 tries left." /
  [Decline] (left) [Pair] (right) / "Waiting until 14:41. After the third wrong try, the device is
  declined." The outcome stays under the field. After the third wrong try: "The digits did not match, so
  nothing was paired. Show a new code to try again."
- **V5 · Paired, and what is shared.** "Paired with “Pixel 8” (9c41e0aa). Check that this is the device
  you meant." (stays until closed). "Paired devices 2": "Pixel 8 · Phone · 9c41e0aa · seen 3 min ago"
  [Remove] (quiet ink); "Mac mini · Computer · 51be07d4 · seen 1 d ago" [Remove]. "Shared with paired
  devices": "Helpers on this computer · Your phone's HQ can start helpers here · Shared"; "Your phone's HQ ·
  Opens in the HQ window on this computer" [Open]; "Your phone's web pages · HQ Hub opened this. The HQ window
  uses it." Technical details: "gate on this computer: 127.0.0.1:7747, opened by HQ Hub", "web of 9c41e0aa:
  127.0.0.1:47480, opened by HQ Hub".
- **V6 · Remove asks.** "Remove “Pixel 8”?" / "It loses access to this computer at once and its open
  connections close. HQ also leaves Claude Code and Claude Desktop here. You can pair it again later." /
  [Keep it paired] [Remove device] (red). Esc, the close button and a click outside mean Keep it paired.
  No Undo. Done: a toast "Pixel 8 removed." (B18) that stays until closed.
- **V7 · Removed, with a warning.** A toast kept until closed: "Pixel 8 removed. Couldn't take HQ out
  of Claude Desktop. Its settings file has a mistake." [Open Helpers] (opens that app on the Helpers
  tab). With the bridge not running: "Couldn't remove “Pixel 8”. Pairing is off. Switch it on and try
  again." With nothing paired: "Nothing is shared while no phone uses this computer."
- **V8 · A forward another program holds (WB3).** Scrolled down, the web row: "Another program holds this
  port. HQ Hub won't send your sign-in there." amber with an icon, tag "Port in use"; Technical details
  "web of 9c41e0aa: 127.0.0.1:47480, held by another program". "Your phone's HQ" cannot open until HQ
  Hub holds the port itself. Often the other program is an HQ pairing started in a terminal on this computer.
- **V9 · 320 px.** The pairing code: the QR above the link, the buttons wrap, nothing is cut off; the
  title and state in the top bar, the state wrapping.
- **Claude moved to Helpers.** The Claude tab is gone. Claude Code is one of the helpers, and Claude
  Desktop is in "Apps that use HQ", both on the Helpers tab; each one's detail lists the HQs it can
  reach, this computer's and the paired ones, with Connect to HQ and Remove the connection.

### The HQ window, showing your phone's HQ (1200 by 800, drawn at 1024)

- **W1 · Connected.** Window title "HQ" (" · HQ on your phone" in the title fact). The web app's shell;
  status line "Connected · HQ on your phone"; the balance chip (Connected). A closable card once: "This
  window shows HQ on your phone, Pixel 8" / "Notes, tasks, chats and helpers work here." / "Notices come
  from HQ Hub, not from this window." / "Files you export are not saved from this window. Use HQ in your
  browser for that." / "A saved copy is kept for this HQ only, apart from this computer's own HQ." The start
  screen's words match Chat: "What would you like to do?"
- **W2 · Signed out of this HQ.** "Sign in to see HQ on your phone" / "Nothing from it is shown until you
  sign in." [Sign in]. The sheet: "Sign in to HQ on your phone" / "On your phone, open Settings, then
  Other devices, then Sign in another device. Type the code it shows." / field "Code from your phone" /
  "This is a shared computer · Signed in for this window only. Nothing is saved." / "HQ never asks for
  this code in a chat or a call." / [Not now]. A phone's HQ has its own sign-in, so the window asks once
  (no token is put in the address for a paired hub).
- **W3 · The phone's HQ is out of reach.** Status "Your HQ can't be reached"; the strip "Saved in HQ Hub
  at 14:32. HQ on your phone can't be reached." (`../../resolutions.md` B11). "Editing waits until HQ on your phone answers. New
  notes wait to send." The phone is asleep, out of range, or Devices is off on this computer; it never says
  stopped. Reading stays on. No balance chip.

## 5. Exact copy

| Key | String |
|-----|--------|
| `dev_off` | "Pair a phone or another computer" · "Off" · "A device you pair can reach this computer directly, with no account to make. Switching on opens one network port. Only devices you pair get through." · "Switch on" |
| `dev_on` | "Pairing is on" · "On" · "Paired devices know this computer as {Studio PC} · {3f9a1c2e}. Only devices you pair here get through." · "Show a pairing code" · "Switch off" |
| `dev_none` | "No devices paired yet. Show a pairing code, then scan it with the other device." |
| `dev_relay_none` | "Go-between server" · "None" · "Devices must reach each other directly, for example on one network." |
| `dev_code` | "Pairing code" · "Works once · until {14:41}" · "On the other device, scan this code, or paste the link." · "Copy link" · "Stop pairing" · "Waiting for the other device" |
| `dev_wants` | "“{Pixel 8}” ({9c41e0aa}) wants to pair" · "Type the six digits it shows. It pairs only if they match." · "Six digits on {Pixel 8}" · "Decline" · "Pair" · "Waiting until {14:41}. After the third wrong try, the device is declined." |
| `dev_wrong` | "Not the right digits. {2} tries left." |
| `dev_declined` | "The digits did not match, so nothing was paired. Show a new code to try again." |
| `dev_paired` | "Paired with “{Pixel 8}” ({9c41e0aa}). Check that this is the device you meant." |
| `dev_row` | "{Phone} · {9c41e0aa} · seen {3 min ago}" · "Remove" |
| `dev_shared` | "Shared with paired devices" · "Helpers on this computer" · "Your phone's HQ can start helpers here" · "Shared" · "Your phone's HQ" · "Opens in the HQ window on this computer" · "Open" · "Your phone's web pages" · "HQ Hub opened this. The HQ window uses it." |
| `dev_remove_dialog` | "Remove “{Pixel 8}”?" · "It loses access to this computer at once and its open connections close. HQ also leaves Claude Code and Claude Desktop here. You can pair it again later." · "Keep it paired" · "Remove device" |
| `dev_removed` | "{Pixel 8} removed." (B18) |
| `dev_removed_warn` | "{Pixel 8} removed. Couldn't take HQ out of Claude Desktop. Its settings file has a mistake." · "Open Helpers" |
| `dev_remove_off` | "Couldn't remove “{Pixel 8}”. Pairing is off. Switch it on and try again." |
| `dev_port_taken` | "Couldn't switch on pairing. Another program uses its port. Close it and try again." |
| `dev_nothing_shared` | "Nothing is shared while no phone uses this computer." |
| `win_card` | "This window shows HQ on your phone, {Pixel 8}" · "Notes, tasks, chats and helpers work here." · "Notices come from HQ Hub, not from this window." · "Files you export are not saved from this window. Use HQ in your browser for that." · "A saved copy is kept for this HQ only, apart from this computer's own HQ." |
| `win_signin` | "Sign in to see HQ on your phone" · "Nothing from it is shown until you sign in." · "Sign in to HQ on your phone" · "On your phone, open Settings, then Other devices, then Sign in another device. Type the code it shows." · "Code from your phone" · "Signed in for this window only. Nothing is saved." · "HQ never asks for this code in a chat or a call." · "Not now" |
| `win_unreachable` | "Saved in HQ Hub at {14:32}. HQ on your phone can't be reached." · "Editing waits until HQ on your phone answers. New notes wait to send." |

Retired: "Revoke", "Confirm revoke", "Cancel" on the pairing code, "Revoked. Its open connections were closed.", "In use elsewhere",
"Accepted. It shows in the list once the other device finishes." (as a vanishing toast), the Claude tab.

## 6. Behaviour rules

1. **Outcomes stay** under the field or on the row until the next action (Drift 06). Toasts carry a close
   button and do not time out.
2. **Decline left, Pair right.** Only matching digits pair; three wrong tries decline.
3. **Remove asks once** (V6) and removes Claude's entries for that hub first (`desktop.rs:260`); a
   failure to remove them is the V7 warning, and the removal itself still stands.
4. **A held forward** is reported from `running_elsewhere`; the proxy refuses it (`../wave-0/01-live-bugs.md`).
5. **The HQ window's saved copy** is keyed by the HQ's id and labelled "Saved in HQ Hub at 14:32" (`../../resolutions.md` B11, confirmed by the owner, which replaces
   WR5's saved line); Waiting to send works there, keyed by the HQ's id too. The web group there is
   "This window"; no web push, Needs you arrives as HQ Hub's system notice.
6. **No push, no downloads, no install in the HQ window.** Export is off there with the reason (WR13);
   Copy link works (the clipboard, on a click) and Share is hidden (WR29); notices come from HQ Hub.
7. **The first-sign-in note** on This PC closes by hand; HQ Hub does not guess whether the window signed
   in.

## 7. Browser and platform notes

- The HQ window is a WKWebView on macOS with no download handler: a download would fail silently, hence
  rule 6. A future `save_file` command could lift it (WR13).
- Pairing over iroh needs the local network permission on macOS (`shell_config.rs:243`).

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/desktop/ui/index.html`, `ui/src/main.ts` | Devices V1 to V9 with `aligned-ui.css`; outcomes on the page (285-289); `renderPeers()` with the dialog (312-338). | yes |
| `crates/hq-desktop/src/pairing.rs` | `PairingSnapshot` carries the latest outcome and per-forward state for the page. | yes |
| `crates/hq-desktop/src/desktop.rs` | `revoke()` result words; the window's title fact. | yes |
| `apps/hq-web/src` | The HQ window card (W1) when `x-hq-window: hub` and the HQ is a paired one. | yes |

## 9. Server, desktop or release changes

Desktop only: snapshot fields, the dialog, the HQ window header. No new command here (`pairing_revoke`
exists); the HQ window's knowledge that it is inside HQ Hub comes from the proxy's `x-hq-window` header
(`../wave-1/06-settings-groups.md`).

## 10. Acceptance checks (each fails on today's build)

1. After a wrong six digits, "Not the right digits. 2 tries left." is still on screen after 5 s. Today it
   clears at 3 s.
2. Remove opens "Remove “Pixel 8”?" with Keep it paired on the left; Esc keeps it. Today the button arms
   itself as "Confirm revoke" and stays armed.
3. A held web forward shows "Another program holds this port. HQ Hub won't send your sign-in there." on
   the row. Today it shows "In use elsewhere".
4. In the HQ window with a paired hub, the status line reads "Connected · HQ on your phone". Today the web
   app does not know.
5. The pairing code's second button reads "Stop pairing"; removing a device ends with "Pixel 8 removed."
   (B18); the HQ window's strip reads "Saved in HQ Hub at {time}" (B11). Today the button reads
   "Cancel".

## 11. Check on a laptop or phone

- Pair a phone (`docs/PAIRING.md`): type wrong digits twice, look away, look back: the line is there.
  Remove it: the dialog, then the Claude entries are gone from Claude Code (`claude mcp list`).
- With the phone as the hub, open HQ: the card, then sign in with a code from the phone.

## 12. Open decisions it depends on

- **W38, WR5 (its saved line replaced by B11), WR13, WR29** (Recommended); `../../resolutions.md` B11, B18.
- Depends on: `01-this-pc.md`, `03-helpers-tab.md` (Claude moved there), `../wave-0/01-live-bugs.md`.
