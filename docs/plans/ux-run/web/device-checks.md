# Device checks for the web and desktop fixes

Every item here needs a real browser, a real window or a real phone, beyond the tests each spec lists. Run
the items for a wave before the wave is handed over, write the result (pass, fail, the value found) next to
the item in the PR that builds the wave, and paste Copy details from This browser › Data used › Technical
details ("What HQ noticed") for each browser you used. Where a check also has an automated test, the test
is in the spec; the device check proves the browser does what the test assumes.

## The matrix

| Where | Why | Notes |
|-------|-----|-------|
| Chrome (current) on macOS or Linux | The owner's main browser; push, install, shortcuts | Also Edge on Windows for Lite. |
| Firefox (current) | Push with actions on Windows and Linux; no `beforeinstallprompt`; Ctrl K goes to its search bar | Private window as a stand-in for "no storage kept". |
| Safari (current) on macOS | WKWebView engine (the same as the HQ window); IndexedDB eviction after 7 days | Notifications need macOS 13+. |
| Safari on an iPhone, from the Home Screen | Push only from the Home Screen (iOS 16.4+); share sheet | And once outside the Home Screen for the "Add HQ to your Home Screen first" card. |
| Chrome on an Android phone (installed web app) | Lock-screen privacy of web push; the installed app's name and shortcuts | A Galaxy A32 class phone matches the phone specs' matrix. |
| HQ Hub's HQ window (macOS) | No push, no downloads; two HQs on one address | With this computer's HQ, and with a phone as the hub. |
| HQ Hub's control window (macOS) | Every wave 3 state | 940 by 700 and maximised on a large screen. |
| Windows 11 with WSL2 (Ubuntu 24.04) | Windows users (`docs/WINDOWS.md`); the WSL frame (future scope W50) | Edge and Chrome; forced colours (Contrast themes). |
| Windows without WSL, HQ Lite | The Lite edition | Edge. |
| A second browser profile, signed in as a shared computer | "This is a shared computer": nothing kept, 30-minute idle sign-out | Chrome profile or a Firefox container. |
| A phone as the hub (the Android app with HQ on it) | "HQ on your phone" in the HQ window and in a browser | Pair with HQ Hub's Devices or `hq bridge` (`docs/PAIRING.md`). |
| A 320 px viewport and 200% zoom | Rules-web 8 | Chrome device mode 320 by 640; a 1280 by 800 window at 200%. |

## How to set up

- **HQ for testing:** `./scripts/install-hq.sh` (once: `sudo ./scripts/install-hq.sh --link`), then
  `hq web --json` and open `login_url`. For web work in progress: `cd apps/hq-web && bun run dev` (port
  4747, proxies to hq on 5678). `./scripts/install-hq.sh --check` says whether the running build is stale;
  never compare `--version`.
- **A second HQ for "two HQs on one address":** a phone as the hub, or a second `hq` with its own
  `HQ_CONFIG_PATH`, vault and port, paired as a hub.
- **HQ Hub:** `cd apps/desktop && bun install --frozen-lockfile && bun run build:ui && cd src-tauri && cargo
  run` (or `bun run build` for `HQ.app`). Settings in `~/.hq/desktop/settings.json`; the host log at
  `~/.hq/desktop/host.log`.
- **Push as posted:** Chrome `chrome://gcm-internals` (receive log) and `chrome://serviceworker-internals`;
  Firefox `about:debugging` › This Firefox › Service Workers; on the Android phone `adb shell dumpsys
  notification --noredact`.
- **Storage:** DevTools › Application (Chrome) or Storage (Firefox): IndexedDB names (`hq-query-cache:{id}`,
  `hq-store:{id}`, `hq-outbox:{id}`), Cache Storage (`hq-api:{id}`), localStorage (`hq-web-token`,
  `hq-noticed`), sessionStorage (shared computer, embedded flag).
- **Network:** DevTools › Network with "Offline", "Slow 3G", and request blocking of `/ws` (Reconnecting)
  or `/api/hello` (can't be reached). Stop HQ with `hq web stop` for the real thing.
- **Forced colours:** Windows Contrast themes, or Chrome DevTools › Rendering › Emulate CSS media feature
  `forced-colors: active`.
- **Screen readers:** VoiceOver (Safari, macOS and iOS), NVDA (Firefox or Chrome on Windows), TalkBack on
  the Android phone.

## Wave 0 · live bugs and shared parts

| Check | Spec | How to perform it |
|-------|------|-------------------|
| A push shows no content and only Open | `wave-0/01-live-bugs.md` A | Chrome on Android, installed, push on, screen locked: have a helper ask to run a command. The lock screen shows "Claude needs your OK" / "Open HQ to see what it wants to do." and no Allow or Decline. Expand the notification and the history: no command. Repeat on desktop Chrome and Firefox. |
| An old service worker gets no buttons | 01 A5 | Before updating the web build, keep a tab with the old worker; send a push from the new server: no buttons appear. |
| Reply alerts say only "HQ answered" | 01 rule 6 | Ask a question, switch tabs before it finishes: "HQ answered" / "Tap to read." |
| The hub proxy refuses a held port | 01 B | macOS: run `hq bridge serve` in a terminal so it holds the web forward, open HQ Hub: Open HQ is off with "Another program holds this port. HQ Hub won't send your sign-in there." Confirm with `lsof -i :47480` that no request from the HQ window reached the other process. |
| A refused sign-in wipes everything | 01 C | Sign a laptop out from another device's Other devices. Reload the laptop with DevTools Offline: only Sign in, and Application shows no `hq-*` store or `hq-api` cache. Repeat in Safari (WKWebView) and in the HQ window. |
| Another tab follows the wipe | 01 C3 | Two tabs open; sign out from the first: the second shows Sign in within a second. |
| Unguarded routes refuse | 01 D | From a page on another origin (a local `python3 -m http.server` page with a `fetch` POST to `/api/harness-sessions/{id}/drive`): refused. The web app's own Drive or answer still works. |
| The seven connection states in words | `wave-0/02-shared-parts.md` | Stop HQ, start it, block `/ws`, go offline, stop the AI service's key: each state's words appear in order, wrap at 320 px, never "stopped". |
| Two HQs never mix | 02 rule 7 | In the HQ window, open this computer's HQ, then the phone's: each shows only its own saved notes after going offline. |
| The outbox never sends twice | 02 rules 11 to 13 | Airplane mode, send two messages, flip back: "Sending 1 of 2", and HQ's thread has exactly two messages. Kill the tab mid-send, reopen: still exactly two. |
| Safari keeps the saved copy | 02 notes | Safari, installed to the Dock or not: leave HQ unopened for a week (or clear website data): the place shows "Nothing saved in this browser yet", not an error. |

## Wave 1 · rules across the existing app

| Check | Spec | How to perform it |
|-------|------|-------------------|
| Status line at every width | `wave-1/01-shell-status.md` | 320, 390, 900, 1440, 2200 and 200%: the state first, wrapping, never ellipsed; the chip only while Connected. |
| Forced colours | 01 | Windows Contrast theme: every state reads in words; chips and cards keep their borders. |
| Contrast | 01 rule 8 | Lighthouse accessibility audit on Home and Tasks: no contrast failures for body text. |
| Dialog order and Esc | `wave-1/02-dialogs-approvals.md` | Keyboard only, every red dialog: safe left, action right, Esc keeps, Tab cycles inside; VoiceOver and NVDA read the title first. |
| Stacking at 200% and with large text | 02 rule 1 | Chrome at 200% zoom and with Settings › Appearance › Font size "Very large" (root 24 px): long-label dialogs stack, safe on top. |
| A losing approval is red on the right | 02 | A helper asks to delete a file: the Inbox card shows `vault_delete` and its arguments, Decline left, red "Allow delete" right. |
| Undo across devices | `wave-1/03-undo.md` | Archive a chat on the laptop; within 10 minutes press Undo on the phone (or the other way): restored on both. After 11 minutes: "Couldn't undo. It changed on another device." |
| Toasts never cover the field | 03 rule 8 | 200% zoom, composer focused, archive a chat from the list: the toast sits above the composer. |
| Plain words | `wave-1/04-plain-words.md` | Read every place once with a fresh vault: no "agent", "session", "native", "claude-code", "ago ago". 12-hour locale: "2:32 PM". |
| Shared computer | `wave-1/05-sign-in-safety.md` | Second profile, sign in with "This is a shared computer": sessionStorage holds the token, IndexedDB is empty, push is not offered; close the tab: signed out; idle 30 minutes: signed out. |
| New sign-in item | 05 | Sign Firefox in with a code: Chrome's Inbox shows "A new sign-in to your HQ" naming "Firefox on {system}". Sign it out: Firefox shows Sign in on its next request. |
| 90 days without use | 05 rule 2 | Server test only (time travel); on device, check the row reads "used {when}" and Keep signed in toggles. |
| Settings groups | `wave-1/06-settings-groups.md` | 390: four groups; 1440: This browser › Notifications open; `/settings#keys` lands on keys. |
| Notifications page per browser | 06 | Chrome, Firefox, Safari: turn on from the Inbox card, then from Settings; Send a test shows "Notifications from HQ are on in this browser." Block the site: the blocked card. iPhone outside the Home Screen: the Home Screen card. |

## Wave 2 · routes and editions

| Check | Spec | How to perform it |
|-------|------|-------------------|
| Old links keep working | `wave-2/01-route-table.md` | Tap an Inbox item made before the release (`/sessions?id=…`), a reply alert (`/chat?thread=…`), a printed device QR (`/vault#code=…`) with a phone camera, and a login link (`/#token=…`): each lands on its new place and signs in where it should. |
| The installed app's name and shortcuts | 01 | Chrome desktop and Android, installed before the release: within two days the name is AgentHQ and the shortcuts are Chat, Notes, Inbox. Reinstall to check at once. |
| Embedded pages | `wave-2/02-embed-and-lite.md` | The phone app's Usage, Board and Timeline: one set of bars, the phone's own. |
| Lite | 02 | Windows without WSL, Lite installed (`docs/WINDOWS.md`), Edge: three places, Notes first, "Connected · HQ Lite"; `/chat` opens Notes with "This HQ is HQ Lite. It has no Chat."; the gear shows This browser and Other devices only. |
| A newer HQ | `wave-2/03-version-and-data.md` | Keep a tab open with a draft, update HQ: "A newer HQ is running. Reload to keep going."; Reload keeps the draft. |
| The data budget | 03 | Chat idle for one hour with DevTools Network open (preserve log): under 1 MB. Chat first load (cache disabled): under 300 KB transferred. A hidden tab for an hour: no requests. Record the numbers in the PR. |
| The phone pays | 03 rule 10 | Phone as the hub, the HQ window idle for an hour: the phone's data counter and This window's "Counted on your phone" agree within 10%. |
| Time zones | `wave-2/04-time-zones.md` | DevTools › Sensors › Location to another zone (or change the laptop's zone): times move, Spending names UTC and the local time, a scheduled task names HQ's city. |

## Wave 3 · HQ Hub

| Check | Spec | How to perform it |
|-------|------|-------------------|
| First open and a missing program | `wave-3/01-this-pc.md` P1, P10 | Open AgentHQ from the Dock with no `hq` installed: "Couldn't find HQ on this PC". Choose a program in `/tmp`: refused. Install HQ, Look again: "HQ is off". |
| Starting, stopped by progress | 01 P2, P3, P8 | Start HQ on a large vault: the steps and "Usually about {n} s on this PC". `kill -STOP` the process during start: after twice the usual time, "Stopped at: {step}". `kill` it: "HQ has stopped" and Open the log shows the last lines, keys blanked. |
| Stop HQ asks | 01 P6 | Stop HQ: the dialog; Esc keeps it running. |
| Started outside HQ Hub | 01 P9 | `hq start all` in a terminal, then open HQ Hub: no Stop button, the why line. |
| The phone as the hub | 01 P11 to P14 | Pair the phone: "Start helpers here"; the first-sign-in note; a held port. |
| Lockdown and lifting it | 01 P15, P16 | From the phone, Stop every helper on this computer with a reason: the card quotes it; Allow helpers again runs `hq host unlock` (check `~/.hq` lockdown file gone); the web shows no lift button. |
| The system notice | 01 P5 | A helper on this computer asks: a macOS notice "Claude needs your OK" with Open only; Open lands on that helper in the HQ window. |
| Devices outcomes stay | `wave-3/02-devices.md` | Pair with wrong digits twice, look away for 10 s: the line is still there. Remove: the dialog; Claude Code's entry is gone (`claude mcp list`). |
| Helper checks are safe | `wave-3/03-helpers-tab.md` | With Claude Code, Codex and Cursor installed: first open asks; Check this PC; Activity Monitor never shows more than two helper processes; a second open runs none. Turn the network off: checks still pass (no network needed). |
| Try it never answers a trust prompt | `wave-3/04-helper-detail.md` D3 | Cursor installed, no trust given: Try it reports "Needs you on this PC" and offers the line to copy; Antigravity: Try it answers its trust prompt (spec.rs) and says so. |
| Remove a connection with Undo | 04 D6 | Remove Claude Code's HQ entry, Undo within 10 minutes: `~/.claude.json` is byte for byte as before. |
| Window names and look | 01, `wave-3/05-commands-and-platforms.md` | The Window menu lists "HQ Hub" and "HQ"; the Dock shows AgentHQ; both windows are dark with Clear Sans. |
| Sizes | 01 sizes | 320 px wide window (minimum allowed is 640 by 480 today: resize the content with the Tauri dev tools zoom), 200% text (macOS display scaling "Larger Text"), maximised on a 2200 px screen. |
| Windows and Linux (future scope) | 05 | Only when W50 is confirmed: the same table on Windows 11 with WSL2 and on Ubuntu desktop. |

## Wave 4 · the places

| Check | Spec | How to perform it |
|-------|------|-------------------|
| Chat: a dropped link mid-reply | `wave-4/01-chat.md` | Airplane mode on the laptop mid-reply for 10 s: "Reconnecting", then the reply goes on; HQ shows one turn. |
| Chat: Stop with the socket gone | 01 rule 6 | Block `/ws` mid-reply and press Stop: the turn ends (HTTP stop). |
| Chat: input methods | 01 rules | Japanese IME in Chrome and Safari: Enter commits the word; Enter again sends. A touch-screen laptop or an iPad: Enter adds a line. |
| Chat: blocks and sheets | 01 4.8, 4.9 | A reply with a wide table and a chart at 390 and 1440; Expand; Copy the table into a spreadsheet (tab separated). Export a note as PDF from the note sheet; on a shared computer the ask appears first. |
| First run | `wave-4/02-first-run-sign-in.md` | A fresh HQ: `/setup`; wrong key refused in plain words; Open HQ without checking; `/connect` from claude.ai (with a public test HQ) shows Decline left. |
| Notes: two places edit one note | `wave-4/03-notes.md` X1 to X4 | Edit the same note on the laptop and the phone; save the phone first, then the laptop: "This note changed somewhere else."; See their version; Keep both makes "{title}, your version". |
| Notes: browser zoom is the browser's | 03 rule 7 | Ctrl plus on a note: the whole page zooms; text size only from More. |
| Notes: print | 03 widths | Print a note: black on white, no shell, links print their note title. |
| Search | `wave-4/04-search.md` | Ctrl K from every place in Chrome, Firefox (does not open Firefox's search bar), Safari; VoiceOver reads a combobox; Back from a result reopens Search once. |
| Inbox | `wave-4/05-inbox.md` | Answer a helper on the laptop while the phone shows the same item: the phone says "Already answered…". Clear all updates, Undo from the phone. |
| Tasks | `wave-4/06-tasks.md` | Keyboard only: pick three with Shift and click, mark done, Undo; Board: move a card with M and the arrows; Timeline: Month remembers. On a phone, the list shows before verification. |
| Helpers | `wave-4/07-helpers.md` | Opening a helper streams nothing until Technical details opens (Network panel). Typing to the helper: Esc reaches the helper, Ctrl ] leaves. Stop every helper with one computer asleep: "That computer can't be reached, so its helpers were not stopped." |
| Settings | `wave-4/08-settings.md` | Every page at 320, 900 and 1440; with HQ stopped (saved values, buttons off with why); signed out (Sign in only). |
| Spending | `wave-4/09-spending.md` | The chip from every place on a phone; a test limit past 100%: "HQ stopped using AI"; delete a limit and Undo. |

## A shared computer, every wave

Run once per wave in the shared-computer profile: nothing appears in IndexedDB, Cache Storage or
localStorage except per-browser conveniences; no push prompt; exports ask first; after closing the tab,
reopening shows Sign in.

## Windows via WSL, every wave from 2

On Windows 11 with HQ in Ubuntu (WSL2, `docs/WINDOWS.md`), open the web app in Edge and Chrome on Windows
at `login_url`: the status line, Settings and the places behave as on macOS; forced colours on; the installed
app is named AgentHQ in the Start menu.

## A phone as the hub, every wave from 2

With the Android app running HQ and paired to the laptop: open the web app in a laptop browser (through the
pairing) and in HQ Hub's HQ window. The status line reads "Connected · HQ on your phone"; saved copies stay
apart from the laptop's own HQ; Data used counts the phone's data; the HQ window offers no push and no
export (with the reason).
