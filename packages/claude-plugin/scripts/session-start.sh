#!/usr/bin/env bash
# SessionStart hook: check that HQ answers, then remind the agent how to use it.
# Always exits 0 so a down HQ never blocks a session.

url="${CLAUDE_PLUGIN_OPTION_HQ_URL:-}"
[ -n "$url" ] || exit 0

base="${url%/}"
base="${base%/mcp}"

source=""
if [ ! -t 0 ]; then
  input="$(cat)"
  if command -v jq >/dev/null 2>&1; then
    source="$(printf '%s' "$input" | jq -r '.source // empty' 2>/dev/null)"
  elif command -v python3 >/dev/null 2>&1; then
    source="$(printf '%s' "$input" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("source",""))' 2>/dev/null)"
  fi
fi

code="$(curl -s -o /dev/null -m 4 -w '%{http_code}' "$base/health" 2>/dev/null)"

if [ "$code" = "200" ]; then
  echo "HQ is reachable. Track multi-step work as HQ tasks and hold a lease while you work (hq-tasks skill)."
  echo "Send bounded work to other agents with harness_session_spawn and a task_id (hq-fleet skill)."
  echo "Put knowledge other sessions need in the vault, not in local memory (hq-vault skill)."
  if [ "$source" = "compact" ]; then
    echo "Context was just compacted: if you hold a task lease, send task_heartbeat with a checkpoint now."
  fi
else
  echo "WARNING: HQ is unreachable (health check returned status ${code:-000}). HQ tasks, agents and the vault will fail."
  echo "Tell the user HQ is down instead of silently doing the work yourself."
fi
exit 0
