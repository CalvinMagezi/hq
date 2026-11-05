# Web app and HQ Hub: the handover

This folder is the web and desktop half of the UX run. It tells a coding agent on a laptop, with no
access to the design canvases, how to build every change to the web app (`apps/hq-web`,
`crates/hq-web`) and the desktop hub (`apps/desktop`, `crates/hq-desktop`).

Read first, in this order:

1. `../resolutions.md`: the last word on every conflict between the phone, web and hub boards. It wins
   over everything below.
2. `../decisions.md`: every decision and its status.
3. `../rules-shared.md` and `../rules-web.md`: one set of rules (`resolutions.md` A1). `rules-web.md`
   adds only what a browser or the hub needs; where the two differ and `resolutions.md` is silent,
   `rules-shared.md` wins.
4. `../changes.md`: the server, desktop and release work each wave depends on.
5. This file, then the specs in wave order.

Source of truth: page 2 ("Aligned with mobile") of the "HQ Web & Desktop" canvas, final board files of
5 Nov 2026: AlignedWebRules, AlignedReview (blockers WB1 to WB7, majors M1 to M16), aligned-ui.css, the
41 Aligned* screen boards, and Drift (the as-built findings, page 1). Page 1 ("As built, 5 Nov 2026")
shows today's behaviour.

## 1. Today's web app and hub, in brief

Verified at `e5f9d5f`.

### The web app (`apps/hq-web`)

A static single-page app (TanStack Start in SPA mode, React 19, Vite, Tailwind v4, zustand, React
Query), served by the Rust server, which is its only backend: REST under `/api`, chat and live updates
over `/ws`. `apps/hq-web/README.md` describes it (and still calls it "HQ Control Center").

| Route file | Address | What it is today |
|------------|---------|------------------|
| `src/routes/index.tsx` | `/` | Redirects to `/vault` (line 5). |
| `src/routes/vault.tsx`, `vault/index.tsx`, `vault/$.tsx` | `/vault`, `/vault/$path` | Home (Get started, briefing, Pinned, Recent, In Progress Tasks, System Activity) and the note page. |
| `src/routes/chat.tsx` | `/chat` (`?thread=`) | `ChatView`: thread list, messages, composer, Watching panel. |
| `src/routes/tasks.tsx` | `/tasks` (`?view=`, `?task=`) | List, Board, Timeline, Verification bar, While you were away briefing, task drawer. |
| `src/routes/sessions.tsx` | `/sessions` (`?id=`, `?task=`) | The Workbench: coding agents, live terminal, answer panel, Stop all. |
| `src/routes/usage.tsx` | `/usage` | Usage, budgets and forecast. |
| `src/routes/notifications.tsx` | `/notifications` | "Inbox & Approvals": Pending, All Activity, Resolved History, push toggle. |
| `src/routes/settings.tsx` | `/settings` (`#section`) | Grouped rows with a section nav from 1024 px (`NAV` at line 437). |
| `src/routes/setup.tsx` | `/setup` | First model key. |
| `src/routes/connect.tsx` | `/connect` | claude.ai connector consent (Deny and Approve). |

**The shell** is in `src/routes/__root.tsx`:

- `AppRail` (`src/components/AppRail.tsx`): Home (`/vault`), Chat, Tasks, Workbench (`/sessions`, with
  a badge), Usage, Inbox (`/notifications`, with a badge), Settings.
- `BottomNav` (`src/components/BottomNav.tsx`, phone): Home, Chat, Tasks, Workbench, Inbox. No Usage.
- A header row (`__root.tsx:295` on): `HeaderClock`, a search button ("Search vault (Cmd+/)"), a chat
  button labelled "Overlay" ("Open chat (Cmd+K)"), `CopilotChip` (the balance or budget chip), a gear,
  and a dot with "live", "saved" or "offline" (`__root.tsx:369`).
- Banners above the header: `InstallPrompt`, `SignInPrompt`, `ReachabilityBanner` (which assumes
  Tailscale), a system notice and a new-version prompt (`__root.tsx:249-292`).
- Overlays: `SearchOverlay` (Ctrl or Cmd /), `QuickNoteOverlay` (Ctrl or Cmd N), `GlobalChatOverlay`
  (Ctrl or Cmd K or H).
- Embedded mode: `?host=android` hides the chrome (`src/lib/embedHost.ts`, kept for the tab's life).
- Offline: React Query persisted to IndexedDB for 7 days (`src/lib/offlineCache.ts`), two zustand
  stores in IndexedDB (`src/store/hqStore.ts`, `src/store/threadStore.ts`), chat drafts in
  localStorage (`src/components/chat/draftStore.ts`), and the service worker's `hq-api` cache
  (`src/sw.ts`). Sign-in is a web token in localStorage (`src/lib/hqAuth.ts`, key `hq-web-token`).
- Styling: `apps/hq-web/app.css` (dark glass, Inter first, `--text-dim #6b6b8a`).

### The server side (`crates/hq-web`)

Routes in `crates/hq-web/src/lib.rs`. Relevant modules: `web_push/` (push), `dialog_answer.rs`
(answer tokens), `origin.rs` (origin guard and the `x-hq-client` header), `device_auth.rs` and
`device_auth/devices.rs` (codes and device sign-ins, stored by `crates/hq-db/src/web_auth.rs`),
`notifications_api.rs`, `vault_api.rs`, `threads_api.rs`, `tasks_api.rs`, `sessions_api.rs`,
`workbench_api.rs`, `settings_api.rs`, `budgets_api.rs`, `profile_gate.rs` (what Lite serves),
`edge_worker_api.rs`, `verification_api.rs`.

### The desktop hub (`apps/desktop`, `crates/hq-desktop`)

- `apps/desktop` is a Tauri 2 shell in its own Cargo workspace (`src-tauri`), outside the root
  workspace. `tauri.conf.json`: `productName` "HQ", identifier `online.agenthq.desktop`, bundle target
  `app` (macOS only), a strict CSP, no plugins.
- Two windows, both titled "HQ" (`src-tauri/src/main.rs:90` and `:174`): the control window (940 by
  700, minimum 640 by 480) with tabs Host, Devices and Claude (`ui/index.html`, `ui/src/main.ts`,
  `ui/style.css`, plain TypeScript, no framework), and the HQ window (1200 by 800), which loads the web
  app through a loopback proxy.
- Commands: `src-tauri/src/commands.rs`, listed again in `src-tauri/build.rs` and
  `src-tauri/capabilities/control.json`. Only the control window has a capability.
  `crates/hq-desktop/tests/shell_config.rs` fails when the three lists differ or a plugin appears.
- Logic in `crates/hq-desktop/src/`: `desktop.rs` (wiring, `upstream_addr`), `host.rs` (start and stop,
  `HostSpec::for_config`), `pairing.rs` (over `hq-bridge`), `proxy.rs`, `claude.rs`, `agents.rs`,
  `window.rs` (navigation allow-lists), `settings.rs` (`~/.hq/desktop/settings.json`).
- Docs: `docs/DESKTOP.md`, `docs/security/DESKTOP.md`.

## 2. Running them on a laptop

Commands from `CLAUDE.md`, `apps/hq-web/README.md`, `docs/DESKTOP.md` and `../testing.md`. Where they
differ, `../testing.md` is the one to follow.

### HQ itself

```bash
sudo ./scripts/install-hq.sh --link   # once per machine
./scripts/install-hq.sh               # build, install, restart the daemon, verify
./scripts/install-hq.sh --check       # content hashes: is anything stale? (never compare --version)
hq web --json                         # starts the web server detached, prints JSON with login_url
hq web status                         # is it running
hq web stop
hq web link                           # a new login link if the old one is lost
```

Open `login_url` in the browser. It carries `#token=`, which the page stores and drops from the address.

### The web app while you change it

```bash
cd apps/hq-web
bun install
bun run dev          # Vite on :4747, proxies /api and /ws to hq on :5678
bun run build        # dist/client, served by hq (web_static_dir, or web/dist next to the vault)
bun test
```

Run `hq web --json` (or `hq start all`) first so something answers on :5678. Sign in to the dev server
with the same `#token=` link, pointed at `http://localhost:4747/`. To reach the dev server from a phone
on the same network, add its address to `web_allowed_origins` in `~/.hq/config.yaml`.

### The desktop hub

```bash
cd apps/desktop && bun install && bun run build:ui && bun run tauri dev   # a dev window
cd apps/desktop && bun run build          # the app: tauri build --bundles app; unsigned HQ.app
cd apps/desktop && bun install && bun run typecheck
cargo test -p hq-desktop                  # includes shell_config.rs (commands, capabilities, no plugins)
```

See `docs/DESKTOP.md` for pairing and the HQ window.

`bun run build` writes `target/release/bundle/macos/HQ.app` under the Cargo target directory. On a
tight disk build with `CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0`. The real `hq bridge join` test
is opt-in: `cargo build -p hq-cli` then `HQ_BRIDGE_BIN=target/debug/hq cargo test -p hq-desktop --test
cli_join -- --ignored`. The hub builds for macOS only today (W50: Windows and Linux later).

### The gates before a PR

Exactly as `../testing.md` "Before every hand-over":

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace            # read the totals: a partial run is not a green run
cd apps/hq-web && bun install && bun run build && bun test
cd apps/android && ./gradlew :app:assembleDebug :app:testDebugUnitTest :app:lintDebug
cd apps/desktop && bun install && bun run typecheck     # when the hub changed
cargo test -p hq-desktop          # includes shell_config.rs (commands, capabilities, no plugins)
```

Then write the wave's "Try this now" note in `../progress.md`. Shared-computer and signed-out checks use
a second browser profile, signed in with a one-time code from Settings › Other devices.

## 3. Design system reference (from `aligned-ui.css`)

One design system: the MTS tokens as the phone ships them (`HqTheme.kt`, object `Mts`), and the
phone's components one to one. Class stems follow the phone's `native-ui.css` (`n-top`, `n-row` and so
on) with an `a-` prefix. Layout responds to the width of `.a-app` (a CSS container), not the window, so
a 390 px frame renders the phone layout.

**How to use it in the code:**

- **React (`apps/hq-web`):** add `src/styles/aligned-ui.css` (NEW), copied from the canvas's
  stylesheet without the board chrome (`.ab*` classes) and with the `@font-face` URLs pointing at
  bundled fonts (`public/fonts/ClearSans-*.woff2`, NEW). Import it from `__root.tsx` and retire
  `app.css` place by place. Wrap the app in `<div class="a-app">`, which is the container the
  breakpoints read. Build small components that emit these classes (`Shell`, `TopBar`, `StatusLine`,
  `Rail`, `BottomBar`, `Panes`, `Row`, `Card`, `StatePill`, `Dialog`, `Sheet`, `Toast`, `Field`,
  `Switch`, `Chip`, `Tabs`, `Segmented`, `Composer`), in `src/components/ui/` (NEW). Tailwind can
  stay for layout glue, but colour comes only from the tokens.
- **Tauri control window (`apps/desktop/ui`):** the same stylesheet, copied into `ui/aligned-ui.css`
  by `apps/desktop/scripts/build-ui.ts` (the CSP allows only `'self'` for styles and fonts, so the fonts are bundled
  into `ui/dist/fonts/`). The window is plain TypeScript: write the markup with the same classes. The
  control window then looks like the HQ window (Drift desktop 10).

### Tokens

| Token | Value | Use |
|-------|-------|-----|
| `--surface-000` | #000000 | App ground, top bar, lists, panes |
| `--surface-100` | #231F20 | Rail, bottom bar, cards, sheets, settings sets |
| `--surface-200` | #322E2F | Selected row and pill, tonal button, dialog, your chat bubble |
| `--ink` | #FFFFFF | Text and icons; done, paired, met, within a limit |
| `--ink-muted` | #D0D2D3 | Grey: fine. Secondary text, meta, status line, the Connected dot. Replaces `#6b6b8a` |
| `--line` | #8A8687 | Control borders (3:1 on 000 and 100) |
| `--hairline` | #4A4647 | Dividers and card outlines only, never a control border or text |
| `--brand-green` | #22FF37 | The one primary action that loses nothing; selected place icon; tab underline; brand |
| `--brand-blue`, `--text-blue`, `--focus-ring` | #00ADEE | Working, links, quiet buttons, unread dot, progress, focus ring |
| `--brand-indigo` | #2E3191 | Blocks with white text; the swipe-to-tomorrow ground |
| `--brand-sky` | #00C6FA | Large display words and the second chart series only |
| `--warn` (derived, the phone's `Mts.Amber`) | #FFB300 | Needs you, waiting, stopped, late, failure lines, field errors, near or over a limit |
| `--danger` (MTS danger-text) | #FF6B6B | Only the fill of a destroying button, black text 7.6:1 |
| `--danger-hover` (derived) | #FF8F8F | Hover of that button |
| `--mark-ground` | #0A0A0F | Behind the HQ mark only |
| `--snack-action` | #00668F | Undo on the inverted toast (6.4:1 on ink) |
| `--scrim` | rgba(0,0,0,.62) | Behind sheets and dialogs |
| `--on-green`, `--on-blue` | #000000 | Text on green and blue fills |
| `--action-border`, `--action-hover` | #22FF37, #7DFF8B | Primary button border and hover |
| `--font-sans` | "Clear Sans", system-ui, -apple-system, "Segoe UI", sans-serif | All text |
| `--font-mono` | ui-monospace, "JetBrains Mono", Menlo, Consolas, monospace | Terminal and quoted commands only |
| `--space-1` to `--space-8` | 4, 8, 12, 16, 24, 32, 48, 64 px | 16 page gutter on a phone, 24 from 768, 48 touch target, 64 top bar |
| `--radius-sm`, `-md`, `-row`, `-lg`, `-xl`, `-pill` | 6, 12, 16, 20, 28, 9999 px | toast; card and field; list row; bubble; sheet and dialog; buttons and chips |
| `--target` | 48 px | Touch target; nothing interactive under 44 |
| `--rail-w`, `--top-h`, `--nav-h`, `--list-w`, `--side-w` | 88, 64, 96, 360, 320 px | Shell sizes |

Light theme values exist under `[data-theme="light"]` (surfaces #F9F9F9, #FFFFFF, #F1F2F2; ink #000000;
muted #555555; line #767676; hairline #D0D2D3; text-blue #00668F; text-green #006B1A; danger #C4001A;
warn #8A5A00) but are not drawn and wait on the owner (W3: dark default, light later).

Contrast (from Foundations): ink 21.0, 16.3, 13.4 on 000, 100, 200; ink-muted 13.8, 10.7, 8.8; line 5.8,
4.5, 3.7; hairline fails for text (2.3 on 000); green 15.4, 12.0, 9.8; blue 8.2, 6.4, 5.2; warn 11.7,
9.1, 7.5; danger 7.6, 5.9, 4.8; the retired `#6b6b8a` is 4.1 on 000 and fails.

Retired colours: `--text-dim`, neutral-500, `#ff4444`, rose, purple, cyan, emerald. Each maps to
ink-muted, warn or brand-blue. Pins on boards use #FFB300, not #FFB547.

### Type

| Class | Size | Use (HqType name) |
|-------|------|-------------------|
| `.a-display` | 32/38 Bold | First run and full-screen covers only (display-md) |
| `.a-h1` | 28/34 Bold | Empty and error states |
| `.a-h2` | 22/28 Bold | Sheet and dialog titles (H2) |
| `.a-h3`, `.a-title` | 18/24 Bold | Top bar title, pane titles (ScreenTitle), the one title style |
| `.a-body` | 16/24 | Reading text, chat, notes (Body) |
| `.a-body-b` | 16/24 Bold | Row and card titles (BodyBold) |
| `.a-body-sm`, `.a-body-sm-b` | 14/20 | Meta lines; Bold for buttons and links |
| `.a-cap` | 12/16 | Status line, times; nav labels in Bold (Caption) |
| `.a-label` | 13/16 Bold, upper case, .06em | Group headings (Group) |
| `.a-mono` | 13/18 mono | Terminal and quoted commands only |

Helpers: `.a-muted`, `.a-warn-t`, `.a-danger-t` (amber after alignment), `.a-ok-t` (ink after
alignment), `.a-link` (blue, bold, underlined), `.a-kbd` (a key cap), `.a-sr` (screen reader only),
`.a-skip` (skip link, visible on focus), `.a-ic` (24 px icon; `.sm` 20, `.xs` 16), `.a-mark` (the HQ
mark on `--mark-ground`).

### Components, phone to web

| Phone (Compose) | Phone board class | Web class | What to build |
|-----------------|-------------------|-----------|---------------|
| HqShell | `.np` | `.a-app > .a-shell` | `Shell`: grid with areas top, main, nav; rail from 768. |
| TopBar | `.n-top .n-tt` | `.a-top .a-tt .a-title .a-status` | `TopBar`: title, status line (a button that opens the connection sheet), Search, balance chip, place actions, gear (phone only). Grows to fit a wrapped state. |
| status dot | `.n-tt i u` | `.a-dot` (grey), `.work` (blue), `.warn` (amber), `.off` | Part of `StatusLine`; never the only signal. |
| BalancePill | `.n-bal` (`.am .rd`) | `.a-bal` (`.warn .over`) | `BalanceChip`: 48 tall button, 32 tall pill, line border, max 156 px; only while Connected. |
| IconButton Search | `.n-ib` | `.a-find` | `SearchButton`: an icon below 1024, a field-shaped button `min(360px, 32cqi)` with Ctrl K from 1024. |
| IconButton Gear | `.n-ib` | `.a-ib.a-top-gear`, rail `.a-rail-item` | In the top bar below 768, at the foot of the rail above. |
| (320 px menu) | none | `.a-top-more` | `TopBarMore`: below 360 px holds Search, Spending and Settings. |
| BottomBar, PlaceIcon | `.n-nav a .pl` | `.a-nav .a-nav-item .a-pill` | `BottomBar`: below 768 only, 96 tall, five places. |
| (rail) | none | `.a-rail .a-rail-item .a-pill`, `.a-rail-gap` | `Rail`: 88 wide, the bar on its side, same order and labels, gear at the foot. |
| RaisedChat | `.n-nav a.hq .hqb` | `.a-nav-chat .a-hqb .a-mark`, `.a-rail-chat` | The Chat mark: 48 in the rail, 56 raised on the bar; green ring when Chat is open. |
| Inbox badge | `.n-nav .bg` | `.a-badge` | Amber, black text, counts Needs you, 99+ past 99. |
| list row | `.n-row` (`.sel`), `.n-av` | `.a-row` (`.is-sel`, `aria-current`), `.a-row-main`, `.a-row-trail`, `.a-av`, `.a-unread` | `Row`: a real link; buttons beside it, never inside. |
| group heading | `.n-grp`, `.n-lbl`, `.n-sech` | `.a-grp .a-count`, `.a-label` | One class for all three. |
| settings group | `.n-set .n-li` | `.a-set .a-li .a-li-main .a-li-trail` | `SettingsGroup`, `SettingsRow`. |
| detail row | `.n-drow` | `.a-drow` | A fact: label left, value right. |
| Inbox row | `.n-msg` | `.a-msg .a-msg-main .a-msg-time` | Two-line clamp. |
| TaskRow | `.n-trow .n-chk` | `.a-trow .a-trow-main .a-trow-title .a-trow-meta .a-chk .a-prog`, `.a-trow-act` | Swipe becomes the check box and a row menu; "2 of 3 sub-tasks done" on a blue bar. |
| HelperCardView, StatePill | `.n-card`, `.n-state` | `.a-card` (`.needs` amber edge, `.warn`), `.a-card-head`, `.a-card-actions`, `.a-state` (`.needs .offline .working .ready`) | `HelperCard`, `StatePill`: one colour per state, always with words. |
| WorkingBar, Spinner | none | `.a-work`, `.a-spin` (`.lg`) | Blue; still under reduced motion. |
| filter chips | `.n-fchip` (`.on`) | `.a-chips .a-chip` (`aria-pressed`), `em` count | On is ink. |
| kind tag | `.n-kind`, `.n-ro` | `.a-kind`, `.a-kind.sm`, `.a-kind.warn` | Tags such as "Claude", "Mac mini", "Read only", "T-12". |
| tabs | `.n-tabs .n-tab` | `.a-tabs .a-tab` (`aria-selected`), `em` count, `.fit` | Split one place's contents; `.fit` packs left from 1024. |
| (view switch) | none | `.a-seg .a-seg-item` | Tasks List, Board, Timeline. Web only. |
| PrimaryButton | `.n-btn` | `.a-btn` | One per view, green, black text. |
| OutlineButton | `.n-btn.o` | `.a-btn.secondary` | Line border, ink. |
| tonal button | `.n-btn.t` | `.a-btn.tonal` | surface-200. |
| blue text action | `.n-link` | `.a-btn.quiet` | text-blue, no border. |
| DangerText action | TextButton in DangerText | `.a-btn.danger` (fill), `.a-btn.quiet.danger` (ink) | Red fill only on the button that destroys. |
| size and width | none | `.a-btn.compact` (44), `.a-btn.block`, `.a-btns` | |
| why it is off | none | `.a-why` | An amber icon and a line: never only the 50% dim. |
| Start a helper, New note | `.n-fab` | `.a-fab`; from 1024 an `.a-btn` in `.a-pane-head` | No floating button on desktop. |
| BasicTextField | inline styles | `.a-field .a-field-label .a-input .a-field-msg`, `.is-error` | Error amber with an icon. |
| notes search pill | `.pill52` | `.a-search` | 52 tall pill. |
| Switch | `.n-sw` | `.a-sw` (`role="switch"`, `aria-checked`) | On is ink. |
| check box | none | `.a-chk` with `u` | 48 target, 22 box; on is ink. |
| choice card | `.n-rcard .n-radio` | `.a-rcard .a-radio` | Chosen has an ink edge. |
| menu | none | `.a-menu .a-menu-item .a-menu-sep` | surface-200, line border. |
| SheetFrame | `.n-sheet .n-handle .n-scrim` | `.a-sheet` (`.side`, `.center`), `.a-handle`, `.a-sheet-head .a-sheet-body .a-sheet-foot`, `.a-scrim` | Bottom below 768; from 768 side (440 wide, from the right) or centred (640 wide). |
| AlertDialog | `.n-dlg` | `.a-dlg .a-dlg-actions` | `Dialog`: side by side, safe left; stacked rule in `../rules-web.md` 11. |
| Snack | `.n-snack` | `.a-snack .a-snack-act` | `Toast`: bottom right from 768. |
| offline banner | `.n-banner`, `.sbar` | `.a-banner` (`.work`), `.a-stale` | Always with the status line; one strip "Saved in this browser at 14:32". |
| NotBuilt, empty | `.cov` | `.a-empty .a-empty-ic` (`.warn`), `.a-loading`, `.a-skel` (`.t .s`) | |
| Bubbles | `.n-u .n-a .n-meta .n-tool` | `.a-thread .a-u .a-a .a-meta .a-tool` | Thread capped at 760 wide. |
| Composer | `.n-comp .n-field .n-send` | `.a-comp .a-comp-in .a-comp-field .a-send` (`.stop`) | `.a-comp-in` centres it from 1024. |
| attachments | `.n-att .n-thumb` | `.a-att .a-att-item .a-thumb`, `.is-error` | Max 240 wide chip. |
| terminal, keys | `.n-term .n-keys` | `.a-term .a-keys .a-key` | Keys are real buttons. |
| quoted command | `.n-quote` | `.a-quote` | |
| table | none | `.a-table`, `.a-scroll-x` | |
| (panes) | pages pushed over the shell | `.a-panes` (`.is-detail`, `.no-list`, `.has-side`), `.a-pane-list .a-pane-detail .a-pane-side .a-pane-head`, `.a-back`, `.a-wrap`, `.a-side-toggle` | A pushed page becomes the detail pane from 1024. |

### Widths

| App width | Layout |
|-----------|--------|
| under 360 | The phone layout; the top bar keeps the title and the state, the rest in `.a-top-more`. |
| under 768 | Top bar with the gear, one pane, the bottom bar with Chat raised. An open item replaces the list, with a back arrow. The details panel is a sheet. |
| 768 to 1023 | Rail with the gear at its foot. Still one pane. Sheets from the right or centred. |
| 1024 to 1399 | List 360 wide and the open item. Search becomes a field. Details is a side sheet from a Details button. |
| 1400 and wider | The details panel docks on the right, 320 wide. |
| 2200 | Nothing new appears: panes keep their widths, the reading column stays 760 (notes 720, Notes home 1080, Inbox details 720, HQ Hub 760) and centres. |
| 200% zoom | A 1280 by 800 window is 640 by 400 CSS px: the phone layout. Browser zoom is never taken over. |

## 4. Specs, by wave

The waves line up with the phone's (`../mobile/`): wave 0 safety and shared parts, wave 1 the rules
across the existing app, wave 2 routes and editions (the phone's wave 2 is downloads, updates and
startup), wave 3 the desktop hub (the phone's is the assistant), wave 4 the place redesigns (the
phone's is offline, search and web pages).

| Wave | Spec | Closes |
|------|------|--------|
| 0 | `wave-0/01-live-bugs.md`: web push content and buttons, the hub proxy and a taken port, wiping the browser on a refused sign-in, guarding drive, goal and answer | WB1, WB2, WB3, live bug 4 |
| 0 | `wave-0/02-shared-parts.md`: failure catalogue rows for the web, the Connection state model, the saved-copy store by HQ id, the outbox with client ids, counters, one string catalogue | M1, M3, M9, M10, M15, decisions C, D, E, O |
| 1 | `wave-1/01-shell-status.md`: status line and colours across today's shell | M1, M2, W03, Drift 13, 15 |
| 1 | `wave-1/02-dialogs-approvals.md`: the shared dialog, approvals with the losing flag | WB6, W32, W39, M5 dialogs |
| 1 | `wave-1/03-undo.md`: Undo with HQ restores | WB4 (Undo), M5, W23, WR7, N18 |
| 1 | `wave-1/04-plain-words.md`: plain words and names, "This browser" | M4, W09, W35, M12 words, Drift 08, 09, 14 |
| 1 | `wave-1/05-sign-in-safety.md`: sign-in safety and device names | WB1 (shared computer), M7 |
| 1 | `wave-1/06-settings-groups.md`: Settings groups, the card slot, one address per section | M5, W05, W37, Drift 05 |
| 2 | `wave-2/01-route-table.md`: one route table, redirects, stored links, QR codes, the manifest | M14, decision N |
| 2 | `wave-2/02-embed-and-lite.md`: `?embed=1` beside `?host=android`, Lite | decision N, W1, WR1 to WR4 |
| 2 | `wave-2/03-version-and-data.md`: version skew reload, the data budget | M10, M14 |
| 2 | `wave-2/04-time-zones.md`: instants, display zone | M11 |
| 3 | `wave-3/01-this-pc.md`: HQ Hub This PC | WB3, WB5, M1 to M4, M16, Drift desktop 01 to 05, 08 to 10 |
| 3 | `wave-3/02-devices.md`: HQ Hub Devices | Drift desktop 06, 11, WB3 |
| 3 | `wave-3/03-helpers-tab.md`: HQ Hub Helpers | M6, W26, W47, WB5 |
| 3 | `wave-3/04-helper-detail.md`: one helper, Try it | WB7, M6 |
| 3 | `wave-3/05-commands-and-platforms.md`: every command and capability change; Windows and Linux as future scope | D03, D07, D08, D09, Drift desktop 12, W50 |
| 4 | `wave-4/01-chat.md` (blocks and sheets included) | WB6, W15 to W17, M9, D7 |
| 4 | `wave-4/02-first-run-sign-in.md` | M16, W34, WR8, WR9 |
| 4 | `wave-4/03-notes.md` | WB4 (notes), W19, W21, W22, Drift 10, 12 |
| 4 | `wave-4/04-search.md` | D20, W27, WR12 |
| 4 | `wave-4/05-inbox.md` | WB6, W31, M5, M12 |
| 4 | `wave-4/06-tasks.md` (views and checking included) | W28 to W30, W33, WR14 to WR16, Drift 16 |
| 4 | `wave-4/07-helpers.md` | WB5, W8, W25, W26, WR17, WR18, Drift 01, 03, 04 |
| 4 | `wave-4/08-settings.md` | W34, W36, M12 settings |
| 4 | `wave-4/09-spending.md` | W2, W41, M11, M12 rate limits |
| all | `device-checks.md`: laptop, browser and desktop checks per wave | |

Every spec follows the mobile spec format: the finding and review ids it closes; Before; Decision;
Screens and states; Exact copy; Behaviour rules; Browser and platform notes; Code touchpoints; Server,
desktop or release changes; Acceptance checks; Check on a laptop or phone; Open decisions.

## 5. Decisions and conflicts

Decisions live in `../decisions.md` and the cross-platform settlements in `../resolutions.md`; this file
does not repeat them. The ids each wave depends on:

| Wave | `decisions.md` | `resolutions.md` |
|------|----------------|------------------|
| 0 | N1, N8, N19, D16, D17, W16, W20, W40, WR5, WR6, WR26, WR30 | A2, A4, A5, A6, A7, B11, B16, B19, C-11, C-12 |
| 1 | D2, D3, N17, N18, N19, W6, W16, W23, W29, W31 to W37, W39, W40, W44, WR2, WR5, WR7, WR8, WR10, WR14, WR25, WR27, WR28 | A3, A6, B1 to B5, B8 to B12, B14, B17, B18, C-4, C-13 |
| 2 | Decision N, W1, W2, WR1 to WR4, WR12, WR28 | B6, B13, B17, C-9, C-10, C-14 |
| 3 | W4, W5, W38, W39, W42 to W50, WR13, WR19 to WR25, WR29, WR31 | A7, B4, B9, B11, B12, B18, C-12 |
| 4 | D7, D13, D20, N3, N17, N18, W1, W2, W6 to W17, W19, W21 to W34, W36, W41, WR2, WR4, WR8, WR9, WR11 to WR18, WR26 to WR30 | A2, A3, A6, B1 to B8, B10, B11, B13 to B18, C-5, C-7 |

Owner to confirm before the part that depends on it: N19, W34, W44, W50, WR18, WR22 (the full list is
in `../decisions.md`). B11 was confirmed on 5 Nov 2026.

### The conflicts found while carrying the boards over

1. **Lite's gear.** Settled: resolutions.md B13 (This browser and Other devices; Notifications hidden in
   Lite unless `lite.allow_egress` lists the push service, WR2).
2. **A link to Chat or Helpers on Lite.** Settled: resolutions.md B13 (Notes with "This HQ is HQ Lite.
   It has no {place}.").
3. **The top bar at 320 px.** Settled: `../rules-web.md` sections 8 and 11 (title and state only;
   Search, the chip and the gear in one More menu). No resolution was needed.
4. **The HQ window's saved copy and outbox.** Settled: resolutions.md B11, confirmed by the owner (group
   "This window", saved copy and outbox keyed by the HQ's id, "Saved in HQ Hub at 14:32", no web push,
   HQ Hub system notices).
5. **Chosen chips, switches and check boxes.** Settled: resolutions.md A3 (ink, never green).
6. **Dialog geometry.** Settled: `../rules-web.md` section 11 (the web uses the stylesheet's `.a-dlg`;
   order, colours and words follow the rules). No resolution was needed.
7. **When a dialog pair stacks on the web.** Settled: decisions.md WR27 (stack when the two buttons do
   not fit side by side on one line; layout decides, not a zoom reading).
8. **Get started steps on the Chat start screen.** Settled: resolutions.md B15 ("Let HQ reach you",
   "Open HQ on your phone", "Messaging apps (optional)").
9. **The balance chip near a limit.** Settled: resolutions.md B5 ("Today 85%"; over the limit "Today
   104%" in amber).
10. **The saved copy's size.** Settled: resolutions.md B16 (one cap on both: 256 KB a place, 2 MB in
    all, per HQ).
11. **Undo timing.** Settled: resolutions.md A6.
12. **The HQ window's saved line.** Settled: resolutions.md B11 ("Saved in HQ Hub at 14:32", which
    replaces WR5's saved line).
13. **The sign-in code warning.** Settled: resolutions.md C-4 for the code screen ("Type this only into
    HQ on your own device. HQ never asks for it in a chat or a call."); the Sign in sheet keeps its own
    line ("Use a code only from your own signed-in device. HQ never asks for it in a chat or a call.").
14. **A pairing failure reason not in the catalogue.** Settled: resolutions.md A2 ("Pairing is off.
    Switch it on and try again.").
15. **How the web detects "text at 200%" (was OD-W1).** Settled: decisions.md WR27.
16. **Lite's `/settings/spending` (was OD-W2).** Settled: decisions.md WR28.
17. **The HQ window's Copy link and Share (was OD-W3).** Settled: decisions.md WR29.
18. **The outbox's cap (was OD-W4).** Settled: decisions.md WR30.

### Still open

All of these are now settled in `resolutions.md` section E (5 Nov 2026); build what section E says. The notes below explain how each was reached.

1. **The Stop HQ body that names the helpers.** B9 says the red dialog's body
   "names the helpers" but gives no sentence. Recommendation, used in `wave-3/01-this-pc.md` P6 and
   `wave-1/02-dialogs-approvals.md`: "Chats and scheduled tasks pause. {Codex and Claude} are working
   here and stop too. Their work so far is kept."
2. **"Open it to see." as a button.** B18 replaces "Take a look", which on the web is a button on the
   stuck-helper card. Recommendation: the button label "Open it to see" (no full stop on a button),
   used in `wave-4/05-inbox.md` N6.
3. **Claude's own menu names on Claude on the web.** B14 keeps "connector" to Technical details, but the
   setup steps quote Claude's own Settings, Connectors menu. Recommendation: quote Claude's menu names
   as Claude shows them and use HQ's own words ("Address for Claude") everywhere else, as
   `wave-4/08-settings.md` C1 does.

## 6. Code paths the boards name that do not exist

- `web_auth.rs` in `crates/hq-web`: it is `crates/hq-db/src/web_auth.rs` (device tokens; `mint` takes
  an optional expiry and a device sign-in never expires on its own, line 90).
- `lib/replyAlerts.ts` exists at `apps/hq-web/src/lib/replyAlerts.ts`; "web_push/mod.rs:227,237" match.
- `routes/__root.tsx (CopilotChip)` exists (line 113).
- `HelperWords.enterMeansOnce`, `HelperWords.canAnswerInline`, `PairingWords.kt`, `TaskSheets.kt`,
  `ChartViews`: phone (Kotlin) names, not web code.
- `workbench.ts statusInfo` exists as `apps/hq-web/src/lib/workbench.ts`.
- `lockdown.json`: written by the host (`hq host` lockdown), not a web file.
- `preflight.rs`, `category.rs`, `spec.rs`: under `crates/hq-tools/src/harness_session/`.
- `hq lite export` exists (`crates/hq-cli/src/commands/lite.rs:260`).
- `/api/status/ai`, `/api/edition`, a starting flag, restore routes for chat archive, Inbox items and
  spending limits, `open_main_window(path)`, `host_log`, `pick_program`, `host_unlock`,
  `helpers_check`, `helpers_list`, `helper_try`, `helper_try_stop`, `helper_connect`,
  `helper_disconnect`, `helper_restore`, `helper_signin`: all NEW, named in the specs as such.
