# Wave 3 · 04 · The assistant hand-off: never lose an answer (finding #8, M4, M5, D7)

Board: **FixAssistantHandoff** ("Fix: assistant handoff"), finding #8 with two smaller ones, design only,
revised after review, 5 Nov 2026. Shared rules: `../../rules-shared.md` sections 1, 2, 4, 8, 10 (D7)
win. Notice words match `../wave-0/01-lock-screen.md`.

Headline: **Closing the assistant no longer loses the answer: HQ finishes it in the chat, the phone says
when it is ready or needs an OK, Stop only stops, and words heard on a cold start are kept.**

## 1. Finding and the review items it closes

**Problem.** Closing the sheet (a tap outside, Back, Home, the screen timing out during a long lookup)
sends HQ a stop, yet one error says the answer "may still finish in the chat", and the sheet has no way
to that chat. Nothing marks the thread as a spoken one. Also from the spec cards: Stop always opens the
microphone again, and a press right after a boot can throw away words already heard.

**Who it hurts, how often.** Anyone who looks away during a lookup. The first live question took 24.8 s
to its first word, and many phones time the screen out at 30 s. The cold-start loss hits the first press
after every reboot.

What changed in this revision:

- **M4:** a turn that ends waiting for an OK posts "HQ needs your OK to finish" on Needs you, not "HQ
  answered" (After 12). Approvals from a voice turn quote "You said: …" (After 13). Keep going after an
  approval (After 14). Closing in the first 3 s, before any tool, stops the turn (rule 2).
- **M5:** keep-screen-on is capped at 30 s while Thinking; the "Screen on" note is gone.
- **#8:** the watcher is keyed by the client id, so closing before the thread id arrives still notifies.
  It is saved to disk and survives process death. It re-reads the thread on `stream_lag` and asks HQ
  whether the turn is still running, so a long silent tool never causes a false post at 180 s.
- **R1:** "HQ answered · Tap to read." carries no answer text by default (After 8).
- **D7:** the spoken mark comes from a source field HQ stores, set by the runtime, shared with the web.
  The phone-only id list is gone; the owner question is closed.
- **Failure lines** are rules section 2's sentences, amber with an icon (After 5, 15). "HQ is not
  answering", "went quiet" and "missed on the way" are retired.
- **Added:** server HQ variant (After 15), Android versions card, a 360 dp frame at font scale 2.0 (After
  16), "Connected" in the chat header, letter pins, server changes, acceptance checks that fail on
  today's build.

## 2. Before (as built, verified at `c04a100`)

Paths under `apps/android/app/src/main/kotlin/example/hq/spike/voice/` unless noted.

- `VoiceActivity.kt:90-94` `onStop` finishes the sheet; `VoiceSession.kt:92` `quiet()` calls
  `interrupt()` and `connection.stop(threadId)`. The screen timing out is an `onStop` too.
- `VoiceSession.kt:64` `micPressed()` answers Stop with `interrupt(); listen()` (Stop always listens
  again).
- `VoiceSession.kt:132` `ears.prepare()` runs before the microphone opens (about 1 s cold).
- `VoiceTurn.kt:108` a `ChatEvent.Lagged` ends the turn with `MISSED`; `:148` `NOT_CONNECTED = "HQ is not
  answering yet. Try again in a moment."`; `:151` `TOO_QUIET = "HQ went quiet in the middle of the answer.
  It may still finish in the chat."`; `:152` `MISSED = "Part of the answer was missed on the way. The full
  answer is in the chat."`.
- Problem lines are drawn in `Mts.DangerText` (red).
- HQ: `crates/hq-web/src/ws/mod.rs:44` `ClientMsg::Chat` has no `source`; `:112` the `Lagged` branch sends
  `stream_lag`; `:519` `spawn_supervised` (a turn runs in its own task); `crates/hq-db/sql/060_chat_message_meta.sql`
  and `crates/hq-db/src/chat.rs:202` `add_message_with_meta`; `crates/hq-web/src/threads_api.rs:24`
  `running`; `crates/hq-agent/src/governance/owner_approval.rs:49` `APPROVAL_WAIT`, `:138` `ask()`;
  `crates/hq-web/src/ws/mod.rs:655` `owner_approval()`.

### Before frames

- **Before A · Cut off mid-answer.** "Tap to ask" / "What is on my list today" / "You have three tasks due
  today. The dentist call is due at" / red "HQ went quiet in the middle of the answer. It may still finish
  in the chat." (pin B) / [Ask]. Pin A on the closing.
- **Before B · Answering, with Stop.** "Answering" / question / partial answer / [Stop] (pin C).
- **Before C · First press after a boot.** "Tap to ask" / "What is on my list today" / "HQ is not
  answering yet. Try again in a moment." (pin D) / [Ask] (pin E).

Pins: **A** closing sends a stop (`VoiceActivity.onStop` calls `VoiceSession.quiet()`). **B** the line
promises what the code prevents, and there is no button to the chat (`TurnOutput.Thread` already gives
the sheet the thread id); every problem line is red. **C** Stop always listens again (`micPressed` runs
`interrupt(); listen()`). **D** words thrown away: the socket is not open yet after a boot, `post` returns
false, and the sheet shows `NOT_CONNECTED`, a retired failure phrase. **E** cold recogniser: the
microphone opens only after `ears.prepare()`, so words said during "Getting ready" are never heard.

## 3. Decision

- **Only Stop stops HQ, with one exception:** closing in the first 3 s, before any tool, stops it (M4).
  That is when people close a misheard question, before it spends money or starts tools.
- **The sheet opens the chat.** "Open in chat" appears once HQ has put the question in a thread.
- **One quiet notice** when an answer ends after the sheet closed, with no answer text (R1). A turn that
  ends waiting for an OK posts on Needs you instead (M4).
- **The watcher never guesses.** It is saved by client id, so it outlives the process, and it asks HQ
  whether the turn still runs before it says anything.
- **Voice approvals quote the owner,** so a misheard question is caught at the OK. After the OK, Keep going
  finishes the turn on a tap.
- **Screen on for 30 s while Thinking only (M5).** The hand-off makes a longer hold unneeded.
- **Stop stops.** Ask is the way to speak again.
- **Cold start keeps the words.**
- **Spoken threads are marked by HQ (D7):** a source field set by the runtime from the voice sheet's
  frame, shared with the web, kept across a reinstall, never used for permissions.

Rejected:

- **Keep streaming into a closed sheet** with a foreground service. It needs a second ongoing notice and
  costs battery, and the hub already finishes and saves the turn.
- **Dropping keep-screen-on entirely.** The first live question took 24.8 s to its first word; 30 s keeps
  it visible on most phones.

### Closing and the turn, from the moment the question is sent

- **0 to 3 s, no tool started yet:** closing sends `stop`. Nothing is posted. This is when a misheard
  question gets closed (M4).
- **From 3 s, or once a tool has started:** closing never stops HQ. The watcher follows the turn and posts
  once when it ends.
- **Stop, at any time:** stops HQ and keeps what arrived.
- **Screen kept on (M5):** only while Thinking, at most 30 s. Released at the first word, a tool start,
  closing, or 30 s.

## 4. Screens and states (After)

The voice sheet: label, the question in bold, the answer streaming, an info line, then a bottom row with
the round button and "Open in chat". Failure lines amber with an icon.

1. **Looking it up (after 3 s).** "Looking it up" / "What is on my list today" / "You can close this. The
   answer will be in the chat." (pin 2) / [Open in chat] (pin 1) / [Stop] (pin 3).
2. **The answer, with the way into the chat.** "Tap to ask" / question / "You have three tasks due today.
   The dentist call is due at 4 pm, and the backup review and the invoice draft are in progress. Want me
   to go through the rest of the week?" / [Open in chat] / [Ask].
3. **Stop stops; the microphone stays off** (pin 4). "Stopped" / question / partial answer "You have
   three tasks due today. The dentist call is due at" / [Open in chat] / [Ask].
4. **Heard before HQ is up: it waits** (pin 6). "Waiting for HQ to start" / question / "HQ is starting on
   this phone. Your question goes as soon as it is up." / [Don't send].
5. **HQ stopped while it waited** (pins 7, 8). "Tap to ask" / question / amber "Couldn't send your
   question. HQ has stopped. Start HQ to try again." / [Send again] / [Ask].
6. **The socket dropped, HQ still running** (pin 10). "Reconnecting" / question / partial answer / "The
   answer will be in the chat." / [Open in chat] / [Stop].
7. **Cold start: listening before the recogniser loads** (pin 5). "Listening" / "Keep talking. Your words
   show in a moment." / [Done].
8. **Closed before the end: one quiet notice** (pin 11). Shade (generic): "AgentHQ · Updates · now / HQ
   answered / Tap to read."
9. **The chat it opens, marked as spoken** (pin 15). Chat header "Chat · Connected · Copilot 912 left",
   name row "What is on my list today" with a mic; the user bubble "What is on my list today" with
   "Spoken" under it; the reply. Bottom bar.
10. **Chats: spoken threads have a mic** (pin 16). Chats sheet, [New chat]. Today: "What is on my list
    today · Spoken · 2m ago · You have three tasks due" (mic icon in place of the chat icon, blue unread
    dot), "Samsung battery steps · 1h ago · Nothing is blocking it". Yesterday: "When is the invoice due ·
    Spoken · Friday, the 7th", "Telegram check · The relay is connected".
11. **Next open, notifications off** (pin 17). "Tap to ask" / "Your last question was answered after you
    closed this." / [Open in chat] / [Ask].
12. **The turn waits for an OK (M4)** (pin 12). Shade: "AgentHQ · Needs you · now / HQ needs your OK to
    finish / Open HQ to see what it wants to do. / Open".
13. **Inbox: the approval quotes what was said** (pin 13). Inbox "Connected", Needs you: card "HQ needs
    your OK to finish" / "You said, by voice · 9:42" / ""Delete the invoice draft task."" / "HQ wants to
    delete the task "Invoice draft". This cannot be undone from the phone." / [Decline] [Allow delete]
    (red).
14. **Allowed: Keep going** (pin 14). Card "Allowed at 9:44" / "HQ can finish now" / "You said, by voice ·
    9:42" / ""Delete the invoice draft task."" / "HQ stopped to wait for you. Keep going asks it to finish
    in the same chat." / [Open in chat] [Keep going].
15. **Server HQ: can't be reached** (pin 18). Label "Your server HQ, Office" / question / amber "Couldn't
    send your question. Your server HQ can't be reached. Check your internet." / [Send again] / [Ask].
16. **360 dp at font scale 2.0** (pin 19). "Looking it up" / question / "You can close this. The answer
    will be in the chat." / [Stop] centred / [Open in chat] on a full-width row below.

Also: opening the sheet while the last answer is still coming shows "HQ is still answering the last
question." with Open in chat.

## 5. Exact copy (old ones in the "Replaces" column)

| Where | Words | Replaces |
|-------|-------|----------|
| Button | "Open in chat" | |
| While waiting, from 3 s | "You can close this. The answer will be in the chat." | |
| After Stop (label) | "Stopped" | |
| Cold recogniser | "Keep talking. Your words show in a moment." | |
| Waiting (label) | "Waiting for HQ to start" | |
| Waiting (line) | "HQ is starting on this phone. Your question goes as soon as it is up." | |
| Waiting (button) | "Don't send" | |
| Resend button | "Send again" | |
| Socket dropped, HQ running | "Reconnecting" (label) · "The answer will be in the chat." | |
| Long tool, HQ running | "HQ is still working on it. The answer will be in the chat." | |
| HQ stopped | "Couldn't send your question. HQ has stopped. Start HQ to try again." | "HQ is not answering yet. Try again in a moment." |
| No reply in time | "Couldn't send your question. HQ didn't answer in time. Try again." | "HQ did not pick up the question. Try again in a moment." |
| Retired | (the watcher asks HQ instead) | "HQ went quiet in the middle of the answer. It may still finish in the chat." |
| HQ restarting | "Couldn't get the whole answer. HQ is restarting. Try again in a moment." | "The connection to HQ dropped before the answer finished." |
| Retired | (the sheet re-reads the thread) | "Part of the answer was missed on the way. The full answer is in the chat." |
| AI service down | "Couldn't get the answer. The AI service isn't answering. HQ is fine. Try again soon." | |
| Server unreachable | "Couldn't send your question. Your server HQ can't be reached. Check your internet." | |
| Signed out of server | "Couldn't send your question. This phone is signed out of your server HQ. Sign in again." | |
| Still answering | "HQ is still answering the last question." (kept, with Open in chat) | |
| Notice, answered | "HQ answered" · "Tap to read." | |
| Notice, details on, unlocked | "HQ answered" · the first 160 characters of the answer | |
| Notice, failed | "Couldn't get your answer." · the reason sentence from rules section 2 | |
| Notice, waiting for an OK | "HQ needs your OK to finish" · "Open HQ to see what it wants to do." · "Open" | |
| Inbox approval | "HQ needs your OK to finish" · "You said, by voice · {time}" · ""{question}"" · "Decline" · "Allow delete" (losing) or "Allow once" | |
| After the OK | "Allowed at {time}" · "HQ can finish now" · "HQ stopped to wait for you. Keep going asks it to finish in the same chat." · "Open in chat" · "Keep going" | |
| Follow-up sent | "It is approved. Keep going." | |
| Next open | "Your last question was answered after you closed this." | |
| Spoken mark | "Spoken" · TalkBack: "Asked by voice" | |
| Chats subtitle | "Spoken · 2m ago · {first words of the last reply}" | |
| Server label | "Your server HQ, {name}" | |

The board's own note: "Notice words match FixLockScreen. "HQ is fine. Try again soon." is rules section
2 verbatim, though section 7 bans semicolons; flagged for FixRules." (The sentence holds no semicolon, so
the flag looks spurious; see "Open conflicts" in `../README.md`.)

## 6. Behaviour rules

1. **Open in chat** shows from `TurnOutput.Thread` on. It opens `hq://chat/{id}` and closes the sheet.
2. **Closing** (tap outside, Back, Home, screen off, another app) stops the microphone. It sends `stop`
   only within 3 s of sending and before any tool started. "You can close this" shows from 3 s, once
   closing is safe.
3. **Screen on** while Thinking only, at most 30 s (M5). No note on screen.
4. **Stop** sends `stop`, keeps what arrived, shows "Stopped", and leaves the microphone off. Ask asks a
   follow-up in the same thread.
5. **Cold recogniser:** the microphone starts at once; audio waits in the 3 s record buffer while the
   model loads. Over 2.5 s, the label falls back to "Getting ready" and the oldest audio is dropped.
6. **Heard before HQ is up:** the sheet waits while the Connection state is Starting HQ, and the words
   stay. Don't send ends the wait and keeps the words. Server HQ owners never see this state.
7. **Send again** resends the same words with a new client id, after any failure line.
8. **Failure lines** are rules section 2's sentences, amber with an icon, each with its own key. HQ stopped
   mid-wait reads "Couldn't send your question. HQ has stopped. Start HQ to try again."
9. **The watcher** (`VoiceHandoff`) starts when the sheet closes during Thinking, Looking it up or
   Answering, even before a thread id. It is keyed by the client id and saved to `noBackupFilesDir` on
   every change. `HubService` restarts it after process death.
10. **Lag, drops and silence.** On `stream_lag` or a reconnect it re-reads the thread, as `ChatViewModel`
    does. Silence alone never ends it: while `/api/threads` lists the thread as running, it waits (checked
    every 30 s after 3 minutes), up to HQ's own turn limit.
11. **"HQ answered · Tap to read."** posts once on Updates, with no answer text (R1). With "Show details in
    notifications" on and the phone unlocked, the body is the first 160 characters; the public version
    stays "Tap to read." Tap opens `hq://chat/{id}`. A failure posts "Couldn't get your answer." with the
    reason sentence.
12. **Waiting for an OK** (`turn_end` with `waiting_for_owner`): Needs you posts "HQ needs your OK to
    finish". Open only. Never "HQ answered".
13. **The approval quotes the question** above what HQ wants to do. A losing request has red "Allow
    delete" on the right (rules section 4).
14. **Keep going** sends one follow-up in the same thread, marked spoken: "It is approved. Keep going." It
    never starts by itself.
15. **Spoken mark** under each spoken question, from HQ's source field. The name row shows the mic too.
    TalkBack: "Asked by voice". The web shows the same.
16. **Chats:** a mic in place of the chat icon, and "Spoken" first in the subtitle. The unread dot is blue.
17. **Next open** within an hour, if that answer's notice was not seen: the line shows once, with Open in
    chat.
18. **Server HQ:** the label names the server, and the failure lines are the server ones. No waiting state.
19. **Font scale 1.5 and up:** Stop sits centred and Open in chat takes a full-width row below it.

## 7. Android versions

| Version | What differs for this fix |
|---------|---------------------------|
| 8 and 9 | Notices post without a permission. Needs you shows Open only. The sheet opens with `startActivity` (as built). |
| 10 and 11 | Needs you still shows Open only; Allow on notices is 12+ (rules section 9). |
| 12 | Allow once · Decline · Open only with "Show details in notifications" on and the phone unlocked. A losing request like a delete stays Open only. |
| 13+ | Without notification permission nothing posts. The next-open line (After 11) covers "HQ answered"; the Inbox holds the approval. |

Android and HQ constraints, as checked on the board:

- **A closing socket does not end the turn.** In `crates/hq-web/src/ws/mod.rs` a chat turn runs in its own
  task (`spawn_supervised`, line 519). Only a `stop` frame ends it.
- **"Is it still running" exists.** `GET /api/threads` returns `running`, the threads with a turn in
  flight (`threads_api.rs`). `GET /api/threads/{id}/messages` re-reads a thread.
- **Lag is real:** the socket sends `stream_lag` when the broadcast drops events (`ws/mod.rs:112`); the
  voice turn ends on it today.
- **Approvals end the turn.** An owner approval waits 60 s inside the turn, then the model is told it is
  waiting and the turn ends (`owner_approval.rs`, `APPROVAL_WAIT`). Hence rule 12.
- **Process:** `HubService` is a foreground service and runs HQ as its child process. Whether HQ outlives a
  kill of the app process: check on device. The watcher's saved file makes the notice right either way.
- `FLAG_KEEP_SCREEN_ON` on the sheet's window, cleared by a 30 s timer or the first word. No permission.
- Android 13 needs `POST_NOTIFICATIONS`. `VISIBILITY_PRIVATE` with a public version keeps the lock screen
  general even when details are on.
- The sheet runs in its own task, so Open in chat starts `ShellActivity` with `FLAG_ACTIVITY_NEW_TASK`.
- Relays stay chat only: these are phone notices posted by the app.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `voice/VoiceSession.kt` | `quiet()` (line 92) stops hearing, and stops HQ only inside the 3 s window; `stop()`; `micPressed` (line 64) answers Stop with stop; the socket wait; `sendAgain()`; hand the client id and thread id to the watcher on close. `VoicePhase`: `Stopped` and `WaitingForHq`. | yes |
| `voice/VoiceTurn.kt` | `source: "voice"` on the frame; `Lagged` (line 108) re-reads instead of failing; the old error strings (lines 148-152) give way to catalogue keys. | yes |
| `data/ChatConnection.kt`, `data/ChatEvent.kt` (`ChatEvent.TurnEnd`) | `source` and `waitingForOwner`. | yes |
| `voice/Ears.kt` | Start the microphone before `prepare()` finishes. | yes |
| `voice/VoiceHandoff.kt` | The watcher, its saved file, notices on `AlertPoster.UPDATES` and `NEEDS`, the next-open line. | NEW |
| `voice/VoiceActivity.kt` | The bottom row, the 30 s screen timer, `openChat(id)`. | yes |
| `ui/chat/ChatsSheet.kt`, `ui/chat/Bubbles.kt` | The mic and "Spoken" from message meta. | yes |
| Inbox (`ui/inbox/InboxScreen.kt`) | The "You said" quote and Keep going. | yes |
| `res/values/strings.xml` | Every string. | yes |
| Tests | `VoiceTurnTest` (exists); `VoiceSessionTest` (the 3 s window; Stop does not listen); `VoiceHandoffTest` (lag, silence while running, saved and restored). | `VoiceTurnTest` yes; others NEW |

## 9. Server changes needed (`crates/hq-web`, `crates/hq-agent`)

- **D7 source field.** `ClientMsg::Chat` (`ws/mod.rs:44`) takes an optional `source`; only "voice" is
  kept. The runtime saves it in the user message's `meta` (`chat_messages.meta`, `sql/060`) with
  `add_message_with_meta`, echoes it on `turn_start` and returns it from `/api/threads/{id}/messages`. The
  model never sees it; no permission reads it. The web chat draws the mark.
- **Client id in meta,** so a watcher that missed `turn_start` (process death) finds its thread.
- **`turn_end` gains `waiting_for_owner`,** true when the turn's `OwnerApproval` left a request waiting
  (items with the prefix `owner-approval:web:{thread}:`).
- **"You said: …" on approvals.** `OwnerApproval::ask` (`owner_approval.rs:138`) puts the spoken question
  at the top of the inbox item when the turn's source is voice; `ChatTurn::owner_approval`
  (`ws/mod.rs:655`) passes it in.

## 10. Acceptance checks, Galaxy A32 (each fails on today's build)

1. Ask something that starts a tool; press Home at 5 s, during "Looking it up". The hub log shows no stop.
   An Updates notice "HQ answered · Tap to read." appears; tap it: the thread has the answer and the Spoken
   mark.
2. **The 3 s window:** ask a question that needs no tool and press Back 1 s after Done: the hub log shows a
   stop, and nothing posts. Again with Back at 5 s: no stop, one notice.
3. **M4:** say "delete the invoice draft task" and close the sheet. Needs you posts "HQ needs your OK to
   finish" with Open only; "HQ answered" never posts. The Inbox item quotes "Delete the invoice draft
   task." After Allow delete, Keep going finishes in the same thread.
4. **Before the thread id:** with a debug delay of 6 s before `turn_start`, close at 4 s: the notice still
   arrives.
5. **Process death:** close during a lookup, then kill the app process from adb on a debug build. After
   the restart exactly one notice posts.
6. **No false post:** a turn whose tool is silent for 200 s (check on device how to stage it) posts
   nothing at 180 s and one notice when it ends.
7. **Lag:** force `stream_lag` with a debug flag: the sheet shows the full answer, and "missed on the way"
   never shows.
8. **R1:** lock the phone before the answer lands. `dumpsys notification --noredact` holds no answer words.
9. **D7:** ask by voice, open the thread in the web app: the question has the spoken mark. Reinstall the
   phone app and sign in: the mark is still there.
10. **M5:** screen timeout 15 s and a 40 s lookup: the screen stays on until the first word or 30 s, then
    may sleep; the answer arrives as a notice.
11. Tap Stop while answering: the hub log shows a stop, the microphone dot does not light, the label reads
    "Stopped".
12. `grep -rn "is not answering\|went quiet\|missed on the way" app/src/main/kotlin/example/hq/spike/voice`
    finds nothing.

## 11. Check on device

- Whether HQ (the hub's child process) outlives a kill of the app process (FixPlan "Still to prove":
  "Child process after a kill").
- How to stage a tool that is silent for 200 s (check 6).

## 12. Open questions and decisions it depends on

None left on the board. D7 settled where the spoken mark lives. M5 offered "drop it" or "30 s"; the board
takes 30 s (decided: N22, "30 s while thinking only").

Depends on: D7, D9, N22, rules sections 1, 2, 4, 8; `../wave-0/01-lock-screen.md` (notice words, details
switch); `../wave-1/03-destructive.md` (red Allow delete); `../wave-4/01-offline.md` (Connection state);
`../wave-0/04-measure.md` (`answer_notice.*`).
