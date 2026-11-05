# Progress

The building agent keeps this file current: tick each spec when it is built and its gates pass, write
the wave's "Try this now" note, and record what the owner found. One line per item.

## Owner decisions to collect

From `decisions.md` "Owner to confirm": N4, N6, N19, N22, N25, W34, W44, W50, WR18, WR22.
Ask for them in one batch before the wave that needs each. Record the answer and the date here.

| Id | Answer | Date |
|---|---|---|
| B11 (resolutions.md) | Approved as recommended: "This window", saved copy and outbox keyed by HQ id, "Saved in HQ Hub at 14:32", no web push, HQ Hub system notices | 2026-11-05 |

## Web wave 0 (live bugs and shared parts)

- [ ] web/wave-0/01-live-bugs.md: web push content and buttons
- [ ] web/wave-0/01-live-bugs.md: the hub proxy and a taken port
- [ ] web/wave-0/01-live-bugs.md: wipe the browser on a refused sign-in
- [x] web/wave-0/01-live-bugs.md: guard drive, goal and answer routes. Try: nothing to see; a current web app and the phone app keep working (both send the header). A cross-site POST to `/api/harness-sessions/{id}/drive` now gets 403.
- [ ] web/wave-0/02-shared-parts.md

## Phone wave 0 (safety and shared parts)

- [ ] mobile/wave-0/01-lock-screen.md
- [ ] mobile/wave-0/02-safe-mode.md
- [ ] mobile/wave-0/03-data-transfer.md
- [ ] mobile/wave-0/04-measure.md

## Wave 1 to wave 4

Add one line per spec file as the wave starts (see `plan.md` for the order).

## Try this now

Write the note for each finished wave here.

## Found by the owner

| Date | Where | What | Fixed in |
|---|---|---|---|
