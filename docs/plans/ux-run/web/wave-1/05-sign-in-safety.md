# Web wave 1 · 05 · Sign-in safety and device names (WB1, M7)

Boards: **AlignedFirstRun2** ("Signing in to HQ": the Sign in sheet, "This is a shared computer", a new
sign-in in Needs you, the code screen), **AlignedSettings** section 2 ("Other devices, a sign-in code
shown") and section 3 (Sign out this browser), **AlignedInbox2** section 4, **AlignedWebRules** section
3, AlignedReview WB1 and M7. Follows `../../rules-web.md` section 3. Where this spec and the rules
differ, the rules win.

This wave builds the safety parts inside today's screens: the sheet's shared-computer option, device
names and expiry, the new sign-in item, the code screen's warning, Sign out this browser. The full
sign-in redesign (the covers, the status line state) is `../wave-4/02-first-run-sign-in.md`.

## 1. Finding and the review items it closes

- **WB1 (blocker, the rest of it).** Device tokens can be made with no expiry; no "Sign out this
  browser"; no shared-computer sign-in that keeps nothing. (The wipe itself shipped in wave 0.)
- **M7 Sign-in codes and new devices.** "Type this only into HQ on your own device"; a new sign-in is a
  Needs you item with Sign it out.
- **WR10.** The new sign-in item uses "It was me" and a red "Sign it out".

Who it hurts: an owner who signs in once on a borrowed or shared computer, or whose code is phished
("send me the code you see"), or who loses a laptop. Today the sign-in lives forever and is named "Signed
in with a code".

## 2. Before (as built, verified at `e5f9d5f`)

- `crates/hq-web/src/device_auth.rs:24` `CODE_SIGN_IN_LABEL = "Signed in with a code"`; `:238` every
  code sign-in is minted with that label and `expires_at: None`; `:26` codes last `10 * 60` s.
- `device_auth.rs:248-255` the owner notice: "A new device signed in to HQ" / "A device signed in with a
  one-time code at {YYYY-MM-DD HH:MM} UTC. If it was not you, open Settings, Devices in HQ and sign it
  out." (an Updates-style notice with no button).
- `crates/hq-db/src/web_auth.rs:20-25` `DeviceToken {label, created_at, last_used_at, expires_at}`;
  `:90-93` "a device's sign-in never expires on its own"; `:15` `last_used_at` moves at most once per
  resolution.
- `apps/hq-web/src/components/settings/DevicesSection.tsx:57-78` the code and QR card;
  `:121-163` each device with Sign out; `:128` `window.confirm('Sign this device out? …')` only for the
  current one; `:168-171` the operator-link note (`operator_link_active`); `:179-209` Sign out
  everywhere.
- `apps/hq-web/src/lib/deviceSignIn.ts:53-66` `exchangeErrorMessage()`: "That code is wrong, already
  used or expired. Get a new one on the signed-in device." and others, with "(error {status})" for the
  rest.
- `apps/hq-web/src/components/SignInPrompt.tsx:116-138`: the banner; "This device was signed out, or the
  server no longer accepts its sign-in."
- `apps/hq-web/src/lib/hqAuth.ts:7` the token in localStorage for every sign-in; no session-only mode.

## 3. Decision

**Name every sign-in by browser, system and when; let sign-ins expire after 90 days without use; tell
the owner of every new one with a way to cut it off; warn on the code screen; offer a shared-computer
sign-in that keeps nothing; and let a browser sign itself out with a wipe.**

Rejected:

- **Expire every sign-in after a fixed time.** Owners who sign in once on their own phone would be
  signed out for no reason. "Without use" keeps active devices signed in.
- **Ask for a password on top of the code.** HQ has no account to hold one; the code and the paired
  device are the proof.
- **Name devices by IP address.** Changes with every network and means nothing to an owner.

## 4. Screens and states

### The Sign in sheet (today's `SignInPrompt`, centred from 768, bottom below)

- **S1 · Signed out.** Title "Sign in to HQ", line "Sign in to use HQ in this browser." Field "One-time
  code" with [Use code]. Under it: "Use a code only from your own signed-in device. HQ never asks for it
  in a chat or a call." Then "Get a code on a device that is signed in: Settings, Other devices, Sign in
  another device. On a phone, scan its QR code with the camera instead." A switch row **"This is a shared
  computer"** with "Signed in for this tab only. Nothing is saved here, nothing waits to send, no
  notifications. Signed out after 30 minutes idle." Then "or", "Your login link" field with [Sign in].
  Last line: "None of those? Send /login to your HQ bot in Telegram or Discord. If HQ runs on your own
  computer, run `hq web link` there. Otherwise ask whoever set up your HQ."
- **S2 · Wrong code.** Under the field, amber with an icon: "That code did not work. Codes last ten
  minutes and work once. Get a new one." The sheet stays open.
- **S3 · Signed out by HQ.** The sheet opens once by itself and leads with "This browser was signed out,
  or HQ no longer accepts its sign-in. Nothing from this HQ was kept here."
- **S4 · From a QR code.** `/vault#code=` (kept): "Signing you in" / "Using the code from the QR code.
  This takes a moment." then Chat. The code leaves the address at once (`hqAuth.ts:54-62`).
- **S5 · A browser that never signed in.** The full page (WR8) with "This browser has not opened your HQ
  before. Sign it in once and it stays signed in, unless it is a shared computer."
- **Widths.** 320: the sheet fills the width, the code field and Use code stack, the state wraps under
  the title; a 640 px window at 200% zoom looks the same. 768 to 1023 and 2200: centred, 640 wide; at
  2200 the rest stays empty. Lite: the same; once signed in the status line says "HQ Lite" and HQ opens
  on Notes. HQ window: sign-ins are kept by the HQ's id, so the computer's HQ and the phone's never mix;
  its sheet says "Sign in to HQ on your phone" / "On your phone, open Settings, then Other devices, then
  Sign in another device. Type the code it shows." with "Code from your phone", "This is a shared
  computer · Signed in for this window only. Nothing is saved.", "HQ never asks for this code in a chat
  or a call." and [Not now].

### Other devices (today's Settings › Devices)

- **D1 · The code card.** Title "Sign in another device", countdown "9:41 left". "One-time code"
  `K4M7Q-2XPA9` and its QR. "Works once, for 9:41. Enter the code on the other device." The warning,
  amber edge: **"Type this only into HQ on your own device. HQ never asks for it in a chat or a call."**
  "A device signed in this way is signed out after 90 days without use." [New code] [Copy code]. At zero:
  "The code ran out. Get a new one." with New code.
- **D2 · Signed in.** One row per device: "Firefox on Windows" / "New · signed in today at 14:20" (amber
  tag "New" with an icon) / [Sign it out]; "Chrome on macOS" / "Signed in 3 Nov · used just now"; "Safari
  on a MacBook Air" / "Signed in 1 Nov · used 3 h ago" / [Sign out]; "HQ on Pixel 8" / "Signed in 28 Oct
  · used 2 min ago · kept signed in". This browser's own row has no Sign out: "Sign out this browser" is
  its own row in This browser (it also wipes).
- **D3 · The rule line.** "A sign-in not used for 90 days signs out by itself. To keep one longer, open
  its menu and choose Keep signed in."
- **D4 · Operator link.** Only while `operator_link_active`: "The first sign-in link from whoever set up
  your HQ still works. Sign out everywhere turns it off."
- **D5 · Sign out everywhere.** "Every other device is signed out, and so is any sign-in link you made
  earlier. Claude connections end too. HQ restarts for a moment." Then "Pair a device by QR code on your
  phone or in HQ Hub. A paired device reaches HQ directly."
- **D6 · Sign out a device.** The red dialog: "Sign out {Safari on a MacBook Air}?" / "It stops reaching
  HQ at once. To use it again, sign it in with a new code." / [Keep signed in] [Sign out device]. Done:
  "{Safari on a MacBook Air} signed out." (a toast kept until closed, no Undo).
- **D7 · Sign out this browser** (Settings › This browser): row "Sign out this browser" / "Wipes what
  this browser saved". The red dialog, stacked (label over 18 characters): "Sign out this browser?" /
  "HQ wipes what this browser saved: notes, lists, drafts and messages waiting to send. Notifications
  stop here too." / [Keep signed in] on top, [Sign out this browser] below. Then Sign in only.

### A new sign-in, in Needs you

- **N1 · The card** (`.a-card.needs`): title "A new sign-in to your HQ", pill "Needs you". "Chrome on
  Windows signed in with a one-time code at 14:32 today." "If it was not you, sign it out now. Then make
  a new code only on your own device." [It was me] (left, secondary) [Sign it out] (right, red).
- **N2 · Sign it out asks once.** "Sign out Chrome on Windows?" / "It loses access at once. Anything it
  saved is wiped the next time it opens HQ." / [Keep it signed in] [Sign it out]. "Keep it signed in" is
  17 characters, so side by side. Back, Esc and a click outside keep it signed in.
- **N3 · After.** "Edge on Windows is signed out." HQ also lists it under Other devices as signed out.
- **N4 · It was me.** The card clears and the device row loses its "New" tag.
- **Lite.** The same card (Lite serves `/api/auth/devices` and the Inbox).

## 5. Exact copy

| Key | String |
|-----|--------|
| `signin_title` | "Sign in to HQ" |
| `signin_line` | "Sign in to use HQ in this browser." |
| `signin_code_label` | "One-time code" |
| `signin_use_code` | "Use code" |
| `signin_phishing` | "Use a code only from your own signed-in device. HQ never asks for it in a chat or a call." |
| `signin_where_code` | "Get a code on a device that is signed in: Settings, Other devices, Sign in another device. On a phone, scan its QR code with the camera instead." |
| `signin_shared` | "This is a shared computer" |
| `signin_shared_line` | "Signed in for this tab only. Nothing is saved here, nothing waits to send, no notifications. Signed out after 30 minutes idle." |
| `signin_link_label` | "Your login link" |
| `signin_else` | "None of those? Send /login to your HQ bot in Telegram or Discord. If HQ runs on your own computer, run hq web link there. Otherwise ask whoever set up your HQ." |
| `signin_wrong_code` | "That code did not work. Codes last ten minutes and work once. Get a new one." |
| `signin_too_many` | "Too many tries. Wait a few minutes, then get a new code." |
| `signin_signed_out_by_hq` | "This browser was signed out, or HQ no longer accepts its sign-in. Nothing from this HQ was kept here." |
| `signin_first_time` | "This browser has not opened your HQ before. Sign it in once and it stays signed in, unless it is a shared computer." |
| `signin_from_qr` | "Signing you in" · "Using the code from the QR code. This takes a moment." |
| `code_title` | "Sign in another device" |
| `code_left` | "{9:41} left" |
| `code_works_once` | "Works once, for {9:41}. Enter the code on the other device." |
| `code_warning` | "Type this only into HQ on your own device. HQ never asks for it in a chat or a call." |
| `code_expiry` | "A device signed in this way is signed out after 90 days without use." |
| `code_ran_out` | "The code ran out. Get a new one." |
| `device_name` | "{Browser} on {System}" ("Firefox on Windows", "Safari on a MacBook Air", "HQ on Pixel 8") |
| `device_line_new` | "New · signed in today at {14:20}" |
| `device_line` | "Signed in {3 Nov} · used {just now}" (· "kept signed in") |
| `device_rule` | "A sign-in not used for 90 days signs out by itself. To keep one longer, open its menu and choose Keep signed in." |
| `device_keep` | "Keep signed in" (menu item) |
| `new_signin_title` | "A new sign-in to your HQ" |
| `new_signin_body` | "{Chrome on Windows} signed in with a one-time code at {14:32 today}." · "If it was not you, sign it out now. Then make a new code only on your own device." |
| `new_signin_push` | "A new sign-in to your HQ" · "Open HQ to see it." |
| `it_was_me` | "It was me" |
| `sign_it_out` | "Sign it out" |
| `sign_it_out_dialog` | "Sign out {Chrome on Windows}?" · "It loses access at once. Anything it saved is wiped the next time it opens HQ." · "Keep it signed in" · "Sign it out" |
| `signed_out_done` | "{Edge on Windows} is signed out." |
| `sign_out_browser_row` | "Sign out this browser" · "Wipes what this browser saved" |
| `sign_out_browser_dialog` | "Sign out this browser?" · "HQ wipes what this browser saved: notes, lists, drafts and messages waiting to send. Notifications stop here too." · "Keep signed in" · "Sign out this browser" |
| `export_shared_warn` | "The file stays in this computer's Downloads" (full dialog in `../wave-4/03-notes.md`) |

Retired: "Signed in with a code" (as a device's name), "A new device signed in to HQ", "… at {date} UTC
… open Settings, Devices in HQ and sign it out.", "That code is wrong, already used or expired. Get a
new one on the signed-in device.", "Could not sign in (error {status}). Try again.", "This device was
signed out, or the server no longer accepts its sign-in.", "Sign this device out? You will need a code,
a login link or /login to get back in."

## 6. Behaviour rules

1. **Names.** At exchange, HQ stores the browser and the system from the `User-Agent` (parsed on the
   server into a short name: Chrome, Edge, Firefox, Safari, Samsung Internet, the HQ app; Windows,
   macOS, a MacBook Air where the platform says so, Linux, Android, iPhone, iPad) and the time. The
   phone app's own sign-in is "HQ on {its device name}". Never the raw user agent on screen.
2. **Expiry without use.** A code sign-in gets `idle_expires_after = 90 days`; `verify()` refuses a
   token whose `last_used_at` (or `created_at`) is older than that and deletes it. "Keep signed in" sets
   it to never for that device. The operator link and the configured token keep today's rules.
3. **A new sign-in is Needs you.** The exchange writes an `ActionNeeded` item (source
   `device-sign-in`) with the device id and the two actions. "It was me" resolves it; "Sign it out" asks
   (N2) and deletes the token (`devices.rs:84` `delete_handler`), its push subscription, and resolves the
   item. The push follows rules-web 1 (title and "Open HQ to see it.", Open only).
4. **The code screen** shows `code_warning` above the code at all times, amber edge.
5. **A shared computer.** The switch on the sheet is off by default. On: the token goes to
   `sessionStorage` (this tab only), the saved copy, outbox and push are off
   (`../wave-0/02-shared-parts.md` rule 10), the device is stored with `shared = 1` (Recommended, not
   drawn: its row reads "{Browser} on {System}, shared"), and the tab signs out after 30 minutes with no input, wiping as on a refused
   token. Every export asks first (`../wave-4/03-notes.md`).
6. **Sign out this browser** calls `DELETE /api/auth/devices/{own id}` (or `/api/auth/devices/current`,
   NEW), then wipes (`../wave-0/02-shared-parts.md` rule 9) and shows Sign in only.
7. **A refused code is not a refused sign-in.** A 401 from `POST /api/auth/exchange` shows S2 and wipes
   nothing; only a 401 on a request that carried this browser's token wipes.
8. **Sign out everywhere** keeps today's server behaviour (`devices.rs:129` `rotate_handler`: every
   device and the configured token, connector grants and push subscriptions; this browser keeps the one
   new token) and then wipes this browser's saved copy and push subscription before it loads again with
   the new token (rules-web 3).

## 7. Browser and platform notes

- User-agent reduction (Chrome 110+) hides the exact system version but keeps the platform; the name
  needs only the family. `navigator.userAgentData` is not sent to the server; parse the header.
- Safari on iOS and iPadOS reports "iPhone" and "iPad"; an iPad in desktop mode reports macOS, so its
  name reads "Safari on macOS". Acceptable.
- `sessionStorage` is per tab: a shared-computer sign-in does not survive a new tab or a restart, by
  design.
- The HQ window in HQ Hub: "This is a shared computer" keeps nothing after the window closes.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `crates/hq-web/src/device_auth.rs` | Name from the user agent; idle expiry; the Needs you item (248-255); the `shared` flag from the exchange body. | yes |
| `crates/hq-web/src/device_auth/devices.rs` | List returns name, system, created, last used, kept, new; `keep` and `current` routes. | yes |
| `crates/hq-db/src/web_auth.rs`, `crates/hq-db/sql/1xx_device_names.sql` | Columns `browser`, `system`, `shared`, `keep`, `idle_expires_after`; `verify()` applies the idle rule. | yes, migration NEW |
| `apps/hq-web/src/components/SignInPrompt.tsx` | The sheet of S1 to S5, the shared switch. | yes |
| `apps/hq-web/src/lib/hqAuth.ts` | Session-only storage when shared; idle timer; wipe on idle. | yes |
| `apps/hq-web/src/lib/deviceSignIn.ts` | `exchangeErrorMessage()` words (53-66); send `shared`. | yes |
| `apps/hq-web/src/components/settings/DevicesSection.tsx` | Code card warning and expiry line; rows by name; Keep signed in; dialogs. | yes |
| `apps/hq-web/src/components/notifications/NotificationCard.tsx` | The new sign-in card's two buttons. | yes |

## 9. Server, desktop or release changes

Server: device names, the idle expiry and Keep signed in, the Needs you item with Sign it out, the
`shared` flag (`../../changes.md` "Device sign-ins store browser, system, created and last used; expire
after 90 days without use" and "A new sign-in creates a Needs you item"). Desktop: the HQ window's sheet
words in wave 3. Release: `docs/security/WEB_AUTH.md` gains the expiry.

## 10. Acceptance checks (each fails on today's build)

1. `cargo test -p hq-web device_auth`: exchanging a code with a Firefox on Windows user agent lists the
   device as "Firefox on Windows". Today it is "Signed in with a code".
2. A token last used 91 days ago is refused by `verify()` and removed; one with `keep` is accepted.
   Today both are accepted.
3. The exchange writes an `ActionNeeded` item whose actions resolve or sign out. Today it writes a plain
   notice with no actions.
4. `bun test`: with "This is a shared computer" on, the token is in `sessionStorage` and not in
   localStorage, and no IndexedDB database is opened. Today there is no such switch.
5. A 401 from the exchange route leaves the stores in place. (Guards rule 7; passes today, must keep
   passing after wave 0.)
6. The code card contains "Type this only into HQ on your own device." Today it does not.

## 11. Check on a laptop or phone

- Sign a second browser profile in with a code, ticking "This is a shared computer"; close the tab;
  reopen: signed out, nothing stored. Leave a shared tab idle for 30 minutes: signed out.
- Sign Firefox in: Chrome's Inbox shows "A new sign-in to your HQ" with "Firefox on …"; Sign it out:
  Firefox shows only Sign in on its next request.

## 12. Open decisions it depends on

- **WR8, WR10** (Recommended). **M7**, rules-web 3.
- Depends on: `../wave-0/01-live-bugs.md` and `../wave-0/02-shared-parts.md` (the wipe),
  `02-dialogs-approvals.md` (dialogs), `06-settings-groups.md` (This browser group),
  `../wave-4/02-first-run-sign-in.md` (the covers and the status line state).
