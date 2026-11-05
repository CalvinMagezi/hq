# The UX run (November 2026)

A full UX pass across the Android app, the web app and the desktop hub (HQ Hub), designed, reviewed
adversarially twice and settled, ready to build. This folder is the whole hand-over: a building agent
needs nothing else, and the design canvases are only a visual reference.

## What the run does, in one paragraph

It makes HQ one product on three screens.
- **One design system:** the MTS design system, as the phone ships it.
- **The same five places:** Notes, Tasks, Chat (the start screen), Helpers, Inbox.
- **The same words and the same rules:** for status, failures, dialogs, approvals, Undo and colour.

It also closes the problems the owner and three rounds of reviewers found:
- notifications that leak private text;
- an assistant nobody can find;
- updates nobody sees;
- downloads that burn paid data;
- a phone that can lose its notes to a bad update;
- a web app that keeps private data after sign-out;
- a desktop hub that can send a sign-in to the wrong program;
- copy full of config words.

The owner lives in Uganda. Many owners are on paid data, older or budget phones, shared computers and
English as a second language, and every decision was weighed for them.

## Order of authority

When two documents disagree, the first one in this list wins:

1. `resolutions.md`: settles every conflict left after the final cross-platform check.
2. `decisions.md`: every decision, with its status (Recommended, Revised, Owner to confirm).
3. `rules-shared.md`, then `rules-web.md`: the rules every screen follows.
4. The specs in `mobile/` and `web/`.
5. The design canvases.

## What is in this folder

| File | What it is |
|---|---|
| `plan.md` | The five waves, phone and web side by side, in build order, with the owner checkpoints and decisions each wave needs |
| `resolutions.md`, `decisions.md` | The settled answers |
| `rules-shared.md`, `rules-web.md` | Notification channels and privacy, the failure catalogue, colour, dialogs and Undo, first run, Settings, the Connection states, words, links, Android versions, safety switches; plus what only a browser or the hub needs |
| `changes.md` | Server, desktop, release and docs changes the specs depend on, by wave |
| `mobile/README.md`, `mobile/wave-*/` | The Android app as built, how to build and test it, and one spec per fix |
| `web/README.md`, `web/wave-*/` | The web app and HQ Hub as built, how to run them, the design system as classes and tokens, and one spec per area |
| `mobile/device-checks.md`, `web/device-checks.md` | What only a real phone, laptop or browser can prove, by wave |
| `testing.md` | The gates before every hand-over, and how the owner tries each wave on a laptop and a phone |
| `progress.md` | The checklist the building agent keeps up to date, the owner's answers, and what the owner finds |

Every spec has the same sections:
- the finding and review ids it closes;
- Before (today's code, with file:line);
- the decision;
- every screen and state, with exact strings and colour tokens;
- an exact-copy table;
- numbered behaviour rules;
- Android or browser version notes;
- code touchpoints (NEW where the file does not exist yet);
- the server, desktop or release changes it needs;
- acceptance checks, each written to fail on today's code;
- device checks;
- the decisions it depends on.

## The design canvases (visual reference only)

- **HQ Mobile Design:** https://claude.ai/artifact/NHAFigk4nxaVEdkzwNV6LP.
  - Page 1: the app as built.
  - Page 2: the plan, the adversarial review, the shared rules and every fix board, by wave.
- **HQ Web & Desktop:** https://claude.ai/artifact/NYE2RxYEeLk5uMEV9sJGnm.
  - Page 1: as built.
  - Page 2: the review, the web rules and the aligned redesign, including HQ Hub.

The canvases are private to the owner. They predate `resolutions.md`, so where a board and a spec
differ, the spec wins.

## Before you start

1. Read `CLAUDE.md` at the repo root: the gates, the writing rules and what never to do.
2. Run `./scripts/install-hooks.sh`.
3. Read this file, `plan.md`, `resolutions.md`, `rules-shared.md`, `rules-web.md` and `decisions.md`.
4. Start with web wave 0, the four live bugs.

## Status

- Designed, reviewed and settled: 5 Nov 2026.
- Building: see `progress.md`.
- Owner decisions still open: N4, N6, N19, N22, N25, W34, W44, W50, WR18, WR22 (`decisions.md`).
