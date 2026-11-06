# The web looks like the phone: one plan for the web app and HQ Hub

The UX run (`docs/plans/ux-run/`) gave the web app and HQ Hub the phone's words and behaviour, but
not its look. The target look already exists: page 2 of the HQ Web & Desktop design canvas,
"Aligned with mobile" (46 boards, owner's canvas, private). This plan builds those boards on the
code that is there today. It is a visual and structural run: no new features, and almost no server
work.

## Where things stand (audited 6 Nov 2026 against `main` a892a04)

- `apps/hq-web/src/styles/aligned-ui.css` is loaded (`routes/__root.tsx:33`), but it is an older
  copy of the canvas stylesheet: it misses about a dozen rules (listed in item 1).
- The shell never renders the canvas's root, `.a-app > .a-shell`. Every rule in the stylesheet that
  changes with width is a container query on `.a-app`, so panes, side sheets, the raised Chat
  button and the 320 px menu are dead everywhere except `ConnectionSheet.tsx:70` and
  `SignInPrompt.tsx:240`.
- 70 of the stylesheet's 136 `a-*` classes are used by no screen. The shell, Chat, Notes, Tasks,
  Helpers, Inbox and Settings still render the older `hq-*` classes, Tailwind and inline styles from
  `apps/hq-web/app.css`: an unlabelled icon rail, a floating glass bottom pill, Inter and Nunito
  type, glass and blur, a tree sidebar in Notes, a full-screen drawer for a task.
- HQ Hub's control window (`apps/desktop/ui`) is closer: tabs exist, but there is no header with
  the mark and status line, and its lists use `a-row` where the boards use `a-set`/`a-li`.

## Which source wins

1. The owner's decisions made after the boards: `docs/plans/ux-run/decisions.md`,
   `resolutions.md` and `progress.md`. Concretely, the claude.ai connector is retired (W34), so
   there is no "Claude on the web" row or page; HQ Hub's first tab is "This computer" (W44), not
   the boards' "This PC"; a helper can start on a task (WR17), so the boards' "For a task is gone"
   note is out of date.
2. The UX run's specs and the string catalogue (`apps/strings/en.yaml`) for every word and
   behaviour. This run changes markup and style, not sentences, unless a board names a string the
   catalogue lacks.
3. The boards for layout, components, spacing, type, colour and states at each width.

Owner questions this plan needs answered are at the end; nothing in waves A and B waits on them.

## Rules for every item

- Build from `aligned-ui.css` classes. A board-only class (`an-*`, `tk-*`, `bd-*`, `tl-*`, the hub's
  inline styles) moves into a stylesheet beside the place that uses it, never inline.
- Delete the `hq-*` rule a screen stops using in the same change; no dead CSS left behind.
- Colours only from the stylesheet's tokens (CLAUDE.md colour discipline).
- Widths to check on every item: 390, 834, 1280, 1440, 2200, 320 px and 200 % zoom, plus Lite and
  the HQ window inside HQ Hub.
- Each item's acceptance checks become `bun test` tests that fail on today's code (render to static
  markup and assert the classes and structure, as `helpersWave4.test.ts` does), plus a browser check
  at the widths above.
- Gates per PR: web `bun test`, typecheck and build; hub UI `bun test` and typecheck when
  `apps/desktop` changes; the catalogue build when a string is added. Chat's first load stays under
  300 KB gzipped.

## Wave A: the foundation (blocks everything else)

1. **Sync the stylesheet.** Bring `styles/aligned-ui.css` level with the canvas copy:
   - `.a-mark` (the HQ mark, given a real path under `public/icons`);
   - Clear Sans Bold Italic;
   - `.a-top`/`.a-tt` `height:auto`, so the status line wraps;
   - `.a-top-more` and its `max-width:359px` container rule;
   - `.a-tab em` counts and `em.need`;
   - `.a-panes.no-list .a-pane-detail`;
   - the ink on-states of `.a-chip`, `.a-sw` and `.a-chk`;
   - `.a-field.is-error .a-input`;
   - `.a-banner.work`.

   `apps/desktop/scripts/build-ui.ts` copies this file into the hub, so one sync serves both windows.
   Test: a check that every class the canvas stylesheet defines is defined in the web copy.
2. **Clear Sans and the MTS grounds.** Body type `var(--font-sans)`; drop the Inter and Nunito
   imports (`app.css:4-10, 31-32`). Remove the glass:
   - `.hq-ambient-bg` and `hq-glass-pane` (`__root.tsx:354, 423`);
   - the blur on the rail and bar (`app.css:1055, 1258`);
   - the retired tokens (`--accent-red` #ff4444, `--text-primary` #e8e8f0, `--text-dim`).

   Raised means one surface lighter (surface-000, 100, 200), never a shadow.
3. **The shell root.** `__root.tsx` renders `.a-app > .a-shell` with `.a-skip`, `.a-top` and `.a-main`.
   - **The top bar** (`.a-top`) holds `.a-tt` (the place title as one 18/24 bold style, then the
     status line) and `.a-top-actions`: `.a-find` shaped like a field from 1024 with the placeholder
     and Ctrl K, a search icon below that, the balance chip, and the gear (`.a-top-gear`) below 768
     only. Below 360 these fold into `.a-top-more`.
   - **The rail** (`.a-rail` from 768) has `.a-rail-item` per place: icon in `.a-pill` plus its
     label, in the order Notes, Tasks, Chat, Helpers, Inbox. Chat is raised (`.a-rail-chat`) with
     the 48 px `.a-hqb .a-mark`, and Settings sits at the foot after `.a-rail-gap`.
   - **The bottom bar** (`.a-nav` below 768) is flush: `.a-nav-item`s, Chat raised at 56 px with
     the mark, and the Inbox count as an amber `.a-badge`.
   - **Removals:**
     - "HQ Vault" in the top bar (`__root.tsx:378`);
     - the overlay chat and its Cmd-H (`__root.tsx:112, 278-288`);
     - the hard-coded "Agent HQ home" and "Settings" labels (`AppRail.tsx:37`, `__root.tsx:405`);
     - the viewport-media shell rules (`app.css:1046-1356`).
   - The status line gets the real working-helpers count (`__root.tsx:385` passes 0 today), so
     "Connected · 2 working" shows.
   - Tests: the rail renders five labelled places plus Settings with Chat raised; the bottom bar
     renders the same five; the top bar has no "HQ Vault"; the status line shows the working count.
4. **The pane scaffold.** One shared component that every place uses:
   - **Structure:** `.a-panes` with `.a-pane-list` (360), `.a-pane-detail` (reading column 760
     through `.a-wrap`) and an optional `.a-pane-side` (320). Each pane has an `.a-pane-head`.
   - **Below 1024:** one pane at a time, with `.a-back`.
   - **1024 to 1399:** list and detail, with details in a side sheet opened by `.a-side-toggle`.
   - **From 1400:** the side panel docks.
   - **At 2200:** the reading column stays 760.
   - **Adding things:** Start a helper, New note and New task are an `.a-fab` below 768 and a
     button in the list head above it.
   - **Tests:** the pane rules at each width, through the container query, using a fixed-width
     `.a-app`.
5. **One sheet and one dialog.** `a-sheet` (side from 768, centred where the board says, bottom with
   `.a-handle` below 768) and `a-dlg`, each as a component with Esc, focus trap and return focus.
   Every hand-rolled overlay moves onto them later in its place's item. Rebuild the shared ones now:
   `components/tasks/verification/Sheet.tsx` (fixes the sweep and audit sheets too),
   `QuickNoteOverlay.tsx` and `TaskFormModal.tsx`.

Try it after wave A: every place sits in the new shell with labelled navigation and the phone's
type and colours, even before its own screen is rebuilt.

## Wave B: the places, one PR each (B1 to B7 can be built side by side)

**B1 · Chat** (AlignedChat, Chat2, ChatBlocks, ChatSheets, Shell frames 1 and 3)

- **Chats list:**
  - The list (`components/chat/ThreadSidebar.tsx:40`) becomes the 360 `.a-pane-list` from 1024 and
    a bottom sheet (120 px down) below that.
  - Rows are `.a-row` with `.a-row-trail` for the time and `.a-unread`. "Working" is blue with
    `.a-spin`, not sky.
  - Day groups are `.a-grp`.
- **The open chat:**
  - The chat head (`ChatHeader.tsx:19`) becomes `.a-pane-head` with `.a-back` and the info button.
  - The Details side panel replaces `SessionsPanel` (`ChatView.tsx:340`). It holds Watching cards
    (`.a-card`, `.a-state`, `.a-drow`, `.a-sw`) and This chat (Rename, Archive chat). It opens even
    when nothing is watched.
- **Thread and composer:**
  - The thread is `.a-thread` at 760 (`MessageList.tsx:60` is 896 today). Messages are
    `.a-u`/`.a-a`/`.a-meta` with no avatars, no glass and no inline colours (`ChatBubble.tsx:69-99`).
    "What I did" is `.a-tool`.
  - The composer is `.a-comp`, `.a-comp-in`, `.a-comp-field` and `.a-send` (`ChatComposer.tsx:113-170`).
    Stop is the grey `.a-send.stop`. File chips are `.a-att`.
  - The plus opens Add to this chat: an `.a-menu` from 768, the four-tile sheet below. It holds the
    note and task pickers.
- **Start screen:** `.a-display` type, the three suggestions, Get started as an `.a-card`, and three
  Recent rows once it is done (`StartScreen.tsx:16-41`).
- **Blocks:**
  - Tables are `.a-table` with the size bar ("8 rows · 5 columns", Copy, Expand), a pinned first
    column at 390, and the expanded sheet (`MarkdownViewer.tsx:111`).
  - New note and New task are cards; read notes are chips ("+N more", "Opening…", "not found").
  - A file card. Checklist done items are ink with a check (`ChatBubble.tsx:142` is emerald today).
  - These cards and chips need strings added to the catalogue, and the tool results mapped to
    blocks. That mapping is web work: the tool calls already stream.
- **Sheets:** the note sheet is `.a-sheet.side` at 440 from 768 and a bottom sheet 96 px down below
  it (`VaultNoteDrawer.tsx` is 672 px at every width). Touch gets a long-press sheet for message
  actions. Export gets its "Making your PDF…" sheet with Cancel.
- **Notices:** the away banner and the drop overlay use `.a-banner`/`.a-stale`, not amber and
  emerald Tailwind (`ChatView.tsx:326, 373`).

**B2 · Notes** (AlignedNotes, Notes2, NotesNote, NotesNote2)

- **Home** is a centred `.an-home` up to 1080 (`routes/notes/index.tsx:53-104`):
  - the search pill, folder chips, Pinned as `.an-pin` cards scrolling sideways below 768, a
    helper's suggestion as the dashed `.an-sug` card, and Recent as `.a-row`s with the blue folder
    trail;
  - the pane head "Your notes" with New note and its key hint;
  - the empty state "No notes yet" with "Ask HQ to write one";
  - "In Progress Tasks" and "System Activity" leave Notes home.
- **Folders:**
  - The tree sidebar (`routes/notes.tsx:160-271`, `VaultSidebar.tsx`, `VaultTree.tsx`) is deleted.
  - A folder is the list pane: a breadcrumb trail, "Folders" rows with counts, "Notes, A to Z",
    "See all 26" past 25, and "New note here". It replaces the emoji tile grid (`notes/$.tsx:70-98`).
- **A note:**
  - The bar (`notes/$.tsx:348-466`) becomes `.an-bar` chips: Edit, Ask HQ, Make a task, Pin and
    Export, plus More. Copy link by id, Copy note text and text size sit in More, and Ctrl plus and
    minus go back to the browser.
  - The text is `.an-md` at 720.
  - About this note moves into `.a-pane-side` from 1400, and a sheet below that
    (`notes/$.tsx:479` renders it under the note today).
  - The gone page is `.a-empty` (`n.$id.tsx`).
- **Editor:** Save and its status move into the pane head, with the format bar (`.an-fmt`) and the
  tinted marks. The changed-elsewhere card keeps its behaviour in the `.a-card` look.
- **States:** `.a-skel` and `.a-loading` instead of `animate-pulse` glass. The outbox shows on home.
  Tables scroll inside `.an-tblk` at 320. A print stylesheet.
- **Words to fix:**
  - "Convert to task" becomes "Make a task".
  - "Copy path" in the note bar becomes "Copy link".
  - "Recent Project Notes", "Vault Explorer" and "Press ⌘/" go.
  - "Your notes", "Folders", "Notes, A to Z" and "See all" are added to the catalogue.

**B3 · Tasks** (AlignedTasks, Tasks2, Tasks3, Tasks4, TasksViews)

- **Layout:**
  - Panes replace the sidebar tree plus the full-screen drawer (`routes/tasks.tsx:563-737`,
    `TaskDetailDrawer.tsx:116`). A task is the detail pane at `/tasks/$id`.
  - Dates, links and history fold into the side pane.
  - The pane head has `.a-seg` for List, Board and Timeline.
- **Rows and groups:**
  - Rows are `.a-trow` with a real `.a-chk`, `.a-prog` "1 of 3 sub-tasks done" and the meta line,
    instead of `hq-card` cards (`TaskCard.tsx:100-160`).
  - Today groups as Waiting on you, In progress, Due or starting today, then Done today folded,
    using foldable `.a-grp` (`TaskListView.tsx` groups by initiative only).
  - `.a-tabs` for Today, Upcoming and All, with counts. `.a-chips` with "Ready to check (n)".
  - The list head has `.a-search` "Find in tasks" and Collapse all. Stale and long-running are grey
    `.a-kind` chips with icons.
- **On a task:**
  - The check card sits on top as an `.a-card`: Send back on the left, Mark done on the right,
    earlier checks folded.
  - Decisions, checkpoint and relations sit under the facts as `.a-card`/`.a-drow`.
  - An archived task is read-only with Restore. Delete for good is only in its menu, as a dialog.
- **Adding tasks:** a quick add ("What needs doing?", Add task) with Add details opening the full
  sheet form. Names and tags are chips. The form's title is "New task", not "New Task".
- **Lists filter:** a sheet of `.a-rcard`s, with New area and New list typed in place
  (`TasksSidebar.tsx`).
- **Pick several:** the tab row becomes the action bar, with an `.a-menu` and an `.a-snack` Undo
  (`BulkActionBar.tsx`).
- **States:**
  - "Getting your tasks" with two skeleton rows, and the empty text per tab ("Nothing for today",
    "Nothing coming up"). Late is amber with an icon.
  - Below 768: an `.a-fab` New task and "Show as" in the list menu.
  - Lite hides "Working on it now" and Check it.
  - "How checking is going" is its own detail page (`.a-set`).
- **Board and Timeline:**
  - Board columns are `.bd-col` at 272 with a warn edge on Stuck and Ready to check. Cards are
    links with a Move menu and an Undo. There is no emerald drop target (`TaskBoardView.tsx:76-153`).
  - The board snaps one column at a time on a phone, with jump chips.
  - Timeline is `.tl-grid` with a fixed names column. Week shows two weeks from this Monday
    (`gantt.ts:57` sizes it to the tasks today). Each bar names its state.

**B4 · Helpers** (AlignedHelpers, Helpers2)

- **Layout:** panes with `.has-side`: list 360, the helper page 760, details 320. The list head
  says "5 helpers", with a compact Start a helper and a More menu (`routes/helpers.tsx:94-165`).
- **The list:**
  - Groups are `.a-grp` with `.a-count`. Filter chips are `.a-chips`, with the count in `em`.
  - Each helper is an `.a-card` (`.needs` when it waits). The head has the title link and an
    `.a-state`, then "Claude · on your Mac mini · 40 min", then the "Right now" line
    (`SessionRow.tsx:61-92`).
  - One More menu holds Show archived, Archive finished and stopped, and Stop every helper.
- **A helper:**
  - The details panel is `.a-drow` rows: Helper, Computer, Folder, Started, Last seen, Task, and the
    chat following it (`SessionInspector.tsx`).
  - The answer card sits in the page, not in a composer strip (`BlockedCallout.tsx:80`).
  - The screen uses `.a-term`, `.a-keys` and `.a-key`. "Typing to the helper" is an `.a-sw`.
    Errors are amber, not `--accent-red`. The aria labels say "helper".
- **Sheets:** Start a helper and Stop every helper move onto the sheet component. The purpose is
  `.a-chip`s, not a select; Keep and Stop sit side by side.
- **States:** the saved state uses `SavedStrip`, and the empty state per chip is `.a-empty`.

**B5 · Inbox** (AlignedInbox, Inbox2)

- **Layout:** panes replace the column and the slide-over drawer (`routes/inbox.tsx:175-356`,
  `NotificationDetailDrawer.tsx`). The list head has `.a-tabs` with counts, `.a-search` and Refresh.
  `/inbox/:id` is the detail pane, with its answer footer fixed at the bottom.
- **Needs you:** each item is an `.a-card.needs` with `.a-av`, then who, where and age, then the
  `.a-quote`, then "In HQ's words", then the actions with Decline on the left. Details are `.a-drow`
  rows (Asked by, When, Waits until). `ApprovalCard.tsx` already does most of this; the plain notice
  cards in `NotificationCard.tsx` follow it.
- **Updates:** `.a-msg` rows under day `.a-grp` headings, with the chips All, Tasks, Helpers and HQ.
  Telling an item's kind may need a field from HQ: check `value_items` first.
- **Caught up:** "You're all caught up" with Checked at, plus "You decided today". This replaces the
  hard-coded English at `inbox.tsx:343-352`.
- **A new sign-in:** "It was me" and a danger "Sign it out" that asks in a dialog.

**B6 · Settings and Spending** (AlignedSettings, Settings2, SettingsMore, SettingsMore2, Spending,
Spending2)

- **Settings home:** panes, with the four groups as the list pane and an `.a-grp` heading for each.
  Every `.a-li` row has its leading icon (`SettingsHome.tsx`).
- **Pages:**
  - AI service and keys, Folders and Safety become `.a-set`/`.a-li` lists instead of one card per
    item. Safety's view switch is `.a-tabs`.
  - Other devices: the QR beside "One-time code" and the code in `.a-mono`. Devices are an `.a-set`
    with an amber "New", and a row's menu is an `.a-menu`.
  - The key sheet goes on the sheet component, with `a-field` wrapping the input.
- **Loading and failure:** `.a-skel` on first load, and no "Load failed" while it is still loading
  (`settings.tsx:173`).
- **Spending:**
  - Limits are an `.a-set` of `.a-li` rows, each with "Each day", a menu and an 8 px `.a-prog`.
  - "What used the most" is two bar cards in the chart palette.
  - Skeletons while loading.
  - The limit sheet sends the "Warn me at" value, not a fixed 80.

**B7 · Search and the full-page covers** (AlignedSearch, Search2, FirstRun, FirstRun2)

- **Search:**
  - The field is `.a-search`, rows are `.a-row`/`.a-trow`, and the needs-you pill is `.a-state`
    (`SearchOverlay.tsx:305, 437, 445`).
  - Group heads are upper-case `.a-grp`.
  - No `shadow-2xl`, and container queries instead of `md:`.
  - A 300 ms debounce when the phone is the hub.
- **Covers and sign-in:**
  - `/setup`, `/connect`, sign-in and the other full-page covers render inside `.a-app`
    (`__root.tsx:333-342`).
  - The sign-in page is centred at 640, with `.a-field`/`.a-input`/`.a-btn` instead of monospace
    Tailwind (`SignInPrompt.tsx:8-11, 264`). The shared-computer row is `.a-chk`.
  - The gear and the places are hidden while signed out.

## Wave C: HQ Hub (AlignedHub, Hub2, Hub3, HubDevices, HubDevices2, HubHelpers, HubHelpers2,
HubHelperDetail, HubHelperDetail2)

Builds on wave A item 1. Desktop UI only (`apps/desktop/ui`), no Rust.

1. **Header and lists:** an `.a-top` with the mark, "HQ Hub" and its status line above the tabs,
   inside `.a-main > .a-wrap` (`index.html:11-16`).
   - Groups are `.a-grp`, with a count where a group counts something.
   - Lists are `.a-set`/`.a-li`. `a-row-hit` goes, since it is undefined in the stylesheet.
   - `#paste-path` becomes an `a-input` inside an `a-field`. `.hub-need` text is `--surface-000`.
2. **This computer:**
   - a "Starting" step card with progress ("Step 3 of 4 · 12 s"; check whether `host_status`
     reports steps, else it needs a field from the host);
   - each at-work row opens `/helpers/:id` in the HQ window through `open_main_window`, which
     exists;
   - Stop HQ is red only inside its dialog.
3. **Devices:**
   - the pairing card with the QR beside the link;
   - the six digits as an `.a-card.needs` with Decline and Pair;
   - "Paired devices" and "Shared with paired devices" as `.a-set` lists, with a quiet Remove;
   - technical details folded;
   - the share-model checkbox becomes an `.a-sw`.
4. **Helpers and one helper:**
   - the group count counts only helpers waiting for an OK, not setup states (`main.ts:~818`);
   - one helper's page has "All helpers" as `.a-back`, its facts as `.a-drow` (Version, Program,
     Last tried), connections as an `.a-set`, and technical details folded.

## Wave D: the HQ window and the edges

1. **"HQ on your phone" in the HQ window.** When HQ Hub's proxy serves a paired hub, the page says
   so in its status line and Settings, as the boards draw it. HQ Hub already sends
   `x-hq-window: hub`. Add the hub's kind (this computer or a paired phone) the same way: desktop
   and web, no HQ server change.
2. **Lite and edition checks** for every place (B1 to B7): only Notes, Tasks and Inbox; Ask HQ, pin
   suggestions, "by HQ", Export and Check it hidden where the boards hide them.
3. **320 px, 200 % zoom, 2200 and print** across every place, from the boards' width frames.
4. **Retire `app.css`** down to what the boards still need (tokens the stylesheet lacks, the
   markdown reader), and delete the `hq-*` classes no screen uses any more.

## Order and size

- **Wave A** is one person, about four PRs (stylesheet and type, shell, panes, sheets). Nothing else
  starts before the shell and panes merge.
- **Wave B** is seven PRs that touch separate files, so they can go to fleet workers side by side
  with exhaustive briefs (the board text, this plan's item, the files and line numbers above).
  Claude owns the shared components and reviews every diff.
- **Wave C** can run beside wave B.
- **Wave D** comes last.

## Owner decisions (6 Nov 2026)

1. Get started keeps the code's steps ("Open HQ on your phone", "Messaging apps (optional)"); the
   Shell board's list is older.
2. The search placeholder stays "Search notes, tasks, chats and helpers".
3. The Helpers list keeps collapse (as `.a-side-toggle`); the drag to resize goes.
4. Share this computer's model: an `.a-li` with an `.a-sw` under "Shared with paired devices" (the
   recommendation; not objected to).
5. Dark only for this run; the light values stay defined and unused.
