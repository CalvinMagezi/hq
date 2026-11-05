# Wave 4 · 02 · One search, wherever you start it (finding #5, D20, N3)

Board: **FixSearch** ("Fix 5: one search"), finding 5, designed, not built, revised after review,
5 Nov 2026. Area: `ui/search`, `ui/notes`, `ui/shell/Links.kt`. Shared rules: `../../rules-shared.md`
sections 2, 3, 4, 7, 8 win.

## 1. Finding and the review items it closes

**The problem.** The magnifier searches notes, tasks and chats; the "Search your notes" pill right under
it opens a second, notes-only search that alone has folder filters and Ask HQ, and `hq://search` opens
that second one.

**Who it hurts, how often.** Every owner who looks for a note, several times a day. The two look alike, so
people do not learn which one can hand the notes to HQ, and with HQ down the notes search wrongly says
nothing matches.

What changed in this revision:

- **Review #5, the folder search could lie.** The phone asked HQ for the top 30 hits and kept those inside
  the folder, so a folder with matches below the top 30 said "No notes match that." `/api/search` takes
  only `q` and `limit` today (`crates/hq-web/src/api.rs:47-60`). The folder now goes to HQ as a `folder`
  parameter, filtered before the limit. The server change is on this board.
- **Helpers scope (D20):** All · Notes · Tasks · Chats · Helpers, matched on the phone from the copy
  Helpers saves (FixOffline). A helper on a server HQ says "on your server HQ".
- **Chats come only from HQ.** There is no saved copy of chats, so while HQ is down chats are not
  searched, and the screen says so. A5 claimed they were searched on the phone.
- **Failure lines from rules section 2,** amber with an icon: "Couldn't search your notes and chats. HQ is
  still starting. It will try again by itself."
- **New:** Clear recent searches with Undo (A10), HQ stopped (A8), a 360 dp and font scale 2.0 frame
  (A11), an Android versions card and the server HQ variant (A9).

## 2. Before (as built, verified at `c04a100`)

Paths under `apps/android/app/src/main/kotlin/example/hq/spike/`.

- `ui/shell/Links.kt:44` `"search" -> LinkTarget(Place.Notes, search = true)`: `hq://search` opens the
  Notes search.
- `ui/notes/NotesPages.kt:298` `SearchPage` (the second search, with folder chips and Ask HQ);
  `ui/notes/NotesViewModel.kt:180` `query()` turns a failed call into an empty list ("Nothing matches
  that.").
- `ui/search/SearchModel.kt:31` `remember()` (recent searches); `ui/search/SearchScreen.kt` the
  magnifier's search (All, Notes, Tasks, Chats; no folders, no Ask HQ).
- `data/HqClient.kt:472` `searchNoteHits(query, limit = 30)`: no folder.
- `ShellActivity.kt:345` `SearchHost`; `LocalOpenWeb` is defined in `ui/webhost/WebHost.kt:21` and
  provided at `ShellActivity.kt:307`.
- HQ: `crates/hq-web/src/api.rs:47` `search_handler` reads `q` and `limit` only;
  `crates/hq-db/src/search/fts.rs:66` `keyword_search(conn, query, limit)` has no path prefix.

### Before frames

- **B1 · Notes home: two ways in.** "Notes · Live · Copilot 912 left" with the magnifier (pin A); pill
  "Search your notes" (pin A); folders Inbox, Projects, Daily, Logs; Recent "Backup-Setup-2026-10-20 · 3h ·
  Projects › Agent-HQ", "Blind-Relay-Design-2026-10-20 · 1d · Projects › Agent-HQ", "Quick idea: widget
  copy · 1d · Inbox"; [New note].
- **B2 · The magnifier's search.** Chips "All 5 · Notes 2 · Tasks 2 · Chats 1". Notes 2: "Samsung battery
  steps · Projects › Agent-HQ · Open Settings, then **Battery**. Find HQ in the list of apps.",
  "Mobile-First-Roadmap · Projects › Agent-HQ · Samsung may undo the **battery** setting after an
  update." Tasks 2: "Check Samsung battery whitelist · T-5 · To do", "Nightly off-device backup · T-2 ·
  Ready to check". Chats 1: "Samsung battery steps · 3m ago · Nothing is blocking it" (pin B).
- **B3 · The Notes search.** Chips "All 3 · Projects 2 · Inbox 1" (pin C). "Mobile-First-Roadmap ·
  Projects · Projects › Agent-HQ · Workstream C: a native view… **roadmap** for the phone",
  "Native-Bridge-Plan · Projects · … · The bridge between the **roadmap** and the app.", "Quick idea:
  widget copy · Inbox · Inbox · A **roadmap** item for later."; [Ask HQ about these 3 notes] (pin C).
- **B4 · The Notes search, HQ down.** "Nothing matches that." (pins D, E).

What goes wrong: **A** two ways in that look like one (60 dp apart, two screens). **B** the magnifier's
search has no folder filter and no Ask HQ; it is the only one with recent searches. **C** folder filters
and Ask HQ live only in the Notes search, which finds no tasks or chats. **D** four ways reach the Notes
one: the pill, the folder page's magnifier, "Search for it" on a note that is gone, and `hq://search`,
against the link's name. **E** with HQ down it says "Nothing matches that." (`NotesViewModel.query`).

## 3. Decision

**One search screen for the whole app: the top bar's SearchScreen.** The Notes pill, the folder page's
magnifier, "Search for it" and `hq://search` all open it; the only thing that differs is the scope it
starts on. The folder filters and Ask HQ move into it, so every way in can do everything.
`NotesRoute.Search` and its `SearchPage` are deleted.

Why: the owner thinks "search", not "which search". One screen means one recent list, one set of words,
one failure message, and Ask HQ wherever notes are found.

**HQ does the folder search.** The folder goes to `/api/search` and HQ filters before it cuts the list, so
"No notes match that" inside a folder is true. Filtering 30 hits on the phone is rejected: it hides notes
that rank lower.

**Only search what the phone can honestly show.** Notes and chats come from HQ and are not saved on the
phone. Tasks and helpers have saved copies (FixOffline), so they are searched while HQ is down, and the
screen gives the copy's time.

Rejected:

- **Keep both and name them apart** ("Search everything", "Search notes"). Two result lists and two rule
  sets to keep in step, and the owner still has to remember which one can ask HQ.
- **Remove the Notes pill.** It is the first thing on Notes and the place people reach for when they want
  a note. It stays, looks the same, and opens the one search on Notes.

## 4. Screens and states (After: one SearchScreen; where you open it only sets the scope)

- **A1 · From the magnifier, before typing.** Scope chips "All · Notes · Tasks · Chats · Helpers" (pin 1,
  All selected; the chosen chip is ink, never green, `../../resolutions.md` A3). "Recent searches" (pin 2): "battery", "roadmap", "soak test"; "Clear recent searches".
- **A2 · Results on All, with Ask HQ.** Chips "All 5 · Notes 2 · Tasks 2 · Chats 1 · Helpers 0"; groups as
  B2, matched words in bold, never washed in a colour (A3); pill [Ask HQ about these 2 notes] (pin 3).
- **A3 · From the Notes pill.** Chips "All 4 · Notes 3 · Tasks 1 · Chats 0 · Helpers 0" with Notes selected
  (pin 4); folder chips "All folders 3 · Projects 2 · Inbox 1" (pin 5); the three roadmap notes (path line
  "Projects › Agent-HQ" or "Inbox"); [Ask HQ about these 3 notes].
- **A4 · From inside a folder.** Chips "All 3 · Notes 2 · Tasks 1 · Chats 0 · Helpers 0"; folder chip "In
  Agent-HQ" with a clear cross (pin 6); "Phone-Cutover-Evidence-2026-10-20 · Projects › Agent-HQ · The
  **cutover** ran on the A32 for 48 hours.", "Mobile-First-Roadmap · Projects › Agent-HQ · After the
  **cutover**, retire the old WebView app."; caption "Searching only in Projects › Agent-HQ and the
  folders inside it. HQ does the folder search, so a note below the first 30 hits is still found."; [Ask
  HQ about these 2 notes].
- **A5 · HQ starting: notes cannot be searched** (pin 7). Chips "All 2 · Notes · Tasks 2 · Chats ·
  Helpers 0" (Notes and Chats with no number). Amber with icon "Couldn't search your notes and chats. HQ
  is still starting. It will try again by itself." then "Tasks below are from the copy saved on this phone
  at 14:32." Tasks 2 as before. Caption: "When HQ is up, notes and chats appear here without a tap."
- **A6 · Nothing in this scope** (pin 8). Chips "All 1 · Notes 0 · Tasks 1 · Chats 0 · Helpers 0"; "No
  notes match that." [Search everything].
- **A7 · A result opened, then Back** (pin 9). Note page "Mobile-First-Roadmap · Projects › Agent-HQ ·
  edited 1d ago" with Edit, Ask HQ, Make a task, Unpin; body "This is where AgentHQ goes from here.
  Workstream C is a native view instead of the WebView." / "Test on the A32 first. Samsung may undo the
  battery setting after an update." Caption: "Back opens the search again with "battery", the All scope
  and the list where it was. Back once more closes it."
- **A8 · HQ stopped** (pin 10). Chips as A5; amber "Couldn't search your notes and chats. HQ has stopped.
  It will try again when HQ is back." (A2: the search runs again by itself) / "Tasks below are from the copy saved on this phone at 14:32." [Start HQ];
  Tasks 2.
- **A9 · Helpers, one on a server HQ** (pin 11). Chips "All 3 · Notes 1 · Tasks 0 · Chats 0 · Helpers 2";
  "Move the build from scripts to **Gradle** · Claude · on your Mac mini · needs your OK"; "Clean old
  **Gradle** caches · Codex · on your server HQ · saved at 14:18" (pin 12). Caption: "Helpers are matched
  on this phone, from the list Helpers saved. Your server HQ can't be reached, so its helper shows as saved
  at 14:18."
- **A10 · Clear recent searches, then Undo** (pin 13). Chips (no counts); "Type to search notes, tasks,
  chats and helpers."; snackbar "Recent searches cleared · Undo".
- **A11 · Results at 360 dp, font scale 2.0** (pin 14). Chips "All 5 · Notes 2 · Tasks 2" scroll
  sideways; "Samsung battery steps" with its path and excerpt wrapping; [Ask HQ about these 2 notes] above
  the keyboard.

## 5. Exact copy

| Where | String |
|-------|--------|
| Field, scope All | "Search notes, tasks, chats and helpers" |
| Field, scope Notes | "Search notes" |
| Field, Tasks, Chats, Helpers | "Search tasks" · "Search chats" · "Search helpers" |
| Scope chips, empty field | "All" · "Notes" · "Tasks" · "Chats" · "Helpers" |
| Scope chips, typed | "All 5" · "Notes 2" · "Tasks 2" · "Chats 1" · "Helpers 0" (0 is shown: it is the answer) |
| Scope chip, not searched | "Notes" · "Chats" (no number) |
| Notes past the first page | "Notes 30+" (heading); folder chips show no numbers until the list is complete |
| Folder chips | "All folders 3" · "Projects 2" · "Inbox 1" |
| Folder chip, from a folder | "In Agent-HQ" (with a Clear cross; read as "Only in Agent-HQ. Clear to search all notes") |
| Folder caption | "Searching only in Projects › Agent-HQ and the folders inside it." |
| Helper row | "Claude · on your Mac mini · needs your OK" · "Codex · on your server HQ · saved at 14:18" |
| Ask HQ pill | "Ask HQ about this note" · "Ask HQ about these 3 notes" · "Ask HQ about the first 10 notes" |
| Group heading | "Notes 2" · "Tasks 2" · "Chats 1", then "Show all 7" past five (rule in `../wave-1/05-progress.md`) |
| Recent | "Recent searches" · "Clear recent searches" |
| Cleared | "Recent searches cleared" · "Undo" |
| Empty field, no recent | "Type to search notes, tasks, chats and helpers." (Notes scope: "Type to search your notes.") |
| No match, All | "Nothing matches that." |
| No match, one scope | "No notes match that." · "No tasks match that." · "No chats match that." · "No helpers match that." (`../../resolutions.md` D-9) Button: "Search everything" |
| HQ starting | "Couldn't search your notes and chats. HQ is still starting. It will try again by itself." |
| HQ restarting | "Couldn't search your notes and chats. HQ is restarting. It will try again by itself." |
| HQ stopped | "Couldn't search your notes and chats. HQ has stopped. It will try again when HQ is back." Button: "Start HQ" |
| No reply in time | "Couldn't search your notes and chats. HQ didn't answer in time. Try again." |
| Second line | "Tasks below are from the copy saved on this phone at 14:32." |
| Notes home pill | "Search your notes" (kept) |
| Back on a result page | Read as "Back to search results" |

String resource keys for the failure line: `fail_search_notes_chats_starting`, `_restarting`, `_stopped`,
`_timeout`. Today's "HQ is not answering" lines in `SearchWords.kt` go. The outcomes are A2's "waits and
retries" ones, since the search runs again by itself (rule 7); only no reply in time says "Try again."
"No helpers match that." is added for the Helpers scope (D-9).

## 6. Behaviour rules

1. **Scope chips show before anything is typed,** so the owner sees where it will look and can change it.
   They are tabs to TalkBack, with the selected one read as selected.
2. **One recent list** of eight, shared by every way in, kept on the phone. A query is kept once it leads
   to an opened result (as now). "Clear recent searches" sits under the list.
3. **Ask HQ shows on All and Notes whenever notes are found.** It counts and attaches notes only, at most
   ten, inside the folder filter if one is on, and opens Chat with them attached, as the Notes search does
   today.
4. **Where you start sets the scope.** Magnifier: All. Notes pill: Notes. Folder page magnifier: Notes
   plus that folder. "Search for it" on a note that is gone: Notes, with the note's name typed in.
   `hq://search`: All. `hq://search/notes`: Notes.
5. **Folder chips** appear on the Notes scope once there are note hits, top folder first, Inbox first
   among equals. They are counted from the hits only when HQ returned fewer than the limit; past it they
   show no numbers. Picking one asks HQ again with that folder and keeps the query.
6. **From a folder,** the chip is that folder. HQ matches by path prefix before it cuts the list, so
   sub-folders count and nothing ranked lower is lost. Clearing it searches all notes.
7, 10. **HQ not answering:** one amber line with an icon says notes and chats could not be searched, from
   rules section 2, never "Nothing matches that". Tasks and helpers match from their saved copies, with
   the copy's time. Notes and chats show no number on their chips. When HQ answers again the search runs
   by itself.
8. **No match in one scope** offers "Search everything", which switches to All and keeps the query.
9. **Opening a result closes search and opens it in its place.** Back from that page reopens search once
   with the query, scope, folder and scroll as left; Back again returns to where search was opened (Notes,
   a folder, Chat).
11, 12. **Helpers scope:** matched on the phone from the list Helpers saves, by task title and helper
   name. A server HQ's helper says "on your server HQ" and, when that server can't be reached, "saved at
   {time}".
13. **Clear recent searches** acts at once and shows "Recent searches cleared · Undo" for the
    accessibility timeout, 10 s on Android 8 and 9. Undo puts the eight back.
14. **At 360 dp and font scale 2.0,** chips scroll sideways, titles wrap, and Ask HQ stays above the
    keyboard.

- **Unchanged:** 250 ms wait after typing, 30 note hits from HQ per ask, five rows a group on All.
- **Changed:** matched words are bold, never washed in a colour (`../../resolutions.md` A3); the green
  wash goes.
- **Links that change:** `hq://search` now opens this search on All (it opened the Notes search). New:
  `hq://search/notes`. A link never types a query.

## 7. Android versions

| Version | Behaviour |
|---------|-----------|
| 8 and 9 | Undo after Clear recent searches stays 10 s: these versions have no accessibility timeout setting. |
| 10 and 11 | Undo stays for the phone's accessibility timeout (AccessibilityManager, Android 10+), at least 10 s. |
| 12 | No difference for this fix. |
| 13+ | Predictive Back (14+) previews the screen Back goes to: from a result, search with its list. |
| Every version | The scope chips scroll sideways when they do not fit (360 dp, or font scale 1.3 and up). At font scale 2.0, result titles wrap instead of ending in "…" (A11). |

**Server HQ:** search asks the HQ on this phone for notes and chats, as today. A server HQ's helpers are
found through Helpers' saved list and say "on your server HQ". Searching a server HQ's notes is an open
question (N3).

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `ui/search/SearchModel.kt` | Add `SearchStart(scope, folder, query)`, `SearchScope.Helpers`, `SearchMatch.helpers()` and `clearRecent()` beside `remember()` (line 31). No phone-side folder test: the folder goes to HQ. | yes |
| `data/HqClient.kt` | `searchNoteHits(query, limit = 30, folder)` (line 472, today `q` and `limit` only) adds `&folder=`, URL-encoded. | yes |
| `ui/search/SearchViewModel.kt` | `open(tasks, chats, start)`; folder filter in `SearchUi`; keep the last state for Back (`resume()`); a failed notes call sets `notesFailed` with the connection state, never an empty list; chats match only the list HQ returned while connected, and none while HQ is down. | yes |
| `ui/search/SearchScreen.kt` | Chips shown before typing; new shared `FolderFilterRow` and `AskHqPill`, moved from NotesPages; `askWords()` moves with them. | yes |
| `ui/search/SearchWords.kt` | Every string in Exact copy. | yes |
| `ui/notes/NotesPages.kt`, `NotesState.kt`, `NotesViewModel.kt` | Delete `SearchPage` (NotesPages.kt:298), `FilterChips`, `Filter`, `HitRow`, `NotesRoute.Search`, `SearchState` and `query()` (NotesViewModel.kt:180); `onOpenSearch` calls the shell. | yes |
| `ui/shell/HqShell.kt` | A `LocalOpenSearch` composition local, like `LocalOpenWeb` (which lives in `ui/webhost/WebHost.kt:21`), so any screen opens search with a start. | yes (`LocalOpenSearch` NEW) |
| `ShellActivity.kt` `SearchHost` (line 345) | Passes Ask HQ to the chat with the notes attached; reopens search on Back from a result. | yes |
| `ui/shell/Links.kt` | `LinkTarget.search` (line 16, a Boolean) becomes a `SearchScope?`; parse `hq://search` and `hq://search/notes` (line 44). | yes |
| `res/values/strings.xml` | `fail_search_notes_chats_starting`, `_restarting`, `_stopped`, `_timeout`, and every string above. | yes |

## 9. Server change this needs

`GET /api/search?q=&limit=&folder=Projects/Agent-HQ` in `crates/hq-web/src/api.rs` (`search_handler`,
lines 47-98). The folder is a path under `Notebooks`. HQ refuses one with "..", a leading "/" or a
backslash with 400. `hq_db::search::keyword_search` (`crates/hq-db/src/search/fts.rs:66`) gains a path
prefix in its WHERE, before LIMIT. The substring walk starts at `Notebooks/{folder}`. Lite's path filter
still applies. A test in `api.rs`: 40 matches outside the folder and 2 inside give exactly the 2.

## 10. Acceptance checks (each fails on today's build)

1. The pill, the magnifier on every place, the folder magnifier, "Search for it" and both links open the
   same composable; a UI test finds no `SearchPage` left.
2. Each way in starts on the scope in rule 4, with the right field words.
3. With note hits, Ask HQ is on screen on All and on Notes, and the chat it opens has at most ten notes
   attached.
4. With HQ stopped, typing on Notes shows "Couldn't search your notes and chats. HQ has stopped. Start HQ
   to try again." and never "Nothing matches that." Fails today: `NotesViewModel.query` turns the failure
   into an empty list.
5. Folder search, end to end: a test vault with 40 notes that match "cutover" outside `Projects/Agent-HQ`
   and 2 inside it. Searching from that folder lists both. Fails today: the phone filters the top 30 and
   shows "No notes match that."
6. hq-web test: `/api/search?q=x&folder=../secrets` answers 400; `folder=Projects/Agent-HQ` returns only
   hits under it, including from the substring walk.
7. With HQ stopped, no chat row appears and the Chats chip has no number, even if the chat list was read
   earlier in the session.
8. Clear recent searches empties the list; Undo within the timeout restores all eight in order; after the
   timeout they are gone from the phone's storage.
9. A query that opened a result from the Notes pill shows under Recent searches from the magnifier.
10. Back from a note opened from search shows the same results at the same scroll.
11. LinksTest: `hq://search` gives scope All, `hq://search/notes` gives Notes, `hq://search?q=x` gives All
    with no query.
12. All new strings are string resources; a test renders each failure line and matches it to the rules
    section 2 catalogue, and covers the plural and zero cases.
13. A matched word renders bold with no background colour; with the Helpers scope chosen and no match,
    "No helpers match that." shows (A3, D-9).

## 11. Check on device

- Predictive Back preview (14+) on a Pixel image: from a result, it previews search with its list.
- FixPlan "Still to prove": none.

## 12. Open questions and decisions it depends on

- **Decided (D20):** helpers are searched, from Helpers' saved copy. The Inbox stays out: it is short
  enough to scan.
- **Search a server HQ's notes too?** Today only the HQ on this phone is asked. The server already serves
  `/api/search` to the phone's device token. Recommendation: yes, in wave 4, as its own "On your server
  HQ" group, never mixed into the phone's notes. (Decided: N3, "Wave 4, its own group." The board draws no
  frame for that group yet; it is drawn when wave 4 starts, `../../resolutions.md` D-9.)

Depends on: D20, N3, `../../resolutions.md` A2, A3, B6, D-9, rules sections 2, 4 (Undo), 8 (links); `01-offline.md` (saved copies of tasks and
helpers, the Connection state); `../wave-1/05-progress.md` ("Show all N").
