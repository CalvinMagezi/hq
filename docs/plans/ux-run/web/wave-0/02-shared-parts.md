# Web wave 0 · 02 · Shared parts: failure lines, the Connection state, the saved copy, the outbox, counters, one catalogue

Boards: **AlignedWebRules** sections 2, 3, 4 and 9, **AlignedFoundations** ("Connection states",
"The saved copy", "Failure lines"), **AlignedShell** section 5 ("The connection-state pattern" and its
table), **AlignedChat2** sections 1 and 2, **AlignedSettingsMore** section 2 (Data used, What HQ
noticed), and AlignedReview M1, M3, M9, M10, M15. Follows `../../rules-web.md` sections 2, 3, 4 and 9 and
`../../rules-shared.md` sections 2, 6, 7 and 10. Where this spec and the rules differ, the rules win.

These are the parts every later web spec builds on. Nothing here is a screen of its own; each part is
a module with tests, used first by wave 1.

## 1. Findings and the review items they close

- **M1 Status words and order.** "Live", "Saved · 3 min", "Can't reach HQ", hub "Running"; the place's
  fact before the state; the state cut off by an ellipsis.
- **M3 Retired failure lines and seven saved-copy wordings.** "HQ is not answering", "HQ didn't reply",
  the 60 s "HQ did not start"; "Checking…" rows.
- **W13 (Drift 13), partly closed.** The web cannot tell starting from stopped (a bare 502); the AI
  status route is missing; offline copies need caching work; signed-out data stays.
- **M8** (part): two HQs share one address in the HQ window, so sign-ins and saved copies mix. Key
  storage by HQ id.
- **M9 A dropped link mid-reply.** No replay; a resent message runs and costs twice; a lost Stop keeps
  spending (the client-id half is here; resume and Stop over HTTP are in `../wave-4/01-chat.md`).
- **WB4** (create half): create has no client id, so a retry makes "Title 2".
- **M10 Data and first load** (the counters half), **M15 Translation and proof:** inline strings, no
  plurals, assembled dates, no web counters.
- Decisions C (connection states), D (what a browser keeps), E (outbox), O (proof), N19 (one catalogue).

Who it hurts: every owner, every time HQ is out of reach or slow. Today each place says it its own way,
and some of them are wrong ("stopped" when HQ is only starting).

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/hq-web/src/lib/reachability.ts:8` `Reachability = 'ok' | 'device_offline' | 'server_unreachable'`;
  `:6` `UNREACHABLE_GRACE_MS = 8000`; `:17` a socket that is closed for 8 s and a browser online means
  "server_unreachable". Starting, Reconnecting and the AI service are not states.
- `apps/hq-web/src/components/ReachabilityBanner.tsx:45` "This device is offline" / "Can't reach your
  HQ"; `:53` "Check that Tailscale is switched on…"; `:56` the Tailscale admin console.
- `apps/hq-web/src/routes/__root.tsx:353` the dot's title "Connected" / "Sign in to HQ to sync" /
  "Showing data saved on this device"; `:369` the words "live", "saved" or "offline" plus an age
  (`SavedDataLabel`, line 389).
- `apps/hq-web/src/lib/offlineCache.ts`: one IndexedDB cache for everything (`hq-query-cache`), 7 days,
  not keyed by HQ; `hq-last-sync` one timestamp for the whole app.
- `apps/hq-web/src/store/hqStore.ts:107`, `src/store/threadStore.ts:322`: stores not keyed by HQ.
- No outbox on the web. `apps/hq-web/src/components/chat/draftStore.ts` keeps the composer's text only.
  The chat socket takes a `client_id` (`crates/hq-web/src/ws/mod.rs:52`, passed at `:128` and `:135`)
  but HQ does not drop a repeat. `POST /api/note/create` makes "Title 2" on a clash
  (`crates/hq-web/src/vault_api.rs:359` `create_unique`, `:367` `"{base} {n}.md"`).
- `crates/hq-web/src/api.rs:15` `/health`: `{"status": "ok", "version", "git_sha", "build_time", "mcp"}`
  or 503 "vault db missing". No starting flag, no edition, no install id.
- No `/api/status/ai`. No counters. Strings are inline in each component; no plural or date formatter
  shared between places (`lib/time.ts`, `components/tasks/activity.ts:27-30` builds "{n}m ago" by hand).

### Before frames (Drift, page 1)

- **B1 · Reachability banner.** "Can't reach your HQ" with three Tailscale steps.
- **B2 · Home while signed out.** "Could not load pinned: unauthorized" with Retry (Drift 13).
- **B3 · The header dot.** "saved · 3 min" in amber text at 10 px.

## 3. Decision

**One state model, one saved-copy store per HQ, one outbox, one counter book and one catalogue, used
by every place.**

- The **Connection** module computes the seven browser states of rules-web 2 from what a browser can
  see, first true row wins, and exposes the words, the colour, a sheet model and the actions that are
  off.
- The **saved copy** lives in one IndexedDB database per HQ id, capped at 256 KB a place and 2 MB in
  all (`../../resolutions.md` B16, the same cap as the phone), with one "saved at" per place, wiped on
  the rules' three events.
- The **outbox** holds messages, new notes and new tasks with a client id each; HQ drops repeats.
- **"What HQ noticed"** counts the rules' list in this browser only.
- **One catalogue** of whole sentences builds the web's strings (and, with N19, the phone's).

Rejected:

- **A banner per state.** The status line is the only place the connection is reported (Shell 4); a
  banner may only add a line in Chat, where a draft is at stake.
- **Guessing "stopped" from a timer.** A browser cannot tell starting from stopped (rules-web 2). Only
  HQ Hub, which owns the process, says stopped, by progress.
- **Keep one cache for all HQs.** The HQ window serves this computer's HQ and a phone's on one origin
  (rules-web 3, M8).

## 4. Screens and states

The parts are drawn on the boards named above. Their visible pieces:

### The status line (`.a-status` in `.a-tt`)

The state first, then at most one fact of the place after " · " ("Connected · 9 active",
"Your HQ can't be reached · 9 active", "Connected · HQ on your phone · 2 working"). It wraps and the top
bar grows; it is never cut off. The dot is grey for Connected, blue for Starting and Reconnecting,
amber for the rest; it never carries the meaning alone. Choosing the status line opens the connection
sheet. The balance chip shows only in Connected.

| # | State | Words | Dot | Saved strip | Actions | Connection sheet |
|---|-------|-------|-----|-------------|---------|------------------|
| 1 | Signed out | "Signed out of this HQ" | amber | none: every store is wiped (query cache, the worker's cache, drafts, the outbox, push) | Sign in only | The Sign in sheet: Use a code, and "This is a shared computer" |
| 2 | Offline | "You're offline" | amber | "Saved in this browser at 14:32" | Reading works. New messages, notes and tasks wait to send. | "Couldn't send your message. You're offline. It will try again when you're back online." |
| 3 | Can't be reached | "Your HQ can't be reached" | amber | "Saved in this browser at 14:32" | Off, with "These wait until your HQ can be reached." | Last answer, next try, this browser online or not, Try again now. Never "stopped". |
| 4 | Starting | "Starting HQ · 12 s" | blue | "Saved in this browser at 14:32" | Off, with "You can answer once HQ has started." | "HQ is still starting. It will try again by itself." The start steps, from HQ's starting flag. No 60 s "did not start". |
| 5 | Reconnecting | "Reconnecting" | blue | current | Send queues as "Waiting to send". Stop is also sent over HTTP. | The socket's next try. A dropped reply resumes by message id. |
| 6 | AI service down | "The AI service isn't answering" | amber | current | Send stays on. A failed reply says "The AI service isn't answering. HQ is fine. Try again soon." | Which service, from the signed-in AI status route. Open AI service and keys. |
| 7 | Connected | "Connected" then the balance chip | grey | current | All on | Nothing to fix. The sheet lists the HQ's name and address. |
| hub | HQ has stopped | "HQ has stopped" (stop icon) | amber | saved, as row 3 | Off, with "HQ has stopped. Start HQ to try again." | "Stopped at: {step}". Start HQ, Copy details. |
| hub | Restarting HQ | "Restarting HQ" | blue | saved, as row 3 | Off, with "HQ is restarting. Try again in a moment." | The start steps, as row 4 (`../../resolutions.md` C-12). |
| hub | You stopped HQ | "You stopped HQ" | grey | saved, as row 3 | Off, with "Start HQ to use this." | Start HQ. |

HQ Hub's "HQ has stopped" follows `../../resolutions.md` A7: the process is dead, or its start step has
not moved for the longer of 60 s and twice its usual start time. No fixed timer.

Lite in Connected reads "Connected · HQ Lite" (WR3). The HQ window showing the phone's HQ reads
"Connected · HQ on your phone" and its strip "Saved in HQ Hub at 14:32" (`../../resolutions.md` B11,
confirmed by the owner on 5 Nov 2026; it replaces WR5's saved line).

### The connection sheet, state 3 (AlignedShell 5)

`.a-sheet` (bottom below 768, side from 768). Title "Your HQ can't be reached". Body "This browser can't
reach your HQ at hq.example.ts.net. It may be starting, restarting or updating. A browser can't tell
which." Facts (`.a-drow`): "Last answer · 14:02, 3 min ago", "Trying again · in 20 s", "This browser ·
Online". Line "You can read the saved copy. New messages wait to send." Primary "Try again now";
quiet "What else can I check?". When HQ answers again a toast "HQ is back. Lists are up to date." and
the strip goes.

### The saved strip (`.a-stale`)

One per place, under the status line, warn icon, `--surface-100`: "Saved in this browser at 14:32".
A copy from before today names the day: "Saved in this browser on Tue 3 Nov" (B16). With a state, the strip
leads with it as one sentence where a board draws that: "Your HQ can't be reached. Saved in this
browser at 14:32." or "HQ is starting. Saved in this browser at 14:32." Nothing saved yet: the place's
own empty state ("Nothing saved in this browser yet") with the reason and "Try again now".

### Waiting to send (in the thread or list where the item would be)

Group heading "Waiting to send" with a count (`.a-grp .a-count`); each item with its line "Saved in this
browser. It will send when HQ is up." (starting), "…when HQ is back." (can't be reached), "…when you
are back online." (offline), "Saved in this browser. It is next." and "Sending 1 of 2" while it drains.
Each is removable. After 24 h: "Couldn't send your message. It waited 24 h, so it was not sent." with
Send now (and Remove).

### What HQ noticed (Settings › This browser › Data used › Technical details)

Drawn on AlignedSettingsMore 2:

> Undo used: chat archived 2 · Inbox cleared 1 · limit deleted 0
> Messages edited within 10 s of Enter: 3
> Repeats dropped from Waiting to send: 1
> Approvals answered here: 4
> Data an hour, idle: 0.4 MB
> First load: 1.2 s
>
> Counts only, kept in this browser. Nothing is sent. Copy details includes them. [Copy details]

### Widths, editions and the hub

320 px: the status line sits under the title and wraps to two lines; Search, the chip and the gear
move into the More menu. 768 to 1023, 1440 and 2200: the same line in the top bar. 200% zoom: the
phone layout. Lite: rows 1 to 4 and 7 apply, row 6 never (no AI service), "Connected · HQ Lite". The
phone as the hub, in the HQ window: rows 1 to 7, with "Saved in HQ Hub at 14:32" and "· HQ on your
phone" as the fact.

## 5. Exact copy

### The catalogue's web reasons

The catalogue is `../../resolutions.md` A2 (copied into `../../rules-shared.md` section 2). Every line
is "Couldn't {action}." + the reason sentence + the outcome sentence; the outcome depends only on
whether the action waits and retries by itself (it is in the outbox or HQ retries it) or not. The web
uses these rows:

| Reason key | Reason sentence | If it waits and retries | If it does not | When |
|------------|-----------------|-------------------------|----------------|------|
| `starting` | "HQ is still starting." | "It will try again by itself." | "Try again in a moment." | State 4 |
| `restarting` (hub) | "HQ is restarting." | "It will try again by itself." | "Try again in a moment." | HQ Hub, which owns the process |
| `stopped` (hub) | "HQ has stopped." | "It will try again when HQ is back." | "Start HQ to try again." | HQ Hub only; a browser never says stopped |
| `no_reply` | "HQ didn't answer in time." | (none) | "Try again." | Any call that timed out while Connected |
| `unreachable` | "Your HQ can't be reached." | "It will try again when HQ is back." | "Try again when it's back." | State 3 (WR6, as A2 words it) |
| `signed_out` | "You're signed out of this HQ." | (none) | "Sign in again." | Only where a place still shows a line; state 1 shows the Sign in sheet |
| `offline` | "You're offline." | "It will try again when you're back online." | "Try again when you're back online." | State 2 |
| `ai_down` | "The AI service isn't answering." | (none) | "HQ is fine. Try again soon." | State 6 |
| `pairing_off` (hub) | "Pairing is off." | (none) | "Switch it on and try again." | HQ Hub Devices |
| `undo_too_late` | "It changed on another device, or too much time passed." | (none) | (none) | Undo past the window, an Undo that waited in the outbox, or a conflict |
| `almost_full` (hub) | "This computer is almost full." | (none) | "Free up {size} and try again." | HQ Hub (C-12) |

Web-only reasons that keep their own whole sentences (no outcome sentence is added to them):

| Reason key | Sentence | When |
|------------|----------|------|
| `waited_too_long` | "It waited 24 h, so it was not sent." | The outbox's limit |
| `outbox_full` | "Couldn't add this. 100 things are already waiting to send." (whole line, WR30) | The 101st item |
| `shared_computer` | "Notes can't wait here on a shared computer." / "On a shared computer nothing waits to send." | Shared computer |
| `question_changed` | "The question changed on the helper's screen. Nothing was done." | An answer to a moved dialog |
| `port_held` (hub) | "Another program holds this port. HQ Hub won't send your sign-in there." | HQ Hub |
| `world_writable` (hub) | "Anyone on this computer can change its folder. Choose one that only you can change." | HQ Hub, Choose the program |
| `bad_settings_file` (hub) | "Its settings file has a mistake." / "Claude Desktop's settings file has a mistake in it. Fix or remove it, then try again." | HQ Hub, Connect to HQ |

### Failure lines, by key (every one the web boards draw)

Which ones wait: a chat message, a new note and a new task wait in the outbox; a read HQ repeats by
itself (lists, search) waits; every change the owner made by hand (archive, stop, answer, delete, add
a list, plan a check) does not.

| Key | String |
|-----|--------|
| `fail_send_message_starting` | "Couldn't send your message. HQ is still starting. It will try again by itself." |
| `fail_send_message_unreachable` | "Couldn't send your message. Your HQ can't be reached. It will try again when HQ is back." |
| `fail_send_message_offline` | "Couldn't send your message. You're offline. It will try again when you're back online." |
| `fail_send_message_no_reply` | "Couldn't send your message. HQ didn't answer in time. Try again." |
| `fail_send_message_waited` | "Couldn't send your message. It waited 24 h, so it was not sent." |
| `fail_finish_reply_unreachable` | "Couldn't finish this reply. Your HQ can't be reached. Try again when it's back." (Retry once back) |
| `fail_finish_reply_ai_down` | "Couldn't finish this reply. The AI service isn't answering. HQ is fine. Try again soon." |
| `fail_stop_reply_starting` | "Couldn't stop the reply. HQ is still starting. Try again in a moment." |
| `fail_stop_reply_unreachable` | "Couldn't stop the reply. Your HQ can't be reached. Try again when it's back." |
| `fail_stop_reply_offline` | "Couldn't stop the reply. You're offline. Try again when you're back online." |
| `fail_stop_reply_no_reply` | "Couldn't stop the reply. HQ didn't answer in time. Try again." |
| `fail_archive_chat_starting` | "Couldn't archive this chat. HQ is still starting. Try again in a moment." |
| `fail_archive_chat_unreachable` | "Couldn't archive this chat. Your HQ can't be reached. Try again when it's back." |
| `fail_archive_chat_offline` | "Couldn't archive this chat. You're offline. Try again when you're back online." |
| `fail_archive_chat_no_reply` | "Couldn't archive this chat. HQ didn't answer in time. Try again." |
| `fail_undo_too_late` | "Couldn't undo. It changed on another device, or too much time passed." |
| `fail_load_recent_no_reply` | "Couldn't load Recent. HQ didn't answer in time. Try again." |
| `fail_save_note_no_reply` | "Couldn't save the note. HQ didn't answer in time. Try again." |
| `fail_save_changes_no_reply` | "Couldn't save your changes. HQ didn't answer in time. Try again." |
| `fail_make_pdf_no_reply` | "Couldn't make the PDF. HQ didn't answer in time. Try again." (and "the Word document", "the picture" and so on per format) |
| `fail_search_{reason}` | "Couldn't search your notes and chats." plus the reason and its waiting outcome (`starting`, `unreachable`, `offline`), or `no_reply` with "Try again."; Lite: "Couldn't search your notes." |
| `fail_send_answer_no_reply` | "Couldn't send your answer. HQ didn't answer in time. Try again." |
| `fail_send_answer_unreachable` | "Couldn't send your answer. Your HQ can't be reached. Try again when it's back." |
| `fail_send_answer_changed` | "Couldn't send your answer. The question changed on the helper's screen. Nothing was done." |
| `fail_load_tasks_unreachable` | "Couldn't load your tasks. Your HQ can't be reached. It will try again when HQ is back." |
| `fail_load_lists_unreachable` | "Couldn't load your lists. Your HQ can't be reached. It will try again when HQ is back." |
| `fail_add_list_unreachable` | "Couldn't add the list. Your HQ can't be reached. Try again when it's back." |
| `fail_plan_check_unreachable` | "Couldn't plan the check. Your HQ can't be reached. Try again when it's back." |
| `fail_delete_task_unreachable` | "Couldn't delete the task. Your HQ can't be reached. Try again when it's back." |
| `fail_change_some_tasks` | "{N} tasks done. Couldn't change {T-4}. It changed on another device, or too much time passed." |
| `fail_stop_helper_unreachable` | "Couldn't stop {Codex}. Your HQ can't be reached. Try again when it's back." |
| `fail_resume_helper_{reason}` | "Couldn't resume {Codex}." plus the reason and its outcome |
| `fail_start_helper_{reason}` | "Couldn't start {Claude}." plus the reason and its outcome |
| `fail_load_section_{reason}` | "Couldn't load {your messaging apps, Safety and limits}." plus the reason and its outcome |
| `fail_draw_chart` | "Couldn't draw this chart." plus the reader's own reason ("Series 1 needs 3 values, one for each label.") |
| `fail_add_outbox_full` | "Couldn't add this. 100 things are already waiting to send." |
| `fail_open_hq_phone_port` (hub) | "Couldn't open HQ on your phone." plus `port_held` |
| `fail_find_hq` (hub) | "Couldn't find HQ on this computer." |
| `fail_use_program` (hub) | "Couldn't use that program." plus `world_writable` |
| `fail_switch_on_pairing` (hub) | "Couldn't switch on pairing. Another program uses its port. Close it and try again." |
| `fail_remove_device_pairing_off` (hub) | "Couldn't remove “{Pixel 8}”. Pairing is off. Switch it on and try again." |
| `fail_remove_claude_entry` (hub) | "{Pixel 8} removed. Couldn't take HQ out of Claude Desktop. Its settings file has a mistake." |
| `fail_connect_bad_file` (hub) | "Couldn't connect. Claude Desktop's settings file has a mistake in it. Fix or remove it, then try again." |
| `fail_read_version` (hub) | "Couldn't read its version in 2 s." |

### State words and lines

| Key | String |
|-----|--------|
| `state_signed_out` | "Signed out of this HQ" |
| `state_offline` | "You're offline" |
| `state_unreachable` | "Your HQ can't be reached" |
| `state_starting` | "Starting HQ · {s} s" |
| `state_reconnecting` | "Reconnecting" |
| `state_ai_down` | "The AI service isn't answering" |
| `state_connected` | "Connected" |
| `state_lite` | "Connected · HQ Lite" |
| `state_hub_stopped` | "HQ has stopped" |
| `state_hub_restarting` | "Restarting HQ" |
| `state_hub_you_stopped` | "You stopped HQ" |
| `saved_at` | "Saved in this browser at {time}" ("on {weekday} {day} {month}" before today: "Saved in this browser on Tue 3 Nov") |
| `saved_at_window` | "Saved in HQ Hub at {time}" (`../../resolutions.md` B11) |
| `off_until_reached` | "These wait until your HQ can be reached." |
| `off_until_started` | "You can answer once HQ has started." |
| `off_hub_stopped` | "HQ has stopped. Start HQ to try again." |
| `off_hub_you_stopped` | "Start HQ to use this." |
| `back` | "HQ is back. Lists are up to date." |
| `outbox_heading` | "Waiting to send" |
| `outbox_line_{state}` | "Saved in this browser. It will try again by itself." (starting) / "Saved in this browser. It will try again when HQ is back." (can't be reached) / "Saved in this browser. It will try again when you're back online." (offline) / "Saved in this browser. It is next." In the HQ window "Saved in HQ Hub." replaces "Saved in this browser." |
| `outbox_sending` | "Sending {n} of {m}" |

Retired everywhere on the web: "Live", "live", "saved", "offline" (as the dot's word), "Saved · 3 min",
"Can't reach HQ", "Can't reach your HQ", "This device is offline", "HQ is stopped", "HQ did not start",
"HQ is not answering", "HQ didn't reply", "HQ did not answer", "HQ is not running", "Not answering",
"Checking…", "Live · AI service not answering", "Signed out · Saved · 2 h", "Try again soon" after
"Your HQ can't be reached", "Saved on this PC", every Tailscale step, any HTTP or exit code outside
Copy details.

## 6. Behaviour rules

### Connection

1. **Inputs.** (a) The last non-cached HTTP status from HQ (`noteAuth`); (b) `navigator.onLine` and the
   `online`/`offline` events; (c) the chat socket's state from `WebSocketContext`; (d) `GET /api/hello`
   (new, below); (e) `GET /api/status/ai` (new, signed in); (f) in the HQ window, the proxy's
   `x-hq-proxy: upstream-gone` header on its own 502.
2. **Order.** The first true row wins: signed out (a is 401) › offline (b false) › can't be reached (no
   answer from d for 8 s, `UNREACHABLE_GRACE_MS`, or f) › starting (d says `starting`) › reconnecting
   (socket closed while d answers) › AI down (e says not answering) › connected.
3. **Starting HQ · 12 s.** The seconds count up from `/api/hello`'s `started_at`, once a second, and the
   connection sheet lists the start steps HQ reports. A browser never shows "HQ has stopped".
4. **Retry.** While can't be reached, `/api/hello` is tried at 5, 10, 20, then every 20 s; the sheet
   shows the next try ("in 20 s"). Try again now tries at once. Everything pauses while the tab is hidden
   and tries once when it shows again.
5. **The hub's two states** come from `host_status` in HQ Hub only (wave 3). The web never computes them.
6. **The balance chip** renders only in Connected, and only when HQ reports a balance or a limit. No
   "Checking" chip: while unknown it is left out.

### Saved copy

7. **Keyed by HQ id.** `/api/hello` returns `hq_id`, a random install id HQ creates once. Every store
   is named with it: the React Query persister key `hq-query-cache:{hq_id}`, zustand `hq-store:{hq_id}`
   and `hq-thread-store:{hq_id}`, drafts `hq-drafts:{hq_id}`, the outbox `hq-outbox:{hq_id}`, the
   worker's cache `hq-api:{hq_id}` (the page posts the id to the worker). A different id never reads
   another's store.
8. **256 KB a place, 2 MB in all, per HQ, kept until replaced** (`../../resolutions.md` B16, one cap on
   phone and web). Lists and notes already opened. A place's copy past 256 KB keeps its newest items
   up to the cap. When a write would pass 2 MB, the oldest place's copy is dropped first (by its saved
   time). Each place keeps its own "saved
   at"; the strip shows that place's time, never an app-wide one. Today's 7-day `gcTime` becomes "until
   replaced" (no age limit), with the age always shown.
9. **Wipe, at once.** On a non-cached 401 (rules-web 3), on Sign out this browser, and on Sign out
   everywhere (this browser re-signs with the new token after the wipe): the stores of rule 7 for that
   HQ id, the push subscription, `hq-last-sync`, and the stored token. Other tabs follow through
   `BroadcastChannel('hq-auth')`. (`../wave-0/01-live-bugs.md` ships the first version; this rule keys
   it by HQ id.)
10. **A shared computer keeps nothing.** With "This is a shared computer" the token is kept in
    `sessionStorage` for this tab only, the persister is off, the worker's API cache is bypassed, the
    outbox is off, push is never offered, and the tab signs out after 30 minutes with no input
    (`pointerdown`, `keydown`, `wheel`, `touchstart`).

### Outbox

11. **What waits.** Chat messages, new notes and new tasks, while state is 2, 3, 4 or 5. Each entry:
    `{client_id, kind, hq_id, thread_id?, payload, created_at, attempts, last_reason}` in IndexedDB
    `hq-outbox:{hq_id}`. `client_id` is `crypto.randomUUID()` made when the owner presses Send or Save,
    never on retry.
12. **Order and limits.** One at a time, oldest first, per HQ. A chat message waits up to 24 h, then
    stays in the thread unsent with Send now and Remove. Notes and tasks wait until sent or removed.
    The outbox holds at most 100 entries (`../../decisions.md` WR30); the 101st is refused with
    "Couldn't add this. 100 things are already waiting to send."
13. **Never twice.** HQ drops a repeat `client_id` (new on the server: messages through the socket and
    `POST /api/threads/{id}/messages`, `POST /api/note/create`, `POST /api/tasks`) and answers with the
    item it already made. A repeat dropped counts in "What HQ noticed".
14. **Off on a shared computer** (rule 10); Send then says "Send turns on when HQ is back. Your text
    stays in the box."
15. **Wiped** with the saved copy (rule 9). Removing one asks nothing (it is in this browser only).

### Counters

16. **The list (rules-web 9).** `undo.{kind}` per kind (`chat_archived`, `inbox_cleared`,
    `limit_deleted`, `task_archived`, `task_done`, `tasks_archived`), each with the count of actions of
    that kind; `edit.within_10s` (a message edited within 10 s of Enter); `outbox.repeats_dropped`;
    `approvals.{surface}` (`inbox`, `chat`, `helpers`, `task`); `data.kb_per_idle_hour`;
    `first_load_ms` (Chat); in HQ Hub, `try_it.pass.{helper}` and `.fail`.
17. **Where kept.** localStorage `hq-noticed`, this browser only, not keyed by HQ (counts hold no
    content), never sent, shown under This browser › Data used › Technical details, copied only with
    Copy details. The previous version's numbers are kept beside the current ones (N8).
18. **Roll-back** follows rules-shared 10: a kind whose Undo is used on more than about 20% of at least
    10 actions gets its confirm back in the next release.

### Catalogue

19. **Whole sentences, one key each.** Keys look like `fail_send_message_starting`. Never assembled
    from fragments; a placeholder is a name, a number or a time.
20. **Plurals and dates** through `Intl.PluralRules` and `Intl.DateTimeFormat`/`Intl.RelativeTimeFormat`
    with the browser's language and the display zone (`../wave-2/04-time-zones.md`). "{N} updates
    cleared" has plural forms; "14:32" follows the browser's 12 or 24-hour setting.
21. **One file builds both.** With N19 confirmed: `apps/strings/en.yaml` (NEW) holds every sentence with
    its key; `scripts/strings/build.ts` (NEW) writes `apps/hq-web/src/strings/en.gen.ts` and the phone's
    `res/values/strings_catalogue.xml`. Until N19 is confirmed the web keeps
    `apps/hq-web/src/strings/en.ts` (NEW) with the same keys, so moving later is mechanical.
22. **A check fails the build on new inline strings** in `apps/hq-web/src/components` and `src/routes`
    (N20 on the web): a `bun test` that scans `.tsx` for JSX text and string props (`title`,
    `aria-label`, `placeholder`) not taken from the catalogue. Repository names like `{owner}/{repo}` and
    code samples are allowed.

## 7. Browser and platform notes

- `navigator.onLine` is true behind a captive portal or with a dead tunnel; that case is row 3 (no
  answer), never row 2.
- Safari evicts IndexedDB for sites not visited in 7 days unless installed to the Home Screen; the
  saved copy may then be gone, and the place shows "Nothing saved in this browser yet". Call
  `navigator.storage.persist()` once after sign-in (not on a shared computer).
- Firefox private windows allow IndexedDB in memory only; Chrome incognito keeps it until the window
  closes. Both behave as a shared computer would, without the 30-minute rule.
- `BroadcastChannel` is in every current browser (Safari 15.4+).
- `crypto.randomUUID()` needs a secure context: `https` or `localhost`. HQ over plain `http` on a LAN
  address falls back to a 128-bit `crypto.getRandomValues` id.
- The HQ window in HQ Hub (WKWebView on macOS) supports IndexedDB and BroadcastChannel; it has no push.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/hq-web/src/lib/connection.ts` | The state model (rules 1 to 6): inputs, order, words, sheet model, `useConnection()`. Replaces `reachability()`. | NEW |
| `apps/hq-web/src/lib/reachability.ts` | Kept for `UNREACHABLE_GRACE_MS` and `useDeviceOnline()`; `reachability()` retired. | yes |
| `apps/hq-web/src/components/ReachabilityBanner.tsx` | Removed; the connection sheet replaces it (`components/ConnectionSheet.tsx`, NEW). | yes |
| `apps/hq-web/src/lib/savedCopy.ts` | Stores keyed by HQ id, the 256 KB a place and 2 MB in all cap, per-place saved times, `wipe(hqId)`. | NEW |
| `apps/hq-web/src/lib/offlineCache.ts` | The persister key and `gcTime` per rule 8; `markSynced(place)` per place. | yes |
| `apps/hq-web/src/store/hqStore.ts`, `threadStore.ts` | Store names with the HQ id. | yes |
| `apps/hq-web/src/components/chat/draftStore.ts` | Keyed by HQ id; off on a shared computer. | yes |
| `apps/hq-web/src/lib/outbox.ts` | Rules 11 to 15. | NEW |
| `apps/hq-web/src/lib/noticed.ts` | Rules 16 to 18. | NEW |
| `apps/hq-web/src/strings/en.ts` (or `.gen.ts`), `apps/strings/en.yaml`, `scripts/strings/build.ts` | Rules 19 to 21. | NEW |
| `apps/hq-web/src/strings/catalogue.test.ts` | Rule 22. | NEW |
| `apps/hq-web/src/sw.ts` | Cache named per HQ id; bypass on a shared computer. | yes |
| `crates/hq-web/src/api.rs`, `lib.rs` | `GET /api/hello` (no sign-in): `{hq_id, edition, version, min_client, starting: {step, steps, step_name, started_at} | null}`. | NEW route |
| `crates/hq-web/src/status_ai.rs` | `GET /api/status/ai` (signed in): per AI service in the chain, last good and last failed call, failure kind; no prompt or answer text. | NEW |
| `crates/hq-llm` (backend chain) | Records each call's outcome for the route. | yes (crate) |
| `crates/hq-web/src/ws/mod.rs`, `threads_api.rs`, `vault_api.rs`, `tasks_api.rs` | Drop a repeat `client_id` and return the existing item. | yes |
| `crates/hq-db/sql/1xx_client_ids.sql` | A `client_ids (hq_kind, client_id, item_id, created_at)` table, pruned after 7 days. | NEW |

## 9. Server, desktop or release changes

- **Server:** `/api/hello` (install id, edition, version, minimum client, the starting flag); the starting
  flag needs HQ to bind its web port early and answer `/api/hello` while the rest opens (other routes
  answer 503 with `{"starting": true}` meanwhile); `/api/status/ai` (D16); client-id dedup for
  messages, notes and tasks (`../../changes.md`, "HQ drops a repeated client_id").
- **Desktop:** the proxy marks its own 502 for a gone upstream with `x-hq-proxy: upstream-gone`
  (`crates/hq-desktop/src/proxy.rs`).
- **Release:** none.

## 10. Acceptance checks (each fails on today's build)

1. `bun test lib/connection.test.ts`: a table of input combinations gives the seven states in order;
   for `{status401: true, online: false}` the state is signed out; for a closed socket with `/api/hello`
   answering `starting` the words are "Starting HQ · 12 s" with 12 from `started_at`. Today there is no
   such module and `reachability()` returns three values.
2. No string in `apps/hq-web/src` matches `/\blive\b|Saved · |Can't reach HQ|HQ is not answering|HQ
   didn't reply|Checking…|Saved on this PC|Tailscale/` outside tests and `docs`. Today `__root.tsx:369`,
   `ReachabilityBanner.tsx:45-56` and others match.
3. Two HQ ids in one browser: writing a note copy under id A, then switching to id B, reads nothing;
   `wipe('A')` leaves B's stores intact. Today there is one unkeyed store.
4. The saved copy passing 2 MB drops the oldest place's entry first, and one place never holds more
   than 256 KB. Today there is no cap.
5. A message sent with the socket closed is in `hq-outbox:{id}` with a `client_id`; sending it twice to
   HQ makes one message (`cargo test -p hq-web` on the socket and the messages route). Today it is not
   queued, and HQ would store two.
6. `POST /api/note/create` twice with the same `client_id` returns the same path, and no "Title 2"
   exists. Today the second call makes "Title 2".
7. `GET /api/hello` answers without a token and includes `hq_id`; `/api/status/ai` refuses without one.
   Today neither exists.
8. The catalogue check (rule 22) fails on a component with inline text. Today there is no check.

## 11. Check on a laptop or phone

- Stop HQ (`hq web stop`) while a tab is open: within 8 s the status line reads "Your HQ can't be
  reached", the strip shows the saved time, the sheet says "in 20 s". Start it: "Starting HQ · {n} s"
  in blue while it opens, then "Connected" and the toast.
- Turn the laptop's Wi-Fi off: "You're offline". Send a chat message: it shows under "Waiting to send".
  Turn Wi-Fi on: it sends once ("Sending 1 of 1"), and the thread has one copy on HQ.
- HQ Hub with a phone hub and this computer's HQ: open both in the HQ window in turn; each shows only its
  own saved notes.

## 12. Open decisions it depends on

- **N19** (owner to confirm): one catalogue for phone and web. Until then, rule 21's fallback.
- **N8**: counters on by default, one previous version kept. Recommended.
- **WR30** (settled, was OD-W4): the outbox holds at most 100 items.
- Depends on: `01-live-bugs.md` (the first wipe), `../../changes.md` (`/api/status/ai`, the starting
  flag, client-id dedup), D16, D17 (the phone's saved-copy rule), N1, W20, WR5, WR6, WR26, and
  `../../resolutions.md` A2, A6, A7, B11, B16, C-12.
