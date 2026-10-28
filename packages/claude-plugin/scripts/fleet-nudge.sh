#!/usr/bin/env bash
# PreToolUse hook on Agent: the first scout or Explore launch in a session is sent back once,
# so the agent weighs an HQ harness session (recorded, visible to the owner) before a local subagent.
# Retrying the same launch goes through. Always exits 0.

command -v python3 >/dev/null 2>&1 || exit 0
[ -t 0 ] && exit 0

python3 -c '
import json, os, sys, tempfile

NUDGED_TYPES = {"scout", "Explore"}

try:
    data = json.load(sys.stdin)
except ValueError:
    sys.exit(0)
if data.get("tool_name") not in ("Agent", "Task"):
    sys.exit(0)
if (data.get("tool_input") or {}).get("subagent_type") not in NUDGED_TYPES:
    sys.exit(0)

session = "".join(c for c in str(data.get("session_id", "")) if c.isalnum() or c == "-")
if not session:
    sys.exit(0)
marks = os.path.join(tempfile.gettempdir(), "agent-hq-plugin")
os.makedirs(marks, exist_ok=True)
mark = os.path.join(marks, session + ".fleet-nudged")
if os.path.exists(mark):
    sys.exit(0)
open(mark, "w").close()

reason = (
    "Fleet first: a read-only lookup like this can run as an HQ harness session on a cheaper agent "
    "(harness_session_spawn with a task_id, see the hq-fleet skill), which HQ records and the owner can watch. "
    "If no harness fits, HQ is down, or the answer is needed right now, send this same launch again and it will go through."
)
print(json.dumps({"hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "deny",
    "permissionDecisionReason": reason,
}}))
'
exit 0
