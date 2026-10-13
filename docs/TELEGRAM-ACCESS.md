# Telegram owner and guest access

HQ's Telegram relay supports one **owner** and optional **guests** (trusted contacts). Guests talk to the same assistant as the owner, with tool and command limits.

## Connect from the web app

No shell needed. In HQ, open **Settings**, then **Chat apps**, and use the Telegram card:

1. Open [@BotFather](https://t.me/BotFather) in Telegram and send `/newbot`. Pick a display name,
   then a username that ends in `bot`. BotFather replies with a token.
2. Paste the token and press **Check token**. HQ asks Telegram (`getMe`) and shows the bot's
   username before anything is saved.
3. Press **Save and restart**. HQ writes `relay.telegram_token` and `relay.telegram_enabled: true`
   to the config file and restarts itself, then the card shows the relay as running.
4. Press **Pair my Telegram**, then **Open in Telegram** and **Start**. The link opens
   `https://t.me/<bot>?start=<code>`, and Telegram sends `/start <code>`, which pairs exactly like
   `/pair <code>`. The code is shown too, for sending `/pair <code>` by hand. It lasts 15 minutes.

Once paired, the card offers **Pair a different account** (the current owner loses access until the
new account pairs) and **Disconnect** (turns the relay off and clears the token; the paired owner is
kept for next time).

Some things the card cannot change, and it says so:

- A token or switch set through `HQ_RELAY__TELEGRAM_TOKEN` or `HQ_RELAY__TELEGRAM_ENABLED`. Change
  it in the server's environment.
- An owner set in the config (`telegram_authorized_chat_id`, or a `role: owner` entry in
  `telegram_users`). Pairing does not override it.
- Anything at all on an instance without a web token (`web_auth_token`).

HQ restarts itself only under systemd or the launchd job named in `self_update.launchd_label`.
Elsewhere (Docker, `hq start` in a terminal) the card saves the change and asks you to restart HQ by
hand.

An unpaired bot ignores everything except pairing, with two replies so a wrong turn is not a silent
dead end: a bare `/start` gets a pointer to Settings, and a refused code gets a note that it may have
expired.

## Connect from the command line

### Configuration (`~/.hq/config.yaml`)

```yaml
relay:
  telegram_token: "..."
  # Owner chat ID. Without an owner the relay refuses every message.
  telegram_authorized_chat_id: 123456789
  # Legacy allowlist (treated as guest if not the owner ID).
  telegram_allowed_chat_ids:
    - 987654321
    - 111222333
  # Preferred: named users with roles.
  telegram_users:
    - chat_id: 123456789
      name: Alex
      role: owner
    - chat_id: 987654321
      name: Sam
      role: guest
    - chat_id: 111222333
      name: Jordan
      role: guest
```

## Becoming the owner

The relay never adopts the first chat that writes to it. With no owner
configured it ignores every message (and logs one warning with the fix).
An owner is established in one of two ways:

1. Set `relay.telegram_authorized_chat_id`, or list a `role: owner` entry in
   `relay.telegram_users`.
2. Pair from the chat app. Run `hq pair` (or `hq pair --platform telegram`) on
   the machine that runs HQ, then send `/pair <code>` to the bot in a private chat from the account
   that should own it (or open `https://t.me/<bot>?start=<code>`, which sends `/start <code>`). Group chats are ignored, so a group can never become the owner. The code is random, only its hash is stored in
   `_system/.pairing-telegram.json`, it is salted, works once, expires after 15 minutes,
   and is destroyed after 5 wrong guesses. `hq onboard` offers this step after
   you enter a bot token.

Pairing is only accepted while no owner exists. Allowlisted chats and
`telegram_users` guests behave as before.

Inline button taps (value-bus Approve and Dismiss) are honored only from the owner.

## Vault files

| File | Purpose |
|------|---------|
| `_system/.telegram-auth-chat` | Owner ID written by a successful `/pair`, used when `telegram_authorized_chat_id` is unset |
| `_system/.pairing-telegram.json` | Pending pairing code hash and expiry (written by `hq pair` or Settings) |
| `_system/chat-apps.json` | The bot's public username, remembered from the last token check, with a short hash of the token it belongs to |
| `_system/CHANNEL-PRESENCE.md` | Last owner activity (web UI); **not** used for proactive notifications to guests |
| `_gateway/channels/tg-{chat_id}.json` | Per-chat conversation state |

## Behavior

- **Proactive alerts** (value-bus items, mailbox) go to the **owner** only.
- **Guests** run under the `telegram_guest` tool profile (`crates/hq-agent/src/tool_policy.rs`): no shell, file or vault writes, sub-agents, skill edits or remote MCP servers.
- **`/login`** replies with the owner's web login link (`<first web_allowed_origins entry>/vault#token=<web token>`), with link previews off and a reminder to delete the message. Owner only, and only in a private chat: a group gets "Send /login to me in a private chat." and a guest gets the owner-only refusal. Without an origin or a web token it explains that and points at `sudo hq-join --print-link`. `/help` lists every command.

Get chat IDs: message [@userinfobot](https://t.me/userinfobot) or check relay logs (`telegram: incoming message`).
