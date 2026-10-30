# Discord authorization

HQ's Discord relay supports one **owner** via an explicit allowlist, or via a
one-time pairing code if you leave the allowlist unset. Unlike Telegram,
there is currently no separate guest role — the allowlist/owner governs
**both** direct messages and @-mentions in a guild, with no way to scope one
without the other.

## Connect from the web app

No shell needed. In HQ, open **Settings**, then **Chat apps**, and use the Discord card:

1. Open the [Discord developer portal](https://discord.com/developers/applications) and choose
   **New Application**. Open **Bot**, press **Reset Token** and copy the token.
2. On the same page, under **Privileged Gateway Intents**, turn on **Message Content Intent** and
   save. The relay asks for it (`crates/hq-relay/src/discord/mod.rs`), and without it the bot sees
   empty messages.
3. Paste the token and press **Check token**. HQ asks Discord (`GET /users/@me` and
   `GET /oauth2/applications/@me`) for the bot's name and application id; it never decodes the
   token. If the application's flags show the intent off, the card says so.
4. Press **Save and restart**. HQ writes `relay.discord_token` and `relay.discord_enabled: true`
   and restarts itself.
5. Use **Open the invite link** to add the bot to a server you own. The link asks for the `bot` and
   `applications.commands` scopes and these permissions only: View Channels, Send Messages, Embed
   Links, Read Message History, Add Reactions, Create Public Threads and Send Messages in Threads
   (permission integer `309237730368`).
6. Press **Pair my Discord** and DM the bot `!pair <code>`. The code lasts 15 minutes.

The card refuses to change a token or switch set through `HQ_RELAY__DISCORD_TOKEN` or
`HQ_RELAY__DISCORD_ENABLED`, offers no pairing while `discord_allowed_user_ids` is set, and changes
nothing on an instance without a web token. HQ restarts itself only under systemd or its launchd
job; elsewhere the card asks you to restart HQ by hand.

## Connect from the command line

To build the invite yourself, use
`https://discord.com/oauth2/authorize?client_id=<application id>&scope=bot%20applications.commands&permissions=309237730368`.
Turn on Message Content Intent as in step 2 above.

### Configuration (`~/.hq/config.yaml`)

```yaml
relay:
  discord_token: "..."
  # Discord user IDs allowed to send the bot regular chat messages, act on
  # family request buttons (Approve/Deny), and click component interactions
  # generally. Discord carries chat only: notifications go to the web inbox.
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
| `_system/.pairing-discord.json` | Pending pairing code hash and expiry (written by `hq pair` or Settings) |
| `_system/chat-apps.json` | The bot's public name and application id, remembered from the last token check, with a short hash of the token they belong to |

## Verifying the recorded owner is actually you

At startup, the bot resolves and logs the Discord *username* for every id in
`discord_allowed_user_ids` (`"discord: authorized owner"` in the daemon log).
Check that username matches your own Discord account — the id alone in
config is otherwise just a number. This is a manual check; no code can
confirm identity on your behalf.

## Behavior

- Rejections are logged: `"discord: rejecting message from unauthorized user"`.
- **`!login`** and the **`/login`** slash command reply with the owner's web login link
  (`<first web_allowed_origins entry>/vault#token=<token>`), carrying a new device token that
  Settings, Devices lists as "Discord /login" and can sign out on its own. Only an allowlisted or paired
  owner gets it, and only in a direct message. In any server channel, including the family
  channel and its threads, the reply is just "Send !login to me in a direct message.", and
  family users never get the link. `/login` answers with a reply only you can see, which
  Discord doesn't keep; `!login`'s reply stays in the DM, and only the bot can delete it.
- Any I/O failure while claiming or reading the owner file fails **closed**
  (rejects), not open.
- A live test — an actual unauthorized Discord account attempting a DM or
  @-mention, confirmed rejected — is also a manual verification step, not
  something automated tests can substitute for.

## Family guest harness limit

`relay.discord_family_allowed_harnesses` lists the host harnesses a family guest may start. It defaults to `["agy"]`. Set it to `[]` to allow any harness. Approval prompts address the Telegram owner by the name in `relay.telegram_users`.
