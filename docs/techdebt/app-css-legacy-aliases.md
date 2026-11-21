# What is left of the older names and hard-coded colours in `apps/hq-web/app.css` and the viewers

**Reason skipped:** The sweep moved the suite, the note viewers, the shell banner and the chat views off the older `--bg-*`, `--border`, `--accent-*`, `--text-*`, `--font-body`, `--font-heading` and `--glass-*` names onto the design file's tokens (`--surface-000`/`100`/`200`, `--hairline`, `--text-blue`, `--text-green`, `--warn`, `--danger`, `--ink`, `--ink-muted`, `--font-sans`) and deleted every alias grep showed unused. It stopped at what can be checked from the page itself; the rest needs a look at the screen it belongs to.
**Date:** 2026-11-20
**Where:** `apps/hq-web/app.css`, `apps/hq-web/src/components/SignalCard.tsx`, `apps/hq-web/src/components/HtmlViewer.tsx`, `apps/hq-web/src/components/CodeViewer.tsx`
**Remove when:** each item below is done.

Deleted with the aliases (grep across `src`, `scripts` and `public` found no user): `.md-table*`, `.docx-viewer*`, `.glass-heavy`, `.hq-row`, `.hq-field`, `.hq-card`, `.hq-btn-ghost`, `.hq-composer` (the composer's `id` of that name is not a class), `.hq-modal`, `.thinking-dot` and the `--glass-fill`, `--glass-line`, `--glass-hi`, `--glass-blur*`, `--glass-bg*` and `--glass-shadow` names. The two tests that pinned `--font-body` and `--text-dim` now check that they are gone.

Still there:

- `SignalCard.tsx` has no importer (only `shellStatus.test.tsx` reads its text). It, `.glass-card` and `.glass-tag` in `app.css` can be deleted together once that test's file list drops it.
- `--font-mono` is defined twice, in `aligned-ui.css` (system mono) and in `app.css` (JetBrains Mono first). `app.css` is loaded last, so it wins. Pick one.
- `HtmlViewer.tsx` draws a note's HTML in a fixed dark page of its own (`#0c0c16`, `rgba(255,255,255,...)`); it does not follow the light theme. `CodeViewer.tsx` keeps `bg-[#121212]` because the highlighter's `vitesse-dark` theme is dark only; a light highlighter theme would fix both.
- `.markdown-viewer pre` in `app.css` uses `!important` to win over the highlighter's inline background.
- The chat views still use Tailwind colour utilities (`text-[11px]`, `bg-black/40`, `border-white/5`) next to the tokens.
