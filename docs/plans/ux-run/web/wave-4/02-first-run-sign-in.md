# Web wave 4 · 02 · First run, Connect Claude and signing in (M16, W34, WR8, WR9, WB1)

Boards: **AlignedFirstRun** (`/setup`, the first AI key; `/connect`, Claude's consent),
**AlignedFirstRun2** (signing in: the status line, the Sign in sheet, the full page, the shared computer, a
new sign-in, narrow and other frames), **AlignedFlow** ("Covers with no shell"), AlignedReview M16, W32,
W34, WB1. Follows `../../rules-web.md` sections 2, 3 and 6 and `../../rules-shared.md` sections 4 and 5.
Where this spec and the rules differ, the rules win.

`../wave-1/05-sign-in-safety.md` built the sheet's safety parts, device names and the new sign-in item.
This spec builds the covers and the signed-out state in the new shell.

## 1. Finding and the review items it closes

- **M16.** The no-sign-in card gives `hq env set openrouter`, not `OPENROUTER_API_KEY`.
- **Phone words on `/setup`:** Check key, "Checking…", "The service accepted this key.", "You can also
  continue without checking." Base URL and Model name are not on the web, and the spec says so (WR9).
- **W32 on `/connect`:** Decline and Allow, Decline on the left (today Deny and Approve).
- **W34, M4.** The connector lives under Settings, Your accounts, as the row "Claude on the web";
  "connector" shows only in Technical details (`../../resolutions.md` B14).
- **M2.** The chosen card has an ink edge; the warning card uses the amber card class, no inline colour.
- **WB1, WR8.** Signed out shows only Sign in; the shell with an empty view and the Sign in sheet; a full
  page only for a browser that never signed in.
- **M1.** "Signed out of this HQ" in amber, never "Signed out · Saved · 2 h".
- **M14.** `/vault#code=` links from QR codes keep signing in.
- **Drift 13, 14.** Raw "unauthorized" with a Retry under the sign-in banner; Setup and Connect use a
  small mono upper-case title.

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/hq-web/src/routes/setup.tsx:42-43` title "Connect a model" (mono, upper case) and "HQ needs one
  model key to start. You can add more providers or replace this key later in Settings. It is stored on
  this server and never shown again."; `:26` "The provider rejected that key."; `:32` "Setup failed.";
  `lib/setupApi.ts:3` providers `open_router`, `anthropic`, `google`; `:37` one call checks with the
  provider and saves only if it works (the save call already stands alone, `:34`).
- `apps/hq-web/src/routes/connect.tsx:60` "This page opens when you press Connect in Claude."; `:91` "HQ is
  not accepting new connections. Open Settings, choose Allow a new connection, then press Connect in
  Claude again."; `:124` "What Claude may do"; `:166` "Deny" (and "Approve").
- `apps/hq-web/src/routes/__root.tsx:96` `SetupRedirect` sends to `/setup` while no key is set.
- `apps/hq-web/src/components/SignInPrompt.tsx` a banner over every screen when the server answers 401;
  Home under it shows "Could not load pinned: unauthorized" with Retry.
- `apps/hq-web/src/lib/hqAuth.ts:54-62` `#code=` read and dropped; `:11-24` `#token=` likewise.
- `docs/CLAUDE_CONNECTOR.md` the connector (off by default, needs a public machine).

## 3. Decision

**Three full-screen covers with no shell, each with one main action that returns to the start place:
Sign in (`/signin`) when HQ refused the sign-in or this browser signed out, Set up HQ (`/setup`) until an
AI service is connected, Connect Claude (`/connect`) while a request is open. Signed out is a connection
state in the shell, not a banner over every screen.**

Rejected:

- **One button that checks and saves together** (today). An owner whose provider is slow to answer cannot
  get in; WR9 splits Check key and Open HQ, and allows opening without checking.
- **Show Welcome, Restore or Start HQ on the web.** HQ is already running wherever the browser reaches it.

## 4. Screens and states

### `/setup`: the first AI key (First run 1)

Shown instead of the shell while HQ has no AI key, before anything else can be reached. The web has one
step of the phone's first run, Your AI model.

- **F1 · Empty.** Title (display) "Set up HQ"; heading "Your AI model"; "HQ needs a key from an AI service
  to think. It is kept on your HQ and never shown again." "Service" as three choice cards (`.a-rcard`):
  "OpenRouter · Reaches many AI models with one key." (chosen, ink edge), "Anthropic · Claude, straight
  from Anthropic.", "Google AI · Gemini, straight from Google." Field "Key" with the help "Get a key at
  openrouter.ai/keys." [Check key] (secondary) and "You can also continue without checking." [Open HQ]
  (green). "You can add more services or replace this key later in Settings, AI service and keys."
- **F2 · Checking.** "Checking…", fields off.
- **F3 · Accepted.** Under the button, ink with a check: "The service accepted this key."
- **F4 · Refused.** Under the field, amber with an icon: "The service refused this key. Check that you
  copied all of it." Other answers in plain words from the phone's list: nothing answered there, out of
  credit, too busy; never a status code ("The service answered 401." becomes the same words).
- **F5 · No sign-in on this HQ.** "This HQ can't take a key from a browser" / "It has no sign-in yet, so a
  key typed here could be read on its way. Run this on the computer HQ runs on, paste the key when asked,
  then restart HQ:" / `hq env set openrouter` / "This page opens HQ by itself once a key is there." (`hq
  env set openrouter` reads the key from one line, hidden at a terminal, and tests the default model.)
- **Rules:** Check key asks the service and shows its answer under the button. Open HQ saves the key and
  opens Chat, checked or not. No Base URL or Model name (the phone's "Other OpenAI-compatible" has them;
  the web's `/setup` has only its three services; other backends live in the settings file). Lite serves no
  `/setup` and no Settings (`docs/HQ_LITE.md`), so this page never shows there. Elsewhere Chat opens next,
  with Get started: Let HQ reach you, Open HQ on your phone, Messaging apps (optional)
  (`../../resolutions.md` B15).
- **Widths:** 320 to 2200 one centred column, 560 wide at most (Recommended); 200% zoom the same column.

### `/connect`: Claude asks to use this HQ (First run 2)

Still shipped (off by default, needs a public machine). The desktop hub retired its Connect Claude screen;
Claude Code and Claude Desktop on a laptop reach HQ through HQ Hub instead. The web keeps this page and
Settings, Your accounts, Claude on the web (W34, owner to confirm).

- **K1 · The request.** Title "Connect Claude to this HQ"; "Claude on claude.ai is asking to use your HQ.
  Allow it only if you pressed Connect in Claude yourself, just now." Facts: "Asked at · 14:27 today",
  "Allowing sends you back to · claude.ai", "This request is open until · 14:37". "What Claude may do":
  "Read only · Search and read notes, tasks and what helpers did. Claude cannot change anything."
  (chosen unless HQ was set to another default, ink edge), "Tasks only · Read and change tasks, and
  nothing else. No notes, no helpers.", "Everything · Every HQ tool: change notes and tasks, and start
  helpers on your computers. Text Claude reads on a web page or in an email could steer these." Field
  "Name for this connection". "What HQ's tools return becomes part of your Claude chat, which Anthropic
  keeps under its own terms. You can disconnect it any time in Settings, Your accounts, Claude on the
  web."
  [Decline] (left) [Allow] (right, the one primary). Esc means Decline.
- **K2 · Not taking new connections.** "HQ is not taking new connections" / "Open Settings, Your accounts,
  Claude on the web. Choose Allow a new connection, then press Connect in Claude again." [Open Claude on
  the web] (B14).
- **K3 · Run out.** "This request has run out" / "It ran out, was already answered, or was started in
  another browser. Press Connect in Claude again, from this browser."
- **K4 · Signed out.** "Sign in to HQ first" / "This browser is not signed in to your HQ. Sign in, then
  press Connect in Claude again." [Sign in].
- **K5 · No request.** "This page opens when you press Connect in Claude."

### Signed out, in the shell (First run 2, section 1)

- **S1 · Signed out of this HQ.** Shell with the rail; top bar "Notes" / status line "Signed out of this
  HQ" (amber dot) / [Sign in]; no balance chip. The place shows "Sign in to see your notes" / "HQ no longer
  accepts this browser's sign-in, so nothing from it is kept here." [Sign in]. Everything saved was wiped
  (query cache, the worker's cache, drafts, the outbox, the push subscription). No place shows
  "unauthorized" or a Retry that cannot work.
- **S2 · The Sign in sheet.** One-time code first, the phishing line under it, the login link second, the
  last line for everyone else (`../wave-1/05-sign-in-safety.md` S1). Centred from 768, a bottom sheet on a
  phone. Wrong code under the field; the sheet stays open.
- **S3 · Signed out by HQ.** When a saved sign-in stops working, the sheet opens once by itself and leads
  with "This browser was signed out, or HQ no longer accepts its sign-in."
- **S4 · From a QR code.** `/vault#code=` (kept, M14): "Signing you in · Using the code from the QR code.
  This takes a moment." then Chat. The code leaves the address at once.
- **S5 · A browser that never signed in.** The full page `/signin` (WR8): "This browser has not opened your
  HQ before. Sign it in once and it stays signed in, unless it is a shared computer."
- **S6 · The new sign-in item and the code screen** as `../wave-1/05-sign-in-safety.md` N1 to N4, D1.
- **Widths.** 320: the sheet fills the width, the code field and Use code stack, the state wraps under the
  title (a 640 px window at 200% looks the same). 768 to 1023 and 2200: centred, 640 wide; at 2200 the rest
  of the window stays empty, since nothing is shown while signed out. Lite: the same; once signed in the
  status line says "Connected · HQ Lite" and HQ opens on Notes. HQ Hub's HQ window: serves this computer's HQ or HQ on
  your phone; sign-ins and saved copies are kept by the HQ's id (`../wave-3/02-devices.md` W2).
- **Retired here:** "Signed out · Saved · 2 h", the saved notes behind the sheet, "This device", the
  off-grey dot for signed out.

## 5. Exact copy

| Key | String |
|-----|--------|
| `setup_title` | "Set up HQ" |
| `setup_heading` | "Your AI model" |
| `setup_line` | "HQ needs a key from an AI service to think. It is kept on your HQ and never shown again." |
| `setup_service` | "Service" · "OpenRouter" "Reaches many AI models with one key." · "Anthropic" "Claude, straight from Anthropic." · "Google AI" "Gemini, straight from Google." |
| `setup_key` | "Key" · "Get a key at openrouter.ai/keys." (per service: its own key page) · placeholder "Paste your key" |
| `setup_check` | "Check key" · "Checking…" · "The service accepted this key." · "You can also continue without checking." |
| `setup_refused` | "The service refused this key. Check that you copied all of it." |
| `setup_open` | "Open HQ" |
| `setup_later` | "You can add more services or replace this key later in Settings, AI service and keys." |
| `setup_no_signin` | "This HQ can't take a key from a browser" · "It has no sign-in yet, so a key typed here could be read on its way. Run this on the computer HQ runs on, paste the key when asked, then restart HQ:" · "hq env set openrouter" · "This page opens HQ by itself once a key is there." |
| `connect_*` | K1 to K5 as written |
| `signed_out_place` | "Sign in to see your {notes, chats, tasks}" / "Sign in to see what needs you" · "HQ no longer accepts this browser's sign-in, so nothing from it is kept here." · "Sign in" |

Retired: "Connect a model", "HQ needs one model key to start…", "The provider rejected that key.",
"Setup failed.", "Deny", "Approve", "OPENROUTER_API_KEY" on screen, "Claude connector" outside
Technical details.

## 6. Behaviour rules

1. **Covers have no shell** and one main action; each returns to the start place (Chat, Lite Notes).
2. **`/setup` shows only while HQ has no AI key** (`/api/setup/status`); a signed-out browser gets Sign in
   first; a server with no web sign-in gets F5.
3. **Check key** calls `POST /api/setup/test`; **Open HQ** calls `POST /api/setup/provider` (save), checked
   or not (WR9). Both need the client header (they do today).
4. **`/connect`:** Decline left, Allow right; Allow is the one primary; Esc means Decline; the chosen level
   card has an ink edge; Read only is chosen unless HQ was set to another default.
5. **Signed out** is the status line's first state; the shell stays; places show the empty view; the sheet
   opens from Sign in, or once by itself after HQ refuses a saved sign-in.
6. **The full page** `/signin` only for a browser with no stored sign-in for this HQ id (WR8).
7. **Sign-in fragments** are read before any redirect (`../wave-2/01-route-table.md` rule 2).

## 7. Browser and platform notes

- Password managers may offer to save the key field; mark it `autocomplete="off"` and `type="password"`
  with a show button (the key shows as dots).
- A phone camera opening `/vault#code=` uses the phone's default browser, not the HQ app.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/hq-web/src/routes/setup.tsx`, `lib/setupApi.ts` | F1 to F5; two calls (26-43). | yes |
| `apps/hq-web/src/routes/connect.tsx` | K1 to K5; Decline and Allow (60-166). | yes |
| `apps/hq-web/src/routes/signin.tsx` | The full page. | NEW |
| `apps/hq-web/src/components/SignInPrompt.tsx` | The sheet in the new shell. | yes |
| `apps/hq-web/src/routes/__root.tsx` | `SetupRedirect` (96) and the signed-out state through the Connection module. | yes |

## 9. Server, desktop or release changes

None new (the setup routes exist; the sign-in pieces are in wave 1). Docs: `docs/CLAUDE_CONNECTOR.md` uses
"Decline" and "Allow".

## 10. Acceptance checks (each fails on today's build)

1. `/setup` has separate Check key and Open HQ buttons; Open HQ saves without a check. Today one button
   checks and saves.
2. `/setup` with no web sign-in shows `hq env set openrouter`. Today the page has no such state.
3. `/connect` renders Decline left and Allow right; Esc declines. Today "Deny" and "Approve".
4. A 401 shows the shell with "Signed out of this HQ" and the empty view, with no cached content and no
   "unauthorized". Today a banner sits over cached screens.
5. A browser with nothing stored for this HQ opens `/signin`. Today it gets the banner.
6. No text on `/connect` outside Technical details contains "connector"; K1 and K2 name "Claude on the
   web" (B14).

## 11. Check on a laptop or phone

- A fresh HQ with no key: `/setup`; check a wrong key, then open HQ without checking.
- A second browser profile: `/signin`; sign in with a code from Settings › Other devices.

## 12. Open decisions it depends on

- **W34** (owner to confirm), **WR8, WR9** (Recommended), `../../resolutions.md` B14, B15.
- Depends on: `../wave-1/05-sign-in-safety.md`, `../wave-0/02-shared-parts.md`, `01-chat.md` (Get
  started).
