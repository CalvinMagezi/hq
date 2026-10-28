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

# Prints how many tasks have this status, or nothing when HQ or python3 cannot say.
count_tasks() {
  [ -n "${CLAUDE_PLUGIN_OPTION_HQ_API_KEY:-}" ] && command -v python3 >/dev/null 2>&1 || return 0
  curl -s -m 4 "$base/mcp" \
    -H "Authorization: Bearer $CLAUDE_PLUGIN_OPTION_HQ_API_KEY" \
    -H 'Content-Type: application/json' -H 'Accept: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"hq_call\",\"arguments\":{\"tool\":\"task_list\",\"args\":{\"status\":\"$1\",\"limit\":1}}}}" 2>/dev/null |
    python3 -c 'import json,sys; print(json.loads(json.load(sys.stdin)["result"]["content"][0]["text"])["total"])' 2>/dev/null
}

if [ "$code" = "200" ]; then
  echo "HQ is reachable. Track multi-step work as HQ tasks and hold a lease while you work (hq-tasks skill)."
  if [ -n "${HQ_SESSION_ID:-}" ]; then
    echo "You run inside HQ session ${HQ_SESSION_ID}, part of an HQ team. Start a teammate with agent_delegate (hq-session tools), or with harness_session_spawn and from_session \"${HQ_SESSION_ID}\" if you call HQ another way; talk to teammates with agent_message_send (hq-fleet skill)."
  else
    echo "Send bounded work to other agents with harness_session_spawn and a task_id (hq-fleet skill)."
  fi
  echo "Put knowledge other sessions need in the vault, not in local memory (hq-vault skill)."
  in_progress="$(count_tasks in_progress)"
  blocked="$(count_tasks blocked)"
  to_do="$(count_tasks to_do)"
  if [ -n "$in_progress$blocked$to_do" ]; then
    echo "HQ tasks: ${in_progress:-?} in progress, ${blocked:-?} blocked, ${to_do:-?} to do. Before starting new work, check whether a task already covers it (task_list, or /agent-hq:hq-status)."
  fi
  if [ "$source" = "compact" ]; then
    echo "Context was just compacted: if you hold a task lease, send task_heartbeat with a checkpoint now."
  fi
else
  echo "WARNING: HQ is unreachable (health check returned status ${code:-000}). HQ tasks, agents and the vault will fail."
  echo "Tell the user HQ is down instead of silently doing the work yourself."
fi
exit 0
