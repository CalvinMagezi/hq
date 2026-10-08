# Orchestrator role

`governance.orchestrator_role: true` makes web chat plan, delegate, monitor and report instead of
doing development work itself. It is off by default. CLI, Telegram, Discord and MCP sessions keep
the Implementor role and are unchanged.

The limits are in code (tool catalog, sandbox, guardian), not in the prompt.

## What an orchestrator turn has

Everything an Implementor web-chat turn has, minus the tools that change files, git history or
HQ's own config: `edit_file`, `write_file`, `file_edit_batch`, `rollback_file`, `git_commit`,
`git_pr` and `config_manage` (`ORCHESTRATOR_REMOVED_TOOLS` in `crates/hq-agent/src/builder.rs`).
It keeps reading and searching (`read_file`, `grep`, `find_files`, LSP, `git_log`, `git_diff`,
`git_status`), the vault, tasks, memory, web, `convert_*`, and every tool for starting and steering
coding sessions (`harness_session_*`, `host_*`, `spawn_subagents`).

- **Shell.** `bash` runs in a read-only sandbox: no network, only scratch space writable. If the host
  has no sandbox backend (bubblewrap, `sandbox-exec`), `bash` is not offered at all.
- **Children.** `spawn_subagents` children inherit the role: they can read and report but have no
  file or git writers. Drafts come back as text and the orchestrator saves them with
  `vault_write_note`.
- **GitHub.** `github_read` and `github_clone` read GitHub without a shell. HQ builds each `gh`
  command line from validated parts, so a read cannot become a write. Without `gh` installed it reads
  public data over HTTPS. The tool is only as strong as its argument checks: give `GH_TOKEN` a
  read-only, fine-grained token.
- **Out of lane.** Calling a removed tool returns a short message that says to start a coding
  session with `harness_session_spawn` (task id, goal, done criteria), and records one denial through
  the usual denial notifier. A shell write that the sandbox blocks gets the same hint.

## Not covered

`self_update_*`, `skill_manage`, `slash_command_manage`, `watch_create`, `host_add` and `model_switch`
stay available. `host_add` and `model_switch` write the config file. See `TECHDEBT.md`.

## Protecting identity and config for every role

- `harness_session_spawn`, `harness_session_handoff` and `spawn_with` refuse a working directory that
  is the vault, a folder above it or one of `_system`, `_threads`, `_data`, on the host that holds the
  vault. Other hosts keep their own paths.
- `convert_from_markdown` refuses an output path inside `~/.hq` or the vault's private folders.
