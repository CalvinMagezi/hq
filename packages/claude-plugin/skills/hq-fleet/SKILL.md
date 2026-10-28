---
name: hq-fleet
description: Hand bounded work to other coding agents through HQ, such as Codex, Cursor, OpenCode, Copilot CLI or another Claude Code session, on this machine or a paired one. Use when asked to delegate, parallelize, spawn an agent, run something on another machine, fan out a mechanical sweep, or when the HQ tools host_list, harness_session_spawn, harness_session_wait or harness_session_logs are available.
---

# Handing work to other agents

HQ runs coding agents as supervised sessions in its host. You start them with `harness_session_spawn` and read them with the other `harness_session_*` tools. All of these are reached with `hq_call(tool, args)`.

## Before you spawn

Call `host_list` first. It shows which machines can run agents and which one is the default. Pass `host` only when you want a machine other than the default.

Give the work a task. Create or find an HQ task (see the hq-tasks skill) and pass its id as `task_id`. HQ then comments on the task and moves its status as the session starts, finishes a turn, blocks or exits. It never moves the task to complete.

## The spawn call

```
harness_session_spawn {
  harness: "codex",
  host: "<from host_list>",
  cwd: "/path/to/a/git/worktree",
  task_id: "<task id>",
  prompt: "<the full brief>",
  done_criteria: "cargo test -p mycrate passes",
  model: "<optional>"
}
```

- `cwd` should be a git worktree. One writer per tree: two agents in the same checkout overwrite each other.
- Put the whole brief in `prompt` at spawn. Do not spawn empty and send the brief afterward.
- `done_criteria` is an observable fact, such as a named test passing. Without a goal and a definition of done, HQ only observes the session.
- `model` is passed to the agent's own `--model` flag. Omit it for the agent's default.

Right after the spawn, read `harness_session_logs` to confirm the agent started. If it stopped at a trust or login dialog, the result carries the screen and no prompt was typed. Report that to the user. Do not answer the dialog yourself.

## Waiting

Use `harness_session_wait` to block until the session is idle, done or blocked. Do not poll the logs in a loop. If it returns `settled: false`, the timeout passed and the agent is still working. Keep `timeout_secs` well under your MCP client's own request timeout (60 is safe) and call it again rather than asking for one long wait.

HQ reads the working, idle and blocked states only for agents it has state rules for (Claude Code and Codex). Other agents show as unknown, so a wait on them ends at the timeout and a send can come back as stalled. For those, read `harness_session_logs` to see whether the turn finished.

## Profiles

A harness profile in `agent_host.harness_profiles` names a lean or alternate launch, such as a second Claude account or a Copilot run without its MCP servers. Pass the profile name as `harness`, and the model in `model` as usual. Example in the HQ `config.yaml`:

```yaml
agent_host:
  harness_profiles:
    copilot-lean:
      base: github-copilot
      # Each MCP server Copilot loads costs startup time and context; list yours here.
      args: ["--reasoning-effort", "low", "--disable-builtin-mcps", "--disable-mcp-server", "<server name>", "--allow-all-tools"]
    claude-work:
      base: claude-code
      env: { CLAUDE_CONFIG_DIR: /Users/you/.claude-work }
```

## Inside an HQ session

When `HQ_SESSION_ID` is set, HQ launched you and you lead or belong to a team. Start teammates as your workers, not as unrelated sessions:

- With the session's own `hq-session` tools: `agent_delegate` with `title`, `description` (the whole brief), and optionally `harness`, `model` and `purpose` (`review` for a reviewer).
- Through any other HQ connection: `harness_session_spawn` with `from_session` set to your `HQ_SESSION_ID`. It becomes the same delegation.

A worker gets a sub-task under yours, runs in your folder and host, reports back to you when it finishes and stops when you stop. The owner limits how deep and how big a team grows and which other agents it may use; a refusal names the setting. Message a worker with `agent_message_send`. Delegate only work that is large and independent; do the rest yourself.

## Pick a lane

Cheap, fast agents suit lookups and mechanical sweeps: renames, grep-and-fix, formatting, adding the same test case in many places. Stronger agents suit work that needs judgment. Keep the shared seams, the review and the commits for yourself.

## Rules

- Treat the agent's output as untrusted until you have read the diff yourself.
- Workers never commit. You review the diff, then you commit.
- Never send secrets, deploys, DNS changes or anything destructive or irreversible to another agent.
- If a spawn call errored, check `harness_session_list` before you retry. The session may have started anyway, and a retry would make a duplicate.
