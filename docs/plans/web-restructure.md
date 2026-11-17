# Web restructure: build plan

Status: slices 1 and 2 built (Vault navigation), Chat's grammar and the work pane built. Move and rename wait on the server. The rest are proposals.

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

- Move and rename, drag and drop, and Move to... The server has no route for them (it creates, updates,
  trashes, restores and links notes). They need a server route, a rule for whether links to a moved note
  are rewritten, and an Undo that restores the old place. Decide the link rule first.
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

## Next slices, in order

1. The Sheet, Slides and Dashboard viewers in the pane, as the suite milestones land.
2. The palette, then the trigger menu (`@`, `/`, `#`) in the composer.
3. Inbox: the split view, the helper fold and the digest row, Waiting for later.
4. Settings: the six groups and search, the Chat page, the Work page.
5. HQ Hub window: menu bar, Dock badge, the three-state control window.

## Open questions that block parts of the above

- Can the web move or rename notes and folders? `vaultApi.ts` has create, update, delete, restore and
  trash, and no move or rename. Drag and drop and "Move to..." need server work first, and a decision on
  whether links to a moved note are rewritten.
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
