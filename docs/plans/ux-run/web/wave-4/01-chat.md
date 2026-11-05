# Web wave 4 · 01 · Chat, with reply blocks and sheets (WB6, W15 to W17, M9, D7)

Boards: **AlignedChat** (the start screen and chats, a conversation and the details panel, when HQ needs
your OK, the composer), **AlignedChat2** (connection states, Waiting to send, failure lines, every
width, Lite, the phone as the hub), **AlignedChatBlocks** (what a reply can hold), **AlignedChatSheets**
(the note sheet and Export, Add to this chat, commands and message actions), **AlignedShell** frame 1,
AlignedReview WB6, W15 to W17, W22, M9, D7. Follows `../../rules-web.md` sections 1, 2, 4, 6, 8 and 9 and
`../../rules-shared.md`. Where this spec and the rules differ, the rules win.

Chat is the place HQ opens on (W1). It is the phone's Chat at desktop width: the same start, the same
words for a reply in progress, the same composer and sheets. The web keeps what a big screen and a
keyboard add: the chats list beside the conversation, the details panel with Watching from 1400, drag and
drop, and keyboard sending.

## 1. Finding and the review items it closes

- **WB6, W32, W15.** A request in a chat quotes the tool and its arguments, with "In HQ's words" below.
  Decline left, Allow once right, red "Allow delete" when it loses something. "Let HQ answer it" becomes
  "Let HQ say yes to its requests": off, explained once, logged, never a losing request.
- **WB2, W16, W40.** The per-chat reply alert switch goes; alerts are one Updates switch per browser.
- **D7.** Spoken chats carry "Spoken" in the thread and the list; their requests quote "You said, by
  voice".
- **M1, M2.** "Connected" in grey, never "Live". Done ink, working blue, needs you amber.
- **M9, WB4, W23.** Stop goes over HTTP too; a dropped link resumes by message id; HQ drops a resent
  message by client id. "Chat archived · Undo" restores on HQ within 10 minutes.
- **W17, M13.** Enter sends unless an input method is composing or the screen is touch-first, with an
  "Enter sends" setting.
- **M16.** Get started shows the web's real steps; "Connect an AI service" was never a step.
- **W22.** Copy link is a link by the note's id. **WB1:** an export on a shared computer asks first.
- **Drift 03, 06, 09.** Watching in rose with "claude-code" and "native"; chat rows are divs with onClick;
  archive is a trash icon with a red hover.
- **W10, W13, W14.** Get started on the Chat start screen until done; the phone's start words; recent
  chats hidden on the start screen from 1024.

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/hq-web/src/routes/chat.tsx:4-14` `/chat` renders `ChatView`; `components/chat/ChatView.tsx:25`
  `?thread=`; `:21` "Not connected, so the message was not sent. It is back in the box to send again.";
  `:22` "Not connected, so Stop was not sent. The reply keeps running on the server."; `:221` Stop only
  over the socket; `:143-158` archive, `/clear` and `/topic`.
- `components/chat/ThreadSidebar.tsx:75-77` each row is a `<div>` with `onClick`; `:111` a trash icon;
  `:69-70` "Loading chats...", "No chats yet".
- `components/chat/ChatComposer.tsx:9` `TOUCH_QUERY = '(pointer: coarse)'`; `:78-84` Enter sends unless
  Shift, `isComposing`, a touch screen or the slash palette; `:93` the send title "Reconnecting, your
  message will wait" (it does not wait: nothing is queued).
- `components/chat/SlashCommandPalette.tsx:8-9` `/clear` and `/topic <name>` with the phone's words.
- `components/chat/SessionsPanel.tsx`: Watching with "Drive" (`:16` the hint, `:31`), raw statuses
  (`:144`).
- `components/chat/ChatHeader.tsx:29` the reply alert bell.
- `components/VaultNoteDrawer.tsx` the note drawer opened from a backticked path (#210);
  `components/CopyPathMenu.tsx`; `components/ExportMenu.tsx`.
- `components/MarkdownViewer.tsx:69` a chart that fails reads "chart (not rendered: {reason})"; charts by
  `recharts`; `lib/chart.ts`.
- `lib/chatUploads.ts:13` 25 MB per file; `:34` "over 25 MB".
- `lib/getStarted.ts:55-78` steps "Open HQ on your phone", "Connect Telegram or Discord", "Turn on
  notifications"; `components/vault/GetStartedCard.tsx` shows them on Home.
- Server: `crates/hq-web/src/ws/mod.rs:52, 128, 135` the socket takes `client_id` and echoes it on
  `turn_start` but drops no repeat; `:56` and `:164` `Stop {thread_id}`; no HTTP stop; no resume by message
  id; `crates/hq-web/src/threads_api.rs:101` archive only.

## 3. Decision

**The phone's Chat at every width: a start screen with the phone's heading and three suggestions, the
chats list as the list pane from 1024 (a sheet below), the conversation with replies in plain words and
rich blocks, approvals quoted verbatim, the details panel with Watching from 1400, and a composer whose
messages wait to send while HQ is away and never send twice.**

Rejected:

- **A reply alert switch per chat.** One Updates switch per browser (rules-web 1).
- **"Drive" for helpers a chat watches.** It answered prompts toward the goal, losing ones included; "Let
  HQ say yes to its requests" never answers a losing request and lists every answer.
- **Retry a dropped reply by sending the message again.** It would run and cost twice (M9).

## 4. Screens and states

### 4.1 The start screen and your chats (Chat 1)

- **C1 · 1440, first run.** Rail with Chat lit (the HQ mark's green ring). Top bar "Chat" / "Connected" /
  Search "Search notes, tasks and chats" with Ctrl K / chip "Copilot 912 left". List pane "Chats" [New
  chat]; empty: "Your chats will show up here" / "Ask anything in the box on the right. HQ names the chat
  after your first message." Detail pane "New chat": heading (h1) "What would you like to do?", line "HQ
  can read your notes and tasks, write to them, and send helpers to do work.", three suggestions:
  "Summarise this note", "What needs me today?", "Save a thought as a note" (the phone's icons). Card "Get
  started" / "1 of 3 done" [Hide]: "Let HQ reach you · Get a notification in this browser when HQ needs
  you." [Turn on]; "Open HQ on your phone · Sign it in with a code from Other devices." [Get a code]; "Chat
  apps (optional) · Done. Telegram is connected." [Done]. The composer "Message HQ".
- **C2 · 390, first run.** Top bar "Chat" with [Chats] and [New chat]; only the next Get started step
  shows ("Let HQ reach you · Get a notification when HQ needs you").
- **C3 · 390, Get started done.** The card goes for good and the three latest chats take its place:
  "Recent": "Release checklist · Watching 1 helper · 1m", "Samsung battery steps · Open Settings, then
  Battery. · 3m", "What is blocking the soak test? · Reading your tasks · Working" [All chats]. From 1024
  the list pane shows them, so this block is not drawn (W14).
- **C4 · Chats sheet (390).** From the Chats button, 120 px down so the chat stays in view: "Chats" [New
  chat], "Search chats", groups Today, Yesterday, Previous 7 days, Older, as on the phone. Rows: "Release
  checklist · Watching 1 helper · 1m"; "Samsung battery steps · Open Settings, then Battery. · 3m"; "What
  is blocking the soak test? · Reading your tasks · Working" (blue spinner); "What is on my list today ·
  Spoken · You have three tasks due today. · 9:42" (mic icon); "Telegram check · The relay is connected ·
  Tue"; "Plan the phone release · Three steps left before Friday. · Sat"; "Tidy the export names · Done. 12
  files renamed. · 2 Nov". Swipe a row left, or its archive button, to archive: "Chat archived · Undo",
  kept until used or closed.
- **Rules here:** a suggestion fills the box, it does not send. "Summarise this note" shows only when a note
  is attached; otherwise "Summarise this week's notes". Get started's steps are the web's real three
  (`lib/getStarted.ts`), named as in the phone's first run; the AI key is not a step (`/setup` comes
  first). Hide keeps it out of Chat; it stays in Settings, This browser until done. Rows are real links
  with the archive button beside them, never inside. Unread is the blue dot. In Lite there is no Chat.

### 4.2 A conversation, a helper it watches, the details panel (Chat 2)

- **V1 · 1440.** List pane, conversation, and from 1400 the details panel (320). Title "Release
  checklist". You: "Start a helper on the Gradle build and tell me when it needs me." HQ: "I started Claude
  on your Mac mini, on task T-12 Gradle build. I am watching it here and will tell you when it asks for
  anything." "What I did · 2 steps" (folded), meta "14:02 · Copilot". An update from the watched helper:
  "Claude needs your OK · Needs your OK · Move the build from scripts to Gradle · wants to edit one file,
  app/build.gradle.kts." [Answer it in Helpers], meta "14:20 · from the helper you watch". You: "While it
  works: what is left on the release checklist?" HQ, in progress: "Reading Release-checklist…" (blue,
  spinner), the steps on a hairline ("Checked your tasks", "Searched your notes for "release""), then the
  text as it arrives: "Three things are left before Friday: sign the build, write the release notes,
  and". Send has turned into Stop.
- **V2 · Details panel.** "Details" / "Watching 1": the helper's card "Move the build from scripts to
  Gradle · Needs your OK · Claude · on your Mac mini · Started 40 min ago", "Task · T-12 Gradle build",
  "Goal · Move the build from shell scripts to Gradle.", "Done when · The Android app builds with one
  Gradle command." [Answer it]. Switch "Let HQ say yes to its requests" / "Off. When on, HQ answers this
  helper toward the task's goal. Never a request that deletes or stops something." [Stop watching] / "Its
  updates stop coming here. It keeps working." "This chat" / "Alerts when a reply finishes are set once
  for this browser, not per chat: Settings, This browser, Notifications." [Rename] [Archive chat].
- **V3 · 390, while HQ works.** What it is doing now, the steps so far on a hairline, then the text; Send
  turns into Stop in the same place.
- **V4 · Details on a phone.** The info button opens the details panel as a sheet; from 1024 to 1399 a
  side sheet; from 1400 it docks.
- **V5 · Stopped, steps open, scrolled up.** "Release notes": "Here is a first pass, grouped by place.
  Chat. Replies can hold tables and charts. Archive has Undo. Notes. Export as PDF, Word, a picture and
  more." "What I did · 3 steps" open: "Read 2 notes", "Checked your tasks", "Searched your notes for
  "merged"". "Stopped. What I had so far is kept." 14:31. A [Latest] pill returns to the newest message.
  Copy and Regenerate sit under the last reply.
- **Rules:** a reply in progress leads with what HQ is doing now, blue with the spinner. Finished, the
  steps fold into "What I did · N steps"; raw tool names and timings move under Technical details in the
  steps list. Stop: Send turns into Stop; Esc does the same while the box has focus; Stop goes over the
  socket and over HTTP. If neither reaches HQ: "Couldn't stop the reply. Your HQ can't be reached. Try
  again in a moment." Watching: each helper as its card in the Helpers words and colours, never "blocked"
  in rose or "claude-code" and "native".

### 4.3 When HQ needs your OK in a chat (Chat 3)

- **A1 · Owner approval, losing.** You: "Clear out the old soak tasks, then tell me how the Gradle helper is
  doing." HQ: "One old soak task is left, T-9. Deleting a task needs your OK, so I asked below. The Gradle
  helper is waiting for you too." Card "HQ needs your OK to finish · Needs your OK" / "What HQ wants to
  run" / `task_delete` / `{ "id": "T-9" }` / "In HQ's words" / "Delete the task "Fix the share sheet
  title". It has no sub-tasks. This cannot be undone." / [Decline] [Allow delete] (red).
- **A2 · Details panel with Let HQ say yes on.** Watching card: "Wants to delete 3 old build folders. HQ
  does not say yes to this kind of request, so it waits for you." Switch "Let HQ say yes to its requests"
  / "On. HQ answers this helper toward the task's goal. It never says yes to deleting or stopping
  something." "What HQ said yes to · 2": "14:12 · "Use Gradle 8.7?" HQ answered Yes." and "14:18 · Allowed
  once: edit app/build.gradle.kts".
- **A3 · A request from a spoken chat (390).** You: "Check what changed in the repo." with "Spoken". Card
  "HQ needs your OK to finish · Needs your OK" / "You said, by voice · 9:42" / ""Check what changed in the
  repo."" / "What HQ wants to run" / `bash` / `{ "command": "git status --short" }` / "In HQ's words" /
  "Show which files changed in the repository. It only reads." / [Decline] [Allow once] (green: it loses
  nothing).
- **A4 · The first time it is turned on.** "Let HQ say yes to its requests?" / "HQ answers this helper's
  questions and allows its steps, toward the task's goal. It never says yes to deleting or stopping
  something. Those wait for you. Every answer it gives is listed under Watching." / [Keep asking me] [Let
  HQ say yes]. Both labels are short, so side by side.
- **A5 · Answered after HQ stopped waiting.** "Deleting a task needs your OK, so I asked. I will finish
  once you answer." Card "Allowed at 14:24 · Answered", the same quote; "HQ stopped to wait for you. Keep
  going asks it to finish in this chat." [Keep going].
- **Rules:** (9) the tool name and its arguments verbatim from HQ's runtime (`owner_approval.rs` shows them
  without HQ's own `_hq_` keys, up to 3000 characters), HQ's description below under "In HQ's words";
  (10) Decline · Allow once, the phone's words and order; a request the runtime marks as losing, such as
  `task_delete`, gets a red "Allow delete" on the right; Back, Esc and a click outside choose Decline;
  (11) the card is the same Needs you item as in Inbox, so answering in either answers both; HQ waits 60 s,
  then tells the owner what it wants; an unanswered item ends after 24 h, and at most 5 wait per chat;
  (12) Let HQ say yes: off by default, one explainer the first time, every answer listed under Watching
  with its time, never a losing request, and its routes need the HQ client header (`origin.rs`); (13)
  spoken chats show "Spoken" on the message and in the list; requests quote "You said, by voice · 9:42" and
  the words heard; the mark comes from a source field the runtime sets, never the model (D7); the chat
  socket takes no source today, so it is new.

### 4.4 The composer (Chat 4)

One box at every width, centred and 760 wide from 1024: the plus, the box, then Send or Stop. Everything
that goes with the next message is a chip above it.

- **K1 · Empty.** "Message HQ". Send off until there is text or a file. Plus opens Add to this chat.
- **K2 · Typed.** Green send. Enter sends, Shift Enter adds a line, up to 200 px then the box scrolls.
- **K3 · Replying.** Stop in the place of Send. The box still takes the next message.
- **K4 · With a note and a task.** Chips "Mobile-First-Roadmap · Note" and "T-12 Gradle build · Task" over
  "What does the roadmap want?". HQ reads only what is shown there.
- **K5 · Files waiting to go.** "soak-log.txt · 1.2 MB", "screen.png · Uploading…",
  "A-very-long-video-name.mp4 · over 25 MB" (amber), "photo.jpg · couldn't upload" [Retry]. Under: "Remove
  the file over 25 MB to send." Retry only where trying again can work. Up to 10 files; Send waits until
  every file is ready.
- **K6 · Editing a message.** A bar "Editing a message · Sending replaces what came after" [Stop editing];
  the box holds "How do I stop Samsung putting HQ to sleep?". From a message's Edit and resend.
- **K7 · Typing a slash.** "/clear · Archive this chat and start a new one", "/topic <name> · Archive this
  chat and start a new one with that name". Up and Down choose, Enter or Tab fills, Esc closes. "/reset"
  gets "That command is not used here. Send /clear to start a new chat." as a toast; the text stays.
- **K8 · Dropping files (web only).** "Drop files to add them" / "Up to 10 files, 25 MB each. HQ reads them
  before it answers." Same limits and chips as the plus.
- **Keyboard:** Enter send, unless an input method is composing; Shift Enter new line; / commands at the
  start of the box; Esc close the list, or stop a reply; Ctrl V paste a picture as a file. On a
  touch-first screen Enter adds a line and only the button sends (`ChatComposer.tsx` does both checks
  today). New: Settings, This browser, "Enter sends" (W17).

### 4.5 Connection states in Chat (Chat 2, section 1)

The status line says the state first and wraps. Chat adds one banner over the conversation in every state
but Connected, because a draft is at stake.

| Status line | Colour | Banner over the chat | Send | A reply in flight |
|-------------|--------|----------------------|------|-------------------|
| Connected | Grey, balance chip shown | None | On | Streams |
| Starting HQ · 12 s | Blue | "HQ is starting. What you send waits here until HQ is up." | On, into Waiting to send | Goes on once HQ is up |
| Reconnecting | Blue, Chat only | "Reconnecting. The reply goes on when the link is back." | On, into Waiting to send | Goes on from the last part, by message id ("Paused at the dropped link") |
| Your HQ can't be reached | Amber | "Your HQ can't be reached. What you send waits here until it is back." [Details] | On, into Waiting to send | "Couldn't finish this reply. Your HQ can't be reached." Retry once back |
| You're offline | Amber | "You're offline. What you send waits here until you're back online." | On, into Waiting to send | As above, with the offline reason |
| The AI service isn't answering | Amber | "The AI service isn't answering. HQ is fine. Notes and tasks work." [AI service and keys] | On | "Couldn't finish this reply. The AI service isn't answering. HQ is fine. Try again soon." Retry |
| Signed out of this HQ | Amber | None. Only Sign in is shown ("Sign in to see your chats" / "HQ no longer accepts this browser's sign-in, so nothing from it is kept here.") | None | None |
| HQ Lite | Grey | No Chat: a link to `/chat` opens Notes with "This HQ is HQ Lite. It has no Chat." (WR4) | None | None |

Under the banner in states 3 and 5: "Saved in this browser at 14:32." The status for the AI comes from the
signed-in AI status route, never `/health`.

### 4.6 Waiting to send, a dropped link, the failure lines (Chat 2, section 2)

- **W1 · In the thread.** "Waiting to send 1 · Check the backup status · Saved in this browser. It will
  send when HQ is up." (or "…when HQ is back.", "…when you are back online.").
- **W2 · Back, sending in order.** "Check the backup status · Sending 1 of 2"; "Waiting to send 1 · And the
  phone one? · Saved in this browser. It is next." One at a time, oldest first; each carries a client id,
  so HQ drops a repeat.
- **W3 · Waited too long.** "Couldn't send your message. It waited 24 h, so it was not sent." [Send now];
  Remove takes it out.
- **W4 · On a shared computer.** "Your HQ can't be reached. On a shared computer nothing waits to send." /
  "Nothing from this chat is kept on a shared computer." Send off with "Send turns on when HQ is back. Your
  text stays in the box."
- The failure lines per reason and action are the catalogue rows in `../wave-0/02-shared-parts.md`
  (send, stop, archive × starting, can't be reached, offline, no reply in time, AI down; "Not needed:
  nothing is running." for Stop when the AI is down, "Archives as usual." for archive).
- Retired: "Not connected, so the message was not sent", "Picks up where it was", "Live", "Saved · 3 min",
  "Can't reach HQ", "Signed out · Saved · 2 h", "HQ did not finish a reply", "HQ is not answering", the
  Checking balance chip.

### 4.7 Every width, Lite and the phone as the hub (Chat 2, section 3)

- **768 to 1023 (900):** the rail replaces the bottom bar. One pane: the conversation, with Chats in its
  head as on a phone (the list pane starts at 1024). The details panel is a side sheet from the info
  button. The thread stays 760 wide at most.
- **2200 (drawn at 64%):** list, conversation and the docked details panel; the thread stays 760 wide and
  centred; the list pane and the panel keep their widths; the extra space is margin.
- **320:** the top bar holds the title and the state, which wraps; Search, the chip and the gear in the
  More menu (`../README.md` Open conflict 3). Nothing scrolls sideways.
- **200% (a 1024 window lays out at 512):** the phone layout. The toast moves above the box while the box
  has focus, never over it. Browser zoom keys stay the browser's.
- **Lite:** no Chat (4.5).
- **HQ window, the phone as the hub:** title "HQ · HQ on your phone"; the same Chat; saved copies keyed by
  the HQ's id. No downloads, no web push and no install in this window; HQ Hub posts a system notice for
  Needs you items instead. What this window sends and receives travels to the phone and counts as its
  data.

### 4.8 What a reply can hold (Chat blocks)

A reply is Markdown read into blocks. On the web the thread is 760 wide.

- **B1 · Cards for what HQ made.** "New note · Samsung battery steps · Projects › Agent-HQ · just now";
  "New task · Check Samsung battery whitelist · Due tomorrow · High". The whole card is the link: a note
  opens the note sheet over Chat, a task opens it in Tasks.
- **B2 · What it read.** "Read 2 notes": chips "Mobile-First-Roadmap", "Android-On-Device-Assessment".
  Past three the rest fold into "+N more". Backticked paths and [[links]] in the text open notes too.
  "Opening…" while a note is fetched; "· not found" with a dashed edge when it is gone.
- **B3 · Tables.** Fits: drawn plain, last column stretched (a five-column table fits whole at 760, no
  pinned column, no fade, the bar keeps Copy and Expand). Wider than the thread: a bar with its size ("8
  rows · 5 columns"), Copy and Expand, and the box scrolls sideways. At 390 a wide one pins its first column
  at 104 px and fades at the right edge. Passed is ink with a check, Failed amber with a warning sign.
  Example: "Test · Result · Time · Battery · Phone" with rows Cold start Passed 1.2 s 0% Galaxy A32; Reboot
  Failed n/a n/a Galaxy A32; Doze, 8 hours Passed 8 h 3% Pixel 6a; Warm start Passed 0.6 s 0% Galaxy A32;
  Share capture Passed 2.1 s 1% Pixel 6a; Widget add Passed 1.4 s 0% Pixel 6a; Low battery Passed 3 h 4%
  Galaxy A32; Airplane mode Failed n/a n/a Galaxy A32.
- **B4 · Charts.** Line, bar, pie and donut in the chart palette (blue, sky, white, grey, indigo),
  labelled directly, a value on hover and "The numbers" under the expanded chart. Example donut "Where the
  time went · Donut": "Reading notes 16 min · 42%", "Writing 12 min · 32%", "Waiting 7 min · 18%", "Other 3
  min · 8%". Blue and sky sit close, so a line chart's second series is also dashed and the legend names
  the line style ("Web · solid line", "Mobile · dashed line").
- **B5 · Code.** Never wraps; scrolls under a language bar ("bash") with Copy.
- **B6 · A file.** "File · Battery-checklist.docx · Word document · 24 KB" [Open]. It opens in the web
  viewer or downloads. On a shared computer, Download first says "The file stays in this computer's
  Downloads."
- **B7 · Links, quotes, checklists, refusals.** A web link carries the external icon. A quote ("Samsung may
  undo this after an update."). Checklist "Allow background activity, done" (ink with a check, never
  green) and "Turn off automatic removal, not done". "Picture not shown · HQ does not load pictures from
  the internet inside chats." A chart that cannot be drawn: "Couldn't draw this chart. Series 1 needs 3
  values, one for each label." and the data as written (`{"type":"bar","labels":["A","B","C"],
  "series":[{"values":[4,7]}]}`).
- **B8 · Expanded.** Expand opens the block on its own: a centred sheet from 768, a page over the shell on
  a phone. The chart page: "Weekly signups" [Copy the numbers] / "Line chart · 2 series · hover the chart
  for a value" / a hover "W4 · Web 22 · Mobile 16" / "The numbers": Week, Web, Mobile; W1 12 5; W2 18 9;
  W3 15 14; W4 22 16; W5 25 21. The table page: "Table · Soak test results · 8 rows · 5 columns" / "Scroll
  sideways for more columns. The first column and the heading row stay in place." Copy puts it on the
  clipboard tab separated and says "Copied the table".
- **B9 · 320.** The donut sits above its legend; the table pins its first column and scrolls. A 640 px
  window at 200% lays out the same way.
- Chart and code readers load only when a reply needs them (rules-web 9).

### 4.9 What opens over Chat (Chat sheets)

- **S1 · The note sheet.** A New note card, a note chip or a backticked path opens it: from the right from
  768 (440 wide; the chat stays visible; Esc closes it), from the bottom on a phone (96 px down, always
  tall). Head "Samsung battery steps · Projects › Agent-HQ", "Saved by HQ · just now" (or "Edited by you ·
  2 h ago", or nothing when there is no record). The note. [Ask about this] [Open in Notes]. A long note
  ends "This note is long. Open it in Notes to read all of it."
- **S2 · Its menu.** "Pin to top", "Ask HQ about this note", "Export as… · PDF, Word, a picture and more",
  "Copy link · Opens only on your signed-in devices" (or "Share" where the browser can share), "Copy path",
  "Copy note text". From a chat there is no Edit and no Move to trash. Copy link is a link by the note's id
  (`/notes/n/:id`), not its folder; it keeps working when the note is renamed. Copy path copies where the
  note lives, for helpers and scripts.
- **S3 · Export as.** "Export as · Samsung battery steps": "Document": "PDF document · Best for printing
  and sending · .pdf", "Word document · Opens in Word · .docx", "Web page · Opens in a browser · .html",
  "Markdown · Plain text with the marks · .md"; "Picture": "Whole note as one image · One tall picture ·
  .png", "Sharp at any size · A vector image · .svg"; "Tables in this note" (only when the note has a
  table): "Excel workbook · Opens in Excel · .xlsx", "CSV file · Opens in any spreadsheet · .csv", "JSON
  file · For programs · .json".
- **S4 · Making, done, failed, shared.** "Making your PDF… · HQ is laying out the pages. Large notes take a
  little longer." [Cancel]; every other format waits. Done: "Samsung battery steps.pdf is in your
  downloads". Failed: "Couldn't make the PDF. HQ didn't answer in time. Try again." [Try again] (reopens
  Export as with the same format chosen). With HQ away Export is off: "Exporting needs HQ. Your HQ can't be
  reached." On a shared computer: "Save on this computer?" / "This is a shared computer. The file stays in
  this computer's Downloads, where others can open it." / [Don't save] [Save the PDF] (green: nothing is
  lost either way).
- **S5 · Add to this chat.** Plus opens a short menu above the box from 768: "HQ reads what you add before
  it answers." "A note", "A task", "A picture", "A file", and "Or drop files on the chat, or paste a
  picture." Below 768 the phone's four tiles ("A note", "A task", "A photo", "A file"). A photo opens the
  browser's camera or pictures; a file its picker.
- **S6 · Pick a note.** Starts 152 px down; "Search your notes"; rows "Mobile-First-Roadmap · Projects ›
  Agent-HQ", "Roadmap-2026 · Projects › Example"; a chosen row is added as a chip. Pick a task works the
  same. Empty: "Type a few words to find a note." Waiting: "Looking…". No hit: "Nothing matches that."
  With HQ away: "Searching needs HQ. Your HQ can't be reached right now."
- **S7 · Message actions.** On the web they show on hover and keyboard focus: Copy and Regenerate under
  the last reply, Copy and Edit and resend under your own. On touch, a long press opens a sheet: "Reply ·
  14:02", "Copy text", "Select text", "Regenerate reply · Runs the last question again. This reply is
  replaced." On your own message: "Edit and resend: Puts the question back in the box. Sending it replaces
  what came after."

## 5. Exact copy

Every string quoted in section 4 is exact. Keys `chat_*`. The ones most often built wrong:

| Key | String |
|-----|--------|
| `chat_start_title` | "What would you like to do?" |
| `chat_start_line` | "HQ can read your notes and tasks, write to them, and send helpers to do work." |
| `chat_suggest` | "Summarise this note" (only with a note attached; else "Summarise this week's notes") · "What needs me today?" · "Save a thought as a note" |
| `chat_get_started` | "Get started" · "{1} of {3} done" · "Hide" · "Let HQ reach you" · "Get a notification in this browser when HQ needs you." · "Turn on" · "Open HQ on your phone" · "Sign it in with a code from Other devices." · "Get a code" · "Chat apps (optional)" · "Done. {Telegram} is connected." · "Done" |
| `chat_list_empty` | "Your chats will show up here" · "Ask anything in the box on the right. HQ names the chat after your first message." |
| `chat_groups` | "Today" · "Yesterday" · "Previous 7 days" · "Older" |
| `chat_spoken` | "Spoken" · "You said, by voice · {9:42}" |
| `chat_did` | "What I did · {N} steps" |
| `chat_stopped` | "Stopped. What I had so far is kept." |
| `chat_watching` | "Watching" · "Answer it" · "Stop watching" · "Its updates stop coming here. It keeps working." · "Answer it in Helpers" · "from the helper you watch" |
| `chat_say_yes` | "Let HQ say yes to its requests" · off "Off. When on, HQ answers this helper toward the task's goal. Never a request that deletes or stops something." · on "On. HQ answers this helper toward the task's goal. It never says yes to deleting or stopping something." · "What HQ said yes to · {N}" |
| `chat_say_yes_dialog` | "Let HQ say yes to its requests?" · "HQ answers this helper's questions and allows its steps, toward the task's goal. It never says yes to deleting or stopping something. Those wait for you. Every answer it gives is listed under Watching." · "Keep asking me" · "Let HQ say yes" |
| `chat_alerts_note` | "Alerts when a reply finishes are set once for this browser, not per chat: Settings, This browser, Notifications." |
| `chat_approval` | "HQ needs your OK to finish" · "What HQ wants to run" · "In HQ's words" · "Allowed at {14:24}" · "Answered" · "HQ stopped to wait for you. Keep going asks it to finish in this chat." · "Keep going" |
| `chat_composer` | "Message HQ" · "Editing a message" · "Sending replaces what came after" · "Stop editing" · "Drop files to add them" · "Up to 10 files, 25 MB each. HQ reads them before it answers." · "Remove the file over 25 MB to send." · "Uploading…" · "over 25 MB" · "couldn't upload" · "Retry" |
| `chat_slash` | "/clear · Archive this chat and start a new one" · "/topic <name> · Archive this chat and start a new one with that name" · "That command is not used here. Send /clear to start a new chat." |
| `chat_archived` | "Chat archived" · "Undo" |
| `chat_block_file_shared` | "The file stays in this computer's Downloads." |
| `chat_block_picture` | "Picture not shown" · "HQ does not load pictures from the internet inside chats." |
| `chat_chart_fail` | "Couldn't draw this chart." + the reason |
| `chat_table_copied` | "Copied the table" |
| `note_sheet_long` | "This note is long. Open it in Notes to read all of it." |
| `copy_link` | "Copy link" · "Opens only on your signed-in devices" |
| `export_*` | as S3 and S4 |

## 6. Behaviour rules

1. **Chat opens on a new chat** at `/chat`; a chat at `/chat/:id`; `/chat?thread=` redirects
   (`../wave-2/01-route-table.md`). The start screen loads no reader or chart code (W1: stays light).
2. **The chats list** is the list pane from 1024, a sheet below; rows are links; archive beside them; an
   archive icon, never a trash can or a red hover. Archive has no question and offers Undo (restore on HQ
   within 10 minutes, `../wave-1/03-undo.md`); `/clear` archives the same way.
3. **Send while HQ is away** puts the message in Waiting to send with a client id (`../wave-0/02-shared-parts.md`
   rules 11 to 15); Send stays on (WR26) except on a shared computer.
4. **Never twice.** HQ drops a repeat `client_id` on the socket and the messages route.
5. **Resume by message id.** On reconnect the page sends `{type: "resume", thread_id, after_message_id,
   after_part}`; HQ streams the rest of that reply from where the page's last part ended.
6. **Stop over both.** Stop is sent on the socket and as `POST /api/threads/{id}/stop` (NEW); either one
   ends the turn.
7. **Approvals** follow `../wave-1/02-dialogs-approvals.md`; the chat card is the same item as Inbox's.
8. **Let HQ say yes** needs the client header on its routes; the server never says yes to a losing
   request; every automatic answer is stored and listed under Watching.
9. **Spoken** comes from `source: "voice"` on the message meta, set by the runtime from the connection
   (D7); the web shows it, never uses it for permissions.
10. **Enter sends** per K-keyboard; the "Enter sends" setting lives in This browser.
11. **Toasts never cover the composer** while it has focus.
12. **Exports** follow S4; on a shared computer they ask first; in the HQ window Export is off with the
    reason (WR13).
13. **Edited within 10 s of Enter** counts `edit.within_10s` (`../wave-0/02-shared-parts.md`).

## 7. Browser and platform notes

- `isComposing` covers Chinese, Japanese and Korean IMEs in every browser; Safari fires `keydown` Enter with
  `keyCode 229` while composing on some versions: treat `keyCode === 229` as composing too.
- `navigator.share` exists on Safari, Chrome on Android and Edge on Windows; elsewhere Copy link shows in
  its place (S2).
- Drag and drop of files works in every desktop browser; on iPadOS drag from Files works in Safari 15+.
- Paste of a picture uses `clipboardData.files` (today's `ChatComposer.tsx:86-91`).

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/hq-web/src/routes/chat.tsx`, `chat/$id.tsx` | Routes. | yes, `$id` NEW |
| `apps/hq-web/src/components/chat/ChatView.tsx` | Start screen, panes, banner per state, outbox, Stop over HTTP, resume (21-22, 143-159, 221). | yes |
| `apps/hq-web/src/components/chat/ThreadSidebar.tsx` | Links, groups, archive icon, Undo (69-111). | yes |
| `apps/hq-web/src/components/chat/ChatComposer.tsx` | Chips, files, editing bar, keyboard rules, "Enter sends" (9, 78-93). | yes |
| `apps/hq-web/src/components/chat/SessionsPanel.tsx` | Watching cards; Let HQ say yes; the log of answers. | yes |
| `apps/hq-web/src/components/chat/ChatHeader.tsx` | No bell; info button. | yes |
| `apps/hq-web/src/components/chat/LiveTurnView.tsx`, `ToolCallBlock.tsx`, `ChatBubble.tsx`, `MessageList.tsx` | Reply in progress, steps, Stopped, Latest, actions. | yes |
| `apps/hq-web/src/components/chat/SlashCommandPalette.tsx` | Words; `/reset` toast. | yes |
| `apps/hq-web/src/components/MarkdownViewer.tsx`, `lib/chart.ts` | Blocks; chart palette; "Couldn't draw this chart." (69). | yes |
| `apps/hq-web/src/components/VaultNoteDrawer.tsx` | Becomes the note sheet. | yes |
| `apps/hq-web/src/components/ExportMenu.tsx`, `CopyPathMenu.tsx` | Export as groups; Copy link by id; shared-computer ask. | yes |
| `apps/hq-web/src/lib/getStarted.ts`, `components/vault/GetStartedCard.tsx` | Steps on Chat, the web's real three. | yes |
| `apps/hq-web/src/lib/chatUploads.ts` | Limits and words. | yes |
| `crates/hq-web/src/ws/mod.rs` | Drop repeat `client_id`; `resume`; `source` echoed. | yes |
| `crates/hq-web/src/threads_api.rs` | `POST /api/threads/{id}/stop`, unarchive, rename. | yes, routes NEW |
| `crates/hq-web/src/session_driver.rs`, `sessions_api.rs` | Let HQ say yes: refuse losing, log answers, client header. | yes |
| `crates/hq-db` chat | `source`, `client_id` in `chat_messages.meta`. | yes |

## 9. Server, desktop or release changes

Server: client-id dedup, resume by message id, Stop over HTTP, unarchive and rename, `source` on messages,
the losing flag and Let HQ say yes rules, owner approval item with the tool and arguments
(`../../changes.md` rows for these). Desktop: the system notice in HQ Hub. Release: none.

## 10. Acceptance checks (each fails on today's build)

1. The start screen reads "What would you like to do?" with the three suggestions and Get started with
   "Let HQ reach you". Today Chat opens without them and Get started is on Home.
2. Send with the socket closed: the message shows under Waiting to send; on reconnect HQ stores it once.
   Today "Not connected, so the message was not sent."
3. Stop with the socket closed reaches HQ over HTTP. Today it is lost (`ChatView.tsx:221`).
4. A dropped socket mid-reply resumes the same reply without a second turn (server test counting turns).
5. A `task_delete` owner approval in a chat renders `task_delete` and `{"id": "T-9"}` and a red "Allow
   delete". Today the chat shows no such card.
6. Let HQ say yes never answers an item with `loses: true` (server test).
7. Chat rows are `<a>` elements reachable by Tab; archive is an archive icon with "Chat archived · Undo".
8. A failed chart reads "Couldn't draw this chart." plus the reason. Today "chart (not rendered: …)".
9. Charts use only the five palette colours (a test over `lib/chart.ts` output).

## 11. Check on a laptop or phone

- Airplane mode mid-reply on a laptop: Reconnecting, then the reply goes on.
- An IME (Japanese) in Chrome and Safari: Enter commits the word, not the message.
- A phone browser: long press a reply for its sheet.

## 12. Open decisions it depends on

- **W1, W10, W13 to W17, W22, W23, WR26, D7, N17** (Recommended or Revised).
- Depends on: `../wave-0/*`, `../wave-1/02-dialogs-approvals.md`, `../wave-1/03-undo.md`,
  `03-notes.md` (the note sheet's Notes side), `07-helpers.md` (Answer it in Helpers).
