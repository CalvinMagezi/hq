# Discord and Telegram

Notes for working under `crates/hq-relay/`, moved out of the root `CLAUDE.md` so an assistant loads them only when it is in this area. The root file has the rules that apply everywhere.

## Entry points

| What | Path |
|------|------|
| Discord bridge | `crates/hq-relay/src/discord/` |
| Telegram bridge | `crates/hq-relay/src/telegram/`; commands and watch firing shared by both bridges in `chat_commands.rs` and `watch_scheduler.rs` |

## Rules

- **Chat relays are chat only.** Telegram and Discord carry conversations, never
  notifications. Alerts, approvals and notices go to the owner's inbox (`value_items`:
  web app, web push, the phone app). The relay pollers empty `_mailboxes/relay` without
  sending (`hq_relay::relay_common::take_chat_flows`); the one chat flow left is a family
  member's request, answered in Discord. Do not add a path that posts to a relay.
- **Long relay turns**: turns that outlive the ack window
  (`relay.turn_ack_timeout_secs`, default 270s) detach into the persisted
  `background_turns` registry and deliver results asynchronously. A startup and
  6-hourly reconciler interrupts stranded rows past
  `relay.background_turn_max_days` (default 5), resumable via `resume <id>`.
  Detached turns post progress heartbeats (`relay.background_progress_secs`,
  default 300) and the agent can volunteer updates via `report_progress`.
  `/watch <minutes> [for <N>h] <prompt>` schedules durable recurring turns
  (restart-safe); `/unwatch <id>` stops one. The agent can do the same with
  `watch_create`/`watch_list`/`watch_cancel` (`crates/hq-tools/src/background_turns.rs`).
  A firing whose reply contains `WATCH_DONE` delivers once and stops its watch.
  `background_turn_status` looks any turn up by its 8-character short ref.
