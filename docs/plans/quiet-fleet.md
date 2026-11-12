# Quiet fleet: the Inbox without helper noise

Status: server half built (this change). The phone and web screens follow.

## Problem

Helper sessions filled the Inbox: a Needs you item for every prompt an agent or a command
policy could already answer, a notice for every finished session, and requests from sessions
that ended days ago that never went away. A person cannot catch up on 160 items, so the ones
only they can decide were lost among them.

## Rules

1. **Fleet-owned means quiet.** A session has someone else answering for it when it has a
   command policy, a chat that drives or watches it, a parent session (which covers a team) or
   a linked task. A block from such a session makes no item until `inbox.fleet.grace_minutes`
   (15) have passed unanswered. Then it makes one item, not one per sweep
   (`raise_unanswered`, once per block).
2. **Owner-only decisions are never quiet.** A sign-in request, a connector approval, an owner
   approval from a chat turn, a block the command policy escalated, a prompt that would lose
   work and a proposal from the owner's own agents carry `owner_only`. The runtime sets it; no
   tool argument can. Only owner-only items count on the phone badge.
3. **Standing rule `inbox.fleet.helper_policy`:**
   * `nothing` (default): agents and policies handle helper prompts. A prompt nobody answered
     in time reaches the owner after the grace, as a helper item. Escalations come at once.
   * `loses_work`: also at once, and owner-only, for a prompt that would lose work.
   * `everything`: today's behaviour, every prompt at once and owner-only.
   Sessions nobody else answers for (started by hand, no policy, no chat) always ask at once.
4. **Fleet digest.** A finished or stopped helper no longer gets its own Fyi. One item per local
   day (source `fleet-digest`, dedup key `fleet-digest-<YYYY-MM-DD>`) is rewritten in place:
   "7 helpers finished today", "5 ready to check, 2 stopped". Counts come from the session event
   log and the linked task's status. The source is in `DEVICE_ONLY_SOURCES`, so it is never
   pushed. A failed exit (non-zero code) and an exit that left its task blocked stay individual.
   Off with `inbox.fleet.digest: false`.
5. **Tidy, not delete.** `hq_db::inbox_tidy::tidy` moves to `expired` (reason `auto_tidy:<rule>`,
   with a timestamp):
   * helper action items whose session ended, was stopped, is gone, or has been unreachable for
     over an hour (`session_ended`, `unreachable`), or that are older than `ttl_hours`
     (`older_than_ttl`);
   * sign-ins older than `signin_ttl_hours` (`signin_ttl`);
   * helper Fyi notices older than 24 hours (`folded_into_digest`; the digest already counts them).
   An owner-only helper item expires only when its session is gone or ended, never by age or
   silence (sign-ins past their TTL are the one exception). An exit that left its task blocked is
   retired by age only. The unreachable rule runs only after the supervisor has swept in this
   process, and the daily task starts 30 minutes after the daemon. A day's digest expires after 2
   days. A restored item is never tidied again.
6. **Restore.** `hq_db::inbox_tidy::restore_expired(db, since)` and `hq inbox restore --since 1d`
   bring items back (30 days at most).

## Surfaces

* Daemon task `inbox-tidy`, daily, off with `daemon.inbox_tidy: false`.
* `hq inbox tidy [--dry-run|--apply] [--json]`: a dry run is the default and prints counts by
  source, kind and rule, never text. `hq inbox restore --since <30m|12h|7d>`.
* `GET /api/notifications` adds, per item, `owner_only`, `source_group` (`you`, `helpers`,
  `system`) and `expired_reason`, and, in the response, `needs_you_owner_only`,
  `needs_you_helpers` and `expired_count`. Nothing existing changed.
* Migration 125 adds `value_items.owner_only`, `expired_reason`, `expired_at`, `restored_at`.

## Not in this change

The catch-up card, the helper group row, the digest row, the badge switch and the Settings page
for the standing rule are client work. A "Clear old" button needs a route that runs the tidy
for the owner; until it exists, `hq inbox tidy --apply` and the daily task do that work.
