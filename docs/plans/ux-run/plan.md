# The UX run: one plan for phone, web and the desktop hub

Five waves. Each wave has a phone track and a web and hub track that can be built side by side; the
shared parts in a wave come first because both tracks use them. Finish a wave, pass the gates, hand it
to the owner to try (`testing.md`), fix what they find, then start the next.

Every item links to its spec. Build exactly what the spec says, as corrected by `resolutions.md` and
`decisions.md`.

## Wave 0: safety, live bugs and shared parts

Why first: four live problems are risks today, and every later wave reuses the parts built here.

**Web and hub, first:**

1. `web/wave-0/01-live-bugs.md`:
   - web push without content or buttons;
   - the hub proxy refusing a port it did not open;
   - wiping the browser on a refused sign-in;
   - the HQ client header on the drive, goal and answer routes.

   Ship each one as soon as its test passes.
2. `web/wave-0/02-shared-parts.md`:
   - the web's Connection state;
   - the saved-copy store keyed by HQ id, with its wipe rules;
   - the outbox with client ids;
   - counters;
   - the shared string catalogue on the web side.

**Phone:**

3. `mobile/wave-0/01-lock-screen.md`: nothing private in notifications by default, and the five channels created once.
4. `mobile/wave-0/03-data-transfer.md`: data-extraction rules and the moved-phone flow (needs N4).
5. `mobile/wave-0/02-safe-mode.md`: "Save a copy of my notes" when HQ cannot start, the idle gate, and the new channel-file fields (needs N6).
6. `mobile/wave-0/04-measure.md`: "What HQ noticed".
7. The test setup from resolutions D-11: Compose UI tests, a screenshot test, a lint-checks module.

**Shared, server side (`changes.md`):**

- `/api/status/ai`.
- The starting flag.
- Dropping repeated client ids.
- The busy route.
- `hq vault moved`.
- The one string catalogue (N19) with the hard-coded-string check.
- The Connection data model and failure lines for both apps (`data/Connection`, `ConnectionWords`, resolutions A2).

**Owner decisions for this wave:** N4, N6, N19.
**Owner checkpoint:** the four live bugs are gone on a real laptop and phone.

## Wave 1: reach every owner, and one set of rules everywhere

**Phone:**

1. `mobile/wave-1/01-notifications.md`: the notifications step, the upgrader sheet, and Turn on in three places.
2. `mobile/wave-1/03-destructive.md`: one confirm and Undo, the danger colour, and losing approvals.
3. `mobile/wave-1/04-plain-words.md`: plain words and the rendered-text word check.
4. `mobile/wave-1/02-backup.md`: a truthful backup row and Save a copy now.
5. `mobile/wave-1/05-progress.md`: one way to count.

**Web:**

6. `web/wave-1/01-shell-status.md`: the status line, colours and the balance chip.
7. `web/wave-1/02-dialogs-approvals.md`: dialogs, approvals that quote the tool and arguments, and the losing flag.
8. `web/wave-1/03-undo.md`: Undo restored on HQ.
9. `web/wave-1/04-plain-words.md`.
10. `web/wave-1/05-sign-in-safety.md`: device names, expiry, the phishing line, and new sign-ins in Needs you.
11. `web/wave-1/06-settings-groups.md`: the phone's four groups, "This browser", and the card slot.

**Shared, server side:**

- The `loses` flag on approvals.
- Restore routes for every Undo.
- Device sign-in fields and expiry.
- The new-sign-in Needs you item.
- Backup and update status routes.

**Owner decisions for this wave:** W34 (keep the claude.ai connector on the web), N25 (the phone's top bar order).

## Wave 2: stay current without burning data, and one route table

**Phone:**

1. `mobile/wave-2/01-downloads.md`: the download engine, the paid-data and paid-Wi-Fi asks, resume with a hash check, and Undo on cancel.
2. `mobile/wave-2/02-updates.md`: updates kept on disk, installed only when idle, staged and withdrawable; the signed channel fields and promote re-signing.
3. `mobile/wave-2/03-assistant-model.md`: the speech model on the same engine.
4. `mobile/wave-2/04-startup.md`: the Connection sheet's screen, built once in `ui/connection/` (resolutions D-5).

**Web:**

5. `web/wave-2/01-route-table.md`: one route table, redirects for stored links and QR codes, the manifest name and shortcuts.
6. `web/wave-2/02-embed-and-lite.md`: `?embed=1` alongside `?host=android`, and Lite.
7. `web/wave-2/03-version-and-data.md`: "A newer HQ is running", the data budget, change events over the socket, and readers loaded on demand.
8. `web/wave-2/04-time-zones.md`.

**Shared, server side:** times as instants, which edition is running, the minimum-client header, and change events over the socket.

**Owner checkpoint:** a large download on paid data, and an update installed on the phone.

## Wave 3: an assistant people find and trust, and HQ Hub

**Phone** (only after the owner logs the locked Home hold on the A32 and a Pixel image; `mobile/device-checks.md`):

1. `mobile/wave-3/01-assistant-setup.md`.
2. `mobile/wave-3/02-assistant-locked.md`: Pass, Fail A or Fail B decides what ships (N21).
3. `mobile/wave-3/03-assistant-mic.md`.
4. `mobile/wave-3/04-assistant-handoff.md`: needs the `source: voice` and `client_id` fields and `waiting_for_owner`.

**Desktop hub:**

5. `web/wave-3/05-commands-and-platforms.md`: build the new commands and capability entries first.
6. `web/wave-3/01-this-pc.md`.
7. `web/wave-3/02-devices.md`.
8. `web/wave-3/03-helpers-tab.md`.
9. `web/wave-3/04-helper-detail.md`.

**Owner decisions for this wave:** N22 (screen on for 30 s while thinking), W44 ("This PC"), WR22 (the bundle rename to AgentHQ), W50 (whether Windows and Linux hub builds join this run).

## Wave 4: one app, one way

**Phone:**

1. `mobile/wave-4/01-offline.md`: the Connection pattern in every place, saved copies and server HQ rows.
2. `mobile/wave-4/02-search.md`: one search, folder filter on HQ, a Helpers scope.
3. `mobile/wave-4/03-web-pages.md`: web Home retired, While you were away native.

**Web, the place redesigns on the new shell, in this order:**

4. `web/wave-4/01-chat.md` (with blocks and sheets).
5. `web/wave-4/02-first-run-sign-in.md`.
6. `web/wave-4/03-notes.md` (note conflicts and Keep both).
7. `web/wave-4/04-search.md`.
8. `web/wave-4/05-inbox.md`.
9. `web/wave-4/06-tasks.md` (views and checking).
10. `web/wave-4/07-helpers.md` (needs WR18 for "Typing to the helper").
11. `web/wave-4/08-settings.md`.
12. `web/wave-4/09-spending.md`.

**Shared, server side:**

- `folder` on `/api/search`.
- Note base version and 409.
- Resume a reply by message id, and Stop over HTTP.
- The task field on the start route.

**Owner decisions for this wave:** WR18.

## After the run

- Bring the design canvases in line with `resolutions.md` (they are the visual reference; the specs win).
- The items marked "later" in `resolutions.md` C-15 and `decisions.md` (D18, D19, D21, W50 if deferred).
- The marketing site's own findings are separate (the "HQ Website Redesign" canvas, its Drift board).

## Working rules for every item

- One commit per spec item. Push to `ccr-66a72c4c-52kq29` after each item that passes the gates.
- Tick it in `progress.md`, with what to try on the device.
- Never skip or disable a test.
- Never open a pull request or merge until the owner asks.
- If a spec is wrong or impossible, stop and say why with evidence before changing it.
