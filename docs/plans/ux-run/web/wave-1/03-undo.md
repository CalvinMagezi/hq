# Web wave 1 · 03 · Undo that restores on HQ (W23, WR7)

Boards: **AlignedWebRules** section 4 ("Undo restores on HQ"), **AlignedFoundations2** ("Toast ·
a-snack"), **AlignedChat** section 1 (Archive and Undo), **AlignedInbox** section 3 (Clear, named
honestly), **AlignedTasks** section 3, **AlignedSpending** (Deleted, with Undo), **AlignedHubHelperDetail2**
(Remove the connection, with Undo), AlignedReview M5, W23. Follows `../../rules-web.md` sections 4 and 11
and `../../rules-shared.md` section 4. Where this spec and the rules differ, the rules win.

## 1. Finding and the review items it closes

- **M5 (Undo half).** Undo forms and timing differ; restores missing for chat archive, Inbox clears,
  limits, connections.
- **W23 revised.** A web toast may stay until closed, but Undo calls a server restore that expires.
- **WR7, N18.** Every Undo has a restore on HQ: chat archive, Inbox clear, spending limit, task archive,
  connection removal (in HQ Hub, locally).
- **Inbox "Mark all read"** (Inbox board): today it sets every waiting item to dismissed, approvals
  included, with no way back.

Who it hurts: anyone who clears or archives by mistake, on the web or the phone. Today an Undo on one
does not exist on the other, and some actions cannot be undone at all.

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/hq-web/src/components/chat/ChatView.tsx:143-158` `archiveThread()`; `threadApi.ts:145` calls
  `POST /api/threads/{id}/archive` (`crates/hq-web/src/threads_api.rs:101`). No unarchive route, no
  Undo, a trash icon (`ThreadSidebar.tsx:111`).
- `crates/hq-web/src/notifications_api.rs:394-397` `mark_all_read_handler`: `UPDATE value_items SET
  state='dismissed' … WHERE state IN ('pending', 'delivered', 'routed')` (approvals included). `:254`
  maps "dismiss" and "reject" to dismissed. No route sets an item back. The web shows "Mark all read"
  (`routes/notifications.tsx:164`).
- `crates/hq-web/src/tasks_api.rs:908` `restore_task_handler` exists (`POST /api/tasks/{id}/restore`);
  `apps/hq-web/src/components/tasks/TaskDetailDrawer.tsx:221-224` shows an archived task with restore,
  but no Undo toast after archiving.
- `crates/hq-web/src/budgets_api.rs:42` `put_budgets_handler` replaces the whole list; deleting a limit
  is a PUT without it, gone at once.
- HQ Hub: `crates/hq-desktop/src/desktop.rs:245` `claude_change(hub, add)`; removing a Claude entry has
  no Undo (`apps/desktop/ui/src/main.ts:416` "Removed.").
- `apps/hq-web/src/components/tasks/verification/SweepSheet.tsx`: the sweep's own Undo
  (`undo_sweep` in hq-tools), with no time limit. It stays as it is (Tasks 4).
- Toasts: a six-second system notice (`__root.tsx:259`) and per-component messages. No shared toast.

## 3. Decision

**Act at once and offer Undo for everything HQ can put back. Every Undo calls a restore route on HQ
inside a 10-minute window. The toast may stay until closed; after the window it says so.**

- The phone and the web share one restore contract: the same routes, the same window, so an Undo on
  either restores for both.
- What HQ cannot put back (stop a helper, sign a device out, forget a bot token) asks first instead
  (`02-dialogs-approvals.md`). No fake Undo.

Rejected:

- **Hold the change in the browser for the toast's life, send it after.** A closed tab would lose or
  delay it, and the phone would never see the change. HQ holds the soft delete.
- **An Undo that never expires.** HQ would have to keep every archived and cleared item's restore
  forever, and a late Undo would surprise another device that already moved on.

## 4. Screens and states

### The toast (`.a-snack`)

Ink ground, `--surface-100` text 14/20, 48 px tall, 6 px radius; Undo in `--snack-action` #00668F,
Bold; a close button (`.a-ib`, "Close"). Below 768: full width, 8 px from the edges, above the bottom
bar. From 768: bottom right, 24 px in, `min(480px, 100% - 48px)` wide. It never covers the focused
field: with a field focused it sits above that field, or waits until the field loses focus (the note
editor, the composer).

- **T1 · Done with Undo.** "Chat archived" · Undo · close.
- **T2 · Undo pressed.** The item comes back in place and the toast closes. (Recommended, not drawn on the boards: a short "Chat restored" with no action, read by screen readers.)
- **T3 · Too late.** Undo after the window, or when the item changed on another device: "Couldn't undo.
  It changed on another device, or too much time passed." (`../../resolutions.md` A2, A6). Kept until
  closed.
- **T4 · Undo while HQ is away.** The restore waits in the outbox ("Waiting to send") if the window is
  still open when it is sent; if it reaches HQ after the window, T3 (A6).
- **T5 · Several at once.** "3 tasks archived" · Undo, "3 tasks done" · Undo; a partial result "2 tasks
  done. Couldn't change T-4. It changed on another device, or too much time passed." kept until closed.

Widths: 320 and 390 full width above the bottom bar; 768, 1440, 2200 bottom right; 200% zoom the phone
placement. Lite: tasks and Inbox Undo work the same; no chat. HQ window: the same.

### Where Undo appears (this wave adds the toast to today's screens)

| Action | Where today | Done line | Restore route |
|--------|-------------|-----------|---------------|
| Archive a chat (row button, /clear, /topic) | `ChatView.tsx:143`, `ThreadSidebar.tsx:111` | "Chat archived" | `POST /api/threads/{id}/unarchive` (NEW) |
| Clear one update | `NotificationCard` Dismiss | "Update cleared" | `POST /api/notifications/{id}/restore` (NEW) |
| Clear all updates | `notifications.tsx:164` "Mark all read" | "{N} updates cleared" | `POST /api/notifications/restore` with the ids (NEW) |
| Archive a task | task drawer, bulk bar | "Task archived", "{N} tasks archived" | `POST /api/tasks/{id}/restore` (exists) |
| Mark a task done (one, no dependents, no check pending) | task card, drawer | "Done: {title}" (`../../resolutions.md` B8) | `PATCH /api/tasks/{id}` back to its status, guarded by the version (NEW guard) |
| Mark several tasks done (after the Ask dialog, `02-dialogs-approvals.md` D3) | bulk bar | "{N} tasks done" (B8) | as above, per task |
| Move a card into Done | board | "Done: {title}" (B8) | as above |
| Delete a spending limit | `routes/usage.tsx` | "“OpenRouter only” deleted" | `POST /api/budgets/{name}/restore` (NEW) |
| Archive finished and stopped helpers | Workbench | "3 helpers archived" | `POST /api/harness-sessions/{id}/archive` with `archived: false` (exists as Restore) |
| Remove a helper's connection (HQ Hub) | Claude tab "Remove" | "Connection removed" | `helper_restore` command, local (wave 3) |
| Clear recent searches | Search | "Recent searches cleared" | none: this browser only, Undo needs no HQ |
| Add a task | New task | "Task added" | `POST /api/tasks/{id}/restore` after a soft archive (Tasks 3) |

## 5. Exact copy

| Key | String |
|-----|--------|
| `done_chat_archived` | "Chat archived" |
| `done_update_cleared` | "Update cleared" |
| `done_updates_cleared` | "{N} updates cleared" (plural) |
| `done_task_archived` | "Task archived" |
| `done_tasks_archived` | "{N} tasks archived" (plural) |
| `done_task_marked_done` | "Done: {title}" (B8) |
| `done_tasks_marked_done` | "{N} tasks done" (plural, B8) |
| `done_task_moved` | "Task moved to {In progress}" |
| `done_task_added` | "Task added" |
| `done_limit_deleted` | "“{OpenRouter only}” deleted" |
| `done_helpers_archived` | "{N} helpers archived" (plural) |
| `done_connection_removed` | "Connection removed" |
| `done_recent_cleared` | "Recent searches cleared" |
| `undo` | "Undo" |
| `restored_{kind}` | Recommended, not on the boards: "Chat restored", "Updates restored", "Task restored", "Limit restored", "Connection restored" |
| `fail_undo_too_late` | "Couldn't undo. It changed on another device, or too much time passed." |
| `fail_change_some_tasks` | "{N} tasks done. Couldn't change {T-4}. It changed on another device, or too much time passed." |

Retired: "Mark all read" (replaced by "Clear all updates", which clears only Updates), "Removed." in HQ
Hub, any "successfully".

## 6. Behaviour rules

1. **Act at once.** The change is sent at once and shows at once. The toast offers Undo.
2. **The window is 10 minutes, on HQ.** The restore route accepts a restore only within 600 s of the
   change and only when the item has not changed since (a version or `updated_at` match). Otherwise it
   answers 409 and the toast says "Couldn't undo. It changed on another device, or too much time
   passed."
3. **The toast may stay.** A toast that carries Undo stays until it is used or closed (W23,
   `../../resolutions.md` A6; the phone's toast lasts the accessibility timeout instead). It is not
   timed. The Undo button greys out at 10 minutes and the toast then reads the too-late line.
4. **Soft delete on HQ.** Archive, clear and delete keep the item with a timestamp; restore clears it.
   Today's retention of archived and dismissed items is unchanged.
5. **Clear means Updates only.** "Clear all updates" dismisses only the Updates tab's items; Needs you
   items are never cleared in bulk.
6. **HQ away.** A change made while HQ is away waits in the outbox; its Undo removes it from the outbox
   (no HQ call). A restore made while HQ is away waits too, with the original change time, so HQ still
   applies the 10-minute rule.
7. **Counted.** Each Undo press counts `undo.{kind}` and each action `undo.{kind}.actions` in "What HQ
   noticed" (`../wave-0/02-shared-parts.md` rule 16). More than about 20% of 10 or more actions of a
   kind brings its confirm back in the next release.
8. **Toasts never cover the focused field** and are reached last with F6 or Tab; Undo has an accessible
   name "Undo {done line}".
9. **The phone shares it.** The phone calls the same routes (`../../mobile/wave-1/03-destructive.md`
   rules 13 and 18); an archive on the phone can be undone from the web inside the window and the other
   way round.

## 7. Browser and platform notes

- The toast region is `role="status"` (`aria-live="polite"`); a failure toast is `role="alert"`.
- Ctrl Z is not bound to Undo: it belongs to the text field that has focus.
- On a touch screen the toast's close button is 48 px.
- HQ Hub's Undo for a removed connection is local to that computer (`helper_restore`, wave 3).

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/hq-web/src/components/ui/Toast.tsx`, `lib/toast.ts` | One toast host, placement rule, Undo with a 10-minute guard. | NEW |
| `apps/hq-web/src/components/chat/ChatView.tsx`, `ThreadSidebar.tsx`, `lib/threadApi.ts` | Archive with Undo; archive icon; `unarchiveThread()`. | yes |
| `apps/hq-web/src/routes/notifications.tsx`, `components/notifications/NotificationCard.tsx`, `lib/notificationsApi.ts` | "Clear all updates" on Updates only, Undo, `restore()`. | yes |
| `apps/hq-web/src/routes/tasks.tsx`, `components/tasks/BulkActionBar.tsx`, `TaskDetailDrawer.tsx`, `lib/tasksApi.ts` | Undo toasts for archive and done. | yes |
| `apps/hq-web/src/routes/usage.tsx`, `lib/usageApi.ts` | Delete a limit with Undo (moves with Spending in wave 4). | yes |
| `crates/hq-web/src/threads_api.rs`, `crates/hq-db` chat | `unarchive_thread` with the window. | yes, route NEW |
| `crates/hq-web/src/notifications_api.rs` | `restore` one and many with the window; `mark_all_read_handler` limited to Updates kinds or retired. | yes |
| `crates/hq-web/src/budgets_api.rs`, `crates/hq-core/src/config/budgets.rs` | A deleted limit kept with `deleted_at`; restore route. | yes, route NEW |
| `crates/hq-web/src/tasks_api.rs` | `restore_task_handler` gets the window and the changed check. | yes |

## 9. Server, desktop or release changes

Server: the restore routes and the 10-minute, unchanged-since rule (`../../changes.md` "Restore routes
for every Undo"). Desktop: `helper_restore` in wave 3.

## 10. Acceptance checks (each fails on today's build)

1. `cargo test -p hq-web`: archive a thread, unarchive within 600 s: it is back in `/api/threads`; at
   601 s or after a new message: 409. Today there is no unarchive route.
2. Clear one update, restore it: it is back in Updates with its state. Today nothing sets dismissed back.
3. "Clear all updates" leaves every Needs you item pending. Today `mark_all_read` dismisses approvals.
4. Delete a limit, restore it within the window: it is back in `GET /api/budgets`. Today it is gone.
5. `bun test`: archiving a chat shows the toast "Chat archived" with Undo; Undo calls the unarchive
   route; a 409 shows "Couldn't undo. It changed on another device, or too much time passed." Today
   there is no toast.
6. With a field focused, a toast's box does not intersect the field's box.
7. Marking one task done with no dependents and no check pending shows "Done: {title}" with Undo and no
   dialog; marking three shows the Ask dialog first, then "3 tasks done" with Undo (B8). Today the done
   line is "Task marked done".

## 11. Check on a laptop or phone

- Archive a chat on the laptop, open the phone within 10 minutes: the phone's Undo (or the web's) brings
  it back on both.
- Leave a "Task archived" toast open for 11 minutes, press Undo: the too-late line.

## 12. Open decisions it depends on

- **W23, WR7, N18** (Recommended), as `../../resolutions.md` A6 and B8 settle them.
- **W29**: archive with Undo is everyday; Delete for good only on an archived task.
- Depends on: `../wave-0/02-shared-parts.md` (outbox, counters), `02-dialogs-approvals.md`,
  `../wave-3/04-helper-detail.md` (`helper_restore`).
