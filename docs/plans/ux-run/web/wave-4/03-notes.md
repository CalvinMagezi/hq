# Web wave 4 · 03 · Notes: home, folders, a note, editing, export, who wrote it (WB4, W19, W21, W22)

Boards: **AlignedNotes** (home, a folder, when HQ is not connected, New note), **AlignedNotes2** (tablet,
2200, 320 and 200%, Lite, the phone as the hub), **AlignedNotesNote** (reading, export and More, who wrote
it, editing), **AlignedNotesNote2** ("This note changed somewhere else.", widths, Lite, the phone as the
hub), AlignedReview WB4, W19, W21, W22, M12, M13, Drift 10, 12. Follows `../../rules-web.md` sections 3, 4
and 8 and `../../rules-shared.md`. Where this spec and the rules differ, the rules win.

## 1. Finding and the review items it closes

- **WB4 (blocker). Note saves overwrite each other; offline creates duplicate.** `PUT /api/note` has no
  base version (last writer wins over the phone, another tab or HQ), and create has no client id, so a
  retry makes "Title 2". A base hash and a 409; a "Changed somewhere else" state with See their version and
  Keep both; create keyed by client id.
- **W19 revised.** Owner writing counts as checked only for text the owner typed, through `note_review`.
- **W21.** Move to trash, rename and move appear only once HQ has routes for them, with Undo.
- **W22 revised.** Copy link is a note-id link (`/notes/n/:id`), "Opens only on your signed-in devices".
- **M12 (notes).** Send to Chat from selected text is kept. **M13.** Text size lives in the note menu; Ctrl
  plus, minus and 0 stay the browser's zoom (today the note page takes them over).
- **Drift 10.** A suggested pin on Home has no way to keep it. **Drift 12.** Notes lights no nav item on a
  note.
- **W12.** The Notes search pill opens the one Search with Notes chosen. **WB1:** every export on a shared
  computer asks first.

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/hq-web/src/routes/vault.tsx`, `vault/index.tsx`, `vault/$.tsx`: Home (Get started, briefing,
  Pinned, Recent Project Notes, In Progress Tasks, System Activity) and the note page.
- `apps/hq-web/src/components/vault/HomeSections.tsx`, `PinnedCard.tsx:5, 78-81`: "Suggested by
  {claude-code}, not yet in HQ's context." with Keep pin only when `onKeepPin` is passed; Home does not pass
  it (Drift 10); `VaultSidebar.tsx:19` does.
- `routes/vault/$.tsx:129-143` Ctrl or Cmd plus, minus and 0 change the note's text size
  (`MD_ZOOM_STEP`), taking over browser zoom; `:346-360` Edit and View; `:432` "Convert to task";
  `components/CopyPathMenu.tsx` Copy path; `components/ExportMenu.tsx` (6af8ba8).
- `components/NoteEditor.tsx:48` Ctrl or Cmd S saves.
- `components/QuickNoteOverlay.tsx:20` Ctrl or Cmd N opens Quick note (a browser's new-window key); `:141`
  Ctrl or Cmd Enter saves.
- `components/SelectionToolbar.tsx:84` "Send to Chat".
- `crates/hq-web/src/vault_api.rs:621-650` `note_update_handler`: `{path, content}`, no base version,
  `write_atomic` then `record_owner_write`; `:572` create, `:359-367` `create_unique` makes "{title}
  {n}.md".
- No rename, move or delete route for notes: `GET`/`PUT /api/note` and `POST /api/note/create`
  (`crates/hq-web/src/lib.rs`).
- Provenance: `crates/hq-db/src/provenance.rs` (`note_provenance`, `note_review_events`),
  `crates/hq-tools/src/vault/review.rs` (`note_review`); the web shows provenance only in the task activity
  view (`components/tasks/activity.ts:74`).
- `components/AppRail.tsx:32` Home matches `/vault` exactly.

## 3. Decision

**Notes is the phone's Notes at desktop width: home is search first, then folders, Pinned and Recent. A
folder lists its folders and its notes A to Z. A note opens beside the folder from 1024, with the phone's
action chips plus Export, and a details panel from 1400 that says who wrote it and whether anyone checked
it. Saves carry the version they started from; HQ never overwrites silently.**

Rejected:

- **Merge two versions by itself.** HQ does not mix the two texts; the owner copies lines across.
- **Show Rename, Move and Move to trash disabled.** They come back when HQ has routes, with Undo (W21).

## 4. Screens and states

### Home and folders (Notes 1, 2)

- **H1 · 1440.** Top bar "Notes" / "Connected" (chip only then). Pane head "Your notes" [New note] with
  the hint "Ctrl Alt N". The search pill "Search your notes" (opens the one Search on Notes, never a
  second notes search). Folder chips "Projects", "Daily", "Logs". "Pinned 3": "Mobile-First-Roadmap ·
  Projects · 1d ago", "Phone-Cutover-Evidence-2026-10-20 · Projects · 1d ago", "Relay test plan · Projects
  · 30m ago" (a dashed card: "A helper suggested this pin. HQ does not use it until you keep it." [Keep
  pin] [Not now]). "Recent": "Samsung battery steps · Projects › Agent-HQ · by HQ · now",
  "Backup-Setup-2026-10-20 · Projects › Agent-HQ · 3h", "Blind-Relay-Design-2026-10-20 · Projects ›
  Agent-HQ · 1d", "Quick idea: widget copy · 1d". The old Home's Get started moved to Chat, the briefing to
  Tasks › While you were away, system activity to Inbox › Updates.
- **H2 · 390.** Search first, the folder chips, Pinned as a row that scrolls sideways, Recent with the blue
  folder trail. New note in the pane head on every width.
- **H3 · No notes yet.** "Your notes will show up here" / "Write one yourself, or ask HQ to write one for
  you." [New note] [Ask HQ to write one] (opens Chat with "Write me a note about " in the box).
- **H4 · Inside a folder (1440 full width until a note opens).** Trail "Agent-HQ › Projects" (goes up one
  level at a time; Back on a phone does the same) [New note here] (saves into this folder). "Folders 2":
  "Android spikes · 7 notes", "Evidence · 12 notes". "Notes, A to Z 26": rows with the trail and age;
  past 25 the list ends in "Show all 26" (`../../resolutions.md` B6). "Folders" and "Notes, A to Z" are list headings, not controls.
- **H5 · Rows** are real links; the trail is the folder in blue; a note HQ wrote says "by HQ"; ages are
  short: now, 3h, 1d, 2w. Notes stays lit in the rail and the bar on every `/notes` address.
- **H6 · Not on the web yet.** Rename, Move and Move to trash are not drawn: HQ has no route for them yet.

### When HQ is not connected (Notes 3)

- **N1 · Can't be reached.** Strip "Your HQ can't be reached. Saved in this browser at 14:32." "Waiting to
  send · Check the backup status · Saved in this browser. It will try again when HQ is back." Pin and Keep
  pin are off. A new note waits in this browser, listed, each removable.
- **N2 · Signed out.** "Signed out of this HQ" / "Sign in to see your notes. Nothing from this HQ stays in
  this browser." [Sign in].
- **N3 · Starting, with nothing saved.** "Nothing saved in this browser yet" / "Your notes show here once
  HQ has started." Blue. Never an empty Notes.
- **N4 · Loading.** "Getting your notes"; a saved copy shows at once if there is one, the spinner only
  without it.
- **N5 · An empty folder.** "Nothing in this folder yet." [New note here].
- **N6 · A section that failed** (only while Connected, one call failed): "Couldn't load Recent. HQ didn't
  answer in time. Try again." [Try again]. In every other state the status line and the strip say it.
- **N7 · Offline.** "You're offline. Saved in this browser at 14:32." New notes wait to send.
- **N8 · The AI service isn't answering.** Only the status line changes; notes work; Ask HQ still opens
  Chat, which says why.
- **N9 · One saved copy.** "Your HQ can't be reached. Saved in this browser on Tue 3 Nov." (a copy from
  before today names the day). At most 256 KB a place and 2 MB in all, per HQ, the same cap as the phone
  (`../../resolutions.md` B16), kept until replaced, keyed by the HQ's id.

### New note, Ctrl Alt N from anywhere (Notes 4)

- **Q1 · 1440, centred over the place you are in** (drawn over Tasks). Sheet "New note": "Title", "Save
  to" (a menu from 768; "Where new notes go unless you pick another"; every folder indented by depth, with
  a filter "Find a folder"; the chosen one has an ink check), "Note". Hint "Ctrl Enter saves · Esc closes".
  [Cancel] [Save].
- **Q2 · 390.** From the bottom, tall. It saves to Inbox unless you pick another folder.
- **Q3 · Saving while HQ can't be reached.** "Your HQ can't be reached. Save keeps the note in this
  browser. It will be sent when HQ is back." A save that fails while Connected: "Couldn't save the note. HQ
  didn't answer in time. Try again." and the text is kept.
- **Q4 · On a shared computer.** With HQ away, Save is off: "Notes can't wait here on a shared computer."
- Every new note carries a client id and HQ drops a repeat (no "Title 2").

### A note: read, act (Note 1)

- **R1 · 1440.** The folder stays as the list pane, the note beside it; from 1400 the details panel. Head:
  trail "Projects › Agent-HQ", title "Samsung battery steps", "· saved by HQ just now", tag "Not checked
  yet". Action chips: [Edit] (the one green), [Ask HQ] (opens Chat with the note attached), [Make a task]
  (adds it to the Inbox list: "Task added to your Inbox"), [Unpin], [Export], then More. The body: "Do
  these four things on the Galaxy A32 before the soak test starts." and the steps, a table (Phone ·
  Android · Setting · Checked · Undone by update), a quote, "Written for Mobile-First-Roadmap, which says to
  test on the A32 first." "Tasks from this note 1 · Check Samsung battery whitelist · To do · due tomorrow
  · High".
- **R2 · About this note (details panel).** "Who wrote it · This version · HQ, in the chat"; "When · just
  now"; "Versions · 3 · Show history"; [Looks right] [Something's wrong] with "HQ can use what it wrote
  either way. Looks right tells it, and helpers, that you read this version."; "Where it is · Folder ·
  Pinned · Yes · HQ reads it in every chat"; "Tasks from it · 1 task".
- **R3 · 390.** The trail and age, then the chips (they scroll sideways when they do not fit). About this
  note opens the details as a sheet.
- **R4 · Its menu.** In Notes it starts with Edit; then "Ask HQ about this note", "Export as… · PDF, Word, a
  picture and more", "Copy link · Opens only on your signed-in devices", "Copy note text", "Text size".
  Copy link copies a link by the note's id, never a folder path.
- **R5 · A note that is gone.** "This note is not here any more" / "It may have been moved or deleted. It
  was called Old-plan, in Projects." [Search for it] (opens the one Search with its name typed) [Go to
  Notes]. From a stale pin, chip or link.
- **R6 · More than notes.** PDFs, pictures, Word, Excel and web pages still open in place; their bar keeps
  Download and Copy link, with no Edit.

### Export, More and the states around them (Note 2)

- **E1 · Export from 768** opens a menu under its chip; below 768 the phone's Export as sheet (as
  `01-chat.md` S3). Same groups and words; the web keeps CSV and JSON for tables; table formats show only
  when the note has a table.
- **E2 · More.** "A-" "110%" "A+" [Reset text size]: Copy link, Copy note text, then text size, kept per
  note. Ctrl plus, minus and 0 stay the browser's zoom.
- **E3 · Making a file.** "Making your PDF… · Large notes take a little longer" in its row; the others wait.
  Esc or a click outside closes the menu; the file still comes.
- **E4 · Done or failed.** "Samsung battery steps.pdf is in your downloads"; "Couldn't make the PDF. HQ
  didn't answer in time. Try again." [Try again]; a failure keeps its toast until Try again or close.
- **E5 · With HQ away.** "Edit, Make a task, Pin and Export need HQ. Your HQ can't be reached." Ask HQ stays
  on; it opens Chat, which keeps the draft as Waiting to send.
- **E6 · Selected text.** Selecting words offers [Send to Chat] (kept from `SelectionToolbar.tsx`); Chat
  opens with the words quoted.
- **E7 · Copied a link.** "Link copied. It opens only on your signed-in devices." A link such as
  `/notes/n/8f3a2c` opens the note for a signed-in browser or phone, and Sign in for anyone else.
- **E8 · On a shared computer.** "Export on a shared computer?" / "The file stays in this computer's
  Downloads. Others who use it can open it." / [Not now] [Export PDF] (green: nothing is lost).
- **E9 · Not on the web yet.** "No Rename, Move or Move to trash in More." / "HQ has no route to rename,
  move or delete a note yet. GET and PUT /api/note and POST /api/note/create are all there is
  (crates/hq-web/src/lib.rs)." (a builder's note, not owner copy).

### Who wrote it, and whether it was checked (Note 3)

What HQ records for each version, in plain words: no actor ids, no "provenance", no "unattested". The tag
and the panel are the only places this shows. One tag, words always, warn only for Something's wrong.

| State | Example | Meaning |
|-------|---------|---------|
| Not checked yet | "HQ, in a chat · yesterday" | Written by HQ, not checked. Two plain choices: Looks right accepts this version; Something's wrong asks for a short reason. |
| Looks right | "You · today 14:40 · Change" | "You accepted it today at 14:40. A new version starts as Not checked yet again." |
| Something's wrong (amber) | "Claude, a helper, on T-12 · 2 h ago" · ""The A32 steps are out of date."" · "You · 2 h ago" | "Your reason shows with it. Helpers see it when they open the note." |
| Looked at | "Codex, a helper, on T-9" | "A helper read it without vouching for it. Only you can say Looks right." |
| Replaced | "Newer: Samsung battery steps, Nov" | "A later note replaces this one." |
| Not known | "4 Nov" | "Changed outside HQ, so who wrote it is not known. Nothing is guessed, and there is nothing to check." |
| Typed by you | "You, in this editor · 1d ago" | Only text you typed here counts as Looks right (W19); HQ records it through `note_review`. A paste or an import starts as Not checked yet. |

History: "Now · You · 2 h ago", "Before · HQ · yesterday", "First · Claude, a helper · 3 Nov". Show history
lists every recorded version, newest first, who and when, and its state.

### Editing (Note 4)

- **D1 · 1440.** A plain text box with tinted marks (the marks greyed, [[links]] blue; the text itself never
  changed), a format bar (H, B, I), "Unsaved changes", [Preview] [Save], "Ctrl S saves". Notes over
  200,000 characters stay editable here, untinted.
- **D2 · 390.** The format bar sits over the keyboard.
- **D3 · Leaving with changes.** "Save your changes?" / "You edited Mobile-First-Roadmap and did not save.
  Your text stays in this browser unless you discard it." / [Save changes] above, full width, then [Keep
  editing] left and a red [Discard changes] right. Closing the tab asks the browser's own question.
- **D4 · A save that failed.** Amber with an icon: "Couldn't save your changes. HQ didn't answer in time.
  Try again. Your text is kept here." with Try again beside it. The status runs Saved, Unsaved changes,
  Saving…, Couldn't save.

### "This note changed somewhere else." (Note 2, section 1)

- **X1 · 1440.** The editor shows "Not saved"; a card above the text: "This note changed somewhere else." /
  "Your phone saved a new version at 14:31. Your changes are still here, not saved." [See their version]
  [Keep both]. Save is off until you choose.
- **X2 · 390.** The same card above the text.
- **X3 · See their version.** Their text, read only, "Their version · Saved from your phone at 14:31",
  "Lines that differ from yours: 1. Read only." [Back to mine]; Keep both is here too.
- **X4 · Keep both.** Your text becomes a new note beside theirs: "Both kept. Yours is
  "Mobile-First-Roadmap, your version"." [Open]. Nothing is lost, so no question and no red (WR11).
- **Where it came from:** "Your phone", "another tab in this browser", "HQ, in a chat" or "a helper". Not
  known says "somewhere else" only.

### Widths, Lite and the phone as the hub (Notes 2, Note 2)

- **768 to 1023 (900):** the rail; no list pane below 1024, so a folder or a note takes the whole width and
  Back goes up one level. The top bar keeps title, state, Search and chip. Pinned is a grid of two or three.
  A note replaces home, with Back; About this note opens as a side sheet. The action bar fits on one line
  from about 860.
- **2200:** Home stops at 1080 wide and centres (rows longer than about 120 characters are hard to follow;
  Pinned shows at most four in a row). A note: the folder, the note and the details panel each at its own
  width; the text stops at 720 and centres.
- **320:** the top bar holds the title and the state; Search, Spending and Settings move to the menu (one
  More button named "Search, Spending and Settings"; Ctrl K still opens Search). Pinned scrolls sideways.
  The state wraps onto two lines. The keyboard hints hide below 768. A note: the chips scroll sideways; the
  table scrolls inside its own frame, never the page.
- **200% (a 1280 window at 640):** the phone layout. New note fills the height and scrolls; a toast never
  covers the focused field (it waits until the sheet closes; "Couldn't save" shows in the pane head beside
  the field). The editor keeps its format bar on top. Text size in the note menu adds to the browser's zoom
  and never replaces it.
- **Forced colours and print:** cards and chips keep their borders; a printed note is black on white,
  without the shell or the action bar; links print their note title.
- **Lite:** three places, starting on Notes, "Connected · HQ Lite" in the status line. No Ask HQ, suggested pins or "by
  HQ" trails; folders starting with _ or . not shown; search by keyword ("Search your notes"); no notes:
  "Your notes will show up here · Write your first one." A note keeps Edit, Make a task and Pin; no Ask HQ
  and no Export; who wrote it only You or Not known; Copy note text stays ("It is the way to take a note
  out of Lite. "hq lite export" copies the whole folder."); Looks right still records through
  `note_review`.
- **The phone as the hub (HQ window):** "Connected · HQ on your phone"; saved copies and the outbox keyed
  by the HQ's id, labelled "Saved in HQ Hub at 14:32" (`../../resolutions.md` B11); the phone pays for
  every read (Notes loads on demand, pauses when the window is hidden); no web push. Export is off: "The
  HQ window can't save files. Open HQ in your browser to export." (WR13). Copy link works (the
  clipboard, on a click); Share is hidden (WR29). Conflicts:
  edits on the phone and in this window meet the same "changed" check; "Your phone" is named as where it
  changed.

## 5. Exact copy

Every string in section 4 is exact. Keys `notes_*`, `note_*`. Notable:

| Key | String |
|-----|--------|
| `notes_title` | "Your notes" · "New note" · "Ctrl Alt N" · "Search your notes" |
| `notes_pinned_suggested` | "A helper suggested this pin. HQ does not use it until you keep it." · "Keep pin" · "Not now" |
| `notes_empty` | "Your notes will show up here" · "Write one yourself, or ask HQ to write one for you." · "Ask HQ to write one" (Chat prefill "Write me a note about ") |
| `notes_folder` | "New note here" · "Folders" · "Notes, A to Z" · "Show all {26}" · "Nothing in this folder yet." |
| `notes_nothing_saved` | "Nothing saved in this browser yet" · "Your notes show here once HQ has started." |
| `notes_new_sheet` | "New note" · "Title" · "Save to" · "Note" · "Ctrl Enter saves · Esc closes" · "Cancel" · "Save" · "Find a folder" · "Where new notes go unless you pick another" |
| `notes_new_away` | "Your HQ can't be reached. Save keeps the note in this browser. It will be sent when HQ is back." |
| `notes_new_shared` | "Notes can't wait here on a shared computer." |
| `note_actions` | "Edit" · "Ask HQ" · "Make a task" · "Pin" / "Unpin" · "Export" · "More" |
| `note_task_added` | "Task added to your Inbox" |
| `note_gone` | "This note is not here any more" · "It may have been moved or deleted. It was called {Old-plan}, in {Projects}." · "Search for it" · "Go to Notes" |
| `note_away` | "Edit, Make a task, Pin and Export need HQ. Your HQ can't be reached." |
| `note_link_copied` | "Link copied. It opens only on your signed-in devices." |
| `note_export_shared` | "Export on a shared computer?" · "The file stays in this computer's Downloads. Others who use it can open it." · "Not now" · "Export {PDF}" |
| `note_text_size` | "A-" · "{110}%" · "A+" · "Reset text size" |
| `note_review_*` | "Not checked yet" · "Looks right" · "Something's wrong" · "Looked at" · "Replaced" · "Not known" · "HQ can use what it wrote either way. Looks right tells it, and helpers, that you read this version." |
| `note_conflict` | "This note changed somewhere else." · "{Your phone} saved a new version at {14:31}. Your changes are still here, not saved." · "See their version" · "Keep both" · "Their version" · "Saved from {your phone} at {14:31}" · "Lines that differ from yours: {1}. Read only." · "Back to mine" · "Both kept. Yours is "{title}, your version"." · "Open" |
| `note_save_status` | "Saved" · "Unsaved changes" · "Saving…" · "Couldn't save" · "Not saved" |
| `note_window_export` | "The HQ window can't save files. Open HQ in your browser to export." |

## 6. Behaviour rules

1. **One search.** The pill opens the one Search with Notes chosen (Ctrl K), never a second notes search;
   inside a folder it starts on that folder ("In Agent-HQ"). HQ does the folder search (`04-search.md`).
2. **Suggested pins** are a dashed card with Keep pin and Not now, on Home too. Keep pin only pins it;
   checking the note stays in About this note.
3. **Base version on save.** `PUT /api/note {path, content, base}` where `base` is the content hash the
   editor opened with (`hq_db::search::content_hash`); HQ answers 409 `{changed: true, by, at, theirs_hash}`
   when the note moved on and saves nothing.
4. **Keep both** saves "{title}, your version" in the same folder with a new client id; theirs stays.
5. **Create by client id.** `POST /api/note/create {client_id, …}` returns the existing note for a repeat;
   never "Title 2".
6. **Copy link** is `/notes/n/{id}` (HQ gives each note a stable id; NEW), opening only for a signed-in
   browser or phone; Copy path copies where the note lives.
7. **Text size** lives in More, kept per note; the page never handles Ctrl plus, minus or 0.
8. **New note** is Ctrl Alt N (Cmd Option N) from anywhere; Ctrl Enter saves; Esc closes. The old Ctrl N
   and Ctrl . go.
9. **Looks right** records through `note_review` and only for text typed in this editor; a paste or an
   import starts as Not checked yet.
10. **Rename, Move, Move to trash** are not drawn until HQ has routes (W21).
11. **Exports on a shared computer** ask first; in the HQ window Export is off with the reason, Copy link
    works and Share is hidden (WR13, WR29).
12. **Notes stays lit** in the rail and bar on every `/notes` address.

## 7. Browser and platform notes

- Ctrl Alt N on Windows with some keyboard layouts (Polish, for example) types a character (AltGr = Ctrl
  Alt); only bind it when no text field has focus, or accept `e.code === 'KeyN'` with `altKey && ctrlKey`
  and no composition.
- `beforeunload` shows the browser's own leave question; its text cannot be set.
- Print styles: `@media print` hides the shell; `forced-color-adjust` keeps borders.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/hq-web/src/routes/notes.tsx`, `notes/$.tsx`, `notes/n.$id.tsx` | Notes routes (from `vault*`). | NEW |
| `apps/hq-web/src/routes/vault/$.tsx` | Moves into `notes/$.tsx`; drop the zoom keys (129-143). | yes |
| `apps/hq-web/src/components/vault/HomeSections.tsx`, `PinnedCard.tsx`, `HomeSection.tsx` | Home, Keep pin on Home, saved strip. | yes |
| `apps/hq-web/src/components/QuickNoteOverlay.tsx` | The New note sheet, Ctrl Alt N, Save to, outbox (20, 141). | yes |
| `apps/hq-web/src/components/NoteEditor.tsx` | Status, conflict card, base hash, leave dialog. | yes |
| `apps/hq-web/src/components/ExportMenu.tsx`, `CopyPathMenu.tsx`, `SelectionToolbar.tsx` | Export menu and sheet, Copy link by id, Send to Chat. | yes |
| `apps/hq-web/src/components/notes/AboutNote.tsx`, `ReviewTag.tsx`, `NoteHistory.tsx` | The panel and tag. | NEW |
| `apps/hq-web/src/lib/vaultApi.ts` | `base`, `client_id`, review calls. | yes |
| `crates/hq-web/src/vault_api.rs` | Base hash and 409 (621-650); idempotent create (572); a note id route. | yes |
| `crates/hq-tools/src/vault/review.rs`, `crates/hq-db/src/provenance.rs` | Read routes for the panel and history; mark through `note_review`. | yes |

## 9. Server, desktop or release changes

Server: base hash and 409, idempotent create, note ids and `/notes/n/:id` resolution, provenance read and
mark routes (`../../changes.md` "Notes: a save carries the version…"). Desktop: a save dialog command for
the HQ window later (WR13).

## 10. Acceptance checks (each fails on today's build)

1. `cargo test -p hq-web vault_api`: a PUT with a stale `base` answers 409 and leaves the file unchanged.
   Today it overwrites.
2. Two creates with the same `client_id` make one note. Today "Title 2".
3. Ctrl plus on a note page zooms the browser (the page does not `preventDefault`). Today it changes the
   text size.
4. Home's suggested pin has Keep pin. Today it does not.
5. On `/notes/Projects/HQ/Plan.md` the rail's Notes is lit. Today nothing is lit.
6. Copy link puts `/notes/n/{id}` on the clipboard. Today Copy path only.
7. New note opens on Ctrl Alt N and not on Ctrl N.

## 11. Check on a laptop or phone

- Edit a note on the laptop and on the phone at once; save the phone first, then the laptop: the card, See
  their version, Keep both.
- A shared-computer sign-in: Export asks first.

## 12. Open decisions it depends on

- **W12, W19, W21, W22, WR11, WR13, WR29** (Recommended or Revised), `../../resolutions.md` B6, B11,
  B16.
- Depends on: `04-search.md`, `01-chat.md` (the note sheet, Ask HQ), `../wave-0/02-shared-parts.md`
  (outbox, saved copy), `../wave-1/02-dialogs-approvals.md`.
