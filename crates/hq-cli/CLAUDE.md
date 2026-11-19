# The `hq` binary and its commands

Notes for working under `crates/hq-cli/`, moved out of the root `CLAUDE.md` so an assistant loads them only when it is in this area. The root file has the rules that apply everywhere.

## Entry points

| What | Path |
|------|------|
| CLI binary | `crates/hq-cli/src/main.rs` |
| Pull-based updates | `crates/hq-update/`, `crates/hq-cli/src/commands/update.rs`, `deploy/install.sh`, `deploy/update/`, `docs/UPDATE_SYSTEM.md` |
| Encrypted backup and restore (`hq backup create|verify|restore|keygen|prune`, `hq restore`, the opt-in `backup:` daemon task) | `crates/hq-backup/` (`archive.rs` create, verify and restore with every check, `crypto.rs` the sealed container, `key.rs` passphrase and key file, `snapshot.rs` the consistent database copy, `retention.rs`), `crates/hq-cli/src/commands/backup.rs`, `crates/hq-cli/src/commands/start/daemon/tasks_periodic/backup.rs`, `BackupConfig` in `crates/hq-core/src/config/backup.rs`, `hq_db::migrations::newer_migrations`, `docs/BACKUP.md` |
| Restart notices | `crates/hq-cli/src/commands/notify_restart.rs` (`hq notify-restart`) |
| Daemon scheduler | `crates/hq-cli/src/commands/start/daemon/mod.rs` |
| `hq web` (host the web UI: foreground, `--detach`, `status`, `stop`, `--json`) | `crates/hq-cli/src/commands/web.rs`; shared server setup `build_web_state` and `default_static_dir` in `crates/hq-cli/src/commands/start/mod.rs`. Agents setting HQ up for someone should run `hq web --json` (it detaches) and read `login_url`. |
| LLM usage report | `crates/hq-cli/src/commands/usage.rs` (`hq usage`/`cost`/`summary` read hq-db `task_outcomes`) |

## Rules

- **Disk watchdog**: `crates/hq-cli/src/commands/start/daemon/tasks_periodic/disk_watchdog.rs` is a 6-hour daemon task
  that checks overall disk usage, the bun install cache, and `node_modules` and
  `target` bloat under configurable roots, putting a report in the owner's inbox on
  breach. Config: `disk_watchdog:` in `~/.hq/config.yaml`.
- **Tool failure digest**: `crates/hq-cli/src/commands/start/daemon/tasks_periodic/tool_failure_digest.rs`
  is a weekly daemon task that groups the week's failed tool calls (`tool_usage`) by a
  normalized error signature (`hq_db::tool_failures`), compares them with the week before
  and puts one item in the owner's inbox. No model is involved. Off with `daemon.tool_failure_digest: false`.
