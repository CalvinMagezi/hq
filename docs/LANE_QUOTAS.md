# Lane quotas

What is left of each coding-agent lane's allowance (Claude, Codex, Copilot, agy, Cursor, and
any private gateway you add), read per machine and shown in one place.

## How it works

The login lives on the machine the agent runs on, so that machine's host reads it and HQ asks.
A credential never crosses the gate and never reaches a model. HQ may run on a phone and
still show the readings of a laptop it is paired with.

- The host answers `host.quota` (`crates/hq-host/src/quota/`). Readers run side by side, each
  with a timeout, and the host caches a reading for 60 seconds so repeated asks do not
  hammer a vendor.
- HQ asks every configured host, reachable or not (`crates/hq-tools/src/lane_quotas/`). Every
  reading with a real source is stored in `lane_quota_samples` (35 days).
- A host that cannot be reached, or a lane it cannot read now, gives back the last stored
  reading marked `stale`, with its age and the reason. A lane with nothing stored is
  `unknown`. Unknown is never shown as zero.

## Where each lane is read from

| Lane | Source | Notes |
|------|--------|-------|
| `claude` | The login Claude Code keeps, sent to Anthropic's usage endpoint | Five-hour and weekly windows |
| `codex` | The newest local session log with a `rate_limits` block | No log yet means unknown |
| `copilot` | The premium allowance of each account `gh` is logged in to | An unmetered account has no allowance to show |
| `agy` | `agy -p /quota` | One window per limit |
| `cursor` | `cursor-agent about` | Names the tier only: Cursor's CLI shows no usage figure, so the lane is unknown with that reason |
| `opencode`, `pi`, `kimi`, `qwen`, `hermes` | None | No balance API; spend is in HQ's own ledger (`hq usage`), so the lane is unknown |
| A profile with `quota.command` | Your command | See below |

## Pace

For a window with a used percentage and a reset time, HQ works out how much of the window had
gone when it was read and projects use to the reset (`projected_percent`). The start is
inferred: five-hour and weekly windows have those lengths, monthly, credit and dollar windows
begin one calendar month before the reset. The status is `on_pace`, `over_pace` (projected at
or above `quota.warn_percent`) or `over_pace_hard` (at or above `quota.block_percent`). A
window read less than 5 percent into its length is projected as if 5 percent had gone, so one
early request does not look like a runaway.

Each window also carries `warn` and `block` flags: its used percentage is at or above those
thresholds. A window that reset after it was read is marked `expired` and flags nothing.

## Asking

- `hq quota [--json] [--refresh] [--lane x] [--host y]` prints a table; windows at or above
  the warn line carry `!`, at or above the block line `!!`.
- `GET /api/lane-quotas?host=&lane=&refresh=` with the usual owner auth.
- The read-only tool `lane_quotas {host?, lane?, refresh?}`.
- `hq doctor` lists lanes with no reading and the reason.

Without `refresh`, a stored reading younger than `quota.interval_minutes` is used and no host is
asked. The daemon task `lane-quotas` samples every `quota.interval_minutes`.

## Config

```yaml
quota:
  enabled: true          # off: nothing is read or asked
  interval_minutes: 10
  warn_percent: 80
  block_percent: 95
  lanes:
    codex: false         # turn one reader off
```

Turn the sampler off with `quota.enabled: false`.

## A private gateway

Add a `quota` command to a harness profile. It runs on the host, without a shell, and prints
one JSON object on stdout:

```yaml
agent_host:
  harness_profiles:
    gateway:
      base: claude-code
      quota:
        command: ["/usr/local/bin/gateway-usage", "--json"]
        timeout_secs: 10
```

```json
{"account": "team", "windows": [{"label": "month", "kind": "dollars", "used_percent": 12.5,
  "resets_at": "2026-12-01T00:00:00Z"}]}
```

The command does not set the lane or the source. Print no credential: the account is a label.

`host_overview` (`docs/AGENT_HOST.md`) shows these readings next to each host's reachability and
agents, with a `status` per row (`available`, `stale`, `not_exposed`, `check_failed`), under a
per-host time limit.

## Privacy

Readings are owner-only. They are kept in the local database, shown to the owner by the CLI and
the web app, and returned by `lane_quotas` to a session the owner drives. They are not put into
a model's prompt by HQ, not pushed, and not sent to a third party. Slice C (spawn gate, driver
prompt block, alerts) is not part of this.
