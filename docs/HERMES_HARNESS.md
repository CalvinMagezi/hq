# Hermes harness (agent-hq) — Retired 2026-08-10

**Status: fully removed from the codebase and the machine.** This is a
historical record, not a description of anything currently running. Do not
follow any command or code reference below — every one of them refers to
code, config, or a `~/.hermes` install that no longer exists. OpenClaw was
retired in the same pass; see the git history of `CLAUDE.md` for the hq-bus side.

## Why it was retired

The owner stopped using Hermes and OpenClaw day to day — agent-hq's own
sub-agent substrate (`spawn_subagents`/`AgentService`) absorbed the
orchestration role both used to play. With the third tier-1 bus
peer already gone independently, no peer remained for the `hq-bus`
(RFC-001) communication fabric described throughout this doc to talk to, so
it was retired as well, not just its Hermes-specific parts.

## What replaced it

- **Disk watchdog** (was OpenClaw's) → native daemon task,
  `crates/hq-cli/src/commands/start/daemon/tasks_periodic/disk_watchdog.rs`.
- **DeepSeek balance check** (was Hermes's `/deepseek-usage`) → native
  Telegram command, `/deepseek` in `crates/hq-relay/src/telegram/commands.rs`.
- **Mission pull-request review** (was `peer_review` asking Hermes/OpenClaw
  over the bus) → in-process adversarial critic,
  `hq_agent::adversarial::critique_diff`, called directly from
  `mission_executor.rs`.
- **Peer messaging tools** (`agent_ask`, `agent_bus_inbox`, `agent_bus_reply`,
  `agent_broadcast_status`, `fleet_status`, `peer_review`, `team_task_*`) —
  deleted outright; `agent_send_message`/`agent_read_inbox` remain as
  pure vault-mailbox tools, unaffected since they never depended on the bus.

## What this was (historical reference below)

The **hermes** harness was the [Nous Research **Hermes Agent**](https://github.com/nousresearch/hermes-agent)
(`hermes`), a full autonomous agent CLI with its own tool loop, skills,
sessions, and MCP support. It is **not** the bare Hermes model. HQ shells out to
`hermes chat` and Hermes runs its own agentic loop.

Hermes owns its provider chain in `~/.hermes/config.yaml`: Kimi `k3-256k` primary,
then `fallback_providers` — DeepSeek `deepseek-v4-pro`, then Ollama `ornith:latest`.
HQ keeps one fallback of its own (`HERMES_OLLAMA_FALLBACK_MODEL` in
`crates/hq-relay/src/harness.rs`) for when the hermes *process* will not run at all,
which its internal chain never sees. Keep the two in step.

## Install

Installed here from git at `~/.hermes/hermes-agent` (its own venv), exposed as
`~/.local/bin/hermes`. It is **not** a `uv tool` install and will not appear in
`uv tool list`. Update with `hermes update --backup`.

```bash
hermes --version                  # whatever is installed; do not pin it here
```

HQ discovers the binary on `PATH` or at `~/.local/bin/hermes`
(`crates/hq-core/src/config/harness.rs`).

### hq-proxy Provider Plugin

Install the bundled provider plugin so HQ routes Hermes through the local proxy:

```bash
mkdir -p ~/.hermes/plugins/model-providers/hq-proxy
cp docs/hermes-plugins/hq-proxy/__init__.py ~/.hermes/plugins/model-providers/hq-proxy/
cp docs/hermes-plugins/hq-proxy/plugin.yaml  ~/.hermes/plugins/model-providers/hq-proxy/
```

With the plugin installed, HQ can spawn Hermes with `HERMES_INFERENCE_PROVIDER=hq-proxy`
to route its LLM calls through `localhost:4749/v1` instead of calling DeepSeek directly.

## What makes Hermes an HQ orchestrator

Two pieces of host-side configuration turn Hermes from a blind chat responder into a
first-class orchestrator that can drive all of HQ. Both live in Hermes' own config
(`~/.hermes/`), not in this repo.

### 1. MCP bridge — the full HQ tool catalog

Register the HQ MCP gateway as a stdio MCP server inside Hermes:

```bash
hermes mcp add hq --command hq --args mcp-serve --env HQ_FLEET_DEPTH=1
hermes mcp test hq            # verify connection
hermes tools list | grep hq   # confirm hq_discover / hq_call are enabled
```

This exposes the 2-tool gateway (`hq_discover`, `hq_call`) to Hermes' native loop,
which reaches the full HQ toolbelt: vault read/write/search, `spawn_subagent`,
`harness_dispatch` (fleet), codegraph, planning, research, memory, diagrams, webmail.

The gateway design keeps the token footprint flat. Hermes is steered (see SOUL.md) to
call `hq_discover` **with a category or query filter** — a bare discover returns 100K+
characters.

### 2. Steering — `~/.hermes/SOUL.md`

`~/.hermes/SOUL.md` (loaded fresh each message, global to all Hermes use) carries the
orchestrator identity: that the vault is shared memory, how to discover and call HQ
tools, the orchestration patterns (`spawn_subagent` for delegation), and the
project writing-style rules. This is the "knows when" layer on top of the MCP "can do"
layer.

### 3. Antigravity CLI skill — Hermes-native delegation to `agy`

This is a separate integration point from HQ's own `antigravity` harness slot
(`crates/hq-core/src/config/harness.rs:43`, which is HQ dispatching directly to
`agy` via `harness_dispatch`). This section covers Hermes calling `agy` itself,
through its own terminal tool, as a sub-agent.

Install the official skill (global to `~/.hermes/`, applies to every Hermes
session regardless of which harness HQ has active):

```bash
hermes skills install official/autonomous-ai-agents/antigravity-cli
hermes skills list | grep antigravity   # confirm "enabled"
```

The installer runs a security scan that flags `references/cli-docs.md` as
`DANGEROUS` (`CRITICAL supply_chain`) because that file quotes Antigravity's own
`curl -fsSL https://antigravity.google/cli/install... | sh` install line as
documentation text, not code that executes. It is an official, builtin skill
(source: `official/builtin`), so the flag is a false positive on pattern-matched
doc text. Confirm with `y` at the prompt.

Prerequisites (already satisfied on this machine):
- `agy` on `PATH` (`command -v agy && agy --version`)
- Auth: `agy --print 'ping' --print-timeout 30s` should return output with no
  prompt (OS keyring or prior browser sign-in already established)

No MCP registration or config change is needed for this skill. Hermes reaches
`agy` through its existing terminal tool; the skill only adds steering (when to
delegate to Antigravity, correct flag usage, log locations). Key usage
constraints Hermes needs to know:
- `agy --print` (`-p`) is non-interactive but returns plain text only, no JSON
- Use `--print-timeout` (default `5m0s`), not `--max-turns` (does not exist)
- In-session slash commands (`/config`, `/permissions`, `/model`, `/logout`)
  only work inside an interactive `agy` TUI session, not through `--print`
- Logs: `~/.gemini/antigravity-cli/log/cli-*.log`

### 4. Bus presence hook — Hermes on `hq-bus` (RFC-001 stage 4)

`plugins/hermes-hq-bus-presence/` is a gateway hook that publishes Hermes'
liveness, its agent card, and per-turn progress onto the bus.

```bash
mkdir -p ~/.hermes/hooks/hq-bus-presence
cp plugins/hermes-hq-bus-presence/{HOOK.yaml,handler.py} ~/.hermes/hooks/hq-bus-presence/
hermes gateway restart
```

This closes the observability gap created by one-shot dispatch. Because HQ
spawns `hermes -z` per turn with a 6 hour timeout, a running turn was otherwise
opaque: no progress, no tool visibility, no way to tell work from a hang. The
hook forwards `agent:start`, `agent:step` (carrying `iteration` and
`tool_names`), and `agent:end` onto `agents.hermes.event`, and beats presence on
`fleet.presence.hermes` every 30s.

Verify with `hq bus tail` in one terminal and `hermes -z "say hi"` in another.

### 5. ACP — real session continuity (WIRED)

`hermes chat --resume <id>` does **not** give continuity: it restores the session
record but never feeds prior turns into the model's context, so dispatches started
cold. Hermes itself reports `↻ Resumed session ... (2 total messages)` and then
denies knowing what was said.

ACP does. `crates/hq-tools/src/acp.rs` is the client; bus delivery uses it with
the `chat` path as fallback.

```
08:54:56  bus delivery: completed over acp        turn 1 -> "STORED."
08:55:57  bus delivery: acp resumed session 570a01c3
08:56:02  bus delivery: completed over acp        turn 2 -> "41287"
```

Sessions key on the envelope's `context_id` under an `acp-` prefix, so a
follow-up on the same A2A context resumes the thread.

**The model must be pinned.** Hermes' configured default (`kimi`/`k3-256k`) has an
exhausted billing quota, so an unpinned ACP turn returns `HTTP 403`; and a weak
model (the 8B local `ollama:hermes3`) silently fails to use the very context that
makes continuity work. Default is `openrouter:anthropic/claude-haiku-4.5`.

| Env | Effect |
|---|---|
| `HQ_ACP_MODEL` | Override the pinned model |
| `HQ_ACP_OFF=1` | Force the memoryless `chat -q` path |

ACP also streams tool calls, so a long turn reports *which tool* it is running
rather than only the coarse `DispatchSpan` heartbeat.

The verified wire contract, the runnable two-process proof, and the two traps
that produce false conclusions are in `plugins/hermes-acp/README.md`.

## Per-chat sessions (HQ dispatch)

HQ keys each surface to a Hermes session so memory survives across turns in the
same chat, using Hermes' native session store. Session keys:

| Surface | Session key | Source |
|---------|-------------|--------|
| Telegram | `hq-tg-<chat_id>` | `crates/hq-relay/src/telegram/session.rs` |
| Discord | `hq-dc-<channel_id>` | `crates/hq-relay/src/discord.rs` |
| Direct backend | `hq-cli` | `crates/hq-relay/src/harness.rs` |

Each arm invokes:

The arguments are built by `hq_tools::hermes_chat_args`
(`crates/hq-tools/src/external_runner.rs`), which emits:

```
hermes chat --accept-hooks -Q [--resume <id>] -q
```

`--accept-hooks` auto-approves shell hooks for unattended runs. The relay's own
arm inlines a bounded transcript instead of resuming, because the `-z` one-shot
form runs before `--resume` is honoured — see `crates/hq-relay/src/hermes_prompt.rs`.
The dispatch timeout is `HERMES_TIMEOUT_SECS` (6 hours, defined in
`crates/hq-relay/src/lib.rs`) because Hermes can run multi-hour fleet-coordination
loops; the ceiling exists only to bound a genuine hang. The Ollama `hermes3` fallback
is unchanged.

## Selecting the harness

Set `active_harness: hermes` in `~/.hq/config.yaml`, or per Telegram chat with the
relay's harness switch. The default `active_harness` is `pi`.

## Recursion note

Hermes is the active harness, so `harness_dispatch` could in principle route a subtask
back to `hermes`, which spawns another `hq mcp-serve`, and so on. Two mitigations are
in place today:

- SOUL.md steers Hermes to delegate via `spawn_subagent` (HQ-native agents, no
  recursion) and to use `harness_dispatch` only for *other* named harnesses, never
  `hermes`.
- The `HQ_FLEET_DEPTH=1` env marker is passed to the Hermes-spawned MCP server as a
  breadcrumb for a proper guard.

**Tech debt**: a hard depth guard in `crates/hq-web/src/harness_proxy.rs` (refuse
routing to `hermes` when `HQ_FLEET_DEPTH` is set) is not yet implemented. A conservative
alternative is `hermes tools disable hq:harness_dispatch` (keeps `spawn_subagent`).

## Verification

```bash
# Bridge fires in oneshot mode
hermes -z "Use the hq server: hq_discover({category:'vault'}), then read the most recent vault note."

# Session memory
hermes -z "Remember my favorite color is teal." -c hq-test
hermes -z "What is my favorite color?" -c hq-test   # recalls teal
```

Live: set `active_harness: hermes`, send a Telegram message needing a vault lookup,
then a follow-up in the same chat to confirm cross-turn memory.

## Self-Grading (Phase 1)

At the end of a turn, Hermes should POST a self-grade so HQ can learn which routes performed well.

Add this directive to `~/.hermes/SOUL.md`:

```
When you finish a turn, report how it went so HQ can learn which routes worked.
Call POST http://localhost:4749/api/harness/grade-turn with JSON:
  { "turn_id": "<HQ_TURN_ID env value>", "quality": 0.0-1.0, "goal_met": bool,
    "tools_succeeded": bool, "needed_retry": bool, "rationale": "one sentence" }
Grade honestly — an unmet goal must set goal_met=false.
```

Each time HQ spawns Hermes it sets `HQ_TURN_ID` in the environment. The proxy reads
the matching `x-hq-turn` header (forwarded by the hq-proxy plugin) to associate all
LLM calls in a turn with that ID. The `grade_turn_handler` (`crates/hq-web`) records
the grade and updates the per-harness EMA quality score used by the router.

## Related

- Dispatch arms: `crates/hq-relay/src/harness.rs`, `crates/hq-relay/src/telegram/session.rs`, `crates/hq-relay/src/discord.rs`
- External CLI runner: `crates/hq-tools/src/external_runner.rs` (`run_external_cli_harness_with_env`)
- HQ MCP gateway: `crates/hq-mcp/src/server.rs`, `crates/hq-mcp/src/gateway.rs`
- Cursor harness (similar pattern): `docs/CURSOR_HARNESS.md`
