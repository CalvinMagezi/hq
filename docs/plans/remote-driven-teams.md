# Teams driven from a remote client: approvals by policy, events instead of polling

Status: phase 0 built (0.1 to 0.8 below); phase 1 built (see "What phase 1
built" below); the rest proposed. Phase 4 converges
with step 4 of [agent-teams.md](agent-teams.md) (`owns` and `worktree`), so that
work lands once.

## The problem

A remote client (a Claude Code session in the cloud, reaching HQ through the
claude.ai connector) ran a two-agent team on a laptop host: a GitHub Copilot CLI
builder and a Claude Code reviewer. Two backend tasks and their fix rounds took
about 15.5 hours of wall clock, and roughly two thirds of that was waiting. The
agents' work was good, and the reviewer caught four real defects before anything
was pushed. The bottleneck was the owner's attention, or the remote client's,
standing in for it.

| Cause | Cost seen | Root cause in HQ |
|-------|-----------|------------------|
| Copilot approval dialogs nobody answered | about 4 h in total, single waits from 8 min to 4 h 45 m | Copilot has no state rules, so it reads `unknown` and never `blocked`. It also launches in manual approval mode. |
| Host offline | about 4.5 h | The laptop slept. Nothing told anyone. |
| A clarifying question HQ caused | 1 h 36 m | A Drive turn pasted the original brief back mid-task, and it contradicted later instructions. |
| Shared working tree | one near miss | The reviewer ran `git stash` in the folder the builder was editing. |
| Polling from the remote client | about 150 log reads at about 1.5k tokens each | No way to hear about an event except reading the screen. |
| Truncated dialogs | two approvals made half-blind | Logs return the screen, so the top of a long heredoc was cut off. |
| Unreliable keys and timeouts | several retries | Option keys that did not register; `harness_session_wait` past the connector's 60 s limit. |

Several teams of this shape run at once, often with Copilot builders chosen for
cost, so every hour of waiting removed here is multiplied across projects. The
goal is a team that runs unattended inside an owner-set policy and pulls the
owner in only for real decisions, answerable from a phone.

### How we will know

Measured from `harness_session_events` once phase 3 records them, and compared
against the run above on the next batch of the same project:

- **Time a dialog waits.** In-policy commands: no dialog at all on Copilot and
  Claude Code. Anything escalated: answered within minutes, because the owner
  is told at once.
- **Owner interventions per plan task.** Only decisions outside the policy.
- **Remote client cost per plan task.** Compact event reads instead of screen
  scrapes, about a tenth of the tokens.
- **A host that stops answering** is reported within five minutes.

**How to measure (HQ-RDT-009).** Run the next batch of the same project under the new setup:
a team formed with `team`, writers in worktrees, the default policies, decisions answered from
the phone. Then read the numbers with `hq sessions stats --task <the batch's task>` (or
`harness_session_stats`, or `hq sessions stats <root session id>` for one team). They come
from the event log:

- `waits`: how long dialogs waited (median, 90th percentile, longest, total minutes);
- `dialogs.answered_by_owner` against `dialogs.answered_automatically` and `policy`
  (allowed, denied, escalated): the owner's interventions;
- `hosts`: outages and silent minutes;
- `work`: finished turns, commits and exits.

Compare them with the table in "The problem": about 4 h of unanswered approvals, 4.5 h of a
silent host, and the 1 h 36 m clarifying question. Remote client cost is not in the log; count
its `harness_session_events` calls against the 150 screen reads of the first run.

## What the code does today

Facts this plan rests on, all on `main`:

1. **Copilot has no state rules.** The host ships rule files for `claude` and
   `codex` only (`crates/hq-host/src/detect/manifests/`). With no file,
   `Detector::detect` returns `None` and the pane's state is `unknown`, so no
   block alert fires and `harness_session_wait` never settles on `blocked`.
   `Detector::load_overrides` exists but nothing calls it outside tests.
2. **Copilot prompts for everything.** `claude-code` and `antigravity` launch with
   `--dangerously-skip-permissions` and `cursor` with `--force`, because "without
   the permission bypass every tool call in an unattended session blocks on an
   approval dialog nobody is there to answer" (`harness_session/spec.rs`).
   `github-copilot` launches with no permission flags. Copilot CLI documents
   `--allow-all-tools`, `--allow-tool` and `--deny-tool`, with deny taking
   precedence over allow (for example `--deny-tool='shell(git push)'`).
3. **Drive answers dialogs by model judgement.** The driver prompt
   (`crates/hq-web/src/session_driver.rs`) tells the model to approve "an action
   within the goal" by picking keys from the screen. There is no allow or deny
   list and nothing that avoids a "don't ask again" option. No code path types
   the brief again: the driver prompt carries the whole goal on every turn and the
   model wrote it back. Only prompt text forbids repeating an instruction.
4. **A dialog is a state, not data.** Detection returns a state and a rule id.
   The host event carries the rule, but `parse_info` (`agent_host/native.rs`)
   drops it, a blocked wake is the bare string `"blocked"`, and Claude's
   Notification hook message is dropped. Every caller reads the screen.
5. **Text can land in a dialog.** `send_text` (`harness_session/control.rs`)
   refuses on `agent_blocked`, but the native host never returns that code, so a
   text send while a dialog is open is typed into it.
6. **`harness_session_wait` allows 300 s** through the gateway, above the limit
   other connector-facing calls stay under (`hq_ask` 45 s default and 55 s max,
   `launch_bound_secs` at most 50).
7. **Logs have no cursor.** `harness_session_logs` returns the last N lines of
   scrollback plus screen every time.
8. **Nothing creates a worktree.** The unused column `mission_steps.worktree_path`
   and agent-teams step 4 are the only traces.
9. **A team is a delegation tree.** Sessions a remote client starts are separate
   roots, and `from_session` is refused from a chat, so a builder and reviewer
   started from the connector are two teams: no roster, no messages between them.
   `observe` does work across roots (it checks purpose, task and count, not team).
10. **An unreachable host is silent.** The supervisor logs at debug level and
    skips the session (`session_supervisor.rs`). No alert, no timer.
11. **No outbound webhooks.** The only event-driven outbound POST is Web Push.

## Corrections to the original proposal

- **The cloud session's `watch_url` cannot take HQ's events.** Its signing secret
  is sealed to the artifact service, and only that service can sign a delivery
  the session accepts. Phase 3 replaces it with a cheap long-poll and owner push.
- **Two worktrees cannot share a branch.** Git refuses to check out one branch in
  two worktrees. The writer owns the branch; a reviewer gets a detached worktree
  at a commit, which is what worked in the run.
- **Command patterns are not a security boundary.** `bash -c`, `python3 -c`,
  `xargs` and `env` all hide a command from a prefix match. A deny list stops a
  cooperative agent's slip; credentials and the sandbox stop the rest. Each deny
  entry that matters gets a credential backstop (phase 2).
- **Keep Copilot drivable rather than switching builders.** Copilot is chosen for
  cost. Phase 0 removes most of its prompts in a few days, so the builder can be
  picked for the work, not for how easy it is to drive.

## Phase 0: stop the bleeding

Small, independent PRs.

**0.1 Copilot state rules.** A `copilot.toml` rule file (id `copilot`, alias
`github-copilot`), with tests on captured screens:

- `blocked` on the command dialog (`Do you want to run this command?` with
  numbered options), the path dialog (`Allow path access` / `Do you want to allow
  this?`), the edit dialog (`Do you want to edit`), and the ask-user form
  (`needs information` with the `enter accept · … · esc cancel` footer).
- `working` on the `● Working` footer with `esc interrupt`.
- `idle` on the idle footer.

A harness profile whose base is `github-copilot` launches kind `copilot`, so it
is covered too.

**0.2 Rule overrides at runtime.** The host loads `~/.hq/detect/*.toml` at start
through the existing `load_overrides`, and `hq doctor` lists files that failed to
parse. When a CLI changes its screens, the fix is a file, not a release.

**0.3 Copilot launches with its own permission flags.** `github-copilot` gets
`--allow-all-tools` plus `--deny-tool` rules for `git push`, `git stash`,
`git reset`, `git clean`, `rm`, `env` and `printenv` (`COPILOT_ARGS` in
`harness_session/spec.rs`). This gives Copilot what `claude-code` already has,
with a deny list it lacks today. A harness profile with its own `args` replaces
them, so a profile meant to keep the denials must list them too. Copilot rules
match a command or subcommand, not a flag (see below), so `rm` is denied outright
rather than only `rm -r`.

**0.4 `harness_session_wait` stays under the connector limit.** Default 40 s,
maximum 45 s for gateway callers; in-process callers keep 300 s.

**0.5 Drive never sends the brief back.** Two changes:

- The driver prompt gives the goal as context and says the session already has
  its brief.
- A deterministic guard: a driver `harness_session_send` whose text overlaps the
  launch brief or goal (a long shared run, or most of the goal's word shingles)
  is refused with an error the driver sees.

**0.6 No text into an open dialog.** The host's `prompt` returns `agent_blocked`
when detection reads `blocked`, which makes the existing branch in `send_text`
live. Keys stay allowed.

**0.7 A folder request blocks the spawn.** A connector asking for a folder the
owner has not approved used to get an error back at once, worded "this is not a
failure", and nothing retried when the owner approved, so the caller carried on as
if a session had started. Now spawn and handoff file the request and wait for the
answer within the call: an approval starts the session in that call, a decline
fails at once, and no answer fails with `BLOCKED: no session was started` and
tells the caller to say so and call again with the same arguments. The wait is
55 seconds less the launch bound, at most 30, so the launch after an approval
still fits inside an MCP client's minute.

**0.8 A loading agent is not "waiting at a dialog".** A launch whose agent was
still loading when the launch bound ran out (Copilot waits up to a minute on a
slow MCP server) was reported as blocked at a dialog and recorded as
`BlockedAtLaunch` on its task. It is now reported as still starting, with a note
on whether its prompt was typed.

Phase 0 meets the bug's acceptance lines: Copilot reads `blocked`, `working` and
`idle`; `wait` settles on `blocked`; in-scope commands no longer prompt; Drive
does not resend the brief. "Drive never picks don't ask again" becomes structural
in phase 1.

### What a live Copilot session showed (CLI 1.0.105)

Captured from throwaway sessions in scratch folders while building 0.1:

- A dialog is a box at the bottom that replaces the prompt box and footer, with
  `↑/↓ to navigate · enter to select · esc to cancel`. The ask-user form is a
  separate shape between two rules: `enter accept · ctrl+d decline · esc cancel`.
- Some commands (`echo`) run with no dialog even in manual approval.
- **The path dialog pre-selects "Yes, and remember this path for this
  session".** Enter on whatever is highlighted widens the session's
  permissions. Answers must name an option (phase 1), never press Enter blind.
- The digit `1` answers "Yes" at once. The digit `3` only moves the cursor, and
  Enter on option 3 did not register, as seen in the original run. Esc is the
  reliable decline, and it aborts the whole turn ("Operation aborted by user").
- `--deny-tool='shell(git push)'` and `--allow-all-tools` are in the CLI's own
  help, with deny winning over allow. **A deny rule matches a command or a
  command and subcommand, never a flag**: under `--allow-all-tools`,
  `--deny-tool='shell(rm -r)'` let `rm -r dir` delete the folder, and
  `--deny-tool='shell(rm)'` stopped it. 0.3 denies `rm` outright; `git rm` stays
  allowed. Phase 2's matcher must not promise flag-level rules for Copilot.
- An MCP server in the owner's Copilot config that cannot start inside the
  sandbox costs every Copilot launch about a minute (`still waiting on ide`).

## Phase 1: the pending dialog as data

**1.1 Dialog extraction in rule files** (engine version 4). A blocked rule may
carry a `[rules.dialog]` table:

- `kind`: `command`, `edit`, `path`, `question`, `trust` or `other`.
- How to find the command or path text.
- An option pattern capturing number and label.
- A `remember` pattern that marks options like "don't ask again" or "always".
- How to answer (`digit`, `arrows_enter`, `enter`) and the decline key (`esc`).

This also fixes the unreliable keys: each CLI's answer method is recorded once,
in its rule file.

**1.2 The whole dialog.** Extraction reads the emulator's scrollback as well as
the visible rows, from the dialog's top border to its footer, so a long heredoc
arrives whole. A size cap (32 KiB) sets `truncated: true` rather than cutting
silently.

**1.3 Carried end to end.**

- The host's agent info and events gain `pending` and a `pending_seq`.
- `parse_info` keeps `rule` and `pending`, and `AgentInfo` gains both.
- The Claude hook keeps the Notification message as a hint.

**1.4 Two tools.**

- `harness_session_pending(session_id)` returns
  `{seq, kind, rule, question, command, cwd, options: [{id, label, remember}], truncated}`.
- `harness_session_answer(session_id, seq, option | {decline: message})`:
  - Refuses if `seq` is stale, so the answer always goes to the dialog that was read.
  - Refuses a `remember` option from a driver turn, a session token or the connector.
  - Sends the keys the rule file names.
  - Returns the state after, and `not_registered` when the dialog is still open.
  - Counts against the key allowance.

**1.5 `harness_session_logs` takes `since`.** The cursor is an absolute
scrollback row. The reply holds the rows that scrolled in since then, plus the
current screen and the next cursor. A TUI repaints its screen, so only scrollback
is truly incremental; the doc says so.

**1.6 Answer from the web UI.** The Sessions panel shows the pending dialog with
its options as buttons, over the same API. On a phone this is the owner's answer
surface.

### What phase 1 built

All of 1.1 to 1.6, with these differences from the design above:

- **Shared dialog shapes.** Several rules in one file read the same kind of
  dialog, so a file names shapes in `[dialogs.<name>]` and a rule says
  `dialog = "<name>"`; `[rules.dialog]` inline still works. Each shape also has
  `kind_when`, so one Copilot shape reads command, edit, path and trust dialogs.
- **Answers check the cursor.** Every built-in shape answers with the arrow keys:
  the host moves the cursor, waits until the screen shows it on the chosen
  option, and only then presses Enter. That covers Copilot's highlighted
  "remember this path" and its digit 3 (which only moves the cursor) without a
  per-option key table. `digit` stays available to a rule file.
- **Options that open a text field** (`typed`: Copilot's "No, and tell Copilot
  what to do differently", the ask-user form's "Other") are never picked. Decline
  with a `message` instead; it is sent as a prompt once the agent is back at its
  prompt.
- **One block, one alert.** A dialog read a moment after its block was announced
  takes that event's number; only a different dialog in the same block gets a
  new event.
- **The logs cursor is opaque**, not a bare row number: a count plus a hash of
  the last two lines, so it is found again after a full scrollback drops its
  oldest rows, and `reset` says when it cannot be.
- **The web panel's Approve button pressed Enter blind**, which on Copilot's
  path dialog picked "remember this path". A dialog HQ can read now gets one
  button per option instead, with the remember option marked.
- **Not built:** keeping Claude's Notification message as a hint (its dialogs are
  read from the screen), and option buttons in web push and the Android app
  (HQ-RDT-003, which can now name options instead of keys).

## Phase 2: a command policy per session

**2.1 Named policies in config**, owner-set, with generic built-in defaults:

```yaml
agent_host:
  command_policies:
    builder:
      allow: ["git add", "git commit", "git diff", "git status", "git log",
              "git worktree list", "bun test", "bunx tsc", "bunx prettier"]
      deny:  ["git push", "git stash", "git reset --hard", "git clean",
              "rm -r", "rm -rf", "env", "printenv"]
      otherwise: escalate        # escalate | allow | deny
    reviewer:
      extends: builder           # inherits both lists
      allow: ["git fetch", "git checkout --detach", "git push"]
      deny: ["git push --force", "git push -f"]
      deny_remove: ["git push"]  # drops the inherited entry
```

Project-specific entries (a framework's codegen against a dev deployment, a deny
on its deploy and data commands) go in the owner's own config, not the defaults.

**2.2 Matching.**

- Commands are tokenised and split on `&&`, `||`, `;` and `|`. Every segment must
  be allowed and none denied, and deny wins.
- A pattern matches by leading tokens, not by substring.
- Command substitution, `eval`, `sh -c`, `bash -c` and inline interpreter code
  (`python3 -c`, a heredoc into an interpreter) always escalate unless the policy
  sets `scripts: allow`.
- Edits and paths inside the session's folder or worktree are allowed; anything
  outside it escalates.

**2.3 Choosing a policy.**

- `harness_session_spawn`, `harness_session_handoff` and `agent_delegate` take
  `policy`. The default follows purpose: `code` gets `builder`, `review` gets
  `reviewer`, anything else gets none (every dialog escalates, as today).
- The connector and the web chat may name any configured policy. A session token
  may name only a policy no wider than its own.
- The choice is stored on the row (new column `command_policy`) and recorded as
  an event.

**2.4 Where it is enforced.**

- **At launch, in the harness's own terms.**
  - Copilot: `--allow-tool` and `--deny-tool`. Its rules cannot name a flag, so
    a policy entry with one (`git reset --hard`) becomes the whole command at
    launch, or stays with the supervisor when that would deny too much.
  - Claude Code: a `permissions.deny` list in the per-launch settings file HQ
    already writes (`hq-host/src/hooks.rs`). Confirm that deny rules hold under
    `--dangerously-skip-permissions`; if they do not, the same file carries a
    `PreToolUse` hook that returns a deny decision.
  - Other harnesses: nothing at launch.
- **In the supervisor, for any dialog still shown.** A blocked event with a
  `command` dialog is matched against the policy. Allowed: answer with the
  one-time "yes" option. Denied: decline with a fixed message naming the rule.
  Escalated: leave the dialog open and raise it (phase 3). A `remember` option
  is never chosen.
- **Drive.** For a session with a policy, the driver model never answers command
  dialogs; it only sees escalations.

**2.5 Every decision is recorded** as a `policy_decision` event (command,
decision, matching rule, time waited). The task gets a periodic summary comment,
not one comment per decision.

**2.6 A credential backstop for each deny that matters.** `docs/AGENT_SESSIONS.md`
gets a table pairing each deny with what actually stops it. For example, push is
stopped by the builder having no git credentials (hidden credential stores in
the sandbox), and a deploy by the deploy key living only in the reviewer's or
owner's environment, through a harness profile's `env`.

### What phase 2 built

- **2.1** as designed, under `agent_host.command_policies`, with `allow_remove` beside
  `deny_remove` and `scripts`. The built-ins (`hq_core::command_policy::builtins`) are generic:
  the plan's `bun` entries are the owner's to add. The engine is in `hq-core`, so the host's
  hook and HQ judge with the same code.
- **2.2** as designed, with two additions. A pattern's flags may appear anywhere after its
  words, so a trailing `--force` is caught; and git's global options and common wrappers are
  looked through. `sudo` always escalates.
- **2.3** as designed. "No wider" is computed: every allow of the narrower policy is covered by
  one of the wider's, every deny of the wider's is covered by one of the narrower's, and its
  `otherwise` and `scripts` are no looser. The policy is stored in its own table
  (`session_policies`, migration 105) rather than a column, holding the resolved rules, so a
  config edit never changes a running session.
- **2.4** at launch for Copilot (the policy's denies replace the fixed list) and Claude Code (a
  `PreToolUse` hook running `hq host policy-check`, failing closed). The hook was chosen over
  `permissions.deny` because hooks run under `--dangerously-skip-permissions`; that the deny
  decision holds there is from Claude Code's documentation and still needs a run against the
  real CLI. Both agents keep starting with their tools allowed, so `otherwise: escalate` cannot
  make them ask; it governs the dialogs they still show (paths outside the folder, and every
  dialog of agents that ask, such as Codex). The supervisor answers off the sweep, and raises
  the alert if the answer does not take.
- **2.5** records every decision as a `policy_decision` event. The task gets one summary comment
  when a session finishes a turn or exits ("Command policy 'builder' on session ...: 12
  allowed, 2 declined (`git push` x2), 1 left to the owner."), counting from the last summary.
  It quotes only the policy's rules, never a command the agent wrote.
- **2.6** in "What actually stops each deny" in `docs/AGENT_SESSIONS.md`.

## Phase 3: events instead of polling, and decisions on the owner's phone

**3.1 One event log.** The supervisor writes these kinds to the existing
`harness_session_events` table, whose autoincrement id serves as the cursor:

- `blocked`, with the pending dialog;
- `unblocked`, with milliseconds blocked;
- `finished`, `exited`, `commit` (phase 4.2) and `question` (an ask-user dialog);
- `policy_decision` and `escalation`;
- `host_unreachable` and `host_back`.

A team or task is read as one stream by joining on the sessions' task and tree.

**3.2 `harness_session_events(after, session_ids | task_id | team, wait_secs)`.**
A long-poll that holds up to 45 s on the registry's change notification. It
returns compact events (about 100 tokens each, the pending dialog included) and
`next`. The remote client reads events instead of screens, and only escalations
need its attention.

**3.3 Escalations reach the owner where they can answer in one tap.** Each
escalation, agent question and folder request becomes one ActionNeeded item, one
notice per dialog, never one per sweep. Telegram is a poor place for a decision
(answering means opening the app), so decisions go to the two surfaces the owner
already has open, each with buttons that answer without opening anything. Telegram
gets at most a plain notice. Design in "Actionable alerts" below.

### What phase 3 built

- **3.1** as designed, in the existing `harness_session_events` table, with `question` for a
  dialog of kind question and `policy` for the policy a session started under. `unblocked`
  carries the milliseconds since the last `blocked` event, measured when a sweep first sees
  the session no longer blocked. `host_unreachable` and `host_back` are written on each
  session of the host, at the alert and at the return.
- **3.2** as designed, except the long-poll checks the table every 750 ms instead of waiting
  on a change notification: the supervisor writes from the daemon, which may be another
  process than the one serving the call. A `limit` and `more` keep a reply small; the dialog's
  text is capped at 300 characters (`harness_session_pending` has it whole). The tool is in the
  untrusted-source list, in the session-token scope (its team) and the read-only Spark scope,
  and not in the handoff scope, for the same reason as `harness_session_logs`.
- **3.3** was built with HQ-RDT-003.
- The opt-in HTTP recipe for a cloud client is the read-only Spark key on `/mcp`, looping on
  `harness_session_events` ("Following sessions by events" in `docs/AGENT_SESSIONS.md`).

### Actionable alerts: the Android app and web push

What exists:

- The Android app runs a foreground service (`HubService`) that polls
  `/api/notifications?state=pending` every 20 seconds and posts notifications
  with **Allow** and **Not now** buttons for coding-agent prompts. Allow needs
  the phone unlocked. Before answering, it re-reads the live screen and refuses
  if the dialog changed (`AnswerReceiver`, `HelperWords.checkAnswer`).
- Web push already reaches the laptop for ActionNeeded items
  (`crates/hq-web/src/web_push/`), but the service worker shows no buttons and
  only opens a page.

What stops it working for a team on a remote HQ:

1. **The app only talks to the HQ on the phone** (`127.0.0.1:5678`). An HQ on a
   server cannot reach it. Fixed in #298: the phone signs in to the server HQ
   with a one-time code (`/api/auth/exchange`) and asks it once a minute with an
   ETag, so an unchanged list is a bodiless 304 on mobile data; answers go back
   to the HQ that asked. Later it long-polls phase 3's event endpoint instead.
2. **The app never finds the screen excerpt.** The server wraps it in
   `----- session output -----` lines (since #199). The app looked for
   `<hq-output>` (written in #224 against its own fixtures), so `canAllow` was
   false and real prompts got only an Open button. Fixed in #298; once phase 1
   lands, the item carries the pending dialog as data and nothing parses a
   screen.
3. **Allow presses Enter and Not now presses Esc.** This is safe today but
   narrow. Before Enter goes out, the app re-reads the live screen and refuses a
   changed dialog, a missing cursor, or a highlighted choice that is not a plain
   yes once, so Copilot's pre-selected "remember this path" is never taken (and
   since #298 such a card offers Open helper instead of Allow). What it cannot do
   is pick a yes once that is not highlighted, or decline with a message. Phase
   1's answer route, which names an option and carries the dialog's `seq`, gives
   the buttons both.
4. **Web push has no buttons and no credential.** The payload gains the item id,
   the kind and up to two options. The service worker shows them as `actions`
   (Chrome on macOS shows two). A click posts to
   `/api/notifications/{id}/answer` with a token the server put in the encrypted
   push payload: an HMAC with the install's approval key over item id,
   `pending_seq`, option and expiry (15 minutes). The route accepts that token for
   that item and option only, once. The service worker never holds the web
   token.

Folder requests (0.7) use the same buttons, so approving a folder from the phone
starts the waiting spawn.

#### Design for the answer buttons (HQ-RDT-003, after phase 1)

Phase 1 gives every dialog a number and an answer route that refuses a stale one,
so a button can answer without reading a screen.

- **The alert carries the dialog.** When the supervisor raises the block
  alert for a session whose host can read the dialog, it also records a
  `dialog_alerts` row keyed by the inbox item: session, dialog number, kind,
  question, command, the options, and `approve`, the one option a button may
  pick. `approve` is chosen on the server, once, for every client: the first
  option that neither widens permissions (`remember`) nor opens a text field,
  preferring one whose label starts with "Yes". A dialog with no such option
  gets no approve button.
- **One answer route.** `POST /api/notifications/{id}/answer {choice}`, where
  `choice` is `approve` or `decline`. It looks the row up and calls phase 1's
  answer as the owner (`Answerer::Owner`): the host refuses a dialog that moved
  on (409, "already answered or changed"), and the row is marked answered so a
  second tap from another device is refused too. The inbox item is then
  resolved, so every other device drops its card. The route never takes an
  option id, so no client can pick a remember option or one that was never
  offered.
- **The phone** uses its device token (Bearer), as every other call of the app
  does. It learns about blocks the way it does today: the foreground `HubService`
  asks the HQ on the phone every 20 seconds and a server HQ once a minute with an
  ETag. There is no Firebase: the app works without Google services, and a
  backgrounded app is kept alive by its foreground service. A card reads
  `metadata.dialog` and offers the approve option's own label, Decline and Open.
  It no longer parses the screen excerpt or presses keys itself. An item with no
  dialog (an older server, a dialog HQ cannot read) keeps today's excerpt path.
- **Web push.** The payload gains the item id and up to two actions (approve,
  decline), each with its own token: `exp.mac`, where `mac` is an HMAC-SHA-256
  with the install's approval key (`hq_core::approval_key`, kept outside the
  database) over `dialog-answer:<item>:<seq>:<choice>:<exp>`, and `exp` is 15
  minutes after the push. The payload is encrypted to the subscribing browser
  (RFC 8291), so only it sees the tokens. The service worker posts the token to
  `POST /api/notifications/push-answer`, the one notifications path that takes
  no web token; it checks the MAC, the expiry and that the row is unanswered,
  then answers as above. A token answers one dialog, one way, once. The service
  worker never holds the web token.
- **Telegram** is not given buttons for these. Its notice stays a notice.

Built as designed (migration 102, `crates/hq-web/src/dialog_answer.rs`, the
supervisor's `record_dialog`, `sw.ts`, the app's `AnswerReceiver` and
`InboxWords.dialogCard`). Not yet tried on a phone or in Chrome against a live
agent. Sessions a web chat watches keep their chat-only badge; Drive answers those.

What it costs: one more table, one route behind the web token, one token-checked
route, a payload a few hundred bytes longer. What it buys: a block on any HQ is
answered from the phone's shade or the laptop's notification in one tap, with
no screen to read and no way to pick the wrong dialog or a "remember" option by
accident.

**3.4 Optional: a push for cloud clients.** A documented recipe, off by default:
from the remote environment, a background command long-polls an HTTP endpoint
(`/api/harness-sessions/events?after=…&wait=…`) with a read-only key and exits on
the first event, which wakes the session. It needs the environment's network
policy to allow the HQ host and the key stored as an environment secret.

**3.5 Not built: an outbound webhook sender.** If a receiver that third parties
can sign for appears, a sender subscribed to the same log, with HMAC signing, is
a small addition.

## Phase 4: isolation at spawn (with agent-teams step 4)

**4.1 `worktree` on spawn, handoff and `agent_delegate`.**

- `{"branch": "<name>"}` for a writer. The branch is created from the folder's
  current branch if missing, and the call is refused if the branch is checked
  out elsewhere: one writer per branch.
- `{"detached": "<sha or ref>"}` for a reviewer.
- The host creates the worktree with `git worktree add` under its workspace root
  (`Documents/HQ/worktrees/<repo>/<session>`). It counts as approved because it
  comes from an approved folder.
- `owns` from agent-teams applies: overlapping writers without worktrees are
  refused.

**4.2 The reviewer sees commits, not a moving tree.** On each finished turn the
supervisor reads the writer's `HEAD` through the host, and a new commit becomes a
`commit` event (sha, subject). The observer's message names the sha, and the
reviewer checks it out detached in its own tree. The `reviewer` policy allows
pushing that sha to the team branch, never with `--force`.

**4.3 Cleanup.** A stopped session's worktree is removed when clean. A dirty one
is kept, and its path goes in the exit event.

**4.4 A remote client can form one team.** A `team: <root session id>` argument
on spawn and handoff, for owner-grant callers (the connector, a web chat), links
the new session under that root. Roster, messages, stop-with-root and the tree
limits then apply. It is kept apart from `from_session`, which means the caller's
own identity. A reviewer in a team observes the team's writers by default.

### What phases 4.1 to 4.3 built

- **4.1** as designed, on `harness_session_spawn`, `harness_session_handoff`, `agent_delegate`
  (`worktree: true` is a branch `hq/<task id>`), `hq sessions spawn --branch`, and the web's New
  agent dialog. The host's `worktree.add` makes it under
  `<HQ folder>/worktrees/<repository>/<session>` (the repository's main folder names it, so a
  worktree made from another worktree sits with its siblings), refuses names that could be read
  as options, and reports `branch_in_use` from git's own refusal. A launch that fails after
  the worktree was made removes it again. `owns` lives in `session_checkouts` (migration 104),
  next to the worktree and the last commit seen, so the session row is unchanged. The overlap
  check skips the new session's own parent: refusing it would refuse every `agent_delegate`
  of a coding worker into its delegator's folder, which is the default. Reviewers, monitors
  and researchers are never counted as writers. A session the host's agent list no longer
  has is skipped, as the supervisor reads it.
- **4.2** with the commit read in the supervisor's host phase, only for finished agents HQ
  tracks a checkout for, so it costs one `git log -1` per finished turn. The first commit seen
  is where the session started; each later one is named once. The `commit` event goes to the
  existing `harness_session_events` table, ready for phase 3's reader.
- **4.3** at stop and at exit, through the host's `worktree.remove`, which removes only a clean
  worktree under the HQ folder. `harness_session_resume` reopens it.
- The host gate allows `worktree.add`, `worktree.remove` and `agent.head`, so all of this works
  on a paired machine.
- Not built here: the `reviewer` policy's push rule, which is phase 2.

## Phase 5: hosts that stay awake, and noticing when one does not

**5.1 Unreachable alert.**

- The supervisor records when a host with running sessions first fails to answer.
- After `agent_host.unreachable_alert_minutes` (default 5) it raises one
  ActionNeeded item naming the host and its sessions.
- A `host_back` notice follows when the host answers again.
- The state is persisted, so a restart does not repeat the alert.

**5.2 Keep awake while sessions run** (`agent_host.keep_awake: while_sessions`,
off by default). `hq host serve` holds a sleep assertion while at least one pane
is alive: `caffeinate -i -w <pid>` on macOS, `systemd-inhibit --what=sleep` on
Linux. A closed lid on battery still sleeps, which the docs say.

**5.3 Placement.** `docs/AGENT_SESSIONS.md` says to run long teams on an
always-on host, and backend work on the server host where possible.

### What phase 5 built

- **5.1** as designed. The outage lives in `host_outages` (migration 103): the first failed
  poll opens it, a claim makes the alert one per outage, and the first answer ends it. The
  alert is an action-needed item from `session-supervisor` (dedup key
  `host-unreachable-<host>-<since>`), so it reaches the Android app and web push with no new
  path. When the host answers, the item is retired (state `expired`, not the owner's
  dismissal) and an FYI says it is back with how long it was gone. A silence shorter than the
  wait says nothing. A host whose sessions were all stopped while it was silent stops being
  polled, so its outage ends quietly. `unreachable_alert_minutes: 0` keeps tracking but never
  alerts. `hq doctor` lists open outages and suggests `keep_awake` when one exists.
- **5.2** as designed, with two details. On Linux the inhibitor runs `tail --pid=<host pid>`
  so it ends with the host. An inhibitor the OS refuses (it exits non-zero) is reported once
  and not retried, so a server without the permission does not spawn one every check. The
  host checks every 15 seconds.
- **5.3** in "Behavior worth knowing" in `docs/AGENT_SESSIONS.md`.
- Not built: the `host_unreachable` and `host_back` events, which come with phase 3's event
  log.

## Phase 6: goals that move with the work, and a decisions list

**6.1 One task per plan step.**

- The remote client files each plan step as a sub-task of the team's task and
  links the session to the current one.
- Drive reads the linked sub-task, not the launch brief.
- When the goal moves, the driver's message to the session is a fixed template:
  "Continue with <task id>: <title>. Its acceptance is on the task." It never
  carries the description.

**6.2 Decisions needed.** Escalations and agent questions that need the owner
are listed on the team's task, each with its options. Answering one (web,
relay) sends the mapped answer to the session and records who answered. It reuses
phase 3's items and the task's `waiting_on`, not a new store.

### What phases 4.4 and 6 built

- **4.4** as designed, on spawn and handoff, holding the same lock as `agent_delegate` while
  the limits are checked and the session starts. The limit checks are `agent_delegate`'s,
  shared (`a2a::check_tree_limits`), without its rule that the lead works on a task, which
  belongs to filing a sub-task. The root must be the team's root; a claude.ai connection may
  name only a team it started, the handoff key and session tokens none.
- **6.1**: `harness_session_link` moves a running session's goal when it was on another task,
  taking the acceptance criteria under "Acceptance criteria:" as the definition of done. Its
  reply carries the fixed line, and `continue: true` sends it. Drive's prompt reads the goal, so
  it follows the moved task.
- **6.2**: `task_decisions` and `GET /api/tasks/{id}/decisions` list the open dialog alerts of
  the task's sessions and teams (`dialog_alerts` joined to their inbox items), and the task page
  answers them through HQ-RDT-003's route. A task a launch-time dialog blocks says it waits on
  the owner (`waiting_on: owner`, the task's own field for a blocked task). A dialog mid-run
  leaves the task in progress as before, so its decisions are listed rather than flipping its
  status on every prompt.
- Every answer is an `answered` event (who answered, what was picked, whether it took), so
  `harness_session_stats` can tell the owner's interventions from a policy's or Drive's.

## Order of work

| Step | PRs | Unblocks |
|------|-----|----------|
| Phase 0 | built | Copilot teams run with few prompts and visible blocks; folder requests block instead of going quiet |
| Alerts, first PR | #298: the excerpt markers, a server HQ in the app, ETag on the notifications list | The phone shows agent prompts from a server HQ |
| 4.1 to 4.3 | built | No shared working trees |
| Phase 5 | built | A sleeping host is noticed; a laptop can stay awake while agents run |
| Phase 1 | built | Structured answers; option buttons in the web Sessions panel |
| HQ-RDT-003 | built | Answer buttons on the phone and in web push |
| Phase 2 | built | Prompts become policy; Claude Code gets a deny list |
| Phase 3 | built | Events instead of screen reads; escalations pushed |
| 4.4, phase 6 | built | One team per remote client; decisions list |

Each PR ships with tests and, for anything touching the host, a run on a real
host with the CLI in question. After phase 0 and 4.1, the next batch of the
original project runs under the new setup, and its waiting time, interventions
and remote client calls are compared with the run above.

## Decisions (the owner took the recommendations)

1. Copilot defaults to `--allow-all-tools` plus the deny list, matching what
   Claude Code already gets, rather than needing a profile to opt in. Built in
   0.3.
2. The built-in policies' `otherwise` is `escalate`, revisited once phase 3 shows
   how often it fires.
3. Worktrees live under the host's workspace root.
4. `keep_awake` is off by default on macOS hosts, with `hq doctor` suggesting it
   when a host has gone unreachable with live sessions.
5. Decisions reach the owner through the Android app and web push with answer
   buttons, not Telegram.

## Not doing

- A model approving commands. Policy is code. The model only sees what the
  policy escalates.
- Choosing "don't ask again" automatically. It widens a policy without the owner.
- Posting to the cloud session's `watch_url` (see corrections).
- Changing the default builder. With phases 0 and 2 the choice is about the work,
  not how easy the agent is to drive.
