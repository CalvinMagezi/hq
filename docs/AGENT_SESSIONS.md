# Agent sessions

Agent HQ runs and monitors two kinds of agent in its built-in host (`hq host`, see
[AGENT_HOST.md](AGENT_HOST.md)): coding agents (Claude Code, Codex, Cursor, Pi, OpenCode,
Copilot CLI, Kimi, Qwen, Antigravity), which edit code, and general-purpose agents (Hermes,
OpenClaw), full assistants with their own tools and memory that never take code
work (see [General-purpose agents](#general-purpose-agents)). The host owns the terminals,
recognises which agent sits in a pane, and reports its state: `idle`, `working`, `blocked`
(waiting on a dialog or approval), or `done`. It replaced the earlier tmux-based runner,
which could only guess state from screen text.

A **host** is a machine running `hq host`. `native` (also written `local`) is the machine
HQ runs on. Any other host is a machine you paired (see [JOIN_A_MACHINE.md](JOIN_A_MACHINE.md))
and is reached over ssh, which is how HQ on a VPS can run and watch agents on a laptop
whenever the laptop is on and reachable over Tailscale.

### Choosing the model

`harness_session_spawn`, `harness_session_handoff` and `hq sessions spawn --model <id>` take an
optional `model`, passed to the agent's CLI with its `--model` flag (Claude Code, Codex, Cursor,
Pi, OpenCode, Copilot CLI, Kimi, Qwen and Antigravity all take one). The id is the CLI's own,
for example `sonnet` or `haiku` for Claude Code. It is stored with the session, so a resume or a
host restart starts the agent on the same model, and the result reports it as `model` (null
means the agent's own default). An agent with no model flag HQ knows (Hermes, OpenClaw) refuses
a model instead of starting on its default, and so does an id that could read as a flag.

## What HQ can do

| Tool | Effect |
|------|--------|
| `harness_session_spawn` | Start an agent on a host. Returns a `session_id`. |
| `harness_session_send` | Submit a prompt, or press `keys` (`down`, `enter`, `esc`, `ctrl+c`) for a dialog HQ cannot read. |
| `harness_session_pending` | The dialog a blocked session waits at, as data: its number (`seq`), kind, question, command or path, folder and options. |
| `harness_session_answer` | Answer that dialog by option (`seq` and `option`), or `decline` it, optionally with a `message` sent as the next prompt. |
| `harness_session_wait` | Block until the session goes `idle`, `done` or `blocked`. |
| `harness_session_events` | What happened to sessions, a task or a team, as compact events after a cursor, waiting up to 45 s for the next one. See "Following sessions by events". |
| `harness_session_logs` / `_status` / `_list` | Read output and live state. `logs` with `since` returns only what is new. |
| `harness_session_stop` / `_resume` | Close the workspace, or continue a stopped session. |
| `host_list` | Which hosts exist and whether each answers right now. |
| `host_agents` / `host_read` | Read-only view of every agent on a host, including ones you started by hand. |
| `host_send` | Prompt (`text`) or press `keys` in an existing pane by pane id or agent name, including one you started by hand. `host` is required. |

The `harness_session_*` tools only change sessions HQ launched. An agent you started yourself
is visible and readable, and `host_send` is the one way to steer it: it checks the target
first, sends nothing to an unreachable host or a stale pane id, and refuses text while the
agent is blocked at a dialog (it returns the screen so the dialog can be answered with
`keys`). `hq sessions list|spawn|status|logs|send|stop|resume`
mirrors the tools, and `hq sessions spawn <harness> --host laptop --cwd <path on laptop>`
starts one on a remote host. The folder is optional: with none, or with one the machine does
not have, the session starts in a folder inside that machine's own `Documents/HQ`, which the
host creates (a missing folder keeps its name there; no folder gives the agent a folder of its
own). The result says so in `cwd_note`. A folder that exists is used exactly as before, and a
home or root directory is still refused.

There is no terminal UI to attach to: read a session with `harness_session_logs` or the web
app, and list every agent on a machine with `hq host status`.

## Behavior worth knowing

- **Dialogs are not answered for you.** If an agent stops at a startup dialog, `spawn`
  returns the screen and types nothing. Claude Code's workspace-trust prompt defaults to
  "No, exit", so pressing Enter would quit it. Read the screen, then send `keys`. Only
  Antigravity, whose default is verified to be "trust", is accepted automatically.
- **Dialogs are data.** For Claude Code, Codex and Copilot, `harness_session_status`, `_list`
  and `_wait` say when a blocked session waits at a dialog HQ can read (`pending_seq`, its
  number, and `pending_kind`); `harness_session_pending` returns the dialog itself. The dialog's
  text is the agent's output, so reading it counts as reading untrusted content, the same as
  `harness_session_logs`. `harness_session_answer` names that
  number, so it never answers a dialog that replaced the one that was read: it is refused, and
  the caller reads again. HQ moves the agent's cursor and presses Enter only once the screen shows
  it on the chosen option. `registered: false` (`not_registered`) means the dialog was still open
  afterwards.
- **"Don't ask again" is the owner's call.** An option marked `remember` lets the agent do the
  same thing later without asking (Copilot's path dialog even highlights "remember this path").
  Drive, a session's own token, a claude.ai connection, an `hq_ask` reply and a turn that read
  untrusted content may pick only one-time answers or decline. Reading the dialog with
  `harness_session_pending` counts as reading untrusted content, so a model that has read it
  never widens an agent's permissions: the owner does that from the web Sessions panel, which
  shows one button per option. An option that opens a text field is never picked: decline with a
  `message` instead.
- **Following output without rereading it.** `harness_session_logs` with `since: ""` starts a
  cursor; each reply's `next` is the `since` for the next call, and the reply holds only the
  lines that scrolled off the screen since then, plus the screen now. An agent that redraws a
  full-screen view in place (Copilot) scrolls nothing off, so for it only the screen changes.
- **A blocked agent alerts once.** The supervisor (every minute) raises an action-needed
  item when a session blocks, and again only if it blocks in a new state.
- **A finished agent alerts once.** When the host reports `done` (the agent finished a turn and
  is waiting), the supervisor posts the last screen to the relay, so the operator hears about
  completion without the session having to exit. `idle` (nothing new happened) does not alert.
- **An unreachable host is not an exit.** If the laptop sleeps or leaves the tailnet, its
  sessions stay `running` and are left alone. They resume being watched when it answers.
  `harness_session_stop` fails for such a session rather than pretending to stop it.
- **A host that stops answering is reported.** Once a host with running sessions has been
  silent for `agent_host.unreachable_alert_minutes` (default 5, 0 turns it off), the supervisor
  raises one action-needed item naming the host and its sessions, which reaches the phone and
  web push like any other. When the host answers again the item leaves the inbox and a notice
  says it is back, with how long it was gone. A short silence says nothing. The outage is kept
  in the database (`host_outages`), so a daemon restart neither repeats the alert nor forgets
  it, and `hq doctor` lists hosts that are silent right now.
- **Where to run a long team.** A machine that sleeps stops every agent on it. Run long
  unattended teams on a host that is always on (a desktop or a server), and backend work on the
  server's own host where its toolchain allows. On a laptop, `agent_host.keep_awake:
  while_sessions` in that machine's config keeps it awake while an agent runs (see
  [AGENT_HOST.md](AGENT_HOST.md)); a laptop with its lid closed on battery still sleeps.
- **Prompts are confirmed, not just sent.** `agent prompt` writes the text and Enter
  together, but a TUI that is not listening yet can swallow the Enter and leave the text in
  its input box. `harness_session_send` and the initial prompt wait for the agent to start
  working. If it never does and the pane printed nothing after the echo of the typed text, HQ
  presses Enter once more and says so in the result. If the pane did print something (an agent
  HQ has no state rules for, such as Copilot CLI, never reports working), HQ leaves it alone and
  reports the prompt as stalled, because Copilot sends the last prompt again on an empty Enter.
  Read the output before resending anything by hand.
- **Copilot gets its first prompt at start.** Copilot CLI draws its screen, then reloads its
  MCP servers and clears whatever was typed meanwhile, so a fresh `github-copilot` session is
  started with `copilot --interactive=<prompt>` instead of having the prompt typed. A resume
  still types its prompt.
- Exit summaries use the last screen the supervisor captured while the agent was alive.

## A team run from outside

A remote client (a claude.ai connection, a web chat, the owner's own key) that starts a lead,
a builder and a reviewer used to get three unrelated sessions. `team: <root session id>` on
`harness_session_spawn` and `harness_session_handoff` starts the new session under that root
instead, with everything a delegated worker gets:

- the team's roster (`team_roster`);
- messages between members (`agent_message_send`);
- stop-with-root;
- the limits in `agent_host.teams`: depth, children per session, running sessions per team
  and starts per hour.

Two rules apply to `team`:

- The root must be running and must be the team's root; naming a worker is refused with the
  root's id.
- A claude.ai connection may name only a team it started. An agent inside a session adds
  workers with `agent_delegate`, and the handoff key cannot reach a team.

A reviewer joining a team with a `task_id` and no `observe` observes the team's running
coding sessions, so their finished turns and commits reach it. Its command policy may not be
wider than the root's.

**Moving on to the next step.** File each plan step as a sub-task, and move a running session
on with `harness_session_link` to the next one:

- Its goal moves with it: the new task's title and description, with its acceptance criteria
  as the definition of done. Drive reads that, not the launch brief.
- The reply's `next_message` is the one line the session needs: "Continue with <task id>:
  <title>. Its acceptance is on the task." `continue: true` sends it.
- The task's description is never pasted into the session.

**Decisions a task waits on.** `task_decisions`, and "Waiting for you" on the task's page in
the web app, list every dialog the task's sessions and their teams wait on the owner for. Each
comes with its question, command or path, and options, for example a command a policy left to
you or a question an agent asked. Allow once and Decline there answer through the same route
as the phone's notification buttons, so a dialog that moved on is refused and the answer is
recorded with who gave it. The phone's Helpers screen does the same: `GET /api/harness-sessions`
carries each running session's open dialog as `decision` (`item_id`, `seq`, `options`,
`question`), and every screen that answers uses that alert when there is one. Only a helper
with no alert, such as one on an older server, is answered by reading its terminal and sending keys.

## Following sessions by events

A remote client that drives a team used to read every agent's screen to learn whether
anything changed: about 1.5k tokens a read, a hundred and fifty reads a run. The supervisor
now writes what happens to an event log, and `harness_session_events` reads it as one stream.

- **Kinds:**
  - `blocked`: with the dialog when HQ can read it (its number, kind, question, command and
    options, the text capped at 300 characters).
  - `question`: the agent asks the owner something.
  - `unblocked`: with how long it waited.
  - `finished` and `exited`.
  - `commit`: sha, subject and branch (see "Agents that share a repository").
  - `policy_decision` and `escalation`: what a command policy decided, and what it left to
    the owner and why.
  - `host_unreachable` and `host_back`: on each session of a host that stopped answering.
  - `policy`, `goal` and `drive`: written when a session starts or changes.
- **Scope:** `session_ids`, a `task_id` (the sessions working on it and their teams), or a
  `team` (a root session id). None reads every session the caller may see.
- **Cursor:** each reply has `next`. Pass it as `after` and the reply holds only what came
  since, oldest first, at most `limit` (50 by default) with `more` when there is more. Without
  `after` the reply holds the newest events and a cursor.
- **Waiting:** `wait_secs` (at most 45, under an MCP client's 60 second limit) holds the call
  until the first new event, so a client loops on one call instead of polling screens. Every
  waiting call shares one feed that checks for a new event about once a second, so many
  watchers cost no more than one, and nothing polls while nobody waits.
- **Who sees what:** the owner sees every session, a claude.ai connection the teams it
  started, and an agent with its own session token its own team. Naming a session outside
  that is refused.
- **Untrusted:** dialog text is the agent's own output, so reading events counts as reading
  untrusted content, the same as `harness_session_logs`.

**The numbers.** `harness_session_stats` (or `hq sessions stats [<root session id>] [--task
<id>]`) reads the same log into counts and times: dialogs and who answered them, how long
they waited (median, 90th percentile, longest, total), the command policy's allows, denies and
escalations, host outages, and finished turns, commits and exits. It quotes no dialog text.

**From a cloud client, read-only (opt-in).** A client somewhere else (a cloud session, a CI
job) can follow a team without any key that could change it. Set `AGENTHQ_SPARK_API_KEY` on
the server (unset, `/mcp` refuses it), give the client that key and HQ's `/mcp` URL, and have
it loop on one call:

```json
{"tool": "harness_session_events", "args": {"team": "<root session id>", "after": 0, "wait_secs": 40}}
```

sent through `hq_call` with `Authorization: Bearer <spark key>`. Each reply's `next` is the
next call's `after`. The Spark scope reads sessions, logs and events and nothing else: it
cannot answer, send, start or stop a session. Answering stays with the owner's phone, the web
app, or a full key.

## Command policies

A coding agent that asks before every command waits on whoever is there to answer, and one
that asks nothing can push, wipe a tree or print its environment. A **command policy** sits
between: commands it allows go ahead, commands it denies are refused, and only the rest reach
the owner. Policy is code. A model never approves a command.

```yaml
agent_host:
  command_policies:
    builder:                    # replaces the built-in builder
      allow: ["git status", "git add", "git commit", "cargo test", "pnpm codegen"]
      deny: ["git push", "git stash", "git reset --hard", "rm -r", "env", "pnpm deploy"]
      otherwise: escalate       # escalate | allow | deny
      scripts: escalate         # escalate | allow
    careful:
      extends: builder
      allow_remove: ["cargo test"]
      deny: ["cargo publish"]
```

- **Which one.** `harness_session_spawn`, `harness_session_handoff`, `agent_delegate` and
  `hq sessions spawn --policy` take `policy`. Without one, a coding session gets `builder` and a
  review gets `reviewer`; monitors, researchers and general-purpose agents get none, and
  `policy: none` asks for none (every dialog goes to the owner, as before). The built-ins are
  generic: `builder` allows reading the tree, local git and common build and test commands,
  and denies `git push`, `git stash`, `git reset --hard`, `git clean`, `git checkout --`,
  `git restore`, `rm -r`, `env` and `printenv`; `reviewer` extends it and may also
  `git checkout --detach` and `git push`, never with `--force`, `-f`, `--force-with-lease`,
  `--mirror` or `--delete`. An entry of the same name in your config replaces a built-in.
  A session started by another session may run only under a policy no wider than its
  starter's. The policy is resolved and stored with the session at launch (a `policy` event),
  so editing the config later changes new sessions, not running ones; a resume keeps it.
- **Matching.** A command line is split at `&&`, `||`, `;`, `|` and `&`, and is allowed only
  when every command in it is; a deny anywhere wins. A pattern names a command's leading words
  (`git push`) and flags that may appear anywhere after them (`git push --force` also catches
  `git push origin main --force`; `rm -r` also catches `rm -rf`). Git's own options before the
  subcommand (`git -C dir push`) do not hide it, and wrappers (`env X=1`, `nohup`, `timeout`,
  `sudo`) are looked through; `sudo` always goes to the owner. Command substitution, `eval`, a
  heredoc and inline interpreter code (`sh -c`, `python3 -c`, `node -e`) go to the owner unless
  the policy sets `scripts: allow`. Edits and paths inside the session's folder or worktree are
  allowed; anything outside goes to the owner. Relative paths in path dialogs resolve against the directory
  the shell command cd'd into when that directory is inside the session's folder, else against the session folder.
- **At launch.** Copilot starts with every tool allowed (`--allow-all-tools`) and the policy's
  denies as `--deny-tool=shell(...)`. Its rules name a command and subcommand but no flags, so a
  deny with a flag becomes the whole command (`git reset --hard` denies `git reset`), unless the
  policy allows that command (the reviewer's `git push`), when it is left to the dialogs. Claude
  Code keeps its permission bypass and gets a `PreToolUse` hook on its shell tool that refuses
  what the policy denies (see [AGENT_HOST.md](AGENT_HOST.md)). For both, `otherwise` and
  escalations apply only to the dialogs they still show: they do not ask before other commands.
- **In the supervisor.** When a session with a policy blocks at a command, edit or path dialog
  HQ can read, the policy decides. Allowed: HQ picks the one-time yes (never a "remember"
  option). Denied: HQ declines and tells the agent which rule. Anything else becomes the usual
  "waiting for you" alert, with a line saying why the policy left it to you. An answer that
  does not take raises the alert after all. Questions and trust dialogs always go to the owner.
  Each decision is a `policy_decision` event (command, decision, rule, outcome), and the task gets
  one summary line when the session finishes a turn or exits: how many commands the policy
  allowed, declined (by rule) and left to you since the last one.
- **Drive** does not answer a command dialog of a session with a policy: the policy decided
  it, and what it left open is the owner's. A remote client may still answer an escalation.

### What actually stops each deny

A deny guards against a slip. What stops the action for certain is the agent not having what
it needs to do it:

| Deny | What stops it for certain |
|------|---------------------------|
| `git push` | The builder holds no git credential: the sandbox hides the credential stores (`agent_host.sandbox.hide_credentials`, on by default) and HQ passes no token. Give push rights only to a reviewer's profile, through a harness profile's `env`. |
| A deploy, a migration on a shared database | The deploy key lives only in the owner's or the reviewer's environment (a harness profile's `env`), never the builder's. |
| `git stash`, `git reset --hard`, `git clean`, `git checkout --`, `git restore` | A worktree per writer (see below): another agent's changes are not in the tree these act on. |
| `rm -r` | A worktree, and commits: what was committed comes back from git. |
| `env`, `printenv` | The sandbox's allowlisted environment: HQ's own tokens and other agents' environments are not in it. |

## Agents that share a repository

Two agents editing one working tree overwrite each other, and one running `git stash` or
`git checkout` takes the other's changes with it. HQ keeps writers apart:

- **A worktree of its own.** `harness_session_spawn`, `harness_session_handoff`, `hq sessions
  spawn --branch <name>` and the web's New agent dialog ("Its own branch") take `worktree`:
  - `{"branch": "<name>"}` for a writer. The host makes a git worktree of the folder's
    repository at `<HQ folder>/worktrees/<repository>/<session>`, on that branch (made from the
    folder's current commit if it does not exist), and the agent starts there. A branch
    checked out in another worktree is refused, so a branch has one writer.
  - `{"branch": "<name>", "from": "<ref>"}` makes a new branch at that ref instead of the
    folder's current commit (`hq sessions spawn --branch <name> --from <ref>`). `from` is
    ignored when the branch exists, and it is refused with `detached`, which is already at its
    commit. Use it to start a worker at its lead's branch.
  - `{"detached": "<commit or ref>"}` for a reviewer, which gets that commit checked out
    detached.
  - The worktree counts as approved because it comes from a folder that already is. The reply
    has `worktree` (`path`, `repo`, `branch` or `detached`, `head`, and `from`, `copied` and
    `not_copied` when they apply).
  - **Files git ignores.** A new worktree has none of the repository's untracked, ignored files
    (`.env`, `node_modules`). A `.worktreeinclude` file at the repository root names the ones to
    bring along, in gitignore syntax:

    ```
    .env
    .env.local
    node_modules/
    target/
    ```

    Only paths the file names and git ignores are copied, up to 500 paths, from the folder the
    session was started in. They are copy-on-write clones on APFS, btrfs and XFS, and plain
    copies elsewhere. The worktree's own `.worktreeinclude` (the one at the commit it starts
    from) is read first, then the folder's, which may be untracked. A path the worktree already
    has is left alone. Copying happens when the worktree is created, not on a retry or a resume
    that finds it again. What was copied is in `worktree.copied`; paths that could not
    be are in `worktree.not_copied`, and a failure never fails the launch.
  - **A port per session.** Every session's environment has `HQ_PORT`, a number from 20000 to
    29999 worked out from the session id, so two agents' dev servers do not collide. A restart
    gets the same one.
  - **Where it stands.** `harness_session_status` reports `worktree_status` (`changed` files,
    `ahead` and `behind` the commit the session started at, `branch`, `head`) for a session with a
    worktree still in place. If the host cannot say, the reply has `worktree_status_error`
    instead. `harness_session_list` does the same for running sessions when passed
    `worktree_status: true`, with one host call per session; without it the list makes none.
  - `agent_delegate` takes `worktree: true` for a branch named for the worker's sub-task
    (`hq/<task id>`), or the same object.
- **Or paths it owns.** Without a worktree, a coding session (`purpose: code`) in a folder
  another running coding session writes in is refused unless both name `owns` paths (relative
  to the folder) that do not overlap. No `owns` means the whole folder. Not counted: a
  reviewer, monitor or researcher, the new session's own parent (it delegated the work and
  waits on it), and a session the host no longer has.
- **A reviewer sees commits, not a moving tree.** When a writer finishes a turn, the
  supervisor reads the commit its folder is at. A new one is named in the finished alert, in
  the message to whoever delegated it and to the sessions observing it ("committed <sha>
  \"subject\" on branch <b>"), and recorded as a `commit` event. A reviewer then checks out
  that sha detached in its own worktree, so it reviews exactly what was committed.
- **Cleanup.** At `harness_session_stop`, or when the agent exits, a worktree with nothing
  uncommitted is removed; one with changes is kept and the reply or the exit message says
  where. The branch keeps every commit either way. `harness_session_resume` makes the
  worktree again on its branch, or finds it where it was kept.

## Sessions that work on a task

`harness_session_spawn` takes an optional `task_id` (an id or a display id such as
`FR-053`). The task is then the durable record of the work, stored in the session's
`mission_id`, and `harness_session_list` with `task_id` shows every session launched for
it. The task must exist and not be complete, and that is checked before anything is
launched. `harness_session_link` attaches a session that was started without `task_id`;
linking a running session counts as a launch in the table below, and linking one that
already ended only records the link. Each event leaves a comment on the task, and the web Tasks view picks it up at
once. Comments never quote the agent's screen: pane text is untrusted input, and task
comments are read back as trusted, so they point at `harness_session_logs` instead.
Status moves only from the states listed here, and the supervisor never sets `complete`: the
agent saying it is done is not verification. With `tasks.verification` off, the owner decides.
With it enforced, HQ's verifier completes a task only from evidence the session could not write
(a merged pull request with green required checks, a note's content hash, a probed URL) and
sends it back to the agent with the findings when that evidence is missing or says otherwise
(`docs/TASK_VERIFICATION.md`).

| Event | Task status | Notification |
|-------|-------------|--------------|
| Launched or resumed | `to_do`, `blocked` or `ready_for_review` becomes `in_progress` | none; the reply to the launch says it |
| New instruction sent | same as launch; commented only when the status moved | none |
| Working or idle, every sweep | unchanged, no comment | none |
| Blocked on a dialog | unchanged, comment | action-needed item, sent by the next value-bus run (every 5 minutes) |
| Finished a turn (`done`) | `in_progress` becomes `ready_for_review` | relay message, batched with other updates |
| Exited while no other session of the task is running | `in_progress` becomes `blocked` | relay message sent as an interrupt |
| Exited while another session of the task still runs | unchanged | relay message, batched |
| Stopped by hand | unchanged | none |

Each event is recorded right after the registry claim that makes it happen once
(`claim_state_alert`, or the running-to-exited update), and before the exit summary, so
the recorded event survives a daemon restart or a sweep cut off at its timeout. It is
recorded only once.

Offline behavior:

- **Laptop asleep or off the tailnet.** No event is produced and the task does not move. A
  session on that host still counts as running, so another session's exit does not block
  the task. When the host answers again, whatever happened meanwhile is reported on the
  next sweep. A `done` that was followed by an exit while the host was away shows up as
  the exit.
- **Daemon down.** Nothing is watched. On start the supervisor sweeps within a minute and
  reports anything that changed, once.
- **No chat to push to.** Interrupts and value-bus items wait in the relay's pending queue
  until a chat is known. Batched updates are dropped, but the task comments still hold
  them.

A session whose `mission_id` matches no task is treated as having none. Rows written by the
retired mission engine are like that.

## Handing work over from outside (`harness_session_handoff`)

A client that is not a web chat (an MCP client such as another agent) can start tracked work
in one call. `harness_session_handoff` takes `title`, `description`, `acceptance` (alias
`done_criteria`), `harness`, `cwd`, optional `host` (default local), `external_id`, `task_id`,
`prompt`, `space_id`, `initiative` and `drive`. It then:

1. Files an HQ task (acceptance criteria are appended to its description), or reuses the one
   `external_id` already names in that space, or works on `task_id`. The two are alternatives.
2. Starts one session linked to the task, with the task as its goal and `acceptance` as its
   definition of done. With no `prompt`, the agent is told the task title, description and
   acceptance.
3. Creates a web chat thread that owns the session (`owner_thread`), so ready-for-review,
   blocked and exited notices post there and Drive follows the usual gate and
   `agent_host.drive_new_watches` (pass `drive: false` to opt out).

The result carries the task id and display id, session id, thread id and
`links.chat` (`/chat?thread=<id>`) and `links.task` (`/tasks?task=<id>`), and `handoff` says
what happened: `started`, `blocked_at_dialog` (the agent stopped at a dialog, so the prompt
was not typed and nothing is running yet; answer with `harness_session_send` keys) or
`existing_session`.

It is idempotent. The same `external_id` returns the same task (`task.deduplicated: true`),
and a task that already has a session its host still reports as running gets no second one.
A session marked running that its host no longer lists is stale and does not block a new one.
A host that cannot be reached when the task already has a running session is reported as
unknown, and no second session starts. If the launch fails (host unreachable, binary missing,
agent never started) the call errors, the unused thread is archived, no session row is written,
and the task stays so a retry reuses it. If the agent stops at a startup dialog instead, the
session stays registered and the task is set to `blocked` with a comment saying so (the comment
does not quote the pane, which is untrusted); the next instruction sent to the session moves it
back to `in_progress`. The check and the launch share one process-wide lock; two separate `hq`
processes sharing a database could still both start a session.

### Launch bounds, preflight and disconnects

A harness that cannot start used to leave the call waiting past an MCP client's roughly 60
second transport limit, with an orphan pane behind it. Now:

- **Preflight (local hosts, conservative).** Before anything is launched, HQ looks for the
  harness binary (`claude`, `agy`, `cursor-agent`, ... or the first word of a profile's
  `command`). It only does so when that word is a plain program name or an absolute path with no
  spaces, and the command has no `=`, `$`, `~`, quote, `&`, `;`, `|`, `<`, `>`, backtick or
  parenthesis. An env prefix (`CLAUDE_CONFIG_DIR=x claude`), `cd x && claude`, a quoted path or a
  `$VAR` is left to the shell and never blocks a launch. Search roots: the profile's `PATH`
  entries (literal `$PATH` and `~` entries skipped), then HQ's own `PATH` (never replaced), then
  `~/.local/bin`, `~/.bun/bin`, `~/.cargo/bin`, `~/.npm-global/bin`, `~/.volta/bin`,
  `~/.local/share/mise/shims`, `~/.asdf/shims`, `~/.local/share/pnpm`, `~/.claude/local`,
  `~/.nvm/versions/node/*/bin`, `/usr/local/bin` and `/opt/homebrew/bin`. A miss fails at once
  with `harness 'claude-code' is not installed on host 'local' (binary 'claude' not found on
  PATH; searched ...)`, listing the roots. Remote hosts are not preflighted: the gate does not
  expose their filesystem.
- **Bounded start.** The wait for the agent to leave `launch_pending` is `agent_host.launch_bound_secs`
  (default 25, clamped to 5..=50 so it stays under the transport limit). If the last poll still
  shows `launch_pending` with an `unknown` status, or no agent, HQ reads the pane's last lines,
  closes the host workspace and fails with the host, the harness, what the host reported and those
  lines. An agent whose last poll shows `launch_pending: false` is never closed. No row is
  written, so no session claims to be running. Any other failure between the workspace being
  created and the row being recorded (the agent lookup, the database write) also closes the
  workspace; if the close fails the error says so and names the workspace.
- **One launch per agent.** A second launch of the same agent name on the same host (a resume
  retried while the first is still waiting) is refused with "already in progress".
- **Disconnects.** `harness_session_handoff` and every launch (spawn, resume) run on their own
  task, so a client that drops the connection cannot skip the cleanup or leave a thread, a
  workspace or the process-wide lock behind. The bound above is what makes a real error likely
  to arrive before the client gives up. The one launch step that can still outlast the limit
  is accepting a vouched trust dialog (Antigravity), which waits up to 120 seconds.

`task_create` takes the same `external_id`: unique per space, 200 characters at most, a repeat
returns the existing task with `deduplicated: true`. The REST `POST /api/tasks` accepts it too.

### Refusing working directories

Every `cwd` must be an absolute path with no `..` component and none of `~`, `$`, a backtick
or NUL, so the string that was checked is the string the host receives.

A `cwd` shaped like a home directory is refused whichever host the session runs on, because
the agent stops at its folder-trust dialog there and nobody is watching: `/`, `/Users/<name>`,
`/home/<name>`, `/root`, `/var/root`, `/opt/hq`, the bare `/Users` and `/home`, and HQ's own
`$HOME`. Trailing and doubled slashes and letter case do not get past it. There is no
per-host home setting yet, so a home that is not shaped like one of these (say `/data/me`) is
only caught through `agent_host.spawn_cwd_deny`.

`agent_host.spawn_cwd_deny` is a list of path fragments. A `cwd` that contains any entry
(case-insensitive, trailing and doubled slashes ignored on both sides) is refused by
`harness_session_spawn`, `harness_session_handoff`, `harness_session_resume` and
`hq sessions spawn`, with an error naming the entry. It is empty by default. The match is on
the path as written, so a symlink into a denied directory is not caught, and a remote host's
paths are matched as text.

```yaml
agent_host:
  spawn_cwd_deny:
    - /clients/acme
    - /private
  handoff_cwd_allow:       # empty means the handoff key is unrestricted
    - /srv/projects
```

### Scoping an MCP client

Give an external client `AGENTHQ_HANDOFF_API_KEY` instead of the full key. It reaches the
read tools (except `harness_session_logs`) plus `task_create`, `task_update`,
`task_comment_add`, `harness_session_spawn` and `harness_session_handoff` (see
`docs/security/WEB_AUTH.md`). It cannot stop, resume, relink, re-goal, send to or read the
output of sessions, delete tasks or write the vault. Send and logs are withheld because the
registry does not record which key started a session. It can also ask HQ's own chat agent a
read-only question with `hq_ask` (see `docs/MCP_ASK.md`), which is not a harness session. That
reply's session has the session-log, the host and file-reading tools removed, so the key cannot read
through HQ what it cannot call.

**This key is code-execution equivalent.** A spawned `claude-code` session runs with
permissions skipped, on any host and `cwd` the deny list allows. Set
`agent_host.handoff_cwd_allow` to the directories the client may use: a call that arrived on the
handoff key must then have a `cwd` in or under one of them (the gateway marks those calls, a
client cannot set the marker). Left empty, treat the key as shell access to those hosts as
described under "Security model".

### Folders the owner approves

A claude.ai connector, or the handoff key while `handoff_cwd_allow` lists anything, that asks for
a `cwd` outside its folders does not get a dead end. HQ files an approval request for that folder
on that host (your chat, the web inbox and a push notification), and the spawn or handoff call
waits for your answer:

- Approve within the wait and the session starts in that same call. The folder, with everything
  under it, is then allowed on that host until you revoke it.
- Dismiss and the call fails at once saying you declined. The caller is not asked again until the
  request expires (24 hours).
- No answer within the wait and the call fails with an error that starts `BLOCKED: no session was
  started`, telling the caller to say it is waiting on you and to call again with the same
  arguments. Each call waits again, so a caller that keeps calling starts the session as soon as
  you approve. Nothing about the error reads as a started session.

The wait is 55 seconds less `launch_bound_secs`, at most 30 seconds, so the launch that follows an
approval still finishes inside an MCP client's one-minute limit. One caller may have at most 5
requests waiting. Home and root folders and anything `spawn_cwd_deny` matches are refused before
any request is made.

Approving is an owner action (the chat button, the web inbox or `hq queue approve`, all signed with
the install key); no MCP key reaches it. The live grants are listed at `GET /api/session-folders`
and revoked with `DELETE /api/session-folders/<id>`, both behind the web credential.

Keys sent to a session (`keys` on `harness_session_send`, `host_send` and the REST
endpoint) must be logical names matching `^[A-Za-z0-9][A-Za-z0-9+_-]*$`, at most 32
characters, so a key can never be read as a host flag.

## Watching and driving from a web chat

A session launched or linked from a web chat is watched by that chat (`owner_thread`), and
`harness_session_watch(session_id)` makes a chat watch any session. The chat header shows
"Watching N", with each session's task, live agent status and a Drive switch
(`/api/threads/{id}/sessions`, `/api/harness-sessions/{id}/drive`, `.../unwatch`). On a
phone the same panel shows under the chat header with 44px controls, and the button counts
the sessions waiting on an answer.

A session no chat watched starts with Drive on (`agent_host.drive_new_watches`, default true)
when it passes the goal gate below; without a usable goal it starts observation-only. Spawn, link and watch take `drive=false` to start with updates only, and on a session the
chat already drives it stops driving. Every other case starts or stays off, so a tool can
never turn Drive back on once the user switched it off:

- A chat watching a session again keeps its switch where the user left it.
- A session taken over from another chat starts with Drive off.
- A turn that has read untrusted content (pane text, the web, mail, vault notes) starts its
  new watches with Drive off: governance marks those calls (`untrusted_turn`).
- A turn the session driver started never starts a new watch driving.
- A turn in a chat that `hq_ask` opened never starts a new watch driving, and neither does a
  handoff made from one.
- `drive=true` on spawn, link and watch is refused. The one way for HQ to turn Drive on is
  `harness_session_mode` (below), which the gate and an untrusted turn both override.

Sessions already watched when this default shipped keep their stored switch; nothing is
backfilled.

A watched session sends nothing to Telegram, Discord or the value bus. The supervisor turns
each finished turn, block and exit into a durable wake on the row (`pm_wake`), and the web
server's session driver (`crates/hq-web/src/session_driver.rs`) answers it in the chat:

- **Watching only.** A short update names the session, what happened and where its task
  stands. It never quotes the screen.
- **Driving.** HQ runs a turn in the chat as the session's project manager: it reads the
  session, then sends the next instruction, approves a permission prompt, resumes an exited
  session, or leaves a working one alone, all toward the linked task's goal. It asks you in
  the chat instead of acting when the step is outside the task, destructive, touches
  credentials, deploys to production unless the task says so, or spends money. It never
  marks the task complete. A driven session also gets a check-in every
  `agent_host.driver_checkin_minutes` (default 30) while it runs and its host answers, to catch
  an agent that is stuck. After 48 driver turns in a day a session drops back to updates.
  A chat that is archived or deleted lets go of its sessions, whose events return to the
  relay. The limits in "Limits on driving" below stop a session that keeps being nudged.
  These turns are ordinary chat replies tagged with the session, so their tool calls show
  exactly what HQ sent. Stop ends one like any reply.

A wake is claimed before it is acted on, and a chat with a reply already running is retried
on the next pass (every minute, or at once when the supervisor writes a wake in the same
process). If the web server is not running, wakes wait on the row. Driving approves prompts
after reading pane text, which can carry instructions planted by content the agent read;
turn it on only for sessions whose work you would approve yourself.

### Limits on driving

What wakes a driven turn: a finished turn, a block or an exit that the supervisor turns into a
wake (`pm_wake`), and a check-in every `agent_host.driver_checkin_minutes` while the session runs.
What used to stop it: the model deciding the goal was met, and 48 driver turns a day. A model
that judges an unreachable definition of done (a physical-device test, say) as unmet kept
sending another wrap-up instruction, each answered by a finished turn that woke it again.
These limits do not depend on the model deciding to stop. Each one switches Drive off, posts
one notice in the chat (fixed text, never pane text), records a `drive_off` event with actor
`guard` in `harness_session_events`, comments on the linked task, and keeps the reason on the
row (`drive_off_reason`, in the sessions payload and shown under the Drive switch):

| Limit | Default | Setting |
|-------|---------|---------|
| Instructions (text prompts and resume prompts) the driver sends one session, counted since the user last switched Drive on. Claimed in one conditional write before the send, so parallel calls cannot overshoot, and handed back if the send fails. `harness_session_send` and `harness_session_resume` in a driver turn refuse past it, and `host_send` is removed from driver turns so it cannot go around it | 8 (1 to 100) | `agent_host.driver_nudge_budget` |
| Key presses (permission dialogs, menus) the driver sends one session, on the same terms but a separate, larger allowance so a task with many dialogs is not cut off | 40 (1 to 500) | `agent_host.driver_key_allowance` |
| Finished turns in a row that show no new tool activity on the screen the supervisor stored (`Bash(...)`, `Update(...)`, `Ran ...` lines). A tool line the last turn did not show, or one shown more times, counts as work, so re-running a test is not a stall. A finished turn that follows something other than a driver instruction (you typing in the chat) is not judged. A harness that prints none is never judged stalled; the budget covers it | 3 (2 to 20) | `agent_host.driver_no_progress_limit` |
| The linked task is complete, or is not in progress at a check-in while the agent is not working (a working agent means someone is typing in the pane). A finished turn itself moves the task to ready for review, so that is not a stop | none | none |
| The session exited or was stopped (Drive goes off in the registry write, and the exit is reported as an update) | none | none |
| Running sessions HQ drives at once. A default-on watch past it starts observing and says so, and `harness_session_mode` from HQ cannot go past it (the user can); a session the user already switched on is never switched off by it | 3 (1 to 50) | `agent_host.max_driven_sessions` |
| Running sessions started by an MCP client with no chat (`harness_session_spawn`, `harness_session_handoff`), recorded in `harness_sessions.origin` | 3 (1 to 50) | `agent_host.max_mcp_started_sessions` |
| Running sessions started from a chat an `hq_ask` created, including by handoff | 2 (0 to 20) | `agent_host.max_ask_spawned_sessions` |
| Full-mode `hq_ask` questions waiting at once | 2 (1 to 10) | `agent_host.max_full_asks` |

Known harmless prompts are not left to the model. Claude Code's optional feedback survey
(`How is Claude doing this session? (optional)` with `0: Dismiss`) used to stall a driven loop
until a person pressed `0`. The supervisor (`harness_session::dismiss`, called from
`session_supervisor.rs` on its minute sweep) now dismisses it by text match, not judgement:

- Both exact lines (the title and `1: Bad  2: Fine  3: Good  0: Dismiss`) must sit within the last
  12 non-empty lines of the screen, so the same words in older scrollback are ignored. The screen
  is read again just before the key goes out, so a survey that already closed gets no stray `0`.
- The key is the constant `0` (a bare `0` works with `agent send-keys` on a real claude-code
  session). A tail that shows a dialog needing a human is never touched: `do you trust`, `do you
  want to`, `would you like`, `permission to`, `requires permission`, `enter to select`, `esc to
  cancel`, `(y/n)`, `allow this`, `no, exit` and `yes, proceed` all veto. The word "permission"
  alone does not, because Claude's bypass-mode footer (`bypass permissions on`) always sits in
  the tail. A prompt without a recognised dismiss option is never answered, and trust dialogs
  still default to "No, exit" and are reported.
- Nothing is typed while the host reports the agent `working` (the text would be the agent's own
  output), and nothing is typed when the input line below the survey holds any text. The input
  line is recognised by its glyph: Claude Code's `❯` (U+276F), `>`, or either inside a `│ … │`
  box. A ghost suggestion looks like a draft and cannot be told apart from screen text, so it
  counts as one and the survey is left for a person.
- After a dismissal the supervisor remembers a hash of the screen tail
  (`harness_sessions.last_dismiss_tail`). If the next sighting has the same tail, the key did
  nothing: HQ types nothing more, records a `dismiss_cap` event ("prompt did not close") and
  posts the same single notice as the cap. Together with the draft check, a survey lookalike
  can cost at most one stray `0`. Switching Drive on clears the hash.
- Host calls are bounded: at most one screen re-read and one key send per session per sweep, each
  with a 3 second timeout. The screen is read again just before the key goes out, so a survey
  that already closed gets no stray `0`. A send the host refused is handed back; a timeout or lost
  connection keeps the claim spent (the key may have landed, and a second `0` could reach the
  input box).
- A finished turn is never silenced by a dismissal: its alert and driver wake fire as usual,
  even if the host changes the session's state after the keypress. Only the blocked alert the
  survey itself caused is skipped.
- Only sessions with Drive on and a watching chat. An observe-only session is never typed into.
- Dismissals have their own counter (`harness_sessions.dismissals`, refilled when the user
  switches Drive on) and cap of 5. They spend neither the instruction budget nor the key
  allowance. The sixth sighting sends nothing, records a `dismiss_cap` event and posts one notice.
- Each dismissal is a `prompt_dismissed` event with actor `guard`; the linked task gets one
  comment per session, not one per dismissal.

A second finished turn with no instruction sent since the driver last answered one is posted as
a plain update, not driven. Anything other than the driver sending the session something (the
user typing in the chat, the Sessions page) re-arms that. Notices never create a wake.

Only the user switching Drive on (the panel switch or `POST .../drive`) refills the budgets and
clears the stall streak. A driver turn or an `hq_ask` reply cannot turn Drive on with
`harness_session_mode`, and an HQ-initiated switch does not refill anything: if HQ switches Drive
back on after a budget stop, the next wake switches it off again.

Driver turns (the session driver, not sub-agent follow-ups) may not start, hand off, attach or
link sessions, and cannot type into a pane except through the metered tools. Attach and link are
also refused in `hq_ask` replies. Sessions started over the handoff key never start driven. After a guard stops Drive, the
session keeps running untouched and later events arrive as ordinary updates.

`agent_host.drive_new_watches` stays `true`: a handoff or spawn whose goal and definition of done pass
the gate starts driven, bounded by the limits above. Set it to `false` for every new watch to
start observing.

### Sessions HQ spawned start sessions only as their team

Every pane HQ launches has `HQ_SESSION_ID=<session id>` in its environment. An MCP client in
that pane can pass it on, and the gateway then marks the call as coming from an HQ-spawned
session (the marker only adds restrictions, so the caller cannot gain anything by setting or
omitting it). A marked call to `harness_session_spawn`, or one that names its session in
`from_session`, becomes a delegation from that session: the new session is its worker, on a
sub-task, in its folder and host, under the team limits in `agent_host.teams` (see "Delegating" in
`docs/AGENT_MESSAGING.md`). A marked `harness_session_handoff` is refused, and `hq_ask` is refused
in `mode: full` (`read_only` still works).

- **HTTP `/mcp`**: the client config sends header `x-hq-session-id` filled from the variable.
  In the Claude Code MCP entry for `agent-hq` (verified against Claude Code 2.1.x on 2026-10-04:
  it expands the variable in HTTP headers, and a blank value arrives as an empty header):
  `"headers": {"x-hq-session-id": "${HQ_SESSION_ID:-}"}`. A blank value means no marker. A live
  session started by HQ with this entry was refused `hq_ask` in full mode. Add the header to every Claude profile's `agent-hq` entry
  (`~/.claude.json`, and `.claude.json` in each `CLAUDE_CONFIG_DIR`); HQ does not edit client config.
- **stdio `hq mcp`**: reads `HQ_SESSION_ID` from its own environment when the client passes it on.

A marked call also cannot use `harness_session_resume` or `config_manage`, and can type
(`harness_session_send`, `host_send`) only into its own session.

**This depends on that client config, which HQ does not write, so containment fails open
without it** (see `TECHDEBT.md`). HQ does not pass a per-pane `--mcp-config`. Without the marker
these caps still hold, whatever the client says about itself: a watch past
`agent_host.max_driven_sessions` does not drive; a session started from a chat that `hq_ask` created
never starts driven, and is capped at `agent_host.max_ask_spawned_sessions`; sessions an MCP client
starts with no chat are capped at `agent_host.max_mcp_started_sessions`; full-mode asks wait at most
`agent_host.max_full_asks` at a time; an `hq_ask` reply loses `config_manage`.

### The Workbench page

The Workbench page (`/sessions` in the web app, named Sessions before) lists every registry row with its task, host, goal, live agent
status and whether its host answers (an unreachable host shows "host unreachable", never
"exited"). Picking one shows the pane text (refreshed every 3s), a prompt box, the keys that
answer a dialog, an Adopt button, and the command that lists the agents on that machine
(`hq host status`, or `ssh <host> hq host status` for a remote host; the page notes that this
assumes the host name is an ssh alias on your machine). `ctrl+c` sits apart from the other keys and needs a
second tap. A task's drawer lists its sessions and links here. All of it sits behind the same
web auth as the rest of `/api`, and `send` and `adopt` under `/api/harness-sessions/` must
also carry `X-HQ-Client` (the web app sends it; scripts must add it, or get a 403). `drive`, `goal` and
`unwatch` predate the header and stay open to cached PWAs that do not send it yet. Errors the
endpoints do not recognise are logged and answered generically, never with ssh or the host stderr.

- `POST /api/harness-sessions` starts an agent: `{harness, host?, folder?, new_folder?, prompt?, label?}`.
  The folder is resolved on the computer that runs the agent and must be inside its HQ folder
  (`~/Documents/HQ`; `GET /api/workbench/hosts` reports it, `GET` and `POST
  /api/workbench/hosts/{host}/dirs` browse and create folders there). The agent always starts in
  an approval mode. `POST .../{id}/stop`, `/resume`, `/rename` and `/archive` manage a session;
  an archived session leaves the default list (`include_archived=true` brings it back) and only a
  session that is not running can be archived. All of these need `X-HQ-Client`.
- `GET /api/harness-sessions?task_id=&status=&host=&include_archived=` is `harness_session_list` plus the task
  and goal. `GET /api/harness-sessions/{id}` is one row, live.
- `GET /api/harness-sessions/{id}/screen/stream?lines=N` is the same screen as server-sent events: a
  `screen` event on connect and again only when it changes, an `end` event (`stopped`, `time` or
  `unavailable`) when the session is not running any more or after 15 minutes, and a keepalive
  comment every 15 seconds. At most 8 views are open at once (503 beyond that); the page falls back
  to polling. `screen?styled=true` adds color as ANSI escape sequences when the host can give them
  (`styled` in the reply says whether it did; a stored snapshot is always plain).
- `GET /api/harness-sessions/{id}/screen?lines=N` (default 80, at most 500) reads the pane
  while the agent runs and returns the last stored snapshot after it ends (`source`). A live
  read is one host call (one ssh round trip on a remote host). Concurrent reads of one
  session share a single in-flight call, and a live result is reused for about a second;
  a failed read or a stored-snapshot answer is never reused, and a send, stop or resume
  drops what is held. A session the registry no longer lists as running is answered from its
  snapshot without calling the host. A live answer also carries `host_source` (`recent-unwrapped`
  or `visible`; null for a snapshot). While an agent is working, the host refuses a `recent` read
  of more than about 25 lines with `agent_not_idle`; `ScriptedHost::read` then retries that one
  error once as `--source visible` with the same line count, so the page and the supervisor
  stay live instead of falling back to a stale snapshot. Other errors (`agent_not_found`, an
  unreachable host) are not retried, writes never take this path, and idle agents keep the
  deep `recent-unwrapped` read. The supervisor was left on the same read rather than moved to
  `--source detection`: `detection` is the host's own classifier input with no documented wrapping
  or depth contract, and the stored snapshots and progress hashing assume unwrapped text. The page waits for each response before polling
  again and backs off (up to 30s) while requests fail.
- `POST /api/harness-sessions/{id}/send` takes exactly one of `text` or `keys`, with the same
  rules as `harness_session_send`: text is refused with a 409 while the agent waits at a
  dialog, and a session that is not running or whose host is down is refused too. Each send
  is a `sent` row in `harness_session_events` (key names, or the length of the text, never
  the text itself).
- `POST /api/harness-sessions/{id}/adopt` with an optional `thread_id` makes that chat watch
  a session nobody watches, or creates a chat named "Session: <label>" when omitted. It
  starts observation-only. Repeating it, or naming the chat that already watches it, changes
  nothing; a session another chat watches is a 409 until that chat unwatches it.

### Goal and definition of done (the drive gate)

HQ steers a session only while it can state what the session is for and how anyone could
tell it is finished. Both live on the session row (`goal`, `done_criteria`, migration 063),
for tracked-task and ad hoc sessions alike. Set them with `goal` and `done_criteria` on
`harness_session_spawn`, with `harness_session_goal`, or from the web panel
(`POST /api/harness-sessions/{id}/goal`). A session spawned or linked with a `task_id` and no
goal takes the task's title and description as its goal; the definition of done always has to
be stated.

The gate (`hq_db::harness_drive_gate`) is deterministic, no model involved. Each field must be
non-empty, not a placeholder (`TBD`, `when done`, `it works`, ...), and have minimum substance
(goal: 4 words and 20 characters; definition of done: 3 words and 15 characters), and the
definition of done must not just repeat the goal. When the gate fails, HQ stays
observation-only and the refusal names what is missing, in the tool result, the REST 409 and
the Watching panel. Editing the goal so it no longer passes switches a driven session to
observing on the spot, and the driver re-checks before every turn. The driver prompt carries
the goal and definition of done. A session that exits or goes idle has not thereby met its
goal: the task moves to review at most, and a person completes it.

Every goal change, drive change, refusal and attach is a row in `harness_session_events` with
the goal and criteria in force at the time (`registry::list_events`).

### Attaching and switching mode mid-chat

- `host_agents` lists what the host runs, including agents started by hand.
  `harness_session_attach(agent, host)` starts tracking one and makes the chat watch it,
  observation-only. An agent already tracked is reused, and another chat can take it over
  (Drive off). An unreachable host or a missing agent is an error and records nothing. Family
  guests cannot attach.
- `harness_session_mode(session_id, mode)` switches HQ between `drive` and `observe` for a
  session the chat watches. `observe` always works and takes effect at once: the driver reads
  the row before every turn, so at most the step already in flight finishes. `drive` needs a
  live agent (an ended session or an unreachable host is refused, since HQ cannot confirm
  what it would steer), a passing gate, and a turn that has not read untrusted content. The
  Drive switch in the panel is the same mode change.
- Switching mode never touches the agent: it is not paused or stopped. `harness_session_stop`
  is the only tool that ends it, and it is never called as a side effect of a mode change.
  After a resume the row keeps its goal, so Drive can be turned on again.

## General-purpose agents

Setting one up, everyday use and removal are in [PERSONAL_AGENTS.md](PERSONAL_AGENTS.md).
This section is the reference.

Hermes (`hermes chat`) and OpenClaw (`openclaw tui --local`) are a second category of
harness, `general_purpose`, next to the coding agents. They run in the same host, the same
process sandbox and the same session registry, and the same `harness_session_*` tools, web
page and `hq sessions` commands start, read, steer and stop them. What differs is what HQ
hands them, and that is mechanical rather than a matter of prompt wording:

- **Never code.** Every session records a `purpose`: `code`, `monitor`, `research`,
  `review` or `general`. A coding agent defaults to `code` and takes any purpose. A
  general-purpose agent defaults to `general` and refuses `code` before anything is started,
  naming the coding harnesses instead. Profiles built on a general-purpose base
  (`base: hermes`) inherit the refusal. HQ's own chat prompt is told the same rule.
- **Monitoring, research, review.** `purpose: monitor` and `purpose: review` sessions are
  about someone else's work. Linked to a task, they leave comments on it (`Session ... (hermes,
  a monitor) finished a turn.`) but never hold its work lease or move its status, so a
  reviewer finishing a turn does not put a coding task into review. `research` and `general`
  sessions do their task's own work and move it like a coding session does. Several research
  sessions can run at once; each is an ordinary session with its own task.
- **Observing other sessions.** A monitor or reviewer started with `task_id` can be given
  `observe: [session ids]` (at most 10). When an observed session finishes a turn, blocks at a
  dialog or exits, the supervisor leaves the observer a message on the observer's task thread,
  with the observed agent's last reply or screen tail, and types it in when the observer is
  next idle, the same route a delegated child uses to report to its parent. The text is the
  observed agent's screen and is presented as such, never as an instruction. Spawn reports
  `observing` and says so in `observing_note`. The ids are checked before anything launches.

```
harness_session_spawn(harness: "hermes", purpose: "monitor", task_id: "FR-053",
                      observe: ["hs-claude-code-1a2b3c"],
                      prompt: "Watch the refactor session and tell me when it stalls or asks for a decision.")
hq sessions spawn hermes --purpose review --task FR-053 --observe hs-codex-4d5e6f --prompt "Review the diff when it lands."
```

Security, which is the point of running these agents through the host at all:

- They are confined like every other agent: the process sandbox and no read of the host's token
  or your credential directories. Under `profile: strict` that also means a private copy of the
  temporary directories and an egress allowlist, and their own state in `~/.hermes` or
  `~/.openclaw` is writable for them and hidden from other agents. They reach whichever model
  provider you configured for them; under `strict` add that provider's host to
  `agent_host.sandbox.allow_domains`, as HQ ships no default for it.
- They get no HQ session token and no MCP configuration. HQ never wires itself into a
  general-purpose agent's own tool loop; what reaches the agent is what HQ types into it, and
  what comes back is read from its screen, which `harness_session_logs` treats as untrusted.
- They count against the same origin and driven-session caps, their folders pass the same
  deny and allow lists, and a family guest's harness allowlist does not include them.
- The host has no state rule file for them yet, so their state shows as `unknown` rather than
  `idle` or `working`: `harness_session_wait` cannot wait on them, the finished-turn alert does
  not fire for them, and a message for them is typed once the launch is over rather than at the
  next idle. A rule file for each, built into the host like the Claude Code and Codex ones
  (see [AGENT_HOST.md](AGENT_HOST.md), "Agent state"), fixes all three and is the next step.
  A stack run (below) needs none of this: its process ending is its signal. Resume is a fresh
  start that the agent's own session store may continue.

The launch commands are the ones each project documents; an install that starts differently
gets a profile (`base: hermes`, `command: ...`, `args: [...]`), and a profile may reuse the
built-in name.

OpenClaw runs with `--local`: its default talks to the OpenClaw gateway on a loopback port,
and the sandbox refuses loopback connections. OpenClaw refuses `--local` while a gateway runs
for the same state directory (`A Gateway is running for this state directory ... Run without
--local to use it, or stop the Gateway first`), so on an HQ host do not run OpenClaw's gateway
daemon (skip `--install-daemon` when you set it up). If that machine needs the gateway for
something else, give HQ a harness profile for OpenClaw instead.

### The Agentic Stack: specialists that pull their tasks

An interactive pane is the slow way to use these agents. The faster one is to name them in
your stack and assign them tasks. The words are Kola Academy's: a *stack* of *specialists*,
each with a lane, directed toward outcomes a person reviews
(`docs/plans/agentic-stack.md`).

```yaml
stack:
  specialists:
    - name: hermes                  # tasks assigned to "hermes" are its queue
      harness: hermes               # a general-purpose harness or a profile on one
      purposes: [research, general] # its lane; first is the default; code is refused
      cwd: /home/me/Documents/HQ/stack-hermes   # optional: a folder of its own by default
      run_minutes: 30               # a run past this is stopped and the task blocked
      runs_per_day: 24              # the budget, counted from the session registry
```

- **Trigger.** Assign a task to the specialist's name (web board, `task_update`, chat). Every
  minute the daemon's `stack-dispatch` asks the queue for that name's next task, the same pick
  `task_next` makes (open, dependencies done, nobody holding it, start date reached, most
  urgent first), less anything code-shaped: a task tagged `code` or linked to a commit or
  pull request is never offered to a general-purpose specialist.
- **Run.** One run per specialist at a time. HQ builds a brief from the task (display id,
  title, description as the definition of done, initiative, the resume point from its last
  checkpoint, and what to print at the end) and starts the harness's one-shot command in the
  sandbox: `hermes chat --oneshot -q "<brief>"`, `openclaw agent --local --message "<brief>"`. A profile's
  `one_shot_args` (with `{brief}`) replaces that for an install that differs. The session has
  origin `stack` and label `stack/<name>`, and shows in the Workbench, `hq sessions` and
  `harness_session_logs` like any other. A tag on the task that names a purpose in the lane
  (`review`, `monitor`, `research`) picks the run's purpose.
- **Output.** The process ending is the completion signal. The supervisor keeps the final
  screen as the session's snapshot, closes the pane, and records the run on the task by its
  exit code: zero moves the task from in progress to ready for review and releases its lease;
  anything else is a crash and blocks the task with a reason that names the run and, when it
is short, quotes the last paragraph on its screen, where a CLI prints the error it stopped on
(on a host that reports no code, output on
  screen counts as a finished run and a blank screen as a crash). A run is never resumed:
  fix the cause, then assign the task again or `hq stack run`. You read the output
  (`harness_session_logs`) and complete the task, or not; with task verification on, HQ
  checks the evidence and completes what passes. A monitor's or reviewer's run
  comments and never moves the task, as its interactive turns do.
- **Limits.** `runs_per_day` is counted over a rolling 24 hours. A run past `run_minutes` is
  stopped and its task blocked with the reason. A launch that fails leaves one comment on the
  task and is not retried for 30 minutes. Token cost is not a limit HQ can enforce for these
  agents yet: it reads usage from the coding agents' own transcripts only.
- **Fire now.** `hq stack run <specialist> <task>` starts a run for one task without waiting
  for a sweep, whoever the task is assigned to; the lane, the budget and one-run-at-a-time
  still hold. `hq stack list` shows each specialist with its lane, its queue, what is running
  and the day's count.

What a specialist never gets: an HQ token, MCP wiring, a coding task, or the right to complete
a task. What it keeps: the sandbox, its own state directory, and under `profile: strict` the
egress allowlist, so add its model provider's host to `agent_host.sandbox.allow_domains`. The one-shot arguments are
the ones each project documents and have not been run against a live install here.

## Custom harness profiles

A profile in `config.yaml` names a launcher that is not one of the built-in harnesses (a
wrapper script that selects an account, sets a config directory, or adds flags) and borrows
everything else from a built-in harness:

```yaml
agent_host:
  harness_profiles:
    my-claude:                 # the name callers pass as `harness`
      base: claude-code        # resume, trust and token behavior come from this harness
      command: my-claude-wrapper   # run through a shell; must end up running claude
      args: ["--dangerously-skip-permissions"]   # optional; replaces the base args on a fresh spawn
      env:                     # optional; set for the agent, so for the CLI too
        CLAUDE_CONFIG_DIR: /home/you/.claude-work
```

`harness_session_spawn`, `hq sessions spawn` and session-mode dispatch all accept the profile
name, and `harness_session_resume` finds it again from the stored session. A profile without
`command` launches the base CLI with the profile's `args` and `env`; a resume uses the base
harness's resume arguments. A profile may reuse a built-in name to change how that harness
starts. If no agent appears within the launch bound the spawn fails with the screen; the usual
cause is a command that is not on the `PATH` of the host service.

A separate Claude Code account needs no wrapper: point `CLAUDE_CONFIG_DIR` at that account's
config directory and the host starts `claude` in it (inside the sandbox that directory is the
agent's own, and the default `~/.claude` and other profiles are hidden from it). Write the path
out in full, since `env` values are passed through as written and `~` is not expanded:

```yaml
    claude-work:
      base: claude-code
      env: { CLAUDE_CONFIG_DIR: /Users/you/.claude-work }
```

The spawn result's `harness` field names the profile the session was started with. An unknown
name fails before anything is started, listing the known harnesses and profiles. Profiles run
shell commands on the host, so they belong to whoever can edit `config.yaml`.

## Setup

On the machine HQ runs on, `hq host serve` (or `hq host install`, which runs it as a login
service) provides the `native` host. To use another machine, pair it:
[JOIN_A_MACHINE.md](JOIN_A_MACHINE.md). Settings live under `agent_host:` in `config.yaml`
(all defaults shown, sandbox and idle settings are in [AGENT_HOST.md](AGENT_HOST.md)):

```yaml
agent_host:
  default_host: native          # where new sessions start: native, or a paired host's name
  launch_bound_secs: 25         # how long a launch waits for the agent (5-50)
  command_timeout_secs: 30
  driver_nudge_budget: 8        # instructions the driver may send one session (1-100)
  driver_no_progress_limit: 3   # finished turns without new tool activity before Drive stops (2-20)
  max_driven_sessions: 3        # running sessions HQ drives at once (1-50)
  max_ask_spawned_sessions: 2   # running sessions hq_ask chats may start (0-20)
  max_mcp_started_sessions: 3   # running sessions MCP clients with no chat may start (1-50)
  driver_key_allowance: 40      # key presses the driver may send one session (1-500)
  max_full_asks: 2              # full-mode hq_ask questions waiting at once (1-10)
```

To revoke a paired machine's access, delete its line from that machine's `authorized_keys`
(the one `hq host authorize` added) and remove the host from `agent_host.hosts`.

### Connection reuse (`agent_host.ssh_multiplex`)

Every call to a remote host is one ssh command, and a fresh Tailscale ssh handshake costs
seconds, so HQ keeps one connection per host open between calls (`ControlMaster=auto`,
`ControlPersist=120`, sockets in `~/.hq/run/ssh`, mode 0700, one per host). It also sets
`ServerAliveInterval=15` and `ServerAliveCountMax=2`, so a master whose laptop went to sleep
exits within about 30s instead of holding later calls until their deadline. The key,
`IdentitiesOnly`, `BatchMode`, the forced command and the gate are unchanged: sshd still runs
your `authorized_keys` command for every session on the shared connection.

```yaml
agent_host:
  ssh_multiplex: true   # default; false opens a fresh connection per call
```

The control socket is a bearer credential for the gated key: anything that can open it can
make gated host calls as that key without the key file, so it sits inside the same boundary
as the key. The directory is 0700 and holds nothing else; keep `identity_file` mode 0600 and
owned by the HQ user.

If the socket directory cannot be made private or its path would be too long for a unix
problem before a session opens, a read-only call (`status`, `agent get`, `agent list`,
`agent read`) is retried once on a fresh connection. A write (send, prompt, launch) is never
retried, since the gate may already have run it. A master that hangs rather than failing is
bounded by the normal command timeout. Run HQ with `RUST_LOG=hq_tools::agent_host=debug` to
log the elapsed milliseconds of every host call.

## Security model

The gate limits what the key can ask for, but the host starts arbitrary commands by design
(`agent.spawn`, and any agent it starts can run code). Treat the key as shell access to the
machine as you, scoped to the host's own API and to one source address. Consequences:

- Keep the private key readable only by the HQ user and out of backups you do not control.
- Anyone who can call HQ's `harness_session_*` tools can start an agent on a paired machine,
  and anyone who can call `host_send` can type into any agent pane there, including ones you
  started by hand. Do not expose those tools to callers you would not give a shell.
- Agents run in a process sandbox (see [AGENT_HOST.md](AGENT_HOST.md)): they cannot read the
  host's own files or your credentials directories, and reach only the sites you allow.
- The host's control socket is a local unix socket, so access control is its file permissions
  plus the ssh key above; agents get a separate socket that accepts only their own token.

## Troubleshooting

| Symptom | Likely cause |
|---------|--------------|
| `host_check` or `host_list` shows `unreachable` with `Permission denied` | The pinned key is missing from `authorized_keys` (run the `hq host authorize` command again), the `from=` address is wrong, or `identity_file` is unreadable by the HQ user. |
| `unreachable` with `Host key verification failed` | Trust the machine's host key from the HQ's user: `ssh-keyscan -H <address> >> ~/.ssh/known_hosts`. |
| `unreachable` with `timed out` | Machine asleep, off the tailnet, its host service stopped, or sshd disabled. |
| `unreachable` with `No such file or directory` | The host is not running on that machine: `hq host install`. |
| `gate_denied` | The call used a method the gate does not allow. |
| `agent_not_ready` at spawn | Agent is at a dialog. Read the screen from the spawn result and answer with `keys`. |
| Spawn fails with `cannot sandbox the agent` | No sandbox program works on that machine (install `bubblewrap` on Linux, allow user namespaces on Ubuntu 24.04), or the project directory contains your home. |
| An agent's MCP server or plugin is missing | Under `profile: strict` the sandbox refused its host; see the egress log and add it to `agent_host.sandbox.allow_domains`. The default `open` profile has no allowlist, so look at the plugin's own log. |
| `git` over ssh, `gh`, `aws` or `kubectl` fails inside an agent | The credential stores are hidden from agents by default. Set `agent_host.sandbox.hide_credentials: false`, or reveal one directory to one kind of agent with `agent_readable`. See [AGENT_HOST.md](AGENT_HOST.md). |
| Spawned Claude replies `Login expired` | The user account on that host is not logged in to that CLI. |
| A stack specialist starts nothing | `hq stack list` shows its `problem` (a coding harness, an unknown purpose, a profile without `one_shot_args`), its queue is empty (`task_next` for that name, less code-shaped work), a run is up, or its `runs_per_day` budget is spent. A task whose run could not start is left a comment and tried again after 30 minutes. |
| A stack run ended and its task is blocked | The process exited non-zero (a crash or a refused login). Its last screen is in `harness_session_logs`; fix the cause, then assign the task to the specialist again or `hq stack run <name> <task>`. |
