# Wave 1 · 05 · One way to count progress (finding #13)

Board: **FixProgress** ("Fix 13: counts"), designed, not built, revised after review, 5 Nov 2026.
Area: progress, counts and a label that looks like a button (`ui/tasks`, `ui/notes`, `ui/search`).
Shared rules: `../../rules-shared.md` sections 3 and 7 win.

Headline: **One way to say how far along, one way to count, and a heading that is clearly a heading.**

## 1. Finding and the review items it closes

**The problem.** The same three sub-tasks read "1 of 3 steps done" on Today, "1 of 3 sub-tasks" on All
and "Sub-tasks · 1 of 3" on the task page; in a folder, "A to Z" with a sort icon looks like a control
and does nothing.

**Who it hurts, how often.** Every owner with sub-tasks, each time they move between Today, All and a
task, which is many times a day. Three wordings read as three different things ("are steps the same as
sub-tasks?"). The dead sort control is tapped once per folder visit by anyone who wants newest first.

What changed in this revision:

- **Review #13 closed:** the acceptance grep for "steps done" could never fail, because
  `TasksLogic.meta()` joins "steps" and "done" with a no-break space (`TasksLogic.kt:187`). It is
  replaced by unit tests on `meta()`, `allMeta()` and the task page label for 0, 1 and many, and a grep
  for the whole word `\bsteps\b`.
- **All row wording:** "HQ is working on it · 1 of 3 sub-tasks done · due Sunday", replacing the
  lower-case "hq is working" (`TasksLogic.kt:133`).
- **Shared rules applied (rules sections 3, 7):** the status line reads "Connected" in grey, never a
  green "Live"; the sub-task bar is blue while work goes on; counts come from plural resources, not
  Kotlin strings.
- **New:** a 360 dp, font scale 2.0 frame of the All list (A7), an Android versions card and a server HQ
  note.

## 2. Before (as built, verified at `c04a100`)

Paths under `apps/android/app/src/main/kotlin/example/hq/spike/`.

- `ui/tasks/TasksLogic.kt:187` `Run("${t.subtaskDone} of ${t.subtaskCount} steps${NBSP}done")` in
  `meta()` (Today and Upcoming).
- `ui/tasks/TasksLogic.kt:133` `"hq is working".takeIf { working }`; `:134` `"${t.subtaskDone} of
  ${t.subtaskCount} sub-tasks"` in `allMeta()` (line 124); `:140` `plural(n) = if (n == 1) "1 task"
  else "$n tasks"`.
- `ui/tasks/TaskSheets.kt:309` `Label(if (subs.isEmpty()) "Sub-tasks" else "Sub-tasks · ${done} of
  ${subs.size}")`, with a bar under it.
- `ui/notes/NotesPages.kt:248` `FolderPage`; line 262 `Text("A to Z", …)` with a sort icon in a 36 dp
  row. `SectionLabel` lives in `ui/notes/NotesHome.kt:122`.
- `ui/search/SearchWords.kt:12` `seeAll(count) = "See all $count"`.
- `data/HqClient.kt:456` `FolderListing(folders, notes: List<String>)`: names only, no dates.

### Before frames

- **B1 · Today.** Top bar "Tasks · Live · Copilot 912 left". "In progress 2": "Workstream A: first run
  for a phone-only user · 1 of 3 steps done" (pin A); "Workstream C: a native view instead of the
  WebView · High · due Sunday".
- **B2 · All.** "9 active · Live"; chips Active, Stuck 1, Ready to check 1, Done. "T-3 · Workstream A: … ·
  1 of 3 sub-tasks" (pin B); "T-4 · High · Workstream C: … · hq is working · due Sunday".
- **B3 · The task page.** "Workstream A: first run for a phone-only user · Due Nov 8 · Sub-tasks · 1 of
  3" (pin C); "Write the first-run words", "Draw the start screen", "Test on the A32", "Add a sub-task".
- **B4 · Inside a folder.** "Agent-HQ · Notes › Projects"; "A to Z" with a sort icon (pin D); folders
  "Android spikes 7", "Evidence 12"; notes "Mobile-First-Roadmap-2026-10-20 · Projects › Agent-HQ",
  "Phone-Cutover-Evidence-2026-10-20 · Projects › Agent-HQ"; "New note here".

Pins: **A** "steps" is a word used nowhere else; the task page calls them sub-tasks. **B** no "done", so
"1 of 3 sub-tasks" can read as "the first of three". **C** count first, without "done", joined to the
heading by a dot. **D** icon, bold, 36 dp row: it has every mark of a control. TalkBack reads plain text,
so people with sight tap and nothing happens.

### Every count the app shows today

| Where | As built | After |
|-------|----------|-------|
| Today, Upcoming row | 1 of 3 steps done | 1 of 3 sub-tasks done |
| All row | 1 of 3 sub-tasks | 1 of 3 sub-tasks done |
| Task page heading | Sub-tasks · 1 of 3 | Sub-tasks, then 1 of 3 done at the right |
| Search group, past five | See all 7 | Show all 7 |
| Today group, past five | Show 3 more | Show 3 more (kept) |
| Folder page | A to Z, with a sort icon | Notes, A to Z: a list heading |
| List chips | Stuck 1 · Ready to check 1 · Done | kept |
| Search chips | All 6 · Notes 0 | kept: 0 is the answer there |
| Group headings | In progress, badge 2 | kept |
| Upcoming folds | Later · 1 task | kept |
| Top bar on All | 9 active | kept |
| Briefing | 2 in progress, 1 to do (3 tasks) | kept |
| Inbox badge | 3, 99+ past 99 | kept |

## 3. Decision

**Progress is always "N of M done", finished first.** In a list row the things are named: "1 of 3
sub-tasks done". Beside a heading that already names them, they are not: "Sub-tasks" on the left, "1 of
3 done" on the right. "Steps" is retired; they are sub-tasks everywhere, as "Add a sub-task" already
says. On All, the row starts with "HQ is working on it" when HQ is on the task.

**"A to Z" becomes part of a list heading** ("Notes, A to Z"), drawn like Pinned and Recent, with no icon,
so it reads as a fact about the list. Folders get the matching "Folders" heading.

Rejected:

- **A small bar in every row.** Too heavy for 72 dp rows on an A32, and people read the number, not the
  bar. The bar stays on the task page only.
- **"1/3".** Short, but TalkBack reads it as "one slash three" or a date.
- **Test with a grep for "steps done".** The code joins the words with a no-break space, so that grep
  passes on today's build. Unit tests on the functions that build the words are the check.
- **Make "A to Z" a real sort control now.** The folder listing returns names only (`FolderListing` has
  no dates), so there is nothing else to sort by; a control with one choice is worse than a heading.

### One rule for counts

1. **Progress:** done, of, total, then the things, then "done". Leave the things out only when the
   heading next to it names them.
2. **A count on a chip or a heading** is a number after the label ("Stuck 1"), a badge in headings. It is
   hidden at zero, except on search scope chips, where 0 is the answer.
3. **A count in a sentence** is a number and a noun with the right plural, from a plural resource: "1
   task", "2 tasks". Never "task(s)", never an if/else in Kotlin.
4. **"Show N more"** grows a list where it is; **"Show all N"** opens the whole list.
5. **Digits, not words;** "99+" past 99 in a badge.

## 4. Screens and states (After)

- **A1 · Today.** Top bar "Tasks · Connected" (grey dot) · "Copilot 912 left". "In progress 2":
  "Workstream A: first run for a phone-only user · due Sunday · 1 of 3 sub-tasks done" (pin 1);
  "Workstream C: a native view instead of the WebView · High · due Sunday · 0 of 2 sub-tasks done". "Due
  or starting today 7": "Release build and certificate renewal · due today", then "Show 6 more" (pin 2).
  [New task].
- **A2 · All.** "Connected · 9 active". Chips Active, Stuck 1, Ready to check 1, Done. "In progress 2":
  "T-3 · Workstream A: … · HQ is working on it · 1 of 3 sub-tasks done · due Sunday" (pin 1 on the
  board); "T-4 · High · Workstream C: … · 0 of 2 sub-tasks done · due Sunday". "Done 1": "T-2 · Nightly
  off-device backup · Done 2 h ago · 4 of 4 sub-tasks done".
- **A3 · The task page.** "Workstream A: first run for a phone-only user · Due Nov 8". One line: label
  "Sub-tasks" on the left, "1 of 3 done" on the right (pin 3), a 4 dp blue bar under it; the sub-task
  rows; "Add a sub-task".
- **A4 · Edge cases: all done, none yet.** "Nightly off-device backup": "Sub-tasks · 4 of 4 done" (pin
  4), bar full; "Make the key file", "First run writes one archive", then "2 more done, folded: Show 2
  more". "Shrink or retire the hub droplet": "Sub-tasks · None yet" (muted, no bar), "Add a sub-task" (pin
  5). Caption: "With no sub-tasks, list rows show no count at all."
- **A5 · Inside a folder.** "Agent-HQ · Notes › Projects". Heading "Folders" with badge 2: "Android
  spikes 7", "Evidence 12". Heading "Notes, A to Z" with badge 2 (pin 6): the two notes. "New note here".
  No sort icon.
- **A6 · Search: more than five.** Scope chips "All 9 · Notes 7 · Tasks 2 · Chats 0". Group "Notes 7"
  with "Show all 7" (pin 7): "Backup-Setup-2026-10-20 · Projects › Agent-HQ", "Backup restore drill ·
  Projects › Agent-HQ", "Nightly backup notes · Daily", "Backup key, where it lives · Inbox", "Old
  backups to delete · Inbox". Group "Tasks 2": "Nightly off-device backup · T-2 · Done".
- **A7 · All at 360 dp, font scale 2.0.** "Connected · 9 active"; chips Active, Stuck 1 (the row wraps);
  "T-3 · Workstream A: first run for a phone-only user · HQ is working on it · 1 of 3 sub-tasks done · due
  Sunday", the meta line wrapping between parts, never inside the count (pin 8).

## 5. Exact copy

| Where | String |
|-------|--------|
| Today, Upcoming row | "due Sunday · 1 of 3 sub-tasks done" (after priority and dates) |
| All row, HQ on it | "HQ is working on it · 1 of 3 sub-tasks done · due Sunday" |
| All row, otherwise | "1 of 3 sub-tasks done · due Sunday" |
| Task page heading | "Sub-tasks" · right: "1 of 3 done" |
| All finished | "4 of 4 done" (no special wording) |
| No sub-tasks | "Sub-tasks" · right: "None yet". Rows show no count. |
| TalkBack, heading and bar | "Sub-tasks, 1 of 3 sub-tasks done" (one node) |
| One sub-task | "0 of 1 sub-task done" · "1 of 1 sub-task done" |
| Search group, past five | "Show all 7" |
| Today group, past five | "Show 3 more" (kept) |
| Folder page headings | "Folders" 2 · "Notes, A to Z" 2 |
| Empty folder (kept) | "Nothing in this folder yet." |
| Finished sub-tasks folded | "Show {N} more" |
| `meta()` finished line | "HQ finished" (was "hq finished") |
| Plural resource names | `progress_subtasks_done`, `progress_done`, `count_tasks` |

## 6. Behaviour rules

1. **Rows count sub-tasks that are not archived;** done means status complete. Nothing shows when there
   are none. On Today and Upcoming the count keeps its place after priority and dates; on All it follows
   "HQ is working on it". A meta line too long for the row ends in "…" after the due words, never inside
   the count.
2. **Groups past five rows** keep "Show N more", which grows the group where it is.
3. **The task page heading** pairs the label and the count on one line, with the 4 dp blue bar under it
   (rules section 7); the three read as one node to TalkBack.
4. **All finished** fills the bar; the words do not change. Finished sub-tasks past five fold under "Show
   N more".
5. **None yet** sits where the count goes, in muted text, and the bar is not drawn.
6. **Folder headings** are list headings (semantics heading), not clickable, with the count badge. The
   sort icon goes.
7. **Search's "See all" becomes "Show all N",** and switches to that scope as now.

## 7. Android versions

| Version | Behaviour |
|---------|-----------|
| 8 and 9 | Same words and layout. Plural rules come from Android's resources on every version, so "1 sub-task" and "2 sub-tasks" need no version check. TalkBack reads the merged heading node as on newer phones. |
| 10 and 11 | No difference for this fix. |
| 12 | No difference for this fix. |
| 13+ | No difference. Per-app language (13+) changes the plural rules with the language, which plural resources already handle. |
| Every version | The count is one unit: no-break spaces join "1 of 3 sub-tasks done", so at font scale 2.0 the meta line wraps between parts, never inside the count (pin 8). |

**Server HQ:** no server variant is needed for the counts: they are computed from whichever task list the
phone shows. The status line follows the Connection pattern (`../wave-4/01-offline.md`), including "Your
server HQ can't be reached".

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `ui/shell/Counts.kt` | Built once: `progress(done, total, thing?)`, `badge(n)` (99+), `showMore(n)`, `showAll(n)`. The words are plural resources in `res/values/strings.xml` (`progress_subtasks_done`, `progress_done`, `count_tasks`), chosen by the total, with no-break spaces inside the count. | NEW |
| `ui/tasks/TasksLogic.kt` | `meta()` (line 187, the "steps" run joined to "done" by a no-break space) and `allMeta()` (line 134, the "sub-tasks" part) call `Counts.progress`; "hq is working" (line 133) becomes "HQ is working on it", and "hq finished" in `meta()` becomes "HQ finished"; `plural()` (line 140) moves to Counts. | yes |
| `ui/tasks/TaskSheets.kt` | The Sub-tasks label (line 309, today "Sub-tasks · {done} of {total}"): a row of `Label("Sub-tasks")` and `TasksLogic.subtaskLabel(done, total)`, a new pure function so the page label can be unit tested, merged semantics with the bar. | yes |
| `ui/notes/NotesPages.kt` | `FolderPage` (line 248): drop the Sort row (line 262). | yes |
| `ui/notes/NotesHome.kt` | `SectionLabel` (line 122) gains an optional count badge and `heading()` semantics. (The board places `SectionLabel` in NotesPages; it lives in NotesHome.) | yes |
| `ui/search/SearchWords.kt` | `seeAll` (line 12) becomes `Counts.showAll`. | yes |
| `ui/shell/HqShell.kt` | `PlaceIcon` (line 199): use `Counts.badge`. | yes |
| `ui/tasks/Briefing.kt` | Use `Counts.badge` and `Counts.plural`. | yes |

## 9. Server, release or desktop changes

None. Open question (sort by last edited) needs the folder listing to return edit times (decision D19).

## 10. Acceptance checks (each fails on today's build)

1. Unit test on `TasksLogic.meta()` with sub-tasks 0 of 0, 0 of 1, 1 of 1, 1 of 3 and 12 of 40: no run
   when the total is 0, else "{N} of {M} sub-task(s) done" with the right plural. Fails today: line 187
   says "steps".
2. Unit test on `TasksLogic.allMeta()` for the same cases, with `working` true and false: "HQ is working
   on it · 1 of 3 sub-tasks done · due Sunday" and "1 of 3 sub-tasks done · due Sunday". Fails today: "hq
   is working" and no "done".
3. Unit test on `TasksLogic.subtaskLabel()`: 0 of 0 gives "None yet", 1 of 3 gives "1 of 3 done", 4 of 4
   gives "4 of 4 done". Fails today: the label is built inline as "Sub-tasks · 1 of 3".
4. A source check over `app/src/main` finds no whole word `\bsteps\b` in task strings, no "hq is working"
   or "hq finished", and no "See all". It reads the no-break space as a space, so "steps done" is
   caught.
5. On a folder page TalkBack reads "Notes, A to Z, heading" and the row cannot be focused as a button.
6. A task with no sub-tasks shows no count in any row and "None yet" on its page.

## 11. Check on device

- The A32 at font scale 2.0 and 360 dp shows no "…" inside the count. (FixPlan "Still to prove": none.)

## 12. Open questions and decisions it depends on

- **Sort a folder by last edited?** Notes v2 proposed newest first. It needs the folder listing to return
  each note's edit time. Recommendation: yes; once it does, "Notes, A to Z" becomes a two-choice control
  (A to Z, Last edited), remembered on the phone. (Decided: D19, "Once the listing has times.")

Depends on: D19; rules sections 3, 7; `04-plain-words.md` (plural resources, "HQ is working on it").
