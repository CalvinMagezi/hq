# Personal agents: running Hermes and OpenClaw under HQ

Hermes Agent and OpenClaw are personal assistants with their own tools, skills and memory.
HQ can start, watch and stop them the same way it runs coding agents, inside the built-in
host and its process sandbox, and it can hand them tasks from your board. HQ calls them
**general-purpose agents** in its tools and config. This page is the operator's guide:
what HQ does with them, how to set one up, how to use it day to day, and how to remove it.

The reference material lives in [AGENT_SESSIONS.md](AGENT_SESSIONS.md) ("General-purpose
agents" and "The Agentic Stack") and [AGENT_HOST.md](AGENT_HOST.md) (the sandbox).

## What HQ does with them, and what it never does

HQ treats a personal agent as a guest it supervises, not as part of itself:

- **It never wires itself into them.** A personal agent gets no HQ session token and no MCP
  configuration. What reaches the agent is what HQ types into its terminal; what comes back
  is read from its screen and handled as untrusted text.
- **It never gives them code.** Every session has a purpose: `research`, `review`,
  `monitor` or `general`. A request with `purpose: code` is refused before anything starts,
  and the refusal names the coding agents to use instead. A task tagged `code` or linked to
  a commit or pull request is never offered to one.
- **It sandboxes them.** Each runs in the same process sandbox as a coding agent: its own
  state folder (`~/.hermes` or `~/.openclaw`) is writable, your credential folders and HQ's
  own folder are not, and network access goes through an egress allowlist with no default
  model provider on it.
- **It does not mark their work done itself.** A run that ends moves its task to ready for
  review. You read the output and complete the task, or not. If you have turned on task
  verification (`docs/TASK_VERIFICATION.md`), HQ checks the evidence instead and completes what
  passes, so you read only the exceptions.

## Before you start

You need HQ with its host running on the machine the agent will run on (`hq host status`
answers), and an account with a model provider for the agent. These steps use OpenRouter;
any provider the agent supports works the same way.

1. **Install the agent** from its own project, following its instructions:
   [Hermes Agent](https://github.com/NousResearch/hermes-agent),
   [OpenClaw](https://github.com/openclaw/openclaw). HQ does not install either one.
2. **Give the agent its provider key yourself**, in the agent's own config. HQ never holds
   or passes the key. For example:

   ```bash
   hermes config set model.provider openrouter
   hermes config set model.default anthropic/claude-haiku-5.5
   hermes config set OPENROUTER_API_KEY "<your key>"
   ```

   ```bash
   openclaw onboard --non-interactive --accept-risk --mode local \
     --auth-choice openrouter-api-key --openrouter-api-key "<your key>" \
     --skip-daemon --skip-channels --skip-search --skip-skills --skip-ui
   openclaw models set openrouter/anthropic/claude-haiku-5.5
   ```

   Type the key into your own terminal. Do not paste it into a chat with any agent,
   HQ included.
3. **Do not run OpenClaw's gateway daemon on an HQ host.** HQ starts OpenClaw with
   `--local`, which runs the agent in process, because the sandbox refuses the loopback
   connection the gateway needs. OpenClaw refuses `--local` while a gateway is running for
   the same state folder. Skip `--install-daemon` when onboarding, or stop it with
   `openclaw gateway stop --force`. If that machine needs the gateway for something else,
   give HQ its own install through a harness profile.
4. **Let the agent reach its provider.** Add the provider's host to the sandbox allowlist
   in HQ's config, then restart HQ:

   ```yaml
   agent_host:
     sandbox:
       allow_domains:
         - openrouter.ai
   ```

5. **Check the agent works on its own** before involving HQ:

   ```bash
   hermes chat -Q -q "Reply with exactly: OK"
   openclaw agent --local --agent main --message "Reply with exactly: OK"
   ```

## Using one interactively

Start a session from a web chat, the Workbench page, or the command line. The agent opens
in a pane on the host and HQ types your prompt once it has started:

```bash
hq sessions spawn hermes --purpose research --prompt "Summarise the three newest posts on example.com/blog."
hq sessions spawn openclaw --purpose review --task FR-053 --prompt "Review the draft in the task description."
```

From a chat the tool is `harness_session_spawn` with `harness: "hermes"` or `"openclaw"`;
`harness_session_logs`, `harness_session_send` and `harness_session_stop` read, steer and
stop it. HQ launches Hermes as `hermes chat` and OpenClaw as `openclaw tui --local`.

A monitor or reviewer can watch other sessions. Give it a task and the session ids to
`observe`, and HQ relays each observed session's finished turns, blocks and exits to it:

```bash
hq sessions spawn hermes --purpose monitor --task FR-053 --observe hs-codex-4d5e6f \
  --prompt "Tell me when the refactor stalls or asks for a decision."
```

## Handing them tasks: the Agentic Stack

The faster way to use a personal agent is to name it as a specialist and assign it tasks.
HQ's `stack-dispatch` checks every minute, picks the specialist's next task, and runs it as
a single job: `hermes chat --oneshot -q "<brief>"` or
`openclaw agent --local --message "<brief>"`.

```yaml
stack:
  specialists:
    - name: hermes
      harness: hermes
      purposes: [research, general]
      run_minutes: 10     # a run past this is stopped and its task blocked
      runs_per_day: 10    # the budget, over a rolling 24 hours
    - name: openclaw
      harness: openclaw
      purposes: [research, general]
      run_minutes: 10
      runs_per_day: 10
```

Assign a task to `hermes` or `openclaw` on the board, with `task_update`, or from a chat.
Then:

- **A run that finishes** exits on its own and moves the task to ready for review. Read the
  output with `harness_session_logs` and complete the task yourself.
- **A run that crashes** blocks the task, and the block reason quotes the last short message
  the agent printed, for example OpenClaw's own refusal when a gateway is running.
- **A run that hangs** is stopped at `run_minutes` and its task blocked with that reason.

`hq stack list` shows each specialist, its queue, what is running and the day's count.
`hq stack run <specialist> <task>` starts a run now instead of waiting for the next check.

## Known limits

- HQ has no state rules for these agents yet, so their state shows as `unknown`.
  `harness_session_wait` cannot wait on them and the finished-turn alert does not fire for
  interactive sessions. Stack runs are unaffected: the process ending is their signal.
- On first interactive use, Hermes asks once about sharing usage metrics. HQ reports the
  session as stopped at a dialog and types nothing. Answer it with `harness_session_send`
  and `keys: ["enter"]` to keep the default, or answer it once yourself in a terminal.
- HQ cannot cap their token spend. It reads usage only from the coding agents' own
  transcripts. Use a key with a spending limit set at the provider.

## Removing one

1. Take it out of HQ's config: delete its `stack.specialists` entry, any harness profile
   built on it, and the provider host from `agent_host.sandbox.allow_domains` if nothing
   else uses it. Restart HQ.
2. Use the agent's own uninstaller, which also removes its state folder and the provider
   key stored in it:

   ```bash
   hermes uninstall --full --yes
   openclaw uninstall --service --state --workspace --non-interactive --yes
   npm uninstall -g openclaw
   ```

   Both have a `--dry-run` flag that lists what they would remove.
3. Revoke the provider key at the provider.
