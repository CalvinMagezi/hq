# Web wave 1 · 02 · The shared dialog, and approvals with the losing flag (WB6, W32)

Boards: **AlignedFoundations2** ("Dialog · a-dlg, the destructive pattern", "Approvals and helper
cards", "Destructive · a-btn danger"), **AlignedInbox** sections 1 and 2, **AlignedChat** section 3,
**AlignedHelpers** section 3, **AlignedTasks3** section 3, **AlignedSettings** section 3, AlignedReview
WB6, W32, W39. Follows `../../rules-web.md` sections 6 and 11 and `../../rules-shared.md` section 4.
The phone's twin is `../../mobile/wave-1/03-destructive.md`. Where this spec and the rules differ, the
rules win.

## 1. Finding and the review items it closes

- **WB6 (blocker). Losing approvals get a green Allow, first; "Let HQ answer it" can allow them.**
  "Delete 3 old backups?" with green Allow on the left; the card shows HQ's prose, not the tool and
  arguments being approved.
- **W32 revised.** Allow / Not now becomes the phone's "Allow once · Decline"; losing ones a red "Allow
  {verb}".
- **M5 (dialogs half).** Each losing action is drawn its own way, or not asked at all.
- **Drift 06 (part).** Deleting a task without sub-tasks has no confirm; Archive chat uses a trash icon
  with a red hover.
- **W39.** "Stop HQ" is a red danger dialog (built in wave 3; the component is built here).

Who it hurts: an owner answering in a hurry. A green Allow on the left of a delete reads as "go".

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/hq-web/src/components/notifications/DialogAnswer.tsx:51-77`: "Allow once" first, in green text
  (`--accent-green`), then "Decline" in red text (`--accent-red`), then "Other answers" or "Answer on
  its page". The question's command is shown by the caller, not the tool and arguments of an owner
  approval.
- `apps/hq-web/src/components/sessions/BlockedCallout.tsx:76-85`: "Approve" (green text) then "Decline"
  (red text), under "This agent is waiting for you" (line 63).
- `apps/hq-web/src/components/tasks/TaskDecisions.tsx:74,84,91`: "Allow once", "Decline", "Other
  answers", in that order.
- `apps/hq-web/src/components/notifications/NotificationCard.tsx:137-194`: "Dismiss", "Reject",
  "Approve", "Dismiss"; `NotificationDetailDrawer.tsx:206` "Approve & Promote".
- `apps/hq-web/src/components/sessions/SessionActions.tsx:103-131` `StopButton`: "Stop asks twice":
  inline "Yes, stop it" in red text, then "Keep going".
- `apps/hq-web/src/components/sessions/EmergencyStop.tsx:84-149`: a dialog "Stop every agent" with the
  action "Stop agents on {computer}" or "Stop every agent everywhere".
- `window.confirm()` for losing actions: `components/settings/DevicesSection.tsx:128` ("Sign this device
  out? You will need a code, a login link or /login to get back in."), `ChatAppsSection.tsx:314`
  ("Disconnect the {Telegram} bot from HQ?"), `:318` ("The current owner loses access until the new
  account pairs. Continue?"), `routes/settings.tsx:271` ("Disconnect every Claude client from this
  HQ?"), `:334` (revoke a folder, `lib/sessionFoldersApi.ts:31` "Stop allowing sessions in {path} on
  {host}?").
- `apps/hq-web/src/components/tasks/TaskDetailDrawer.tsx:108-110`: delete with no sub-tasks acts at
  once; with sub-tasks a rose strip (197-206).
- `apps/hq-web/src/components/chat/ThreadSidebar.tsx:111`: archive is a trash icon.
- Server: `crates/hq-agent/src/governance/owner_approval.rs:69` `needs_owner()`, `:193`
  `without_runtime_keys()` strips `_hq_*` keys before the args are shown; no losing flag exists on
  `value_items` or dialog alerts.

### Before frames

- **B1 · Inbox, Pending.** "Delete 3 old backups?" card: green "Approve", "Reject", "Dismiss".
- **B2 · Workbench answer panel.** "This agent is waiting for you", green "Approve", red "Decline", More
  keys.
- **B3 · Settings, Devices.** The browser's own `confirm()` box: "Sign this device out? …" OK / Cancel.

## 3. Decision

**One `Dialog` component with two kinds, and one `ApprovalCard` that quotes what will run, used by every
place.** Sort actions by what can be put back (rules-shared 4):

- **Undo** (HQ can put it back): act at once, toast with Undo (`03-undo.md`).
- **Ask** (nothing is lost, something stops or starts): the dialog with a green action on the right.
- **Red** (it loses something or cuts someone off): the dialog with a red action on the right.

Approvals: Decline on the left, "Allow once" on the right; a request HQ's runtime marks as losing gets a
red "Allow {verb}". The card quotes the tool and its arguments verbatim, then "In HQ's words".

Rejected:

- **The browser's `window.confirm()`.** It cannot name the action, cannot be red, and its button order
  differs per browser.
- **Type the word to confirm.** Slow, and it trains people to type through.
- **Undo for a stop.** HQ cannot undo stopping a helper or signing a device out.

## 4. Screens and states

### The dialog (`.a-dlg`)

`--surface-200`, 1 px `--line` border, 28 px radius, 24 px padding, `min(420px, 100% - 48px)` wide,
centred on a 62% scrim. Title 22/28 Bold ink, a question with the verb and the thing. Body 14/20
`--ink-muted`, at most three sentences of 15 words. Actions right-aligned, 8 px apart, 48 px tall:
safe `.a-btn.secondary` on the left, the action on the right.

- **D1 · Red, side by side.** "Stop Codex?" / "It stops now. Its work so far is kept." / [Keep it
  running] [Stop Codex] (red fill, black label).
- **D2 · Red, stacked** (a label over 18 characters): "Sign out everywhere?" / "Every other device is
  signed out, and so is any sign-in link you made earlier. Claude connections end too. HQ restarts for
  a moment." / [Keep them signed in] on top, [Sign out everywhere] below, both full width.
- **D3 · Ask.** "Mark 3 tasks done?" / "Tasks that wait on them are told they can start. Undo puts all 3
  back." / [Not now] [Mark 3 done] (green).
- **D4 · Spending.** Two equal outlined buttons, the wait first, the cost in the label: "Try OpenClaw?"
  / "Try it asks OpenClaw one short question. That uses a little of your OpenClaw plan." / [Not now]
  [Ask 1 question].
- **D5 · Busy.** The action shows the spinner and its busy word ("Stopping…"); both buttons are off.
- **D6 · Failed.** The dialog stays; an amber line with an icon above the buttons ("Couldn't stop Codex.
  Your HQ can't be reached. Try again soon."); the action keeps its label.
- **D7 · A third choice that loses nothing** sits above the pair, full width: "Save your changes?" /
  "You edited Mobile-First-Roadmap and did not save. Your text stays in this browser unless you discard
  it." / [Save changes] above, then [Keep editing] [Discard changes].
- At 390 and 320: the same dialog, `100% - 48px` wide; the pair stacks when it does not fit. At 200%
  zoom: the phone layout; stacked when the labels do not fit (`../README.md` OD-W1). 768 to 2200: 420
  px, centred.

### The approval card (`.a-card.needs`)

- **A1 · Allow once (harmless).** Head: title "HQ needs your OK" (or "Claude needs your OK"), the amber
  pill "Needs your OK", meta "Your chat Backups · 5 min ago". "What it will run" then `.a-quote` with the
  tool and its arguments verbatim:
  `bash  {"command": "git status --short"}`. "In HQ's words:" and HQ's sentence ("Show which files
  changed in the repository. It only reads."). Actions: [Decline] (secondary, left) [Allow once]
  (green, right). Under them, for a helper: "Allow once lets it change this file one time."
- **A2 · A losing request.** The same card for
  `bash  {"command": "rm ~/backups/2026-09-14.tar ~/backups/2026-09-17.tar ~/backups/2026-09-20.tar"}`,
  "In HQ's words: Delete 3 old backups, from before Sep 21. The 11 newer copies stay." Actions:
  [Decline] [Allow delete] (red fill).
- **A3 · From a tool call.** `vault_delete {"path": "Projects/Old plan.md"}`, "In HQ's words: Moves this
  note to the trash. It is kept there for 30 days." [Decline] [Allow delete] (red). Caption: "HQ's
  runtime marks this request as losing (a new flag), so Allow is red and names the verb. "Let HQ say yes
  to its requests" never answers one."
- **A4 · Answering.** Both buttons dim; the action says "Answering…".
- **A5 · Failed.** Amber line above live buttons: "Couldn't send your answer. HQ didn't answer in time.
  Try again."
- **A6 · Refused.** "Couldn't send your answer. The question changed on the helper's screen. Nothing was
  done." with [Open helper]; "HQ refused your Allow once at 14:31. Nothing was done. The helper now asks
  something else.", "What it asked then" and "What it asks now".
- **A7 · Answered.** "Allowed. Sent at 14:33." on the card (ink).
- **A8 · Long.** One too long to read on a card shows Open on the card and is answered from its details.

These cards appear in today's Inbox (`NotificationCard`), the Workbench answer panel (`BlockedCallout`,
`DialogAnswer`), the task drawer (`TaskDecisions`) and Chat (owner approvals). Wave 4 redraws each place
around the same component.

### Every losing or interrupting action in today's web app

| Action | Where today | Kind | Safe · action · done line |
|--------|-------------|------|---------------------------|
| Stop a helper | `SessionActions.tsx` `StopButton` (inline "Yes, stop it") | Red | Keep it running · Stop Codex · "Codex stopped." |
| Stop every helper | `EmergencyStop.tsx` | Red | Keep them running · Stop every helper (stacked) · result card per computer |
| Sign out a device | `DevicesSection.tsx:128` `confirm()` | Red | Keep signed in · Sign out device · "Safari on a MacBook Air signed out." |
| Sign out this browser | none today | Red | Keep signed in · Sign out this browser (stacked) · Sign in shows |
| Sign out everywhere | `DevicesSection.tsx:179-209` | Red | Keep them signed in · Sign out everywhere (stacked) |
| Disconnect Telegram or Discord | `ChatAppsSection.tsx:314` `confirm()` | Red | Keep Telegram · Disconnect Telegram (stacked) · "Telegram is disconnected. HQ is restarting." |
| Link a different account | `ChatAppsSection.tsx:318` `confirm()` ("Continue?") | Red | "Unlink your Telegram account?" · Keep my account · Unlink account |
| Disconnect Claude connections | `settings.tsx:271` `confirm()` | Red | Keep them connected · Disconnect all |
| Revoke a folder | `settings.tsx:334` `confirm()` | Red | "Revoke the shop-site folder?" · Keep it approved · Revoke folder |
| Delete a task | `TaskDetailDrawer.tsx:108` (none, or a rose strip) | Red, only on an archived task (W29) | "Delete this task for good?" · Keep it · Delete for good |
| Discard note changes | `NoteEditor` leave | Red with a third choice | Save changes above · Keep editing · Discard changes |
| Remove a paired device (HQ Hub) | `apps/desktop/ui/src/main.ts:324` "Confirm revoke" | Red | Keep it paired · Remove device |
| Stop HQ (HQ Hub) | `host-stop` button | Red | Keep it running · Stop HQ |
| Approve a losing request | `NotificationCard.tsx:164` "Approve" | Red | Decline · Allow {verb} |
| Approve a harmless request | `DialogAnswer.tsx:59`, `BlockedCallout.tsx:79` | Green | Decline · Allow once |
| Mark several tasks done | `BulkActionBar.tsx` | Ask | Not now · Mark 3 done |
| Allow helpers again (HQ Hub) | none today | Ask (green, WR25) | Keep them stopped · Allow helpers |
| Archive a chat, clear updates, archive a task, delete a limit | various | Undo (`03-undo.md`) | toast with Undo |

## 5. Exact copy

| Element | String |
|---------|--------|
| Stop a helper | "Stop {Codex}?" / "It stops now. Its work so far is kept." (Helpers adds "Resume starts it again later.") / "Keep it running" · "Stop {Codex}" |
| Stop every helper | "Stop every helper?" / "Every helper on the computers you pick stops now. Their work so far is kept. New helpers can't start there until you allow them again in HQ Hub on that computer." / "Keep them running" · "Stop every helper" |
| Sign out a device | "Sign out {Safari on a MacBook Air}?" / "It stops reaching HQ at once. To use it again, sign it in with a new code." / "Keep signed in" · "Sign out device" |
| Sign out this browser | "Sign out this browser?" / "HQ wipes what this browser saved: notes, lists, drafts and messages waiting to send. Notifications stop here too." / "Keep signed in" · "Sign out this browser" |
| Sign out everywhere | "Sign out everywhere?" / "Every other device is signed out, and so is any sign-in link you made earlier. Claude connections end too. HQ restarts for a moment." / "Keep them signed in" · "Sign out everywhere" |
| Disconnect a chat app | "Disconnect Telegram?" / "HQ stops answering on Telegram and forgets the bot token. HQ restarts for a moment." / "Keep Telegram" · "Disconnect Telegram" |
| Revoke a folder | "Revoke the {shop-site} folder?" / "Helpers on the {Mac mini} must ask again before they work in {~/code/shop-site}. A helper already working there keeps going." / "Keep it approved" · "Revoke folder" |
| Delete for good | "Delete this task for good?" / "“{Old relay notes}” and its comments are removed. This can't be undone. Leave it archived to keep it out of the way." / "Keep it" · "Delete for good" |
| Discard note changes | "Save your changes?" / "You edited {Mobile-First-Roadmap} and did not save. Your text stays in this browser unless you discard it." / "Save changes" · "Keep editing" · "Discard changes" |
| Approval head | "{HQ, Claude, Codex} needs your OK" · pill "Needs your OK" |
| Quote label | "What it will run" (Inbox), "What HQ wants to run" (Chat), "What it would run, as Claude drew it" (Helpers), "What it would run, as Codex wrote it" (Tasks) |
| Description label | "In HQ's words" |
| Approval buttons | "Decline" · "Allow once"; losing: "Decline" · "Allow {verb}" ("Allow delete", "Allow remove", "Allow stop", "Allow sign out") |
| Busy | "Answering…", "Stopping…", "Removing…", "Signing out…", "Disconnecting…", "Unlinking…", "Deleting…", "Revoking…" |
| Answered | "Allowed. Claude is working again." · "Declined. Claude is waiting." · "Allowed." · "Declined. Nothing was done." · "Already answered. Someone answered this somewhere else." · "Allowed. Sent at {14:33}." |

Retired: "Approve", "Reject", "Approve & Promote", "Deny", "Yes, stop it", "Keep going" (as the safe
label of a stop), "Confirm revoke", "Continue?", "Not now" or a bare "Allow" on approvals, every
`window.confirm()` text.

## 6. Behaviour rules

1. **Order.** Safe on the left, the action on the right, side by side, equal width. Stacked when a label
   passes 18 characters, when the pair does not fit, or when the root font is 24 px or more (OD-W1);
   stacked puts the safe choice on top and the action at the bottom, both full width.
2. **Dismiss.** Esc, a click on the scrim and the browser's Back choose the safe side. They never act.
   Focus starts on the dialog's title (`tabindex="-1"`), never on the action; Tab cycles inside; closing
   returns focus to the trigger.
3. **Labels.** The action names the title's verb and object. Never "Yes", "OK", "Continue" or
   "Confirm". The safe label is "Keep …" naming what stays, or "Not now" when nothing is kept.
4. **Busy and failed.** One request per press: both buttons off while it runs. On failure the dialog
   stays with the amber catalogue line and the same labels.
5. **Done.** The dialog closes and a toast says what happened, with no Undo. A sign-out toast stays
   until closed.
6. **Triggers are ink.** Menu items and quiet buttons that open a red dialog are ink
   (`.a-menu-item.danger` and `.a-btn.quiet.danger` are ink after alignment). Red appears only on the
   button that acts.
7. **Quote, not retell.** The approval card shows the tool name and its arguments exactly as HQ's
   runtime holds them (`without_runtime_keys`, up to 3000 characters, monospace, wrapped inside its
   frame), then "In HQ's words" and HQ's description. Never the description alone.
8. **The losing flag.** HQ's runtime marks an approval `loses: true` from the tool's category (delete,
   remove, stop, sign out, revoke, disconnect, drop, overwrite), never from the model's words. The
   approval's verb comes from the same table. Unmarked requests keep the green "Allow once".
9. **"Let HQ say yes to its requests" never answers a losing request.** The server refuses an automatic
   yes when `loses` is set; the request waits for the owner.
10. **One item, two places.** An approval in the Inbox and in a chat or a helper's page is one item;
    answering in either answers both, and the other shows "Already answered. Someone answered this
    somewhere else."
11. **Allow once only when safe.** For a helper's own dialog, "Allow once" shows only when HQ can read
    that the highlighted choice is a plain yes, once; otherwise "Other answers" lists its choices. A yes
    for good says "It will not ask about this again".
12. **Checked against a fresh read.** Before sending a helper answer, HQ reads the screen again; if it
    changed: "It asked something else just now. Look again before you answer." and the buttons wait for
    the new question.

## 7. Browser and platform notes

- `<dialog>` with `showModal()` gives the focus trap, the inert background and Esc in every current
  browser (Safari 15.4+). Use it, styled with `.a-dlg`; the scrim is `::backdrop`.
- Esc fires `cancel`; map it to the safe action and `preventDefault()` while busy so a running request
  is not orphaned.
- In HQ Hub's control window (Tauri, WKWebView) the same `<dialog>` works; Esc and the window's close
  button mean the safe choice.
- Screen readers: `aria-labelledby` the title, `aria-describedby` the body. The red action's accessible
  name is its label; colour adds nothing for them, so the label must name the loss.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/hq-web/src/components/ui/Dialog.tsx` | `kind: 'ask' | 'red' | 'spend'`, words, busy, error, optional third choice; layout by rule 1. | NEW |
| `apps/hq-web/src/components/ui/ApprovalCard.tsx` | Quote, "In HQ's words", Decline left, action right, `loses` decides colour and verb. | NEW |
| `apps/hq-web/src/components/notifications/DialogAnswer.tsx` | Order and colours (51-77) through `ApprovalCard`. | yes |
| `apps/hq-web/src/components/sessions/BlockedCallout.tsx` | "Approve" becomes "Allow once", order Decline then Allow once (76-85). | yes |
| `apps/hq-web/src/components/tasks/TaskDecisions.tsx` | Order (74-91). | yes |
| `apps/hq-web/src/components/notifications/NotificationCard.tsx`, `NotificationDetailDrawer.tsx` | Approve, Reject, Approve & Promote become the card's two buttons. | yes |
| `apps/hq-web/src/components/sessions/SessionActions.tsx` | `StopButton` (103-131) opens the dialog. | yes |
| `apps/hq-web/src/components/sessions/EmergencyStop.tsx` | Words and the stacked pair. | yes |
| `apps/hq-web/src/components/settings/DevicesSection.tsx`, `ChatAppsSection.tsx`, `routes/settings.tsx` | Every `window.confirm()` becomes the dialog (lines 128, 314, 318, 271, 334). | yes |
| `apps/hq-web/src/components/tasks/TaskDetailDrawer.tsx` | Delete only on an archived task, through the dialog (108-110, 197-206). | yes |
| `apps/hq-web/src/components/chat/ThreadSidebar.tsx` | Archive icon, not trash (111). | yes |
| `crates/hq-agent/src/governance/owner_approval.rs` | Set `loses` and `verb` from the tool's category on the approval item. | yes |
| `crates/hq-db/src/value_items.rs`, migration | `loses INTEGER`, `verb TEXT` on approval items; dialog alerts get the same from the host's read. | yes, migration NEW |
| `crates/hq-web/src/notifications_api.rs`, `sessions_api.rs`, `crates/hq-web/src/session_driver.rs` | Return `loses`, `verb`, the tool and arguments; refuse an automatic yes on a losing item. | yes |

## 9. Server, desktop or release changes

- **Server:** the `loses` flag and verb (N17), the tool and arguments on every approval item, and the
  refusal of an automatic yes for a losing request (W15; `../../changes.md` "Owner approvals and Let HQ
  say yes never allow a losing request").
- **Desktop:** the control window adopts the same dialog in wave 3.

## 10. Acceptance checks (each fails on today's build)

1. A component test over every red dialog: the action is to the right of the safe button with labels of
   18 characters or fewer, and below it with a longer label. Today `StopButton` puts "Yes, stop it" on
   the left.
2. No dialog or approval renders a green action when `loses` is true; the action reads "Allow delete" for
   a `vault_delete` fixture. Today the card says "Approve" in green.
3. A test fails when a red action label is "Yes", "OK", "Continue" or "Confirm", or lacks the title's
   verb. Today "Yes, stop it" and "Continue?" fail.
4. `grep -rn "window.confirm" apps/hq-web/src` finds nothing. Today it finds five.
5. Esc, a scrim click and `history.back()` call `onKeep` and never `onAct` in every dialog.
6. The approval card for an owner approval shows `vault_delete` and `{"path": "Projects/Old plan.md"}`
   above "In HQ's words". Today only the notice's prose shows.
7. `cargo test -p hq-agent`: an approval for a delete-category tool has `loses = true`; "Let HQ say yes"
   refuses it. Today neither exists.

## 11. Check on a laptop or phone

- Keyboard only: open each dialog, Tab cycles inside, Esc keeps, Enter on the action acts once.
- VoiceOver (Safari) and NVDA (Firefox on Windows): the title is read first, then the body.
- 320 px and 200% zoom: the long-label dialogs stack, safe on top.

## 12. Open decisions it depends on

- **N17** (Recommended): HQ's runtime marks approvals as losing from the tool's category.
- **D2 revised, D3, W32, W39, WR25.**
- **OD-W1** (`../README.md`): how "text at 200%" is detected.
- Depends on: `03-undo.md` (what acts at once instead of asking), `../wave-0/02-shared-parts.md`
  (failure lines), `../wave-4/05-inbox.md`, `../wave-4/07-helpers.md`, `../wave-4/01-chat.md`.
