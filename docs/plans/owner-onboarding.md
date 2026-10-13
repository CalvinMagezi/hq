# Owner onboarding: from a provisioned server to a signed-in phone and a paired chat

Status: all slices (0 to 8) built. This doc maps the journey for an **owner** who has never used a
terminal. An **operator** provisions an HQ server for them on Hetzner (`docs/HETZNER.md`), and the
server joins the owner's own Tailscale tailnet. Today the owner can't sign in again or connect
Telegram or Discord without the operator's root shell. The target is that, once the operator hands
over one link, the owner never needs them again.

## The journey today

### Operator

1. Creates the server with the deploy form or `deploy/hetzner/hcloud.sh`, then waits for bootstrap
   (3 to 5 minutes).
2. Runs `ssh root@<ip> hq-join`. `tailscale up` prints a sign-in URL in the operator's terminal
   (`deploy/hetzner/hq-join.sh:31-32`). The operator sends it to the owner and waits while they
   sign in.
3. `hq-join` sets `web_allowed_origins` to `https://<fqdn>:8443`, publishes HQ with
   `tailscale serve`, restarts the service, and prints `https://<fqdn>:8443/vault#token=<hex>`
   once (`hq-join.sh:38-66`). It also clears the motd, so the link only exists in that terminal.
4. Sends the link to the owner over some side channel. The link is an admin credential.
5. Closes public SSH.

### Owner

1. Needs a Tailscale account with MagicDNS and HTTPS certificates turned on. Otherwise `hq-join`
   fails (`hq-join.sh:35`, `docs/HETZNER.md:45`). Nothing tells them this before the operator
   starts.
2. Signs in through the operator's Tailscale URL. Installs Tailscale on the phone, on the same
   tailnet. If they skip this, the HQ link just fails to resolve and HQ can't explain why.
3. Opens the HQ link. `adoptLinkToken` saves the token under `hq-web-token` in localStorage and
   strips it from the address bar (`apps/hq-web/src/lib/hqAuth.ts:11-36`).
4. `/setup` asks for a model key (OpenRouter, Anthropic or Google) and saves it with no restart
   (`crates/hq-web/src/setup_api.rs:151-176`). It works once: a second save returns 409.
5. Adds the PWA to the home screen. It opens at `start_url: "/"` with no token in the URL. If
   the installed app doesn't share the browser's storage, the owner is asked to sign in again.
   Whether iOS shares it is unverified, and the fix below doesn't depend on the answer.

### Where the owner gets stuck

| # | Dead end | Where |
|---|----------|-------|
| 1 | Signed out (new device, cleared storage, a rotation). The banner says to run `hq web --json`. That is a CLI command, and on these servers it prints a link with no token, pointing at the local bind address. | `SignInPrompt.tsx:40-41`, `crates/hq-cli/src/commands/web.rs:226-236, 294-298` |
| 2 | Recovery needs root. The token lives only in root-owned `/etc/hq/mcp.env`, loaded by systemd. No API, chat command or web screen returns it. | `deploy/hetzner/bootstrap.sh:107-122` |
| 3 | The token is 64 hex characters. Pasting it on a phone is hard. | |
| 4 | Without a token, the first-run redirect fails silently (a swallowed 401), so the owner sees a sign-in banner instead of setup. | `apps/hq-web/src/routes/__root.tsx:80` |
| 5 | The web app never mentions Telegram or Discord. No route writes relay config, issues pairing codes or restarts the server. | `crates/hq-web/src/settings_api.rs` (read-only) |
| 6 | A Telegram bot needs a token from @BotFather in `relay.telegram_token` **and** `relay.telegram_enabled: true`. The flag defaults to false and the relay doesn't start without it, silently. `hq onboard` writes the token but never the flag. | `crates/hq-core/src/config/relay.rs:65,193`, `crates/hq-cli/src/commands/start/mod.rs:214`, `crates/hq-cli/src/commands/onboard.rs:437-447` |
| 7 | Discord needs the privileged Message Content intent and an invite URL. Neither is documented. | `crates/hq-relay/src/discord/mod.rs:280-282` |
| 8 | Pairing codes come only from `hq pair` in a terminal, and they last 15 minutes. An unpaired bot drops every message and replies nothing. | `crates/hq-cli/src/commands/pair.rs:23-31`, `crates/hq-relay/src/telegram/access.rs:91-96` |
| 9 | Relay tokens, enable flags and allowlists load once at startup. Any change needs `systemctl restart hq`, and nothing in the web app can trigger one. `daemon_restart::restart_daemon` is launchd only. | `crates/hq-relay/src/telegram/session.rs:132`, `crates/hq-core/src/daemon_restart.rs:72` |
| 10 | The web app doesn't show relay health. A silent bot gives no reason. | |
| 11 | Changing the model key after setup means `hq env` or editing `config.yaml`. | `apps/hq-web/src/routes/setup.tsx:43` |
| 12 | Tailscale node keys expire after 180 days. Fixing it needs the operator back on SSH. | `docs/HETZNER.md:47,69` |
| 13 | Rotating a leaked token needs root: edit `mcp.env`, then restart. | `docs/HETZNER.md:66` |
| 14 | Tailscale switched off on the device. The phone is still online, so the cached app opens with saved data and only a small amber "saved" label in the header. Nothing points at Tailscale. This is what an owner actually hit first. | `apps/hq-web/src/routes/__root.tsx:330-332` |

## The target journey

### Stage 0: before provisioning (operator and owner)

A short owner checklist the operator sends first (new doc, `docs/OWNER_GUIDE.md`):

- Create a Tailscale account. Turn on MagicDNS and HTTPS certificates.
- Install Tailscale on the phone and sign in.
- After the server joins, disable key expiry for the server node in the Tailscale admin console.
  That removes dead end 12.

### Stage 1: join (operator)

`hq-join` stays as it is, but its closing message becomes a block the operator can forward as is:
the login link, a line saying it is a password, and a pointer to the owner guide.

### Stage 2: first sign-in and setup (owner)

- The owner opens the link and the token is adopted, as today.
- `/setup` asks for the model key, as today.
- `/setup` gains a final **Get started** step, a checklist that stays on the home screen until it
  is done or dismissed:
  1. Install the app on your phone. Platform instructions, plus a **Sign in on this device** code
     (stage 4).
  2. Connect a chat app (stage 3).
  3. Turn on notifications (Web Push already exists, `docs/WEB_PUSH.md`).

### Stage 3: connect a chat app (owner, web only)

New **Chat apps** section in Settings, one card per platform.

**Telegram**

1. Guided steps with copy-paste text: open @BotFather, send `/newbot`, choose a name, copy the
   token.
2. The owner pastes the token. The server checks it with `getMe` and shows the bot's username
   before saving anything.
3. Save writes `relay.telegram_token` and `relay.telegram_enabled: true` to `config.yaml` through
   the existing `save_patch` path (budgets already use it). Then it applies the change (see
   restart below).
4. **Pair**: the server issues a pairing code from the same store `hq pair` uses and shows a
   button for `https://t.me/<bot>?start=<code>`. Tapping it opens Telegram, and **Start** sends
   `/start <code>`. The relay doesn't handle `/start` today, so `/start <code>` must route to the
   existing `/pair` logic (`crates/hq-relay/src/telegram/access.rs:82-100`). The code is shown
   too, for typing `/pair <code>` by hand.
5. The card shows status from relay health: running, paired as `@name`, last message time, or the
   error.

**Discord**

1. Guided steps: create an application in the developer portal, add a bot, turn on **Message
   Content Intent**, copy the token.
2. The owner pastes the token. The server checks it against `GET /users/@me` and
   `GET /oauth2/applications/@me`, and gets the application id from the API (not by decoding the
   token).
3. Save writes `relay.discord_token` and `relay.discord_enabled: true`, then applies.
4. The card shows a generated invite link with the scopes and permissions HQ needs. Then
   **Pair**: a code to send as `!pair <code>` in a DM to the bot.
5. Status, as for Telegram.

**Applying a change.** The service already runs under `Restart=always` (`deploy/hq.service:25`).
"Save and restart" can exit the process cleanly after replying and let the supervisor bring it
back. The web app shows a reconnecting state until `/health` answers. It is allowed only when a
supervisor is detected: systemd's `INVOCATION_ID`, or the existing launchd path. Otherwise the
card says to restart HQ by hand. This one mechanism applies any config change, so there is no
relay hot-reload supervisor to build.

**Sibling fix.** `hq onboard` must also write the `*_enabled` flags (dead end 6).

### Stage 4: sign in on another device (owner)

Two ways in, neither needing the operator.

- **From a signed-in device.** Settings, Devices, **Sign in another device** shows a QR code of
  the login link and a short one-time code. On the new device, the sign-in banner gets an **Enter
  code** field next to the paste field. `POST /api/auth/device-code` (authenticated) issues the
  code, and `POST /api/auth/exchange` (unauthenticated) trades it for the token.
- **From a paired chat.** `/login` (Telegram) and `!login` (Discord) reply with the login link,
  built from the first `web_allowed_origins` entry plus `/vault#token=<token>`, with a note to
  delete the message once used. Owner only and DM only: refused for Telegram guests and in the
  Discord family channel and its threads, since family users exist and get full tools.

The banner copy changes to match: open your login link, send `/login` to your HQ bot, or enter a
code from a device that is already signed in. `hq web --json` stays only as the operator path.

### Stage 5: rotate the token (owner)

Settings, Devices, **Sign out everywhere** issues a new token, signs this device in with it, and
invalidates the old one. Every other device gets the banner.

Built (slice 5): every sign-in (code, QR code, `/login`, `hq web link`) now mints its own device
token, kept in the database only as a SHA-256. Devices lists them with **Sign out** per row.
**Sign out everywhere** (`POST /api/auth/rotate`) deletes every device token, records the
configured token's hash as signed out, voids the pending code, closes open chat sockets, and
returns one new token to the caller. The QR code now carries the one-time code (`#code=`) instead
of a token. `sudo hq-join --print-link` runs `hq web link` as the `hq` user, so the operator can
always mint a working link. Details: "Device tokens and signing out" in
`docs/security/WEB_AUTH.md`.

### Stage 6: change things later (owner)

- Settings can replace the model key. Setup's one-time rule stays for the unauthenticated first
  run only.
- Chat app cards can replace a token, re-pair, or disconnect.

## Situations an owner can run into

Each row is a situation the owner might hit later, how HQ should tell them, and what gets them
out. The last column is the slice that makes it self-service.

| Situation | What the owner sees | Way out | Slice |
|-----------|---------------------|---------|-------|
| Tailscale off on this device | "Can't reach your HQ" banner: check Tailscale first | Switch Tailscale on, tap Try again | 7 |
| Device has no internet | "This device is offline" banner, saved data | Reconnects by itself | 7 |
| Server restarting or updating | Same banner for under a minute, then live again | Wait. Restart notices also go to the paired chat | 7 |
| Server node expired or offline in Tailscale | Banner persists; server shows as disconnected in the admin console | Prevented by disabling key expiry (owner guide). Otherwise the operator re-adds SSH and runs `tailscale up` | 0, guide |
| New device, cleared storage, or home-screen app with its own storage | "Sign in to HQ" box | Enter a code from a signed-in device, scan its QR, send `/login` to the bot, or paste the link | 1, 4 |
| Lost the link and no other device or chat | "Sign in to HQ" box | Operator runs `sudo hq-join --print-link` | 0 |
| Link leaked, or a phone was lost | Settings, Devices lists every sign-in with its last use | Sign that device out, or sign out everywhere (new token for this device) | 5 |
| Bot says nothing | Chat app card shows the relay status or error; an unpaired bot answers `/start` with how to pair | Fix from the card: replace token, re-pair, restart | 2, 3 |
| Want a second chat app or a new phone number | Chat app card | Pair a different account | 2, 3 |
| Model key out of credit or revoked | Chat replies show the provider's error | Replace the key in Settings | 6 |
| Operator unreachable | Everything above except an expired server node, a deleted server or billing | Those three stay with the operator | |

## Security

- `/api/auth/exchange` is the only new unauthenticated surface. Codes are high entropy (at least
  8 base32 characters, not 6 digits), work once, last 10 minutes, and are stored hashed.
  Attempts are rate limited per IP and globally, reusing the connector's limiter
  (`crates/hq-web/src/oauth/guard.rs`). A successful exchange is logged and pushed to the owner
  with `notify_owner`.
- `/login` puts an admin credential into chat history. That chat can already drive the whole
  agent, so the owner gains no new reach. The reply still asks them to delete it, and guests and
  family users never get it.
- Bot tokens written from the web go into `config.yaml` (0600, owned by `hq`), never into
  responses, logs or the call log. Reads return only "set" or "not set".
- Every new write endpoint goes through the origin guard and the web token, like the budget
  endpoint.

## Open decisions

1. **Where the web token lives, for rotation.** *Resolved (slice 5): the second option. The env
   token stays the bootstrap secret; signing out everywhere records its hash in
   `web_revoked_tokens`, and new device tokens live hashed in `web_device_tokens`. An operator who
   sets a new env value turns the configured token back on.* `HQ_*` env vars are merged after `config.yaml`
   (`crates/hq-core/src/config/mod.rs:601-605`). A token rotated into `config.yaml` would be
   hidden by `HQ_WEB_AUTH_TOKEN` in root-owned `mcp.env`. The options are:
   - an `hq`-owned token file under `_data` that overrides the env var, with bootstrap writing
     the first token there; or
   - keeping the env token as a bootstrap secret, and storing rotated tokens hashed in the DB, so
     auth accepts the newest DB token once one exists.

   The second also makes per-device tokens possible later.
2. **Per-device tokens.** *Resolved (slice 5): built. The exchange, `/login`, `hq web link` and
   rotation each mint a device token, and a single device can be signed out.* The device-code
   exchange could mint a token per device instead of
   handing out the one shared token. That allows signing out one lost phone. It's more work, and
   it isn't needed to unblock owners.
3. **Restart on non-systemd installs.** Docker and plain `hq start` have no supervisor to
   restart them, so the card falls back to manual instructions. Is that acceptable?

## Slices

Each slice is one PR and leaves the product better on its own.

| Slice | What | Removes dead ends |
|-------|------|-------------------|
| 0 | Fix the banner copy. Make the first-run redirect explain a 401 instead of failing silently. Have `hq onboard` write the `*_enabled` flags. Write `docs/OWNER_GUIDE.md` (stage 0) and the forwardable `hq-join` message. | 1 (partly), 4, 6, 12 |
| 1 | Device sign-in: QR code and short code from Settings, code entry on the banner, the exchange endpoint and its limiter. | 1, 3 |
| 2 | "Save and restart" mechanism plus the Telegram card: token check, save with enable flag, `/start <code>` pairing deep link, status. | 5, 6, 8, 9, 10 |
| 3 | Discord card: token and application check, invite link, `!pair`, status. | 5, 7, 8, 10 |
| 4 | `/login` and `!login`: owner and DM only, with the banner copy updated to mention them. | 1, 2 |
| 5 | Token rotation from Settings, after open decision 1. | 13 |
| 6 | Replace the model key from Settings. | 11 |
| 7 | Reachability banner: tell "Tailscale off or server unreachable" apart from "signed out" and from "device offline". | 14 |
| 8 | **Get started** checklist on the home screen after setup (install on phone, connect a chat app, turn on notifications), so an owner finds Devices and Chat apps without being told. | 5 (discovery) |
