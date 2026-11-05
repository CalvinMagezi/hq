# Decisions log

Every product and design decision behind the UX run, for phone, web and the desktop hub. Each has a
status:

- **Recommended**: accepted unless the owner says otherwise. Build it as written.
- **Owner to confirm**: needs a yes from the owner before you build the part that depends on it. Ask
  for these in one batch when you reach the wave that needs them.
- **Revised**: changed after the adversarial reviews; the new text is the one to build.

`resolutions.md` settles the last cross-platform conflicts and wins over this log; then this log; then `rules-shared.md` and `rules-web.md`; then the specs.

## Phone: original decisions (D)

| Id | Decision | Status |
|---|---|---|
| D1 | The backup row is amber only when the page offers something that clears it (Save a copy now): backups only on this phone, failed, overdue. Otherwise grey. | Revised |
| D2 | Dialogs: side by side, the safe choice on the left, the action on the right (red for a losing action). Stack only when a label is over 18 characters or the font scale is 1.5 or more; then the safe choice is on top. | Revised |
| D3 | Sign out everywhere asks once, in one red dialog. | Recommended |
| D4 | The early-builds switch is "Get test versions", with the line "You can go back only when a regular version is newer." | Recommended |
| D5 | The pairing screen says, in plain words, that a free public server may carry the connection. | Recommended |
| D6 | Owners who upgrade learn about the assistant from the Settings row and the release notes only. | Recommended |
| D7 | The spoken mark comes from a `source: voice` field that HQ's runtime sets from the connection (never the model), stored on the chat message and shown on the web too. Never used for permissions. | Revised |
| D8 | Questions that spend data or money have two equal outlined buttons, "Wait for Wi-Fi" first, the size in the other label. | Revised |
| D9 | The assistant never answers while the phone is locked. | Recommended |
| D10 | Update notices: once per version, one reminder at 7 days, daily only when the signed channel file marks it urgent. | Recommended |
| D11 | Installing without Android's confirm stays off until safe mode, the idle gate and a kill switch exist; never on test versions. | Revised |
| D12 | Release notes: three short lines in the signed channel file, by language. | Recommended |
| D13 | "Not now" on the Inbox notifications banner hides it for 7 days. | Recommended |
| D14 | Paid-data downloads have no "don't ask again"; after one, "Used about {N} MB of mobile data". | Recommended |
| D15 | Start steps reported by hq itself come later; "stopped" is judged by progress now. | Recommended |
| D16 | AI service status comes from a signed-in route (`/api/status/ai`), never `/health`. | Revised |
| D17 | The saved copy is kept until replaced and always shows its age; what came from a server HQ is cleared on server sign-out, Disconnect or a refused sign-in; data-extraction rules keep it out of phone-to-phone transfer. | Revised |
| D18 | Stopping a task inside its 24-hour finishing window from the phone: next round. | Recommended |
| D19 | Folders sort by last edited once the folder listing has edit times. | Recommended |
| D20 | Search finds helpers; the Inbox stays out of Search. | Recommended |
| D21 | A native Timeline comes later, after Board, if owners use it. | Recommended |

## Phone: decisions from the correction pass (N)

| Id | Decision | Status |
|---|---|---|
| N1 | Messages sent while HQ is away are held in the chat up to 24 h, in order, each removable, never sent twice. Send stays on. | Recommended |
| N2 | Saved copy size: 256 KB per place, 2 MB in all, measured on a 2,000-note vault in wave 0. | Recommended |
| N3 | Searching a server HQ's notes from the phone: wave 4, as its own "On your server HQ" group. | Recommended |
| N4 | Phone-to-phone transfer carries the vault, then the moved-phone flow ("Your keys did not come with you. Add them again."). Keys, config.yaml, the bridge key, the run mode and preferences stay behind. | Owner to confirm |
| N5 | Owners who upgrade start with "Show details in notifications" off, with a one-time note. | Recommended |
| N6 | The safe-mode copy is a plain .zip with no password, for now. | Owner to confirm |
| N7 | "Save a copy first" before an update shows only when there is no backup or copy under 7 days old. | Recommended |
| N8 | "What HQ noticed" counters are on by default; one previous version's numbers are kept. | Recommended |
| N9 | A Wi-Fi network is recognised by a fingerprint (gateway, DNS, domain); the app never asks for location. | Recommended |
| N10 | "Ask before big downloads on any network" lives in Spending limits, in a Mobile data group. | Recommended |
| N11 | The upgrader notifications sheet shows on Android 13 and newer only. | Recommended |
| N12 | A withdrawn version that is installed takes the top Settings card, above Update ready. | Recommended |
| N13 | The gear's update dot is blue. | Recommended |
| N14 | Rollout: 10% for 2 days, 50% for 2 days, then all; a crash-on-start report sets the share to 0. | Recommended |
| N15 | "Almost full" is under 200 MB free for the app's files, tuned by the counters. | Recommended |
| N16 | A copy saved off the phone keeps the backup row grey for 7 days; a copy in Downloads does not count. | Recommended |
| N17 | HQ's runtime marks an approval as losing, from the tool's category, never the model. | Recommended |
| N18 | HQ gets a restore route for a cleared update (and every other Undo, see W-R7). | Recommended |
| N19 | One string catalogue of whole sentences builds both the phone's resources and the web's strings. | Owner to confirm |
| N20 | The check against hard-coded strings fails the build on new ones; today's 172 reach zero by the end of wave 0. | Recommended |
| N21 | If the locked Home hold never reaches HQ on a phone model, ship the unlocked assistant only and keep the lock-screen switch off for that model. | Recommended |
| N22 | Keep the screen on for 30 s while HQ is thinking, then let it sleep. | Owner to confirm |
| N23 | After declining a request: "Declined. Nothing was done." on phone and web (replaces the phone's "Not now. …"). | Recommended |
| N24 | Phone charts use the web's order (blue, sky, white, grey, indigo); the second series is dashed. "Couldn't draw this chart." | Recommended |
| N25 | The phone's top bar puts Search before the balance chip, as the web does, at its next release. | Owner to confirm |

## Web and desktop (W), after the review

| Id | Decision | Status |
|---|---|---|
| W1 | The web opens on Chat; Lite opens on Notes; the start screen stays light to load. | Revised |
| W2 | Usage folds into Settings › Spending limits, opened by the balance chip. | Recommended |
| W3 | Dark by default; forced colours and print styles now; a light theme later. | Revised |
| W4 | Inside the app named AgentHQ, the windows are "HQ Hub" and "HQ". | Recommended |
| W5 | When a phone is the hub, the hub says "Start helpers here" / "Stop helpers here". | Recommended |
| W6 | The Inbox badge counts Needs you only, in amber. | Recommended |
| W7 | Board and Timeline are a view switch from 768 px, a "Show as" menu below. | Recommended |
| W8 | A helper's live terminal is closed by default (Technical details). | Revised |
| W9 | The details panel docks from 1400 px, a side sheet below. | Recommended |
| W10 | Get started sits on the Chat start screen until done; the briefing moves to Tasks › While you were away. | Recommended |
| W11 | "Stop every helper" stays in the Helpers menu as a danger dialog. | Recommended |
| W12 | The phone's Notes search pill opens the one Search with Notes chosen. | Recommended |
| W13 | Start-screen words follow the phone ("What would you like to do?"). | Recommended |
| W14 | Recent chats are hidden on the start screen from 1024 px. | Recommended |
| W15 | "Let HQ say yes to its requests": off by default, a first-time explainer, every answer logged in Watching, never a losing request, routes guarded by the HQ client header. | Revised |
| W16 | Reply alerts are one Updates switch per browser, words "HQ answered · Tap to read." | Revised |
| W17 | Enter sends, except while an input method is composing or on a touch-first screen; a setting "Enter sends". | Revised |
| W18 | Charts: blue, sky, white, grey, indigo, labelled directly; never amber, red or green. | Revised |
| W19 | Note provenance needs read and mark routes; "Looks right" counts only for text the owner typed, through note_review. | Revised |
| W20 | "Waiting to send" for messages, notes and tasks, with client ids, capped, cleared on sign-out, off on a shared computer. | Revised |
| W21 | Move to trash, rename and move appear only once HQ has routes for them, with Undo. | Recommended |
| W22 | Copy link is a note-id link (`/notes/n/:id`), "Opens only on your signed-in devices". | Revised |
| W23 | A web Undo toast may stay until closed, but Undo calls a restore on HQ inside a 10-minute window; after that "Couldn't undo. It changed on another device." | Revised |
| W24 | Get started has Hide; its steps stay in Settings. | Recommended |
| W25 | Lifting "Stop every helper" happens only in HQ Hub's This PC, on that computer. The web shows where. | Revised |
| W26 | Edge-worker runs are "Done by HQ", read from `/api/edge-worker`, open their chat, no Stop or Rename. | Revised |
| W27 | Search finds helpers; Inbox search stays in the Inbox's own field. | Recommended |
| W28 | Progress wording follows the phone: "1 of 3 sub-tasks done". | Recommended |
| W29 | Archive with Undo is everyday; "Delete for good" only on an archived task, as a danger dialog. | Recommended |
| W30 | Task checking words: "Check them now", "Ask for a second look", "Keep it open", "Spot checks"; tier, shadow, enforce, veto, canary only in Technical details. | Recommended |
| W31 | "Not now" on the Inbox notifications card hides it 7 days. | Recommended |
| W32 | Approvals use the phone's words: Decline left, "Allow once" right; losing requests a red "Allow {verb}". | Revised |
| W33 | Marking one task done acts at once with Undo, unless it has dependents or a check pending; several ask first. | Revised |
| W34 | The web keeps the claude.ai connector under Settings › Your accounts, off by default. | Owner to confirm |
| W35 | The web's own Settings group is "This browser"; machines by their own name; "this computer" generically. | Revised |
| W36 | Claude in Your accounts; "Folders helpers may use" in How HQ behaves; Safety and limits tabs Time limits, Helpers, Tools. | Recommended |
| W37 | `/settings` always opens the same section (This browser › Notifications); anything needing a look takes the one card slot. | Revised |
| W38 | Hub tab "This PC"; "Remove device". | Recommended |
| W39 | "Stop HQ" in the hub is a red danger dialog ("Stop HQ?" · "Keep it running" · "Stop HQ"). | Recommended |
| W40 | Web push follows the lock-screen rule (see rules-web.md section 1). | Revised |
| W41 | Spending limits gain a "One model" scope. | Recommended |
| W42 | Helpers at work move from This PC to the hub's Helpers tab; This PC keeps one summary row. | Recommended |
| W43 | Hub tab order This PC · Devices · Helpers. | Recommended |
| W44 | "This PC" on Macs too (the owner's choice). Reviewers suggested "This computer". | Owner to confirm |
| W45 | A helper update check over the internet sits behind a switch, off by default. | Recommended |
| W46 | No "Connect to HQ" for Hermes and OpenClaw. | Recommended |
| W47 | Claude Desktop sits in its own group, "Apps that use HQ". | Revised |
| W48 | Removing a helper's connection acts at once with Undo. | Recommended |
| W49 | Install lines ship with HQ and are checked at each release. | Recommended |
| W50 | Windows and Linux builds of the hub are new scope, after this run. | Owner to confirm |

## Web and desktop: decisions from the correction pass

| Id | Decision | Status |
|---|---|---|
| WR1 | Embedded mode accepts `?embed=1` and today's `?host=android`. | Recommended |
| WR2 | Lite: the gear opens This browser and Other devices only; no Spending; web push off unless the owner allows the outbound connection (`lite.allow_egress`). | Recommended |
| WR3 | Lite's status line reads "Connected · HQ Lite"; other states replace it. | Recommended |
| WR4 | A link to Chat or Helpers on Lite opens Notes with "This HQ is HQ Lite. It has no Chat." | Recommended |
| WR5 | In HQ Hub's HQ window, the web's group is "This window" and its saved line "Saved on this PC at 14:32". | Recommended |
| WR6 | New web failure reason: "Your HQ can't be reached. It will try again by itself." | Recommended |
| WR7 | Every Undo has a restore on HQ (chat archive, Inbox clear, spending limit, task archive, connection removal). | Recommended |
| WR8 | Signed out: the shell with an empty view and the Sign in sheet; a full page only for a browser that never signed in. | Recommended |
| WR9 | `/setup` splits into "Check key" and "Open HQ"; opening without checking is allowed. | Recommended |
| WR10 | The new sign-in item uses "It was me" and a red "Sign it out". | Recommended |
| WR11 | "Keep both" saves your text as a new note "{title}, your version". | Recommended |
| WR12 | Search waits 300 ms after the last key. | Recommended |
| WR13 | Export in the HQ window is off, with the reason shown, until a save command exists. | Recommended |
| WR14 | Tasks words: "area", "folder" and "list" (for space, folder, initiative); the default list stays "your Inbox list". | Recommended |
| WR15 | Task editing: status, priority and dates one tap on the task page; the rest in an "Edit details" sheet; Delete only on archived tasks. | Recommended |
| WR16 | "Find in tasks" stays in the list as well as the one Search (it filters Board and Timeline). | Recommended |
| WR17 | Starting a helper for a task needs a task field on HQ's start route; until then it is left out. | Recommended |
| WR18 | A "Typing to the helper" switch sends keys (and Esc) to a helper's terminal; Ctrl ] always leaves. | Owner to confirm |
| WR19 | HQ Hub first open reads "HQ is off" (grey); after a Stop, "You stopped HQ". | Recommended |
| WR20 | The log opens in a sheet inside HQ Hub. | Recommended |
| WR21 | "Choose the program…" is a native chooser written in Rust, with pasting a path as the fallback. | Recommended |
| WR22 | Renaming the bundle to AgentHQ: on first open HQ Hub rewrites only the Claude entries it added that point inside the old HQ.app, and says so once. | Owner to confirm |
| WR23 | Try it uses one fixed folder, `~/.hq/desktop/try-it`, emptied after each try. | Recommended |
| WR24 | A helper's sign-in gets up to 10 minutes. | Recommended |
| WR25 | "Allow helpers again" is a green button (allowing loses nothing). | Recommended |
| WR26 | Send stays on while HQ is away on phone and web; messages wait in Waiting to send; off on a shared computer. | Recommended |

## Owner to confirm, in one list

N4, N6, N19, N22, N25, W34, W44, W50, WR18, WR22. (resolutions.md B11 was confirmed on 5 Nov 2026.)
