# Web wave 4 · 05 · Inbox: what needs you, what happened, what you decided (WB6, W31, M5, M12)

Boards: **AlignedInbox** (Needs you answering in place; answers in flight, failed and refused; Updates
with Undo; all caught up and Done; phone width), **AlignedInbox2** (tablet, 2200, 320 and 200%, signed out
and a new sign-in, Lite, the phone as the hub), AlignedReview WB2, WB6, W16, W31, W32, M5, M12, FixRules
6, Drift 07, 08. Follows `../../rules-web.md` sections 1, 4 and 6 and `../../rules-shared.md` sections 1 and
4. Where this spec and the rules differ, the rules win.

## 1. Finding and the review items it closes

- **WB6, W32.** Approvals use the phone's words and order: Decline left, "Allow once" right; a losing
  request gets a red "Allow delete". The card quotes the tool and its arguments as HQ's runtime holds them;
  HQ's description follows, labelled "In HQ's words".
- **WB2, W16.** The notifications card says pushes name who is asking, never what. On yes:
  "Notifications from HQ are on in this browser."
- **M5.** "Mark all read" is gone (today it clears every waiting item, approvals included). "Clear all
  updates" clears only Updates, with "4 updates cleared · Undo".
- **M12.** The Inbox keeps its own search field and a Refresh button.
- **M1, M3.** "Connected", "Your HQ can't be reached" and "Saved in this browser at 14:30". "HQ didn't
  reply", "HQ is not running" and "HQ is not answering yet" are gone.
- **M2.** The badge is amber; failure lines amber with an icon.
- **FixRules 6.** Items from a server HQ say "On your server HQ".
- **Drift 07, 08.** The push button names its state; Inbox copy is out of date.
- **W31.** Not now on the notifications card hides it 7 days.

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/hq-web/src/routes/notifications.tsx:137` "Inbox & Approvals"; `:139` the relay subtitle; `:164`
  "Mark all read"; `:202, 214` "All Activity", "Resolved History" (and Pending); `:225` "Search
  proposals, skills..."; category pills.
- `apps/hq-web/src/components/notifications/NotificationCard.tsx:123` "View Details & Diff"; `:137-194`
  Dismiss, Reject, Approve; raw kind badges ("action needed", "value item").
- `components/notifications/DialogAnswer.tsx:51-77` Allow once (green text) then Decline (red text) then
  Other answers; `NotificationDetailDrawer.tsx:206` "Approve & Promote"; `PushToggle.tsx:63, 75` "Push on"
  / "Turn on push".
- `crates/hq-web/src/notifications_api.rs:394-397` `mark_all_read_handler` dismisses approvals too; `:254`
  dismiss and reject; no restore.
- The six-second amber notice for a refused action (#322, `__root.tsx:259`).

## 3. Decision

**"Inbox & Approvals" becomes Inbox, with the phone's three tabs and cards: Needs you, Updates, Done.
Every card says who needs what. Questions are answered on the card, Decline on the left and the action on
the right. An approval quotes exactly what will run. Turning on notifications is one card, shown once, then
it lives in Settings › This browser › Notifications.**

Rejected:

- **An Inbox scope in the one Search.** The Inbox keeps its own field (W27).
- **Mark all read.** It dismissed approvals with no way back.

## 4. Screens and states

### Needs you, answering in place (Inbox 1)

- **N1 · 1440.** Top bar "Inbox" / "Connected"; tabs "Needs you 3" (amber count), "Updates 4", "Done".
  List pane of cards; choosing a title opens its details in the pane beside (`/inbox/:id`), where the
  answers sit in a footer that stays put.
- **N2 · The notifications card** first, once, until answered: "Let HQ reach you" / "Asked once in this
  browser" / "Get a notice when something needs you, even with HQ closed. It says who is asking, never
  what. Open it to answer here." [Turn on notifications] [Not now].
- **N3 · A helper's question.** "Claude needs your OK · Mac mini · 2 min ago" / "It stopped to ask: Do you
  want to edit Chat.dc.html?" / [Decline] [Allow once] / "Allow once lets it change this file one time."
- **N4 · An owner approval that loses something.** "HQ needs your OK · Your chat Backups · 5 min ago" /
  "What it will run" / `bash  {"command": "rm ~/backups/2026-09-14.tar ~/backups/2026-09-17.tar
  ~/backups/2026-09-20.tar"}` / "In HQ's words: Delete 3 old backups, from before Sep 21. The 11 newer
  copies stay." / [Decline] [Allow delete] (red).
- **N5 · A task to check.** "Nightly off-device backup · Task ready to check · 20 min ago" / "HQ says this
  is finished. Look it over, then mark it done." [Check it].
- **N6 · A stuck helper.** "A helper is stuck · Mac mini · 1 hour ago" / "Codex keeps hitting the same
  question, so HQ stopped answering it." [Take a look].
- **N7 · Details pane.** "What it will run, exactly" / `bash` / the JSON / "The nightly backup wants to
  remove the 3 oldest copies, from before Sep 21. They take 4.2 GB on your Mac mini. The 11 newer copies
  stay." / "If you allow it, HQ runs exactly this, once. The 3 copies are deleted and cannot be brought
  back." / "If you decline, nothing runs. HQ is told you said no." / "Asked by · Your chat Backups", "When ·
  14:27 today", "Waits until · 14:27 tomorrow".
- **Buttons by kind:** a helper's question: Decline and Allow once, when HQ read a plain yes once;
  otherwise Open helper. A request that loses something: Decline and a red "Allow {verb}", from the
  runtime's losing flag (new). A task to check: Check it. A stuck helper: Take a look.
- **What is quoted:** the tool and its arguments, verbatim, as `owner_approval.rs` stores them ("with
  exactly these arguments"), then HQ's own sentence, labelled. It waits 24 hours, then lapses.
- **Keyboard:** Tab reaches each card's title (a link to its details) and then its buttons; cards are not
  clickable divs; Up and Down move between cards; Enter opens the details.

### Answers in flight, failed and refused (Inbox 2)

- **F1 · Answering.** Both buttons dim and the action says "Answering…".
- **F2 · Failed.** Amber above live buttons: "Couldn't send your answer. HQ didn't answer in time. Try
  again."
- **F3 · Refused.** "Codex needs your OK · Mac mini · 6 min ago · Wants to run a command: ./gradlew clean"
  / "Couldn't send your answer. The question changed on the helper's screen. Nothing was done." [Open
  helper]. Details: "HQ refused your Allow once at 14:31. Nothing was done. The helper now asks something
  else." "What it asked then · ./gradlew clean", "What it asks now · Do you want to delete the build
  folder? > 1. Yes 2. No", "Codex on your Mac mini", "Job · Build the Android app".
- **F4 · Done.** "Allowed. Sent at 14:33."
- **The answer's own words** (from the phone's AnswerWords, with Decline for Not now): "Allowed. Claude is
  working again." "Declined. Claude is waiting." "Allowed." "Declined. Nothing was done." "Already
  answered. Someone answered this somewhere else."
- **Refused, not failed:** failed means HQ didn't answer in time, so try again; refused means HQ said no;
  the card names why in plain words and the way that works. The six-second notice of today is gone.
- **Long approvals:** one too long to read on a card shows Open on the card and is answered from its
  details.

### Updates, clearing with Undo (Inbox 3)

- **U1 · 1440.** Tab Updates. [Search the Inbox] (its own field), filter chips "All", "Tasks", "Helpers",
  "HQ 2", [Clear all updates], [Refresh]. "Today": "Backup finished · 2,860 notes saved to your Mac. ·
  14:12"; "A task is ready to check · Nightly off-device backup. Open it to take a look. · 13:32"; "Codex
  finished · Build the Android app. It made 4 changes. · 12:05". "Yesterday": "On your server HQ · Office.
  12,400 notes saved. · 02:10". Each row has [Clear]. Details: "2,860 notes saved to your Mac mini. The
  archive was checked after copying. Next run tonight at 02:00." "From · Backup task", "14:12 today" [Open
  Backup in Settings].
- **U2 · Clearing.** One: "Update cleared · Undo"; all: "4 updates cleared · Undo". Cleared items stay on
  HQ as dismissed; Undo calls the restore route within 10 minutes, so the web and the phone agree; after
  that: "Couldn't undo. It changed on another device."
- **U3 · Search** matches titles and text on the open tab. **Refresh:** changes arrive over the one socket,
  so it is rarely needed; it stays for the owner who wants to be sure, and while Reconnecting.
- **U4 · From a server HQ:** "On your server HQ · Office" leads the line; clearing it clears it on that HQ.

### All caught up, and Done (Inbox 4)

- **A1 · Needs you, nothing waiting.** "You're all caught up" / "Nothing is waiting on you. Checked at
  14:32." "You decided today": "Allowed: Claude edits Chat.dc.html · 14:02 · From the Inbox in this
  browser"; "Declined: Delete 3 old backups · 13:47 · From the Inbox on your phone".
- **A2 · Done** keeps every answer by day: "Checked: Tidy the export file names · Marked done from Tasks ·
  18:20".

### Phone width, 390 (Inbox 5)

- The same markup is the phone's Inbox; the details open over the list with the answers in a footer that
  stays put.
- **P1 · Can't be reached.** "Your HQ can't be reached. Saved in this browser at 14:30." The saved cards
  stay with their buttons off and the reason under them: "Mac mini · 4 min ago · It stopped to ask: Do you
  want to edit Chat.dc.html? · You can answer once HQ can be reached."
- **P2 · Nothing saved.** "Nothing saved in this browser yet" / "What needs you shows here once HQ can be
  reached."
- **The notifications card:** Turn on asks the browser. On yes the card leaves and a test notice says
  "Notifications from HQ are on in this browser." On no or blocked: "Notifications are blocked for this
  site. Allow them in the browser settings, then try again." Then it lives in Settings › This browser ›
  Notifications ("Needs you", "Updates", "Show details in notifications", off). What a notice says:
  "Claude needs your OK · Open HQ to see what it wants to do." Open is the only button, always.
- **Not now** hides it for 7 days. Then a quieter line only if a question waited unseen: "1 question waited
  here without telling you." with Turn on.
- **The badge:** amber, on Inbox only, counting Needs you; 99+ past 99; the only badge in the rail and bar.

### Every width, signed out, Lite and the phone as the hub (Inbox 2 board)

- **768 to 1023 (900):** one pane with the rail; choosing a title opens its details in place of the list,
  with Back; cards keep their buttons. Answer buttons side by side in every card (both labels under 18
  characters); the footer stays put while the text scrolls.
- **2200:** the list keeps 360; the details text stops at 720 and centres; footer buttons stop at 320 each.
- **320:** the top bar holds the title and the state; the quoted command wraps inside its frame; two
  buttons do not fit, so the pair wraps with Decline on top, as the stacking rule orders. **200%:** a 1280
  window at 640 with the bottom bar; the pair stacks; a toast never covers the focused button. **Forced
  colours:** cards keep their borders; the amber edge of a Needs you card becomes a thicker border.
- **Signed out:** "Signed out of this HQ" / "Sign in to see what needs you. Nothing from this HQ stays in
  this browser." [Sign in]. No badge, no cards and no saved time.
- **A new sign-in** as `../wave-1/05-sign-in-safety.md` N1: "New sign-in: Edge on Windows · HQ · 2 min ago"
  / "Edge on Windows signed in to this HQ at 14:20. If it was not you, sign it out." [It was me] [Sign it
  out]; "Sign out Edge on Windows?" / "It is signed out at once. To come back it needs a new code from your
  own device." / [Keep it signed in] [Sign it out]; after: "Edge on Windows is signed out." and listed under
  Settings › Other devices as signed out. New sign-ins expire after 90 days without use.
- **Lite:** Lite serves the Inbox (`/api/notifications` in `profile_gate.rs`). Needs you holds tasks to
  check and new sign-ins only; left out: helper questions, chat approvals and the stuck-helper card; Updates
  are HQ's own housekeeping. No notifications card unless `lite.allow_egress` lists the push service. No
  balance chip; the top bar ends with Search ("Search notes and tasks").
- **The phone as the hub (HQ window):** "Connected · HQ on your phone". No web push, so no notifications
  card; HQ Hub posts a system notice for Needs you items ("HQ Hub · now / Open HQ to see what it wants to do.
  / Open"). The phone also reads a server HQ: "On your server HQ · Office · 5 min ago · Wants to run a
  command: cargo build --release"; details "Asked by · Claude, on your server HQ · 14:25 today"; an answer
  goes to that HQ. Same buttons and words as a browser.

## 5. Exact copy

All strings in section 4 are exact. Keys `inbox_*`. Words that go: "Inbox & Approvals", "relay
notifications", "View Details & Diff", "action needed", "value item", "Pending", "All Activity",
"Resolved History", "Mark all read", "Not now" or a bare "Allow" on approvals, "Push on", "Turn on push",
"Search proposals, skills...", "Approve", "Reject", "Approve & Promote", "Dismiss".

## 6. Behaviour rules

1. **Three tabs:** Needs you (open items that wait on the owner), Updates (what HQ reports, by day), Done
   (every answer and check, by day). `/inbox`, `/inbox/:id`, `?tab=updates`.
2. **Cards answer in place** with `ApprovalCard` (`../wave-1/02-dialogs-approvals.md`); the details pane has
   the same buttons in a fixed footer.
3. **One item, many places:** an approval answered in Chat, Helpers, a task or the phone shows "Already
   answered. Someone answered this somewhere else." here.
4. **Clear** acts at once with Undo on HQ (`../wave-1/03-undo.md`); "Clear all updates" never touches Needs
   you.
5. **The notifications card** shows once per browser on Needs you until Turn on or Not now; Not now hides it
   7 days; the quieter line only if a question waited unseen.
6. **The badge** counts Needs you only, amber, 99+.
7. **Search the Inbox** filters the open tab by title and text, in this browser, over what is loaded.
8. **Server HQ items** lead with "On your server HQ · {name}", and an answer or clear goes to that HQ.
9. **Changes arrive over the one socket;** Refresh stays.

## 7. Browser and platform notes

- Safari requires a user gesture for `Notification.requestPermission()`: Turn on calls it directly in the
  click handler.
- Screen readers: the tab counts read as "Needs you, 3 items"; each card is an `article` labelled by its
  title.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/hq-web/src/routes/inbox.tsx`, `inbox/$id.tsx` | The place (from `notifications.tsx`). | NEW |
| `apps/hq-web/src/routes/notifications.tsx` | Redirect; its search field and chips move (137-225). | yes |
| `apps/hq-web/src/components/notifications/NotificationCard.tsx`, `NotificationDetailDrawer.tsx`, `DialogAnswer.tsx` | Cards by kind, quote, footer, answer words. | yes |
| `apps/hq-web/src/components/notifications/PushToggle.tsx` | Replaced by the card and the Settings page. | yes |
| `apps/hq-web/src/lib/notificationsApi.ts`, `lib/inboxBadge.ts` | Tabs, restore, badge rule. | yes |
| `crates/hq-web/src/notifications_api.rs` | Tabs' queries; clear Updates only; restore; server-HQ origin field. | yes |

## 9. Server, desktop or release changes

Server: the restore route, Updates-only clear, the losing flag and verbatim arguments on approvals, the new
sign-in item (wave 1). Desktop: HQ Hub's system notice (wave 3).

## 10. Acceptance checks (each fails on today's build)

1. The tabs read Needs you, Updates, Done. Today Pending, All Activity, Resolved History.
2. "Clear all updates" leaves Needs you items pending and offers Undo. Today "Mark all read" dismisses
   them.
3. A losing approval shows Decline left and a red "Allow delete" right, with the quoted command. Today
   "Approve" in green.
4. A refused answer shows "Couldn't send your answer. The question changed on the helper's screen. Nothing
   was done." on the card. Today a six-second notice.
5. Card titles are links reachable by Tab. Today cards are divs.
6. The badge is amber and counts Needs you only.

## 11. Check on a laptop or phone

- Answer a helper on the laptop while the phone shows the same item: the phone says "Already answered…".
- Clear all updates, then Undo from the phone within 10 minutes.

## 12. Open decisions it depends on

- **W6, W27, W31, W32, N17, N18, D13** (Recommended or Revised).
- Depends on: `../wave-1/02-dialogs-approvals.md`, `../wave-1/03-undo.md`, `../wave-1/05-sign-in-safety.md`,
  `../wave-1/06-settings-groups.md`.
