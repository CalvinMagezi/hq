# Teams driven from a remote client: approvals by policy, events instead of polling

Status: phase 0 built (0.1 to 0.8 below); the rest proposed. Phase 4 converges
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
   server cannot reach it. Fix: the app's alert loop also polls the owner's
   remote HQ over HTTPS with a device credential from the existing device-code
   exchange (`/api/auth/exchange`); later it long-polls phase 3's event endpoint
   instead of polling every 20 seconds.
2. **The app never finds the screen excerpt.** The server wraps it in
   `----- session output -----` lines (since #199). The app looks for
   `<hq-output>` (written in #224 against its own fixtures), so `canAllow` is
   false and real prompts get only an Open button. Fix in the first PR; once
   phase 1 lands, the item carries the pending dialog as data and nothing parses
   a screen.
3. **Allow presses Enter and Not now presses Esc.** Enter takes the highlighted
   option, and Copilot highlights "remember this path" on its path dialog. The
   buttons must answer a named option through phase 1's answer route with its
   `seq`, so a changed dialog is refused instead of answered.
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

## Order of work

| Step | PRs | Unblocks |
|------|-----|----------|
| Phase 0 | built | Copilot teams run with few prompts and visible blocks; folder requests block instead of going quiet |
| Alerts, first PR | 1: the excerpt markers, remote HQ polling in the app | The phone shows agent prompts from a server HQ |
| 4.1 to 4.3 | 1 to 2, shared with agent-teams step 4 | No shared working trees |
| Phase 5.1 | 1 | A sleeping host is noticed |
| Phase 1 | 2 to 3 | Structured answers; option buttons on the phone and in web push |
| Phase 2 | 2 to 3 | Prompts become policy; Claude Code gets a deny list |
| Phase 3 | 2 | Events instead of screen reads; escalations pushed |
| 4.4, 5.2, phase 6 | 2 to 3 | One team per remote client; decisions list |

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
