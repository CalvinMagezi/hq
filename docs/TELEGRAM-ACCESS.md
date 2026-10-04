# Telegram owner and guest access

HQ's Telegram relay supports one **owner** and optional **guests** (trusted contacts). Guests talk to the same assistant as the owner, with tool and command limits.

## Configuration (`~/.hq/config.yaml`)

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
   that should own it. Group chats are ignored, so a group can never become the owner. The code is random, only its hash is stored in
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
| `_system/.pairing-telegram.json` | Pending pairing code hash and expiry (written by `hq pair`) |
| `_system/CHANNEL-PRESENCE.md` | Last owner activity (web UI); **not** used for proactive notifications to guests |
| `_gateway/channels/tg-{chat_id}.json` | Per-chat conversation state |

## Behavior

- **Proactive alerts** (value-bus items, mailbox) go to the **owner** only.
- **Guests** run under the `telegram_guest` tool profile (`crates/hq-agent/src/tool_policy.rs`): no shell, file or vault writes, sub-agents, skill edits or remote MCP servers.

Get chat IDs: message [@userinfobot](https://t.me/userinfobot) or check relay logs (`telegram: incoming message`).
