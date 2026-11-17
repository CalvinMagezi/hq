# Web restructure: build plan

Status: slices 1 and 2 built (Vault navigation), Chat's grammar, the work pane, the palette, the trigger menu, the Inbox, Settings and the Hub's menu model built. Move and rename wait on the server. The rest are proposals.

The design is the "Restructure" page of the owner's HQ Web & Desktop design canvas (private). This file
is the build order and the decisions that came out of it, so a session can continue without the canvas.

## What the restructure is

The web app and the HQ window keep three places (Vault, Chat, Inbox) plus Settings, and get four
changes in how they work at desktop size:

1. The Vault gets a location bar and a folder tree, so a person always knows where they are and can
   come back up in one move.
2. Chat takes the phone's grammar (one live line, a result line, one slot above the box) and a work
   pane on the right that opens by itself once per chat when HQ makes a document, sheet, slides or
   dashboard.
3. One palette (Ctrl K) replaces the search, quick-note and global-chat overlays.
4. Settings uses the phone's six groups, a search box and one warning card at a time.

Decided with the owner: the pane opens by itself, once per chat, and stays closed after the person
closes it. Navigation is three places and Settings, with a labeled row in Settings and a link at the top
of the Inbox as the doors to Tasks and Helpers (not a fourth item). The Vault tree is a panel the person
can put away, not a permanent sidebar.

## Slice 1 (built): Vault navigation

- `components/vault/VaultNav.tsx`: `LocationBar` (Up, the trail with its middle folded into one menu,
  the Folders button), `FolderTree` (a tree with one tab stop, arrows, Home, End, open path in bold,
  counts, Show all) and `VaultShell`, which `routes/notes.tsx` renders around every Vault page.
- `components/vault/noteRows.ts`: the rules as pure functions (`trailOf`, `upTarget`, `foldTrail`,
  `treeRows`, `treeMove`, `siblingsAround`, `vaultLocation`), tested in `vaultNav.test.ts`.
- The tree opens by itself from 1280 px in a folder or a note and stays closed on Vault home. Ctrl or
  Cmd B toggles it, except while typing, because the note editor uses it for bold. The choice and the
  opened folders are remembered in `localStorage` (`hq-vault-tree`, `hq-vault-tree-open`).
- Folders that start with an underscore are left out of the tree, as the chips already leave them out.
- The note page's list pane (the sibling notes) is replaced by the tree, and by Previous and Next links
  at the foot of the note. The page no longer draws its own breadcrumb line, since the bar does.

The old test "the tree sidebar is gone" was changed on purpose: it recorded the earlier decision, which
the owner reversed. The sidebar files it names (`VaultSidebar`, `VaultTree`, `useSidebarResize`) stay
deleted; the tree is the one component above.

## Slice 2 (built): the rest of Vault navigation

- Back and Forward follow the router's history and are dimmed when there is nowhere to go. The router
  knows how far back it can go (`__TSR_index`) but not whether Forward would do anything, so the furthest
  index seen is tracked (`furthestIndex`). Alt Left, Alt Right and Alt Up work from anywhere in the
  Vault except while typing.
- Every crumb has a chevron menu: the folders beside it (the notes beside it, for a note), with counts,
  a check on the current one, arrow keys, a filter past 8 items, and "Open {folder}". The menu is placed
  in fixed position so the bar's clipping cannot hide it.
- Below 1024 px the Folders button opens the tree as a side sheet (a bottom sheet below 768 px) that
  closes by itself when a folder is chosen. Below 768 px the bar wraps: Up names the parent folder, the
  Folders button sits beside it, and the path is its own scrolling line.
- A link to something under Notebooks that is not in the tree shows "This is not in the Vault any more"
  with three exits: the nearest folder that still exists, a search for the name, and the Trash
  (`VaultMissing.tsx`).
- The tree's menu has Collapse all, Show the open note and Show system folders. System folders
  (underscore) are listed dimmed with a lock, and only when asked for.

## Not built in Vault navigation, and why

- Move and rename: built for a note. `POST /api/note/move` (`{from, to, dry_run}`) runs the same code as the
  `vault_move` tool, so links to the note in other notes are rewritten (the open question is answered: they
  follow) and the index follows. The note bar's Move opens a sheet where typing a new place asks for a dry run
  and says how many notes link here; Move does it and Undo moves it back.
- Folder moves and drag and drop: `POST /api/folder/move` (`{from, to, dry_run}`) renames the folder inside
  Notebooks, refuses a destination that exists, a move into itself, and the protected folders, and re-indexes
  every note under it (search, link graph, provenance). Wikilinks are by note name, so none needs rewriting.
  In the tree, notes and folders drag onto a folder (or onto the tree's empty space for the top folder); system
  folders neither drag nor take a drop; the toast offers Undo. The folder page has a Move button for keyboard
  and touch, so dragging is never the only way. Not built: moving several things at once, and a notes-only
  folder move that also keeps plain relative markdown links working (the vault rewrites wikilinks only).
- New folder, sort by Modified and the grid view on the folder page: the directory listing carries no
  modified time, and nothing creates an empty folder.
- Scroll position on Back is whatever the router restores; it is not guaranteed by this change.

## Chat, first part (built)

- The chat list shows plain text in its preview line (markdown marks are stripped, `lib/replyText.ts`) and a
  "Draft" mark when the box for that chat holds unsent text (`rowLine`, `useDraftText`).
- A finished reply has two more hover actions: Save to Vault (a note in `Notebooks/Inbox`, titled from the
  reply, with an Open and an Undo in the toast) and Make a task (fills the box with a draft the person
  finishes and sends; nothing is created until they send it). `lib/saveReply.ts`.
- Undo on any toast now also drops that toast's action button.

## Chat grammar and the work pane (built)

- The live line (`LiveLine.tsx`, `lib/liveLine.ts`): while a reply streams there is one row above the
  answer, the step running now in the present tense ("Reading “Stock rules”…", words in `live_step_*`),
  with seconds and, after ten seconds, a Stop button. It is replaced by the next step, never stacked. The
  answer's words arrive below it. The phone's `ToolWords` has no present-tense table yet.
- The result comes first. A finished reply shows its text, then the fold "What I did · N steps"
  (`ToolCallBlock`, which no longer has a live mode), then a card for each note HQ wrote
  (`MadeCard`, from `vault_write_note`) and a chip for each note, file and page it read (`SourceChips`).
  A web page is named by its site, never its address.
- One slot above the box (`ChatApprovals` with `otherwise`): a pending approval, else the offer under the
  last reply (`OfferLine`). Offers are not shown while a reply streams. An approval is shown even then,
  since HQ may be waiting on it; this differs from the board, which hides cards while streaming.
- The work pane replaces the old details pane (`WorkPane.tsx`, `store/workPaneStore.ts`). Tabs: Result
  (the note HQ made, with Edit, Ask HQ, Make a task and Pin, and a link that opens the same note full
  width in the Vault), Context (reading now, already used, sources, this reply's steps, seconds and
  credit, built from the tool calls the socket already sends) and Helpers (the sessions the chat watches,
  as before). It opens by itself once per chat when a note is finished, only from 1400 px where it has
  room (`lib/workPane.ts`, `hq-pane-auto` in `localStorage`), and stays closed once closed. Ctrl or Cmd
  and the full stop toggle it. Below 1400 px it lies over the chat; that overlay had no CSS before.
- Keys: Up Arrow in an empty box takes the last message back to edit, Ctrl Shift C copies the last
  reply, Alt Up and Alt Down move between chats. The "Enter sends" checkbox became a line of hints with a
  Change link; the choice moves to Settings with that slice.
- Not built: Export in the pane's bar (it stays on the full page), the Sheet, Slides and Dashboard
  viewers (the hqmd reader and the suite milestones decide them; the Result tab shows any note as
  markdown, which hqmd allows), follow-up chips in the slot (no source for them yet), "Not sent / Try
  again" wording and the auto-title.

## The palette and the trigger menu (built)

- The palette is the Search overlay grown, not a new component (`SearchOverlay.tsx`, `lib/palette.ts`).
  Ctrl or Cmd K opens it. Groups in a fixed order: Vault (folders match too, by any words of their path),
  Tasks, Chats, Helpers, then Do. With nothing typed it shows Recent and the Do actions with their keys
  (New note, New chat, Work pane in a chat, and Go to the Vault, Chat, Inbox, Settings); no row is chosen
  until an arrow key or the pointer moves, so Enter does nothing by surprise.
- The field never ends on "no results": when something is typed, "Ask HQ about ..." is the last row. It
  opens a new chat with the words in the box, ready to send (one more Enter than the board draws, because
  sending needs the chat's socket). `compose=1` now really opens the start screen; before, it landed in
  whichever chat was last open, which also made a note's Ask HQ misplace its text.
- A read-only preview sits beside the results from 1100 px: kind, place and first lines. A note's lines
  are fetched only for the row that stays highlighted for 250 ms, and not at all when the browser asks to
  save data. Shift Enter shows a note in the open chat's work pane; Ctrl Enter asks HQ at once.
- New chat has the key Ctrl Shift O. Every key the Do rows show is one the app handles.
- Differences from the board: Tab stays with the scope chips so they can be reached by keyboard (Ask is
  Ctrl Enter and a row); Alt 1 to 3 and the "Ctrl /" key list are not built, since the board says they
  need checking against browser and desktop-app shortcuts. The quick-note overlay is still there and the
  New note action opens it; there is no global chat overlay left to remove.
- The message box hears `@` and `#` (`lib/triggers.ts`, `TriggerMenu.tsx`). `@` offers notes (pinned notes
  until something is typed, then search), tasks and helpers; `#` offers folders. A pick puts a backticked
  reference in the text, the form Ask HQ already uses, and one Backspace removes a whole reference. Not
  built: files and tags (the web has no index of either), and the Settings choice of what `@` offers.

## Inbox, Settings and the Hub (built, in part)

Inbox (`routes/inbox.tsx`, `components/notifications/`, `lib/inboxView.ts`):
- On a wide screen the card is never empty while something waits: the sign-in opens by itself, else the
  oldest request, and answering one opens the next. The sign-in card in the detail pane now carries its own
  words and "It was me" / "Sign it out", as the list card does.
- Needs you carries the only number; Updates has a quiet dot when something is unread; Done has nothing.
  Links to Tasks and Helpers sit at the top of the list (the decided doors, not a fourth place).
- Requests from helpers fold into one closed row ("From helpers · n") under the owner's own. Three or more
  helper updates in a day become one digest row. Decision buttons have equal weight: neither is filled
  and only the one that loses something is red (this also changes the chat's approval card).
- Later on an update (H, or the button) hides it for three hours, in this browser only, and a closed
  row "Waiting for later" brings it back. Done lists the sites allowed for good, each with Remove.
- Keys: J and K (and the arrows) move, Esc goes back, E clears an update, H says Later, Z undoes the last
  clear. An approval has no key.
- Not built: the catch-up card for a pile (it needs a server rule for what "finished or stopped" means),
  the reason line on each item (no field carries it), "Waiting for later" for offers (the server holds
  those), U to mark unread (no route), and the "Helpers ask me for" setting (see Safety below).

Settings (`lib/settingsGroups.ts`, `components/settings/`):
- The six groups in the phone's order: Work, AI and money, Chats and devices, This browser, Safety, Your
  data. Spending moved under AI and money; Folders under Safety. Lite has no Chat row.
- A search box over every row (`SettingsFind.tsx`, `lib/settingsSearch.ts`) matches the name, the group
  and the line under each row; Enter opens the page; nothing found offers the Vault.
- The Chat page (`/settings/chat`): Enter sends, the work pane choice (automatic, card only, never;
  `lib/paneMode.ts`, honoured by the chat), and every key the app answers, by place. Ctrl or Cmd and `/`
  opens it. The composer's own "Change" link and this switch write the same setting.
- Not built: the Work page between the row and Tasks (the rows link straight to Tasks and Helpers),
  "Let HQ name chats" (no auto-title), and "Helpers ask me for" (it needs a server setting; the board marks
  it "Needs build" and the web has no route for it).

HQ Hub (`crates/hq-desktop/src/menu.rs`):
- The title already follows the phone ("HQ" or "HQ · HQ on your phone", `window::main_title`).
- The menu bar is data and is tested: File (New note, New chat), Go (Vault, Chat, Inbox on Cmd 1, 2, 3,
  and the palette), View (Work pane). A menu item with a key takes the key before the page does, so each one
  opens a place or sends the page the event its own key sends (`hq:open-search`, `hq:open-quick-note`,
  `hq:new-chat`, `hq:toggle-pane`); a test reads the web sources to check the page listens for each. The
  Dock badge rule (decisions only, nothing at zero, 99+) and the one-line notice for a pile past five are
  there too.
- Not built: turning that model into the real menu and Dock badge in `apps/desktop/src-tauri`, and the
  Hub's count following the grace rule. The shell needs a Mac to build and run, and the count needs HQ's
  own answer about which helper prompts are handled by a chat. Both are the next step on a Mac; the open
  questions below still apply (menu and badge versus the no-plugin test; what closing the HQ window does).

## The suite viewers (built: read only)

- `lib/hqmd.ts` reads the parts of an hqmd note the web needs: flat frontmatter (`suite_kind`,
  `suite_fidelity`), top-level typed blocks (`sheet`, `tile`, `plot`) and level 2 headings as slides. It
  never writes, and a block it does not know stays a code block. `lib/sheetCalc.ts` reads a sheet's table,
  shows each column by its type (money as "UGX 4,200", never a stray decimal), and works out formulas of a
  bounded kind: cell references, + - * / and brackets, and SUM, COUNT, MIN, MAX, AVERAGE over a range. A
  formula it cannot read, or that loops, is shown as written and marked, never as a zero.
- `components/suite/SuiteViewer.tsx` draws a Sheet (grid with a formula bar, sticky headers, 200 rows at a
  time, a tab for each sheet), Slides (a strip, one stage, arrow keys, the `two-column` layout) and a Dashboard
  (tiles and bar, line, pie and donut charts in plain SVG). A tile that cannot be worked out says why in
  words. It is its own chunk, loaded only when such a note opens (`NoteBody.tsx`), so Chat's first load does
  not grow.
- The same viewer shows in the work pane's Result tab and on the Vault note page, for a note whose
  `suite_kind` is `sheet`, `slides` or `dashboard`. Everything else, including a doc, stays markdown.
- Editing (`lib/hqmdEdit.ts`, `components/suite/useSuiteEdit.ts`): a sheet's cells (double click, Enter or F2;
  Enter keeps, Escape leaves it, Delete clears), a new row at the end, a card's title and a card's text. Each
  edit is a patch to one place in the note: the cell's row line or the heading or card lines, with every
  other byte and the line endings as they were. It names what the caller saw and is refused if the note
  holds something else now (stale). Values must fit the column (money is stored as the plain number).
  Saves go one at a time against the hash last seen; if HQ says the note changed elsewhere, the edit is
  dropped and the latest text returns. A file kept view-only is never edited. Tiles and charts change
  through Ask HQ, as the board says.
- Present (slides at full window: arrows, Space and Page keys move, Escape leaves; nothing is saved), a
  three-row preview of a sheet on the chat's made-note card, and a wider pane (560 px, list 240) for a sheet,
  slides or dashboard are built. Settings' Tasks and Helpers rows now show live state lines ("12 open · 2
  waiting on you", "3 working here · none need you"). The pane overlay below 1400 px had been applied at every
  width and floated the pane over the thread; it is now limited to narrow windows.
- Not built: the folding of the chat list to a strip, a range that points into another note (it says so), sidecar sheets (`data=`; the layout
  is fixed in suite milestone M4), and filters (`where`, `group`, reserved for M5). The formula reader is
  the web's own and small; when the suite's Rust calculator is exposed, the web should ask it instead.

## Next slices, in order

1. Asking the suite's own calculator for formulas, once it is exposed.
2. On a Mac: the menu, the Dock badge and the Hub's grace-rule count, from the model above.
3. The catch-up card and the Work page, once the server rules they need exist.

## Open questions that block parts of the above

- Is the claude.ai connector retired? `docs/plans/web-matches-phone/plan.md` says so, and this repo's
  overview still documents it. Settings shows a "Claude on the web" row only if it is not.
- Does HQ Hub's helper count and notice follow `agent_host.driver_grace_minutes`, as the Inbox does?
- Can the Tauri shell add a menu and a Dock badge without breaking its no-plugin test
  (`crates/hq-desktop/tests/shell_config.rs`)?

## Gates for every slice

`bun test`, `bun run typecheck` and `bun run build` in `apps/hq-web`; `bun scripts/strings/build.ts
--check` when a string is added; Chat's first load stays under 300 KB gzipped; check at 390, 834, 1280,
1440 and 2200 px and at 200 percent zoom. `src/lib/signInSafety.test.ts` has one test that fails in a
full run on a clean checkout ("a 401 from the exchange ..."); it passes alone and is not part of this work.
