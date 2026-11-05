# Proactive HQ: noticing for the owner without a model on a timer

Status: research and proposal. Nothing here is built yet. The phases at the end are sized as
single pull requests.

## The problem being solved

Things slip between the moments the owner talks to HQ. A task passes its due date and nobody
says so. A helper sits at a permission prompt for an hour. A note that answers an open question
lands in the vault and nothing connects the two. An email that needs a reply ages past the point
where replying is polite. Today HQ only notices some of these, and only when a person asks.

"Proactive" is the industry name for closing that gap, and the obvious way to build it is wrong
for HQ. The obvious way (the design reported for KAIROS, see below) wakes a model on a heartbeat
and lets it decide whether anything is worth doing. That spends tokens on every tick, sends
context to a cloud model on every tick whether or not anything happened, and needs an always-on
computer to host the loop. Each of those costs lands hardest on the people this project most
wants to reach: someone on a mid-range Android phone in Kampala on a metered bundle, with no
monthly model subscription and no cloud VM.

So the problem, stated for the owner and for a future user who is not the owner:

> HQ should tell me about the few things that need me, at a time I can act on them, without
> spending money or sending my data anywhere to work out that nothing happened.

## Why HQ is in an unusual position

1. **The daemon already lives where the owner is.** The `mobile` profile runs the full daemon on
   the phone as a foreground service (`docs/HQ_MOBILE.md`). Other proactive agents run their loop
   on a cloud computer and reach the phone only through a push service. HQ can see the phone's own
   state (time, charging, network type) and act without a round trip.
2. **Most of the noticing is already deterministic.** HQ has around a dozen producers that put
   items in the owner's inbox from rules, with no model involved (inventory below). The pattern
   exists; it is not yet a system.
3. **There is a local path for the parts that need fuzziness.** `daemon.embedding_provider: local`
   embeds on the device, and `inbox-triage` already runs on it under the data saver
   (`MOBILE_DATA_SAVER_REMOVED_DAEMON_TASKS` in `crates/hq-cli/src/commands/start/daemon/mod.rs`).
   "This new note is close to that open task" is a vector comparison, not a model call.
4. **The output channel is settled.** Notices go to `value_items` (web inbox, web push, the phone
   app's "Needs you" and "Updates" channels in `AlertPoster.kt`), never to a chat relay. That rule
   is in `CLAUDE.md` and stays.

## What others are doing

### KAIROS (reported from the leaked Claude Code source)

The Claude Code source leaked through an npm source map on 2026-03-31. Secondary write-ups agree
on the outline and disagree on details; none of the details below were checked against the code
itself, and the pages that claim to have read it could not be fetched from this session.

- An assistant mode behind compile-time flags (`PROACTIVE`, `KAIROS`), absent from public builds.
- A periodic **tick** prompt while the user is idle. On each tick the model judges whether useful
  work exists and, if not, calls a sleep tool. One source mentions a 15-second blocking budget per
  tick; nothing corroborates it.
- Tools only that mode gets: push notifications, sending files to the user, subscribing to pull
  request activity, scheduled self-wakeups.
- **autoDream**: a consolidation pass over the day's logs and changes into longer-lived memory.

The shape to take from it is the toolset (notify, hand over a file, subscribe to an event source,
schedule your own wakeup), which HQ mostly has. The shape to avoid is the decision loop: the model
is the trigger. HQ's own history argues against that loop too. The cognition layer (dream cycle,
curiosity, self-model) and the touchpoint generators (morning brief, digest composer) were
retired (`CLAUDE.md`, "Retired components"). This plan does not bring them back under a new name.

### CopilotKit openbot and OpenDuet

Read from shallow clones (openbot `06f5b84`, OpenDuet `f52288f`); no code was run, so this is what
the source says, not what it was seen to do. Both are MIT, TypeScript on Bun, Postgres, and
AG-UI.

**OpenBot** is a self-hosted, multi-tenant agent platform: each person gets a Docker "computer"
with Chromium, every tool call passes a CEL policy gateway and an audit row. **OpenDuet** is a
personal assistant built on a vendored copy of it, adding a morning brief, mail triage and
suggestion cards.

Their proactive work comes in four forms, all through one work queue:

- **Routines**: cron schedules the model creates in chat. A 15-minute floor, at most 20 per
  person, a fatigue rule (one message on the first failure, switched off after 10 in a row),
  missed windows skipped rather than replayed, and a sweep ledger so the UI can say "no worker has
  checked in" (`server/src/routines/`).
- **Check-ins**: periodic read-only sweeps of connected apps (`server/src/proactive/engine.ts`).
  Deterministic gates run first, cheapest first (person off, bot paused, admin off, daily cap,
  nothing readable), and each skip is written with a plain reason (`SKIPPED_BECAUSE`). Only then
  does a model turn look, told that "most check-ins should record nothing". The read window starts
  at the last run that read everything it tried to, so a failed run never moves it on
  (`window.ts`). Newer OpenBot no longer turns check-ins on by default: a bot offers one after
  reading something that changes (`offer_check_in`).
- **Event triggers**: signed webhooks (GitHub, Linear, Sentry, Slack, inbound mail) with a
  deterministic filter, committed with their queue item in one transaction.
- **Self-scheduled follow-ups**: one-shot wakeups, 1 minute to 7 days out, at most 5 pending and
  20 a day.

What they get right about the output:

- An item must name a tool this run actually read from, or it is refused. At most 5 items a run.
  A repeat (same normalised title or source link, already open, already a task, or hidden in the
  last 30 days, `HIDDEN_DAYS`) is refused across every bot.
- **One ping per person per hour** (`PING_GAP_MINUTES = 60`, `ping.ts`), folding every bot's
  waiting items into it, the slot taken with a conditional update, the text from a template rather
  than a model turn.
- **Hide** is the whole feedback interface: one tap, no reason asked, suppressed for 30 days and
  listed in later prompts. Acceptance is reported as started over started plus hidden.
- OpenDuet triage runs rules first, and the rules may only ever say "can wait": demoting is safe,
  promoting is the model's call and must give a reason. `decidedBy` records `rule:<id>` or
  `model:<name>`, and a 62-message labelled eval is committed with the code.

What does not fit HQ: the deciding step is still a model on a timer (well gated, but a model call
whenever the gates pass); the stack needs Docker, Postgres and an always-on server, with
CopilotKit Intelligence (a hosted service) required for threads and memory; a browser computer per
person; mail and calendar content goes to the model; pings go out through Slack, Teams, SMS and Expo
push. There are no quiet hours that I could find (`server/src/proactive/` has none). The morning
brief is triggered by opening the app, which a headless daemon on a phone does not need.

The lesson for HQ is that their careful engineering is all in the gates and the output contract,
and that is exactly the part that needs no model. HQ can adopt the gates and the contract whole and
stop before the model step that their design treats as the core.

## What HQ already has

Producers that put items in the inbox from rules alone:

| Producer | Trigger | Model? |
|---|---|---|
| `task-stale-digest` | tasks quiet for days, once a day | no |
| `tool-failure-digest` | the week's recurring tool errors | no |
| `disk-watchdog` | disk and cache thresholds | no |
| `verify-briefing` | verification queue health, "while you were away" at an owner-set hour | no |
| session supervisor alerts | a helper blocked at a prompt, a host unreachable | no |
| `turn_reconciler` | stranded background turns | no |
| `followup.rs` | sub-agent runs nobody resolved | no |
| `budget_gate.rs`, `owner_approval.rs` | spend and approval requests | no |
| `email-poll` + `inbox-triage` | new mail | yes (classification and draft), local embedder on the phone |
| `/watch` | every N minutes | yes, a full agent turn per firing |
| `notify_owner` | the agent decides | yes, capped at `web_push.max_agent_per_hour` |

Pieces of a feedback loop that exist and are not connected:

- `value_items` records `engaged_at` and `engagement` (`approved`, `dismissed`) for every item, and
  `hq_db::value_items::task_engagement_stats` aggregates it. Nothing outside tests calls it.
- `_system/USER.md` (`crates/hq-daemon/src/user_model.rs`) holds confidence-tagged preferences such
  as `notif.<source>.<kind>`, and `notif_gate::gate_decision` reads them. Outside tests, only the
  seeded defaults are ever written, and the gate sits on the mailbox path, not on the inbox.
- `dedup_key` on `ValueItem` already gives once-a-day semantics to the digests.

Gaps that matter more than any new feature:

- **Due dates do nothing.** Tasks carry `due_date` and `start_date`
  (`crates/hq-db/src/tasks/rows.rs`), and no producer reads them. For a person running a small
  business on HQ ("collect from this customer on Friday"), this is the single most useful
  proactive behaviour, and it needs no model.
- **Watches are expensive and confined to chat.** Every firing is a full agent turn
  (`MIN_WATCH_INTERVAL_MINS = 5` exists because tight intervals starved real messages), even when
  nothing changed. And `WatchOrigin::from_identity` returns `None` for web and the phone app, so
  the owner's main surfaces cannot create one at all.
- **No interruption budget.** Each producer decides on its own whether to push. There is no quiet
  window, no daily cap across producers, and no single place that answers "why did my phone buzz".
- **The data saver drops work instead of deferring it.** Under `data_saver: true` the timers that
  call a model are removed. They could instead wait for Wi-Fi and a charger.

## The design: sense, decide and gate without a model; spend one only to act

```
signals ──> rules ──> candidates ──> gate ──> inbox item / push / digest
 (no LLM)   (no LLM)   (deduped)     (budget,      │
                                      quiet hours,  └─> optional: one model call, budgeted,
                                      learned mute)     only for a candidate that passed
                    feedback (engaged / dismissed / "less like this") ──> back into the gate
```

### 1. Signals

Everything a rule can read, each tagged with where it came from:

- HQ's own state: tasks (status edges, due and start dates, staleness), the vault watcher's change
  events (`crates/hq-daemon/src/vault_watch/`), harness sessions, verification, backups, usage.
- Mail metadata from `gws` (sender, thread age, whether the owner replied), not bodies.
- The phone, from the Android app over the existing loopback channel: local time and time zone,
  charging, network type (unmetered or not), and later, with an explicit opt-in per signal, the
  calendar (`CalendarContract`). Nothing that needs a restricted Play permission (SMS, the
  notification listener) until a validated use case asks for it.

### 2. Rules

A rule is data, not code, so the owner and the agent can add one without a release. Proposed home:
notes under `_system/proactive/` with frontmatter, validated like skills (`hq proactive validate`,
and a check in `hq doctor`):

```yaml
---
name: due-tomorrow
when:
  task_due: { within: 24h, status_not: [complete, cancelled] }
emit:
  kind: action_needed
  title: "Due tomorrow: {{task.title}}"
  link: "/tasks?id={{task.display_id}}"
  dedup: "due:{{task.id}}:{{date}}"
deliver: push        # push | inbox | digest
---
```

Trigger kinds, all deterministic: `schedule` (cron in the owner's zone), `task_due`,
`task_transition`, `vault_changed` (path glob), `threshold` (a count over a query), `absence`
(nothing happened for N: no reply on a thread, no activity on an initiative), and `similar_to`
(local embedder: a new note within distance d of an open task's text). A rule may set
`compose: model` to have one model call phrase the item or draft the next step; that is the only
way a rule spends tokens, and it runs after the gate, never before. A composed item must cite the
signal that fired it (a task id, a note path, a thread id); one that cites nothing is dropped,
which is OpenBot's "name the tool you read from" rule applied to HQ's own records.

New rules ship **off and offered**: HQ suggests a rule in the inbox when it sees the pattern (a
task with a due date was just created, the owner asked the same question about a page twice) and
the owner turns it on with one tap. Only the due-date rule is on by default.

The built-in producers in the table above become built-in rules over time, so there is one list to
read and one switch per rule.

### 3. Candidates and the gate

Every rule emits a candidate. One module (`hq_daemon::proactive::gate`, new) turns candidates into
deliveries, and is the only code that decides to interrupt:

- **Dedup** by `dedup_key` (exists), widened the way OpenBot does it: a candidate is also refused
  when its normalised title or link matches an open item, an open task, or anything the owner hid
  in the last 30 days.
- **Quiet hours** in the owner's zone. A push inside them becomes an inbox item, except
  `ActionNeeded` items the owner marked as allowed to wake them. HQ has no owner time zone today
  (`tasks.verification.briefing_hour_utc` is in UTC), so this adds `owner.time_zone`, which the
  phone app can fill from the device.
- **Coalescing and a daily budget.** At most one push per window (an hour by default) folding
  everything waiting into it, with a template body; and a cap on pushes a day across all producers
  (say five), spent in score order. What does not fit goes to the next digest.
- **Digest batching** at an owner-set local hour.
- **A skip ledger.** Every candidate the gate holds back is recorded with a plain reason
  ("quiet hours", "budget spent", "hidden 12 days ago"), so "why didn't HQ tell me" has an answer
  as well as "why did it".
- **Rules may demote freely and promote only with a reason.** As in OpenDuet's triage, a
  deterministic rule can always push something down (to digest, to muted); raising an item above
  its rule's default needs either an explicit owner setting or a recorded reason, and each item
  stores `decided_by` (`rule:<name>` or `model:<id>`).
- **Learned demotion.** A rule whose items are dismissed without being opened k times in a row is
  demoted one step (push, then inbox, then digest, then muted), and the inbox says so ("HQ stopped
  pushing due-date reminders for the Personal space; undo"). It is a counter over `value_items`,
  explainable in one sentence, and needs no model.
- **Why am I seeing this.** Every delivered item carries the rule name and the signal that fired
  it, shown in the inbox and in the phone notification's expanded view, with "less like this".

### 4. Privacy tiers

Each rule declares the highest tier it may use, and the owner can cap a tier per source:

| Tier | Leaves the device | Examples |
|---|---|---|
| 0 | nothing | due dates, staleness, thresholds, absence, schedules |
| 1 | nothing (local embedder) | "related note appeared", near-duplicate tasks |
| 2 | one minimised payload to the model | composing a draft reply, summarising a thread |

On `profile: mobile` with the data saver on, tier 2 work is **deferred** rather than dropped: it
queues until the app reports an unmetered network and a charger, or until the owner opens the item
and asks. That turns the data saver from "these features are off" into "these features happen
later, on Wi-Fi".

### 5. Watches that only wake the model when something changed

A watch gets an optional deterministic precheck: a URL whose content hash changed, a vault query
whose result set changed, a task count over a threshold, a mail search with new hits. The firing
runs the precheck first and dispatches the agent turn only when it reports a change. A "check the
tender portal every hour" watch then costs one HTTP request an hour and a model turn on the day
the tender appears. Watches also learn to deliver to the inbox, so web and the phone app can create
them (a watch made in Telegram keeps answering in Telegram, which is a chat flow the owner asked
for). From OpenBot's routines: skip missed windows instead of replaying them after downtime (the
scheduler today fires everything that came due while it was down), and switch a watch off after a
run of failures, with one notice on the first.

Any rule or watch that reads an outside source keeps a **failure-aware window**: it reads from the
start of its last run that read everything it tried to, so a failed fetch never makes HQ skip
what arrived meanwhile.

## How we will know it works

The measure is not how much HQ says; it is how often what it says is acted on.

- **Precision per rule**: items engaged (opened, approved, completed the linked task within a
  day) over items delivered, from `value_items`. A rule below a floor for two weeks is demoted.
- **Pushes per day** on the owner's phone, and how many were in quiet hours (target: zero).
- **Tokens spent by proactive work per week**, split by tier. Tier 0 and 1 should carry most of the
  value at no model cost; if they do not, that is a finding, not a reason to add a tick loop.
- **Missed due dates** before and after the due-date rule ships, on the owner's own instance.

`tool-failure-digest` is the template: a weekly item that reports these numbers with no model.

## Who it is for, and the scale of the value

For the owner now: fewer things slip, the phone buzzes less, and every buzz says why. For a person
who installs HQ on a phone with no model key: tier 0 and tier 1 work with no account and no data
cost, which makes "reminds me what I owe and who owes me" useful to a market trader or a SACCO
treasurer before they ever pay for a model. That is the difference between a proactive assistant
for people with a cloud subscription and one that runs on the phone they already own. The phone
being the server is what makes the marginal cost of one more user close to zero.

Pushback recorded against this plan, to keep it honest:

- A proactive layer is a notification generator by default. If precision cannot be measured, do
  not ship the rule. Phase 0 builds the measurement before the rules.
- HQ has one owner today. The "bottom of the pyramid" claim is untested until someone who is not
  the owner runs the phone edition without a model key for a month. The first external pilot
  should be exactly that, with due-date and absence rules only.
- Phone signals invite scope creep (SMS, notification reading, location). Each one needs a named
  use case and a precision number from the pilot before it is added.

## Not doing

- No model on a timer to decide whether to act. A model call happens only for a candidate that
  passed the gate, or because the owner asked.
- No posting to Telegram or Discord. Notices go to the inbox.
- No screen or app-usage watching, no browser automation (retired), no revival of the cognition
  layer or touchpoint generators.
- No new always-on cloud component. Everything here runs inside the existing daemon, on a phone or
  a server alike.

## Phases

Each is one pull request with tests, sized to pass the repository gates.

**Phase 0: measure and gate.**
`hq_daemon::proactive::gate` with dedup, `owner.time_zone`, quiet hours (`proactive.quiet_hours`),
hourly coalescing, the daily push budget, digest batching and the skip ledger; every existing push
goes through it. Per-source
precision from `value_items` (wire `task_engagement_stats`), reported weekly in the shape of
`tool-failure-digest`. "Why am I seeing this" on inbox items (source and dedup key already stored).

**Phase 1: due dates.**
A built-in rule (still Rust at this point) for tasks due within a window and overdue tasks, one item
per task per day, honouring the gate. This is the first proactive feature a non-owner would feel.

**Phase 2: learned demotion and "less like this".**
The engagement counter, the demotion ladder and its undo, written as `notif.<source>.<kind>` traits
in `_system/USER.md` so the existing user model is finally fed by real behaviour.

**Phase 3: rules as vault notes.**
The `_system/proactive/` format, its validator, and the trigger kinds `schedule`, `task_due`,
`task_transition`, `vault_changed`, `threshold` and `absence`. The Phase 1 rule moves into this
format as the first built-in.

**Phase 4: watch prechecks and inbox delivery.**
`precheck:` on watches, and a `WatchOrigin` for web and the phone app that delivers to the inbox.

**Phase 5: the phone as a signal source, and deferred tier 2 work.**
Charging, network type and local time from the Android app into `hq`; the "defer until unmetered and
charging" queue for model work under the data saver. Calendar only behind its own opt-in.

**Phase 6: `similar_to`.**
Tier 1 rules on the local embedder: a new note close to an open task or initiative.

## Open questions for the owner

1. Default daily push budget and quiet hours: five pushes, 21:00 to 07:00 local?
2. Should rules live in the vault (editable from any surface, synced with backups) or in
   `config.yaml` (reviewed like config)? This plan assumes the vault.
3. Who runs the first pilot without a model key, and what do they track? The due-date and absence
   rules should be designed around that person's week, not the owner's.
