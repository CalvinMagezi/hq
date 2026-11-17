# Web restructure: build plan

Status: slice 1 built (Vault location bar, folder tree, Previous and Next). The rest are proposals.

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
- Below 1024 px the tree is not shown and neither is its button. The bar's Up replaces the page's back
  arrow. A bottom or side sheet for the tree is slice 2.

The old test "the tree sidebar is gone" was changed on purpose: it recorded the earlier decision, which
the owner reversed. The sidebar files it names (`VaultSidebar`, `VaultTree`, `useSidebarResize`) stay
deleted; the tree is the one component above.

## Next slices, in order

2. Vault navigation, rest: Back and Forward buttons with a history that knows when they are dimmed, the
   chevron menu on a crumb that lists the folders beside it, the tree as a sheet below 1024 px, the
   narrow-width location bar (a back arrow naming the parent), a "not in the Vault any more" page, and
   "Show system folders".
3. Chat grammar: the live line, the result line and the fold, the attention slot, hover actions with
   Save to Vault and Make a task, source chips, the composer's own controls and Stop.
4. The work pane with the Context and Result tabs (Context first: it needs no new data, the chat
   socket already sends each tool call's name and arguments), then Doc, then the other viewers as the
   suite milestones land.
5. The palette, then the trigger menu (`@`, `/`, `#`) in the composer.
6. Inbox: the split view, the helper fold and the digest row, Waiting for later.
7. Settings: the six groups and search, the Chat page, the Work page.
8. HQ Hub window: menu bar, Dock badge, the three-state control window.

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
