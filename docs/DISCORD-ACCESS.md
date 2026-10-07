# Discord authorization

HQ's Discord relay supports one **owner** via an explicit allowlist, or via a
one-time pairing code if you leave the allowlist unset. Unlike Telegram,
there is currently no separate guest role — the allowlist/owner governs
**both** direct messages and @-mentions in a guild, with no way to scope one
without the other.

## Configuration (`~/.hq/config.yaml`)

```yaml
relay:
  discord_token: "..."
  # Discord user IDs allowed to send the bot regular chat messages, act on
  # approval buttons (email-triage Send/Skip,
  # value-bus Approve/Dismiss), and click component interactions generally.
  discord_allowed_user_ids:
    - 123456789012345678
```

If you leave `discord_allowed_user_ids` empty, the bot refuses everyone until
an owner is paired. Run `hq pair --platform discord`, then DM the bot
`!pair <code>` from the account that should own it. The code is random, only
a salted hash is stored (`_system/.pairing-discord.json`), it works once, expires
after 15 minutes, and is destroyed after 5 wrong guesses. Pairing is refused in
guild channels so a code is never typed where others can read it. Set the
allowlist explicitly to skip pairing.

## Scope: DMs and guild mentions share one owner

There is exactly one allowlist / one recorded owner, and it governs **both**
surfaces identically:

- A DM from the owner (or an allowlisted id) is authorized.
- An @-mention from the owner (or an allowlisted id) in any guild the bot is
  in is authorized.

There is no per-guild or DM-only/guild-only scoping — see
`crates/hq-core/src/config/relay.rs`'s doc comment on `discord_allowed_user_ids`
for the authoritative field-level statement this page summarizes.

A **genuine bot account** (any other Discord bot user)
is exempt from the allowlist entirely, but only when it explicitly
@-mentions this bot — this keeps cross-agent channels mention-gated and
loop-free without requiring bot accounts to be allowlisted. A
webhook-posted message is *not* exempt even though Discord also marks its
author `bot: true` — webhooks are impersonation-shaped (anyone with Manage
Webhooks on a channel can post as one), not trusted-bot-shaped.

## Vault files

| File | Purpose |
|------|---------|
| `_system/.discord-auth-user` | Owner ID written by a successful `!pair`, used when `discord_allowed_user_ids` is unset |
| `_system/.pairing-discord.json` | Pending pairing code hash and expiry (written by `hq pair`) |

## Verifying the recorded owner is actually you

At startup, the bot resolves and logs the Discord *username* for every id in
`discord_allowed_user_ids` (`"discord: authorized owner"` in the daemon log).
Check that username matches your own Discord account — the id alone in
config is otherwise just a number. This is a manual check; no code can
confirm identity on your behalf.

## Behavior

- Rejections are logged: `"discord: rejecting message from unauthorized user"`.
- Any I/O failure while claiming or reading the owner file fails **closed**
  (rejects), not open.
- A live test — an actual unauthorized Discord account attempting a DM or
  @-mention, confirmed rejected — is also a manual verification step, not
  something automated tests can substitute for.

## Family guest harness limit

`relay.discord_family_allowed_harnesses` lists the host harnesses a family guest may start. It defaults to `["agy"]`. Set it to `[]` to allow any harness. Approval prompts address the Telegram owner by the name in `relay.telegram_users`.
