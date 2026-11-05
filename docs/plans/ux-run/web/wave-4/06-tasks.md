# Web wave 4 · 06 · Tasks: the list, checking, the tools, Board and Timeline (W28 to W30, W33, WR14 to WR16)

Boards: **AlignedTasks** (board 1 of 4: desktop Today with a task open; All, "Ready to check" and the
check; picking several and when one done asks first; While you were away; can't be reached and empty),
**AlignedTasks2** (board 2: every width, Lite, the phone as the hub), **AlignedTasks3** (board 3: areas,
folders and lists; New task and Edit details; decisions, where the work left off, what it waits on; Find in
tasks, Collapse all, the chips), **AlignedTasks4** (board 4: the owner's checking tools: Check them now,
apply and Undo, how checking is going, the audit, an archived task and Delete for good),
**AlignedTasksViews** (Board, Timeline Week and Month, phone width, embedded, other widths), AlignedReview
M1 to M4, M12, W28 to W33, Drift 06, 09, 16. Follows `../../rules-web.md` and `../../rules-shared.md`.
Where this spec and the rules differ, the rules win.

## 1. Finding and the review items it closes

- **M1, M2, M3** on Tasks: the state first, late amber with an icon, a met check ink, red only on Delete for
  good; failure lines from the catalogue.
- **W33 revised.** One done acts at once with Undo, but asks first when other tasks wait on it or a check is
  pending.
- **WB4 (Undo).** "Done: {title} · Undo" and "3 tasks archived · Undo" restore on HQ within 10 minutes.
- **M4, WR14.** "Space" and "Initiative" become "area" and "list"; the filter is "Filter by list".
- **M12 (Tasks).** Dropped capabilities come back: verdict history, areas and lists, the task form,
  dependencies, the checkpoint, decisions, Collapse all, the stale and long-running chips, the tasks search,
  sweeps, queue health, the rollout and the audit, Timeline zoom.
- **W28.** Progress "1 of 3 sub-tasks done". **W29.** Archive with Undo is everyday; Delete for good only
  on an archived task. **W30.** Checking words. **W7.** List, Board, Timeline as a switch from 768; "Show
  as" below. **WR15, WR16.**
- **Drift 06.** Task cards are divs with onClick; the drawer's close button has no name; deleting a task
  without sub-tasks has no confirm. **Drift 09.** "Return to to_do", "waiting for your agree or not",
  "claude-code on claude-code". **Drift 16.** On a phone Tasks opens on verification, not tasks.

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/hq-web/src/routes/tasks.tsx:78` `StatusTab = 'active' | 'all' | 'complete' | 'archived'`; `:397`
  `TasksSidebar`; `:418` `VerificationBar` (with Verify now and the audit) and `:419` `BriefingCard` above
  the filters, filling a phone's first screen (Drift 16); `:70` `?view=` and `?task=`; `matchesSearch` (the
  list's own search).
- `components/tasks/TaskCard.tsx:66-70` the row is a clickable element with `onClick` (a div); chips
  "stale" and "long running".
- `components/tasks/TaskDetailDrawer.tsx:108-110` delete without a question when there are no sub-tasks;
  `:181` a trash icon with a rose hover; `:197-206` a rose strip for cascade; `:221-224` archived note.
- `components/tasks/TasksSidebar.tsx:152` "New space", `:208, 251` "New list"; `:4` imports
  `createFolderClient` but never calls it.
- `components/tasks/TaskFormModal.tsx:89` a task with no list goes to "Inbox"; `:293` "Assigned to
  (comma-separated; an agent named here is notified)".
- `components/tasks/TaskListView.tsx:57, 65` Collapse all, Expand all. `TaskRelations.tsx`,
  `TaskCheckpoint.tsx`, `TaskDecisions.tsx` (`GET /api/tasks/{id}/decisions`), `WorkingNowPanel.tsx`
  ("claude-code on claude-code"), `BulkActionBar.tsx`.
- `components/tasks/TaskGanttView.tsx:56` and `gantt.ts:7` `DAY_PX = { week: 36, month: 12 }`;
  `TaskBoardView.tsx` with a "Check it" button inside each card link.
- `components/tasks/verification/`: `SweepSheet.tsx` (scopes "Everything waiting", "This initiative",
  "Since a date", "The tasks I picked"; stages from `verification.ts`: Reading the evidence, Plan ready,
  Verifying, Paused, Ready to apply, Applying, Applied, Undone, Cancelled; `BUCKET_INFO`),
  `QueueHealthPanel.tsx`, `RolloutPanel.tsx`, `AuditSheet.tsx` (Agree, Disagree with a note, Skip, "What HQ
  saw"), `TaskVerification.tsx:166` "Return to to_do", `BriefingCard.tsx:160` "…waiting for your agree or
  not."
- Server: `crates/hq-web/src/tasks_api.rs:908` restore; `crates/hq-core/src/config/tasks.rs:242`
  `stale_after_hours` (72 by default), `:96` `sweep_confirm_over` (25, `:158`); `undo_sweep` in hq-tools;
  `crates/hq-db/src/tasks/rollout.rs`.

## 3. Decision

**The phone's three tabs, Today, Upcoming and All, with the same groups, words and rows. The web keeps what
it does that the phone does not: the List, Board and Timeline switch from 768, picking several, the list
filter, and HQ's own checking of finished work, shown as a "Ready to check" chip and a card on the task.**

Owner decisions on the boards (all Recommended):

1. **Names (WR14):** "area" for a space, "folder" for a folder, "list" for an initiative, on web and phone
   alike. "Project" was the other choice, but Notes already has a Projects folder. The default list keeps
   the phone's "your Inbox list".
2. **Editing (WR15):** status, priority and dates stay one tap on the task page; everything else moves to
   Edit details, replacing the drawer's pencil buttons. Delete leaves the task page.
3. **Two searches (WR16):** keep "Find in tasks" in the list; Search (Ctrl K) finds tasks among everything
   but cannot filter the board.
4. **Checking words (W30):** "sweep", "tier", "shadow", "enforce", "veto" and "canary" stay in Technical
   details.

Rejected: Delete on every task (today's drawer); a "Check it" button inside a card link; verification
above the list on a phone.

## 4. Screens and states

### 4.1 Desktop, Today, a task open (Tasks 1)

- **T1 · 1440.** Top bar "Tasks" / "Connected · 9 active" / chip / the segmented switch List · Board ·
  Timeline. List pane head "Your tasks" [New task]; tabs "Today", "Upcoming", "All 2". A link row "While
  you were away · HQ checked and finished 6 tasks." (not a block; the phone's first sentence,
  `../../resolutions.md` B7). Groups: "Waiting on you 2": "Soak, reboot
  test, cleanup of old copies · Stuck · waits for a reboot window" (amber Stuck with an icon); "Nightly
  off-device backup · Ready to check · HQ finished 3 h ago" [Check it] (beside the row link, never inside).
  "In progress": "Workstream A: first run for a phone-only user · 1 of 3 sub-tasks done"; "Workstream C: a
  native view instead of the WebView · High · due Sunday · 1 of 3 sub-tasks done". "Due or starting
  today": "Release build and certificate renewal · due today". "Done today" folded.
- **T2 · The open task.** "Workstream C: a native view instead of the WebView" / "In progress", "High", "Due
  Sun 8 Nov", "T-4" / "A native view for the whole app, so the phone stops loading the web pages for Board
  and Timeline. Start with the task list." "Working on it now": "Build the native task list · Working on it
  · Codex · on your Mac mini · 25 min · Writing the list screen". "Sub-tasks · 1 of 3 done": "Map the
  current mobile UI · Done · Tuesday", "Draft the Compose design · In progress · Codex is working", "Spike
  chat for T-5 · To do · waits on Draft the Compose design" [Add a sub-task]. "Comments": "Alex · Yesterday
  18:40 · Keep the swipe gestures from the phone list. Board can wait." [Add a comment].
- **T3 · Details panel (from 1400).** "Status", "Priority", "Starts · Mon 2 Nov", "Due · Sun 8 Nov", "Agent
  HQ › Phone", "Waits on · Nothing", "Time on it · 3 h 10 min"; "Links": "Mobile-First-Roadmap · Note ·
  Projects › Agent-HQ", "Pull request #214 · Opens on GitHub"; "History": "Codex started on it · 25 min
  ago", "You moved it to In progress · 2 h ago", "You made it · Mon" [Show all history].
- **Fixes kept:** every row is a link plus a real check box (48 px), so Tab reaches it; below 1024 the back
  arrow is named "Back to tasks". Counts: rows read "1 of 3 sub-tasks done"; the task reads "Sub-tasks"
  with "1 of 3 done"; the bar is blue while work goes on.

### 4.2 All, "Ready to check" and the check (Tasks 1, section 2)

- **R1 · Chips under All:** "Active", "Stuck", "Ready to check", "Done", "Archived". On Ready to check: a row
  "HQ checks finished work first · 1 being checked, 1 waiting its turn" [Check them now]. Rows "Review the
  signed update path · T-8 · Urgent · Ready to check · HQ finished 20 min ago"; "T-2 · Ready to check · HQ
  finished 3 h ago · 4 of 4 sub-tasks done"; "Being checked 1": "Export menu: add Markdown with front matter
  · T-7 · HQ is checking it now".
- **R2 · The check card on top of the task.** "Ready for you to check" / "HQ says this is finished, 20 min
  ago. It changes how updates are signed, so a person signs it off. Look it over, then mark it done." "What
  HQ checked · 2 of 3 met": "Every download is checked against the pinned key · Met. verify.sh checks the
  signature before anything runs." (ink, check icon); "A missing key stops the update · Not met. No test
  covers a missing key file." (amber, icon); "The docs explain going back a version · Not decided. HQ could
  not tell from what it read." "It looked at pull request #212. A different AI checked it, not the one that
  did the work." [Send back] (left) [Mark done] (right) [Ask for a second look] (quiet). "2 earlier checks"
  folded: "1 of 3 met · sent back to In progress · Yesterday 16:10", "Nothing to check · no pull request was
  linked · Mon 11:02". A second look shows "Another checker agreed." or "Another checker disagreed:" and its
  reason.
- **R3 · HQ will finish it.** "HQ checked this and it passed. It marks it done at 17:30 unless you keep it
  open." [Keep it open]. "More about this task · Description, sub-tasks, comments".
- **R4 · Details:** "Urgent", "Fri 6 Nov", "Agent HQ › Release", "Done by · Claude, on your Mac mini", "Kind
  of change · Risky"; history "HQ checked it: 2 of 3 met · 12 min ago", "Claude said it is finished · 20 min
  ago", "Claude started on it · 1 h ago".
- **Words that change:** "Verification" › "Ready to check"; "Verify now" › "Check them now"; "Second
  opinion" › "Ask for a second look"; "Veto" › "Keep it open"; "Complete anyway" › Mark done; "Return to
  to_do" › Send back; Tier 0, 1 and 2 › "Notes and plans", "Code changes" and "Risky"; "Inspected",
  "artifact" and "inconclusive" are not shown.
- **Mark done and Send back:** Send back left, Mark done right. Mark done gives "Done: {title} · Undo"
  (`../../resolutions.md` B8).
  Send back asks "What should change?" / "Tell HQ in a sentence or two" (e.g. "Also check that the copy can
  be opened on the Mac.") / "Your answer goes on the task as a comment. It moves back to In progress."
  [Cancel] [Send back]; closing the sheet keeps it ready to check.
- **Owner only:** Check them now, Ask for a second look, Keep it open and the spot checks show only on the
  owner's sign-in, never to a helper or a read-only key (`docs/TASK_VERIFICATION.md`).

### 4.3 Picking several, and when one done asks first (Tasks 1, section 3)

- **P1 · Pick several** (in the list head, or Shift and click) turns the tabs into a bar: "3 picked",
  "Select all 9", "Set status", "Set priority", and a menu with "Mark done", "Move to tomorrow", "Archive"
  (ink: it can be brought back). "3 tasks picked · Change them together with the bar. Press Esc to stop
  picking."
- **P2 · Several done asks.** "Mark 3 tasks done?" / "Tasks that wait on them are told they can start. Undo
  puts all 3 back." / [Keep them as they are] [Mark 3 tasks done] (green). Done: "3 tasks done · Undo"
  (`../../resolutions.md` B8).
- **P3 · One task, others wait on it.** "Mark this done?" / "2 tasks wait on "Release build". HQ tells them
  they can start." (the count from its "Waiting on this" list, `TaskRelations`) / [Not now] [Mark done].
  Back and a tap outside choose Not now. Done: "Done: {title} · Undo".
- **P4 · One task, a check pending.** "HQ is still checking it. Marking it done now skips the check."
  [Wait for the check] [Mark done]. Side by side when the pair fits on one line, stacked when it does not
  (`../../decisions.md` WR27).
- **P5 · Undo.** "3 tasks archived · Undo" (restores on HQ within 10 minutes; the toast may stay until
  closed). Too late: "Couldn't undo. It changed on another device, or too much time passed." Partial: "2
  tasks done. Couldn't change T-4. It changed on another device, or too much time passed." (kept until
  closed; `../../resolutions.md` A6).

### 4.4 While you were away (Tasks 1, section 4)

- **W1 · `/tasks/away`** in the detail pane (1024 to 1399; Details is a button there). The phone's words
  and sections, in the phone's order (`../../resolutions.md` B7): "While you were away" / "Last 24 hours
  · checked at 14:32". "Finishing soon 1": "Discord bot reconnect after a new token · T-10 · Finishes at
  17:30" (Keep it open stays beside it). "Checked and finished 6": "Relay · 3 tasks · 2 tested, 1 read
  over", "Web app · 3 tasks · 3 read over". "Went back to Codex to finish 1": "Chat list keyboard focus ·
  Failed a check. The first try missed the archived chats." (a task with no helper reads "Went back to
  you to finish" and "Failed a check. Reopened for you to finish."). "Waiting on you 2": "T-8 · a person
  signs off on release changes"; "Export menu: add Markdown with front matter · T-7 · HQ could not open
  what it made". "Spot checks · Did HQ get this right? · 1 of 2" (web only, owner only): "Chat list
  keyboard focus. HQ marked it done because Tab now reaches every row." [No, reopen it] [Yes, it is
  done].
- **W2 · Briefing for one list.** With the filter on a list, the phone's Briefing (in progress, waiting for
  review, blocked, who worked on it, recent notes) opens at the top of this page instead.

### 4.5 Can't be reached, and empty (Tasks 1, section 5)

- **E1.** "Your HQ can't be reached · 9 active", amber, no chip. "Saved in this browser at 14:02." Rows from
  the saved copy ("Renew the domain · 2 days late · High"). "You can read these. Marking done waits until
  your HQ can be reached." Task: "To do · Renew example.com before the registrar charges the late fee." /
  "Changes need your HQ. It can't be reached right now."
- **E2 · Empty, per tab:** Today "Nothing for today." with "6 tasks are coming up this week." [See
  Upcoming]; Upcoming "Nothing coming up."; All, Active "No tasks yet. Add one and it shows up here." [New
  task]; a chip with nothing "Nothing is ready to check." "Nothing is stuck."
- **E3 · Loading:** "Getting your tasks" with the blue spinner, two skeleton rows. Never an empty state
  before the first answer.
- **E4 · Failed, nothing saved:** "Couldn't load your tasks. Your HQ can't be reached. It will try again
  when HQ is back." [Try again now] (`../../resolutions.md` A2: a load retries by itself). The filter: "Couldn't load your lists." and the same reason.
- **E5 · Other states:** Starting HQ · 12 s (blue): the saved list, actions off, "It will try again by
  itself." You're offline: the same, with "It will try again when you're back online." for a new task,
  which waits in "Waiting to send". Signed out of this HQ: nothing saved is shown, only Sign in.
- **Late:** "2 days late" is amber with the warn icon, never red; the words say it.

### 4.6 Areas, folders and lists (Tasks 3, section 1)

- **L1 · Filter by list.** "All tasks · 9 open"; "Agent HQ · Area · 6 open"; "Phone work · Folder · 4
  open"; "Phone · List · 3 open · has a briefing"; "Tablet · List · 1 open"; "Release · List · 2 open"; [New
  list in Agent HQ] ("Enter adds the list. Esc cancels."); "Home · Area · 1 open" [New list in Home]; [New
  area]. "Tags": "release", "web", "relay" [Clear]. [Show 3 tasks] / [Show 9 tasks]. Typed in place.
- **L2 · Empty and failed.** No areas yet: "Your tasks go to your Inbox list until you make an area."
  Failed: "Couldn't add the list. Your HQ can't be reached. Try again when it's back." The typed name
  stays.
- A folder is made by naming it in the task form, as the web does today.

### 4.7 New task, and Edit details (Tasks 3, section 2)

- **F1 · Quick add first.** "What needs doing?" [Add task]; "Add details" opens the rest.
- **F2 · The form.** "Notes (optional)"; the list ("Agent HQ › Release", "Agent HQ › Phone work › Phone",
  "Home › Inbox"); "Part of (optional)" ("None, a task of its own", "T-6 Release build and certificate
  renewal"); "Priority" None, Low, Normal, High, Urgent; "Starts (optional)"; "Due (optional)" (e.g. "The
  current one ends on 30 Nov."); "Estimate (optional) · In minutes."; "Who should do it (optional)" as
  chips ("codex") with "A helper named here is told about the task."; "Tags (optional)" ("deploy") with
  "What the task is about. A deploy tag marks it as a risky change when HQ checks it." [Cancel] [Add task].
- **F3 · Edit details** is the same form, filled, from the task's menu: "Title", "Starts", "Due", "Who
  should do it", "Notes, part of, priority, estimate and tags are further down." [Save].
- **F4 · Saving and away.** "Task added · Undo". While your HQ can't be reached the task waits in "Waiting
  to send" with a client id, so a retry never makes "Renew the signing certificate 2".
- Every field of `TaskFormModal.tsx` stays: title, description, parent task, space (with New space),
  folder, initiative, priority, start and due dates, estimate in minutes, assignees and tags. HQ mails
  each new name on a task (`notify_recipients` in hq-tools); plain names only, no ids.

### 4.8 On the task: decisions, where the work left off, what it waits on (Tasks 3, section 3)

Order under the task's facts:

- **K1 · "Waiting on you · Codex has a question"** (only while a helper on this task waits at a question).
  "Codex wants to run a command" / "What it would run, as Codex wrote it" /
  `./scripts/sign-release.sh --channel stable` / "In HQ's words: Codex, on your Mac mini, wants to run this
  command for this task." [Decline] [Allow once] / "Other answers, on Codex's page". Allow once shows only
  when the decision carries a one-time yes (`approve` is null otherwise). A losing request: red "Allow
  delete", still on the right; HQ never says yes to one for you. The session id the drawer printed becomes
  the helper's name.
- **K2 · "Where the work left off"** (`TaskCheckpoint.tsx`; hidden when there is none). "So far · Built the
  release once. The signing step failed on the old certificate." "Next · Sign with the new certificate,
  then build the stable channel." "Open questions · Should the old certificate stay valid for one more
  week?" "Files · scripts/sign-release.sh · release/channels.json". "Left by Codex, today 14:02. Notes from
  that helper, not instructions."
- **K3 · "Waits on"**: "Workstream A: first run for a phone-only user · T-3 · In progress · 1 of 3 sub-tasks
  done" [Add a task it waits on]; **"Waiting on this"** (read only): "Shrink or retire the hub droplet · T-9
  · To do", "Publish the 0.9.2 notes · T-14 · To do". Add by number or title from a list of open tasks;
  remove with the close button beside the row link, never inside. A list row shows "Waits on T-3".
- **K4 · Details:** "Mon 2 Nov", "Estimate · 2 h", "Time · 3 h 10 min", "To do · 2 days"; "Helpers on it ·
  Sign the release · Codex · Needs your OK".

### 4.9 The list: Find in tasks, Collapse all, the chips (Tasks 3, section 4)

- **S1 · Find in tasks.** "3 tasks match "release", in titles, notes, numbers and tags." It filters Board
  and Timeline too. Empty: "No task matches "release"." [Clear the search].
- **S2 · Collapse all / Expand all** fold every group.
- **S3 · Chips:** "Waits on T-3"; "No change for 3 days" (in progress, nobody holds it and nothing has
  changed for `stale_after_hours`, 72 by default); "Long-running" (a helper ending does not move it). Both
  grey with an icon: they inform, they do not ask.

### 4.10 The owner's checking tools (Tasks 4)

- **C1 · Check them now · Which tasks?** (owner only): "Everything waiting · 14 ready to check"; "This list:
  Agent HQ › Release · 5 ready to check"; "Waiting since a date · Pick a date below"; "The tasks I picked ·
  3 picked"; "Waiting since". "Planning reads the evidence only. No AI is used and nothing moves." "Earlier
  checks of the queue · Mon 2 Nov · 22 tasks · Applied · 18 moved · Undo still offered". [Cancel] [Plan it].
- **C2 · Plan ready.** "Everything waiting · 14 tasks. Read the groups, then run it. Running records a
  verdict for each and still moves nothing." Groups: "6 · Look done · Marked done when applied. How each
  was checked is kept."; "Need an AI reviewer · Checked when you run it, within your budget."; "2 · Fall
  short · Back to To do with what is wrong."; "Nothing to check · Back to To do with what to link. Never
  marked done."; "Need you · They stay in your list. One is a risky change." "About 3 reviewer runs, from
  this month's budget." [Run it].
- **C3 · While it runs:** "Checking · 9 of 14 done" (`../../resolutions.md` B18) with a blue bar, Pause and Cancel; verdicts as they
  come. Paused: "It stopped between tasks. Run it again to keep going." Cancelled: "Nothing was moved."
- **C4 · When it can't:** checking off: the button off with "Checking is off. Turn it on in How checking is
  going."; a scope with nothing: "Pick a list in the filter first." "Pick some tasks first."; failed:
  "Couldn't plan the check. Your HQ can't be reached. Try again when it's back."
- **C5 · Ready to apply.** "Everything waiting · 40 tasks checked. Applying checks each task again first,
  because the work may have changed." "22 · Marked done.", "Back to To do with what to link.", "9 · Need you
  or could not decide · Left where they are." Past the limit (25 by default, `sweep_confirm_over`) an amber
  confirm with the warn icon: "This moves 31 tasks, more than the 25 HQ moves without asking. I have read
  the groups and want it applied." [Apply · 31 tasks] (green, off until ticked: "Tick the box above to
  apply."). Apply loses nothing (Undo restores it), so it is green, on the right.
- **C6 · Applied.** "31 tasks moved: 22 marked done, 9 back to To do." "Undo puts back every task that has
  not changed since." [Undo]. After Undo: "29 tasks are back in Ready to check." "Left alone: T-31, it
  changed since. T-40, it changed since." "HQ does not check them again until they are finished again."
  This Undo has no time limit (it works on the queue's own record, `undo_sweep`), unlike the 10-minute toast
  restore.
- **C7 · How checking is going** (one row above the Ready to check list opens it in the detail pane):
  "Need you 5", "Queued", "Checks running 0", "Waiting for budget", "Back with whoever did it". "HQ marked 6
  tasks done and checked 14 in the last 24 hours." "The queue clears in about 3 hours at this rate." "The
  oldest task waiting on you is 2 days old." "Reviewer spend this month: $3.40 of $10.00." "Waiting, and
  why · 2 tasks need you · Risky changes. A person signs them off." "Turning it on · Watching only · HQ
  records what it would do and moves nothing. · Next: let HQ finish notes and plans by itself. · One
  condition is not met yet." Conditions (unmet first): "The reviewer catches planted mistakes · Not met. 17
  tried, it needs 20."; "Enough checks compared with what happened next · Met. 38, it needs 30."; "HQ is
  rarely wrong on notes and plans · Met. At most 7%, the limit is 10%." Technical details: the stage's own
  words ("Shadow: verdicts are recorded and nothing moves"), the tier names and the one setting to change
  (for example `tasks.verification.mode: enforce` with `enforce_tiers: [0]`); the web never changes the
  setting itself. Off: "Checking is off. HQ marks nothing done by itself." and only Turning it on. Plain
  words: tier 0 "notes and plans", tier 1 "code changes", tier 2 "risky changes" (security, payments,
  production, deploy, migration, outbound and deletion tags by default); canaries "planted mistakes".
- **C8 · The audit.** "While you were away · Did HQ get this right? · 1 of 5": "T-21 Relay reconnect notes ·
  HQ marked it done 2 days ago. 2 of 2 met." "The note explains a token change · Met. Section 2 covers it."
  "It links the pull request · Met. #198 is linked." "What HQ saw · The note changed 3 times. The pull
  request was merged." [No, reopen it] (left) [Yes, it is done] (right) [Skip this one] (quiet, below).
  Disagree: "What is wrong? · Tell HQ in a sentence" ("The steps miss the case where the bot was kicked.") /
  "It goes on the task, which moves back to To do." [Reopen it]. "How often HQ is wrong · You disagreed with
  3 of 41 audits. · For code changes, HQ is wrong at most 9% of the time, with 95% confidence." (a rate only
  for a kind of change with about 30 audits of its own). Too few: "You disagreed with 1 of 12 audits. · Too
  few to say how often HQ is wrong. It takes about 30 of one kind." None: "No audits yet, so HQ can't say
  how often it is wrong." End: "Nothing left to answer. Ask for more when you like."
- **C9 · An archived task.** "Old relay notes · Archived" / "Archived 3 days ago. Restore it to work on it
  again, with its comments and sub-tasks." [Restore] (green). Its menu: Delete for good, a red dialog
  "Delete this task for good?" / "“Old relay notes” and its comments are removed. This can't be undone.
  Leave it archived to keep it out of the way." / [Keep it] [Delete for good] ("Delete for good" is 15
  characters, side by side). Failed: "Couldn't delete the task. Your HQ can't be reached. Try again when it's back."
  The dialog stays. Archive gives "Task archived · Undo".

### 4.11 Board and Timeline (Tasks views)

- **V1 · Board.** One column per status, in the list's order: To do, In progress, Stuck, Ready to check,
  Done (Stuck and Ready to check carry the amber edge; archived not shown; Done shows the newest five and
  "Show all 8"). Head "Agent HQ · 9 open" [New task]. Cards: "Shrink or retire the hub droplet · T-9 · due
  Saturday"; "Renew the domain · 2 days late · High"; "Check Samsung battery whitelist · T-5 · High · starts
  tomorrow"; "Workstream A… · T-3 · 1 of 3 sub-tasks done" with [Move to] (Stuck, Ready to check, Done);
  "Workstream C… · T-4 · High · due Sunday · Codex is working"; "Soak, reboot test, cleanup of old copies ·
  Stuck 2 days · needs a reboot window"; "Review the signed update path · T-8 · Urgent · HQ finished 20 min
  ago"; "Nightly off-device backup · T-2 · 4 of 4 sub-tasks done"; "Tidy the export file names · T-12 ·
  Done today". Toast "Task moved to In progress · Undo". Every card is a link; the fake "Check it" inside
  each card is gone (the whole card opens the task with its check on top).
- **V2 · Moving:** drag a card, or its Move button, or focus the card and press M. Into Done acts at once
  with "Task moved to Done · Undo"; it asks first when other tasks wait on it or a check is pending. Ready to
  check is not offered for a task nobody has finished. Keyboard: Tab reaches each card then its Move
  button; Left and Right move between columns, Up and Down between cards; Enter opens the task in the detail
  pane over the board, with the board beside it from 1400.
- **V3 · Timeline, Week.** "Mon 2 to Sun 15 Nov" [This week], zoom Week · Month. Two weeks from the start
  of this week; a task with a start and a due date is a bar between them; with one date, a short bar on that
  day. The names column stays put while the days scroll. Bars use the state colours and say their state in
  words ("In progress · Codex is working", "In progress · due Sun", "Stuck · needs a reboot window", "To do ·
  High · 2 days late" amber, "To do · due today", "Done today"). "No dates · 3 tasks · Not on the timeline.
  Open them in the list." Changing dates: drag a bar's ends, or Starts and Due on the task page (no
  drag-only change). Without colour: each bar and name is a link with the dates in its name ("Renew the
  domain, due Tue 3 Nov, 2 days late"). Empty: "Nothing has dates in these two weeks." [This week]. Can't be
  reached: the saved bars under "Saved in this browser at 14:02", dragging off: "Changing dates waits until
  your HQ can be reached."
- **V4 · Timeline, Month.** "2 Nov to 6 Dec": five weeks from this week's Monday; day numbers only on
  Mondays ("Mon 2", "Mon 9", "Mon 16", "Mon 23", "Mon 30", "December from Tue 1"); the built view's 12 px a
  day, wider here because the page is wider. A one-day task keeps its bar. Today's column is the lighter
  band; This week brings it back into view; weekends not shaded at Month. Week or Month is remembered in
  this browser, as the view switch is. The phone's Timeline page opens on Week.
- **V5 · Phone width.** The switch is the "Show as" menu in the list head (remembered in this browser).
  Board scrolls one column at a time; chips under it jump to a column and show its count ("To do", "In
  progress", "Stuck", "Ready to check"); long press on a card picks it up; its Move button does the same.
  Timeline keeps the names column (names shortened to fit 150 px; the full title in the bar's name) and
  scrolls the days; This week and the menu sit in the list head. List stays the phone's home.
- **V6 · Embedded** (`?embed=1`, the phone's Board and Timeline pages): no rail, no top bar, no bottom bar;
  the page still needs the phone's sign-in; links inside open the phone's own screens.
- **V7 · Other widths:** 768 to 1023 the board fills the page beside the rail; 2200 columns keep 272 px, so
  all five fit, and the Timeline shows more days, never wider ones; 320 and 200% one column at a time with
  the jump chips; bars keep the state in words. Lite and the phone as hub: both views work, without "Codex
  is working" in Lite; in the HQ window they are the same pages with "HQ on your phone".

### 4.12 Widths, Lite and the phone as the hub (Tasks 2)

- **390.** The phone's Tasks: the tabs under the bars, New task low on the right (floating), a task over the
  list. The view switch moves into the list's menu ("Show as"). The filter and Pick several stay in the list
  head. Group headings are buttons that fold, read as "Waiting on you, 2, expanded". All rows have no check
  box, as on the phone. Picking several: long press, or Pick several; "Mark 2 tasks done?" / "Tasks that
  wait on them are told they can start. Undo puts both back." / [Keep them as they are] [Mark 2 tasks
  done], then "2 tasks done · Undo" (B8); Archive acts at once: "2 tasks archived · Undo". The check: "Ready for you to check · HQ says this is finished, 3 h ago.
  Look it over, then mark it done." "What HQ checked · 2 of 2 met": "One archive written · Met.", "Every
  note checked after copying · Met. 2,860 notes." [Send back] [Mark done] [Ask for a second look]; "What was
  done · Set up a nightly encrypted copy of the vault to the Mac. First run wrote one archive and checked
  2,860 notes."; "More about this task · Priority, dates, sub-tasks, comments, history". Saved list: "Your
  HQ can't be reached", one strip, check boxes off with the reason; New task stays on (it waits; off on a
  shared computer). Tasks no longer opens on verification (Drift 16): the list is first.
- **768 to 1023 (900).** The rail; the list takes the whole page; a task opens in place of the list, with
  "Back to tasks". The view switch shows in the top bar from 768. Rows keep their 48 px check boxes and
  two-line titles. Details is a side sheet. New task sits in the list head, not floating. Search is an icon
  until 1024.
- **2200 (drawn at 60%).** The list keeps 360, the task text 760, centred, the details panel 320.
- **320 and 200%.** At 320 the top bar holds the title and the state; Search, the balance and Settings move
  to one menu; the state wraps to two lines; the bottom bar keeps five places at 64 px each; chips scroll
  sideways inside their row; the page never does. At 200% a 1040 window lays out at 520: the phone layout;
  the toast ("Done: {title} · Undo") sits above the bottom bar and moves above a focused field. Forced
  colours keep borders; a printed list is black on white with its state in words.
- **Lite.** "Connected · HQ Lite · 9 active"; Notes, Tasks, Inbox; starts on Notes. Example "Quarterly report draft ·
  In progress · due Friday", "Pull the numbers from the shared folder and write the first page.",
  "Sub-tasks · 0 of 2 done". No helpers (no "Working on it now"), no Check it from HQ (Lite runs
  housekeeping only, not the checking of finished work); a task moved to Ready to check by hand still shows
  there; Needs you never holds HQ's checks (Lite refuses the verification routes). A stored link to Chat
  or Helpers opens Notes with "This HQ is HQ Lite. It has no {place}." (WR4; the board's "HQ Lite has no
  Chat." is superseded). The gear shows This browser and Other devices (WR2, `../../resolutions.md`
  B13).
- **The phone as the hub (HQ window).** "HQ on your phone · Pixel 8" in the window, status "Connected · HQ
  on your phone"; "Tasks from your phone · Pick a task to open it here." No web push; saved copies and the
  outbox keyed by the HQ's id, labelled "Saved in HQ Hub at 14:32" (`../../resolutions.md` B11); the held-port line if another program holds the forward; the phone pays for the traffic.

## 5. Exact copy

Every string quoted in section 4 is exact. Keys `tasks_*`, `task_*`, `check_*`, `board_*`, `timeline_*`.

## 6. Behaviour rules

1. **Tabs Today, Upcoming, All;** chips under All: Active, Stuck, Ready to check, Done, Archived.
2. **Rows** are a link plus a real check box (list view on wide screens); the action link sits beside the
   row link.
3. **One done** acts at once with "Done: {title} · Undo", unless it has dependents (P3) or a check
   pending (P4); several always ask (P2) and end with "{N} tasks done · Undo" (`../../resolutions.md`
   B8).
4. **Archive** with Undo everywhere; Delete for good only from an archived task's menu, as a red dialog.
5. **Progress** reads "{n} of {m} sub-tasks done" on a blue bar.
6. **Late and Stuck** are amber with an icon; met checks ink; red only on Delete for good.
7. **Checking tools** are owner only (not agent tools, not reachable with any MCP key).
8. **The sweep's Undo** has no time limit and reports what it could not put back, by task.
9. **The audit** names a rate only with about 30 audits of one kind.
10. **New task** waits in Waiting to send with a client id when HQ is away; off on a shared computer.
11. **Find in tasks** filters the list, Board and Timeline; Ctrl K finds tasks among everything.
12. **Board moves** have a keyboard path (M, arrows) and a Move button.
13. **Timeline** dates change by dragging or on the task page; both dates are buttons there.
14. **Week or Month** and the view are remembered in this browser.

## 7. Browser and platform notes

- Dragging uses pointer events (`usePointerDrag.ts`); on touch, long press picks a card up.
- Shift and click range selection follows the list's visual order.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/hq-web/src/routes/tasks.tsx`, `tasks/$id.tsx`, `tasks/away.tsx` | Tabs, chips, the list first, `/tasks/:id`, `/tasks/away` (78, 397-419). | yes, others NEW |
| `apps/hq-web/src/components/tasks/TaskCard.tsx` | Link plus check box; chips (66-70). | yes |
| `apps/hq-web/src/components/tasks/TaskDetailDrawer.tsx` | The task page in the detail pane; Delete only when archived (108-224). | yes |
| `apps/hq-web/src/components/tasks/TaskFormModal.tsx`, `TasksSidebar.tsx` | Quick add, Edit details, area and list words, New folder through the form (89, 293; 152-251). | yes |
| `apps/hq-web/src/components/tasks/TaskRelations.tsx`, `TaskCheckpoint.tsx`, `TaskDecisions.tsx`, `WorkingNowPanel.tsx` | Order and words; helper names. | yes |
| `apps/hq-web/src/components/tasks/TaskListView.tsx`, `BulkActionBar.tsx`, `useTaskSelection.ts` | Collapse all, the bar, the dialogs. | yes |
| `apps/hq-web/src/components/tasks/TaskBoardView.tsx`, `TaskGanttView.tsx`, `gantt.ts` | Board moves, Timeline zoom and words. | yes |
| `apps/hq-web/src/components/tasks/verification/*` | Words (W30), Sheet primary stays green, colours. | yes |
| `apps/hq-web/src/components/tasks/InitiativeBriefing.tsx`, `verification/BriefingCard.tsx` | While you were away page (160). | yes |
| `crates/hq-web/src/tasks_api.rs` | Restore window; client id on create; "waiting on this" count. | yes |

## 9. Server, desktop or release changes

Server: task create by client id, the 10-minute restore rule, the dependents count for the one-done
question. The checking routes exist (`crates/hq-web/src/verification_api.rs`).

## 10. Acceptance checks (each fails on today's build)

1. On a 390 px viewport `/tasks` shows the list before any verification block. Today the Verification bar
   and briefing come first.
2. Marking done a task with dependents opens "Mark this done?"; one without dependents acts with Undo.
   Today none ask and there is no Undo toast.
3. Delete is not offered on an active task; on an archived task it opens the red dialog. Today delete acts
   at once without sub-tasks.
4. No visible text contains "space", "initiative", "Return to to_do", "agree or not", "tier" outside
   Technical details.
5. Board cards are links with a Move button; pressing M moves the focused card.
6. Timeline Month draws Mondays only and remembers the zoom.
7. A task created while HQ is away is sent once (client id).
8. Marking one task done shows "Done: {title}" with Undo; marking three asks "Mark 3 tasks done?" with
   "Keep them as they are" and ends with "3 tasks done" (B8). Today "Task marked done".
9. `/tasks/away` shows the sections "Finishing soon", "Checked and finished", "Went back to you to
   finish" (or to the helper) and "Waiting on you" in that order, and the Tasks link row reads "HQ
   checked and finished {N} tasks." (B7). The sweep's progress reads "{N} of {M} done" (B18).

## 11. Check on a laptop or phone

- Keyboard only: pick three tasks with Shift and click, mark them done, Undo.
- The phone's Board page (embedded) shows one set of bars.

## 12. Open decisions it depends on

- **W7, W28 to W30, W33, WR14 to WR16, WR27** (Recommended or Revised), `../../resolutions.md` A2, A6,
  B7, B8, B11, B13, B18.
- Depends on: `../wave-1/02-dialogs-approvals.md`, `../wave-1/03-undo.md`, `../wave-2/02-embed-and-lite.md`,
  `07-helpers.md` (Helpers on a task).
