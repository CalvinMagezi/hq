# Web wave 4 · 04 · One Search, from anywhere (D20, W27, WR12)

Boards: **AlignedSearch** (before typing, results, nothing found, notes that cannot be searched, phone
width, folders and helpers on a server HQ), **AlignedSearch2** (tablet, 2200, 320 and 200%, Lite, the phone
as the hub), **AlignedFlow** ("One Search"), AlignedReview M1, M2, M3, M12, D20. Follows
`../../rules-web.md` sections 2, 8 and 9 and the phone's one-search fix (`../../mobile/wave-4/02-search.md`).
Where this spec and the rules differ, the rules win.

## 1. Finding and the review items it closes

- **D20, W27.** One Search over notes, tasks, chats and, new on both, helpers; Inbox search stays in the
  Inbox's own field.
- **M1.** The place behind reads "Connected" or "Your HQ can't be reached", never "Live" or "Saved · 3
  min". The chip only while Connected.
- **M3.** Failure lines from the catalogue: "Couldn't search your notes and chats. Your HQ can't be
  reached. It will try again by itself." Chats are not saved in a browser, so they are never searched while
  HQ is away (the old board said they were).
- **M2.** Words found are bold on grey, no longer washed green.
- **M12.** The folder filter is done by HQ; Inbox text search is kept in the Inbox.
- **WR12.** Search waits 300 ms after the last key.
- **New:** Clear recent searches, with Undo; "See all N" in the phone's words.

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/hq-web/src/components/SearchOverlay.tsx:30` opens on Ctrl or Cmd `/`; `:69` placeholder "Search
  vault..."; vault notes only (`components/useVaultSearch.ts`); `:56` a scrim click closes.
- `apps/hq-web/src/routes/__root.tsx:169` Ctrl or Cmd K (and H) opens the chat overlay, not Search;
  `:318` the header's search button "Search vault (Cmd+/)".
- `crates/hq-web/src/api.rs` `search_handler`: `q` and `limit` only (no folder); FTS plus semantic
  (`crates/hq-db/src/search/`).
- `apps/hq-web/src/routes/notifications.tsx:225` the Inbox's own field "Search proposals, skills...".
- `apps/hq-web/src/routes/tasks.tsx` `matchesSearch` (the tasks list's own search, kept as "Find in tasks",
  `06-tasks.md`).
- `apps/hq-web/src/sw.ts:34` `/api/search` is in the worker's cached reads.

## 3. Decision

**Ctrl K (Cmd K), the field in the top bar, or the magnifier opens the same Search over whatever place you
are in: notes, tasks, chats and helpers. Scope chips show before typing; one recent list; "See all N"; Ask
HQ about the notes found; a folder filter that HQ applies; a plain line when notes cannot be searched. A
centred overlay from 768, the whole screen below.**

Rejected:

- **An Inbox scope here.** The phone has none; the Inbox keeps its own field (routes/notifications.tsx).
- **Search chats from the saved copy.** Chats are not saved in a browser.

## 4. Screens and states

- **Q1 · Desktop, before typing.** The overlay, 760 wide at most, centred over the dimmed place (drawn over
  Tasks, "Connected · 9 active"). The field has focus at once; chips All, Notes, Tasks, Chats, Helpers show
  where it will look. "Recent searches": "battery", "roadmap", "soak test" (each removable), [Clear recent
  searches]. Hints: "Up Down move", "Enter open", "Tab scope chips", "Esc close". Recent searches are the
  queries that led to a result, eight at most, kept in this browser only.
- **Q2 · Where it starts.** Ctrl K, the top bar field and a link with `?q=` start on All. The Notes search
  pill starts on Notes. Inside a folder it starts on Notes and that folder ("In Agent-HQ", with a Clear
  cross). The field says where it looks: "Search notes", "Search tasks", "Search helpers". No recent
  searches: "Type to search notes, tasks, chats and helpers." On Notes: "Type to search your notes."
- **Q3 · Clear recent searches** acts at once: "Recent searches cleared · Undo" (needs no HQ).
- **Q4 · Close.** Esc, the named Close search button, or a click on the dimmed place behind. Focus goes
  back to what opened it; the place behind kept its scroll.
- **Q5 · Results (query "battery").** Chips with counts "All 6", "Notes 2", "Tasks", "Chats", "Helpers".
  Grouped by kind, five rows each, then "Show all N". Notes: "Samsung battery steps · Projects › Agent-HQ ·
  Open Settings, then Battery. Find HQ in the list of apps." (trail blue, words found bold on grey);
  "Mobile-First-Roadmap · Samsung may undo the battery setting after an update."; [Ask HQ about these 2
  notes]. Tasks: "Check Samsung battery whitelist · T-5 · To do", "T-2 · Ready to check · "battery saver
  off"". Chats: last line "3 min ago · Open Settings, then Battery." Helpers: "Test the battery whitelist on
  the A32 · Codex · on your Mac mini · finished 2 h ago · Finished". Up and Down move the highlighted row
  while focus stays in the field; Enter closes Search and opens the row in its own place.
- **Q6 · Helpers in Search.** Found by job, helper name and computer, matched in this browser from the list
  Helpers saved, so they show even when HQ is slow. A helper that needs you shows its amber pill and opens on
  its question.
- **Q7 · Ask HQ.** On All and Notes whenever notes are found: "Ask HQ about this note", "Ask HQ about these 2
  notes", "Ask HQ about the first 10 notes". It opens Chat with them attached.
- **Q8 · Back from a result.** Browser Back from the opened note reopens Search once, with the same words,
  scope and scroll. Back again returns to where Search was opened.
- **Q9 · Nothing found.** Anywhere: "Nothing matches that." (phone width adds "Try fewer words, or another
  spelling."). One kind: "No notes match that.", "No tasks match that.", "No chats match that.", "No helpers
  match that." with [Search everything] (keeps the words), e.g. "Search helpers 0 · No helpers match that. 1
  note does."
- **Q10 · Notes that cannot be searched.** "Your HQ can't be reached" in the place behind. "Couldn't search
  your notes and chats. Your HQ can't be reached. It will try again by itself." / "Tasks and helpers below
  are from the copy saved in this browser at 14:02." Reasons: "HQ is still starting. It will try again by
  itself." "Your HQ can't be reached. It will try again by itself." "You're offline. It will try again when
  you're back online." "HQ didn't answer in time. Try again." Signed out shows only Sign in, so Search is not
  offered. When HQ can be reached again, the notes search runs by itself ("When HQ is up, notes and chats
  appear here without a tap.").
- **Q11 · Counts.** Chips count once something is typed. 0 is shown, it is the answer. Notes and Chats show
  no number when they were not searched. Groups past five end in "See all N".
- **Q12 · While notes load.** A spinner under the Notes heading, "Searching your notes". The other groups
  show at once.
- **Q13 · Folders.** "Search notes 4" with chips "All folders", "Projects", "Inbox" (the top folders of the
  hits, counts from HQ; choosing one is the same as starting inside it). Inside a folder: "Search notes in
  Agent-HQ" with "In Agent-HQ ×": "Searching only in Projects › Agent-HQ and the folders inside it. HQ does
  the folder search. A note past the first 30 hits is still found."
- **Q14 · Helpers on a server HQ.** "Clean old Gradle caches · Codex · on your server HQ · saved at 14:18 ·
  Stopped": "Helpers are matched in this browser, from the list Helpers saved. Your server HQ can't be
  reached, so its helper shows as saved at 14:18." A server HQ's notes are searched on that HQ's own web
  page (open question, as on the phone).
- **Widths.** 390 and 320: the whole screen over the shell; Back or Close search closes it; the keyboard
  hints hide; the chips scroll sideways (Helpers past the edge when there is no room); result lines wrap to
  two lines, then end in "…". 768 to 1023 (900): the centred overlay, 760 at most, hints shown; below 1024
  the top bar field is a magnifier only; a result opens in one pane with Back to Search. 2200: the overlay
  keeps its 760 width and stays centred. 200% (a 1280 window at 640): the whole screen, as on a phone; the
  field keeps focus; Up and Down still move the highlighted row. Forced colours: chips and rows keep their
  borders; the highlighted row has an outline, not only a fill.
- **Lite.** Two scopes plus All (All, Notes, Tasks), "Search notes and tasks", by keyword: "Lite finds the
  words you type. It does not search by meaning." No Ask HQ. The folder filter works the same; folders
  starting with _ or . are never searched. "Couldn't search your notes. Your HQ can't be reached. It will
  try again by itself."
- **The phone as the hub (HQ window).** "Connected · HQ on your phone". Search asks HQ on your phone for
  everything; with the phone away, notes and chats are not searched and tasks and helpers come from the copy
  saved for that HQ ("Saved on this PC at 14:02"; `../README.md` Open conflict 4). Each search costs the
  phone a little data, so Search waits 300 ms after the last key (WR12). Helpers on this computer and on the
  phone show together, each with where it runs.

## 5. Exact copy

| Key | String |
|-----|--------|
| `search_field_all` | "Search notes, tasks, chats and helpers" (top bar) |
| `search_field_scope` | "Search notes" · "Search tasks" · "Search chats" · "Search helpers" · "Search notes in {Agent-HQ}" |
| `search_chips` | "All" · "Notes" · "Tasks" · "Chats" · "Helpers" · "All folders" · "In {Agent-HQ}" |
| `search_recent` | "Recent searches" · "Clear recent searches" · "Recent searches cleared" · "Undo" |
| `search_hint_empty` | "Type to search notes, tasks, chats and helpers." · "Type to search your notes." |
| `search_keys` | "Up" "Down" "move" · "Enter" "open" · "Tab" "scope chips" · "Esc" "close" |
| `search_close` | "Close search" |
| `search_show_all` | "See all {N}" |
| `search_ask` | "Ask HQ about this note" · "Ask HQ about these {2} notes" · "Ask HQ about the first {10} notes" |
| `search_none_any` | "Nothing matches that." · "Try fewer words, or another spelling." |
| `search_none_kind` | "No notes match that." · "No tasks match that." · "No chats match that." · "No helpers match that." · "{1} note does." · "Search everything" |
| `search_fail` | "Couldn't search your notes and chats." + reason |
| `search_saved_note` | "Tasks and helpers below are from the copy saved in this browser at {14:02}." |
| `search_back_note` | "When HQ is up, notes and chats appear here without a tap." |
| `search_loading` | "Searching your notes" |
| `search_folder_note` | "Searching only in {Projects › Agent-HQ} and the folders inside it. HQ does the folder search. A note past the first 30 hits is still found." |
| `search_server_helper` | "on your server HQ · saved at {14:18}" · "Helpers are matched in this browser, from the list Helpers saved. Your server HQ can't be reached, so its helper shows as saved at {14:18}." |
| `search_lite` | "Search notes and tasks" · "Lite finds the words you type. It does not search by meaning." |

Retired: "Search vault...", "Search vault (Cmd+/)".

## 6. Behaviour rules

1. **Ctrl K (Cmd K) opens Search,** from anywhere (it no longer opens the chat overlay); `?q=` opens it
   with the words.
2. **Debounce 300 ms** after the last key (WR12); the notes and chats calls go to HQ; tasks and helpers are
   matched in this browser from the saved lists (and also asked of HQ when Connected).
3. **Groups** of five per kind, "Show all N" switches to that kind; chips count once something is typed.
4. **The folder filter** is HQ's: `GET /api/search?q=&limit=&folder=Projects/Agent-HQ`. HQ filters by the
   folder before the limit and refuses "..", a leading "/" or a backslash.
5. **Recent searches** are the queries that led to a result, eight at most, in this browser only, per HQ id.
6. **Combobox semantics.** The field is a combobox; the highlighted row is its active row
   (`aria-activedescendant`); the chips are tabs with the chosen one read as selected; each group is
   labelled by its heading and count.
7. **Back reopens Search once** with the same words, scope and scroll.
8. **Never claim nothing matches** when notes or chats were not searched; say why (Q10).
9. **Signed out:** no Search.
10. **Inbox search** stays in the Inbox's own field.

## 7. Browser and platform notes

- Ctrl K in Firefox focuses the browser's search bar when the page does not handle it; the page calls
  `preventDefault()` only when no text field outside Search has focus with its own Ctrl K meaning.
- On iOS Safari the overlay's field must be focused inside the tap's handler for the keyboard to open.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/hq-web/src/components/SearchOverlay.tsx` | The one Search: scopes, groups, recent, folders, failure lines (30, 56, 69). | yes |
| `apps/hq-web/src/components/useVaultSearch.ts` | Notes search with `folder` and debounce. | yes |
| `apps/hq-web/src/lib/search.ts` | Tasks and helpers matching from the saved lists; chats from HQ. | NEW |
| `apps/hq-web/src/routes/__root.tsx` | Ctrl K to Search (169); the top bar field (318). | yes |
| `crates/hq-web/src/api.rs` | `search_handler` takes `folder`, filtered before the limit; refuses bad paths; facet counts by top folder. | yes |
| `crates/hq-db/src/search/` | A folder prefix in the FTS and semantic queries. | yes |
| `crates/hq-web/src/threads_api.rs` | Chat search (titles and last lines). | yes, route NEW |

## 9. Server, desktop or release changes

Server: `folder` on `/api/search` and folder counts; a chat search route (`../../changes.md` "`/api/search`
takes `folder`"). Desktop: none.

## 10. Acceptance checks (each fails on today's build)

1. Ctrl K opens Search with the field focused and five chips. Today it opens the chat overlay.
2. Typing "battery" groups notes, tasks, chats and helpers; a helper row shows its pill. Today only notes.
3. `GET /api/search?q=battery&folder=Projects/Agent-HQ` returns only notes in that folder, and
   `folder=../x` answers 400. Today `folder` is ignored.
4. With HQ unreachable, the notes group shows the failure line and tasks come from the saved copy. Today
   the overlay shows nothing or an error.
5. A request is sent 300 ms after the last key, not on every key.

## 11. Check on a laptop or phone

- Search from every place at 1440 and 390; Back from a result reopens Search once.
- VoiceOver: the field reads as a combobox; Up and Down announce the row.

## 12. Open decisions it depends on

- **D20, W12, W27, WR12** (Recommended). **N3** (a server HQ's notes on the phone; open on the web).
- Depends on: `03-notes.md` (the pill), `07-helpers.md` (the saved helpers list), `06-tasks.md` (Find in
  tasks).
