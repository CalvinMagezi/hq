#!/usr/bin/env bash
# PreToolUse hook on Agent: the first scout or Explore launch in a session is sent back once,
# so the agent weighs an HQ harness session (recorded, visible to the owner) before a local subagent.
# Retrying the same launch goes through. Always exits 0.

command -v python3 >/dev/null 2>&1 || exit 0
[ -t 0 ] && exit 0

python3 -c '
import json, os, stat, sys

NUDGED_TYPES = {"scout", "Explore"}


# Markers live in a directory only this user can open ($XDG_RUNTIME_DIR, else ~/.cache), never
# the shared temp directory, where another user could plant a symlink at a marker path.
def marker_dir():
    base = os.environ.get("XDG_RUNTIME_DIR") or ""
    if not (os.path.isabs(base) and os.path.isdir(base)):
        base = os.path.join(os.path.expanduser("~"), ".cache")
    d = os.path.join(base, "agent-hq-plugin")
    try:
        os.makedirs(d, mode=0o700, exist_ok=True)
        st = os.lstat(d)
        if not stat.S_ISDIR(st.st_mode):
            return None
        if hasattr(os, "getuid"):
            if st.st_uid != os.getuid():
                return None
            if st.st_mode & 0o077:
                os.chmod(d, 0o700)
    except OSError:
        return None
    return d

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
marks = marker_dir()
if not marks:
    sys.exit(0)
# Created exclusively and never through a symlink: anything already at the path, or a failure to
# create it, means this session was nudged already or cannot be, so the launch goes through.
try:
    os.close(os.open(os.path.join(marks, session + ".fleet-nudged"), os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), 0o600))
except OSError:
    sys.exit(0)

how = "harness_session_spawn with a task_id"
if os.environ.get("HQ_SESSION_ID"):
    how = "agent_delegate, or harness_session_spawn with from_session set to " + os.environ["HQ_SESSION_ID"]
reason = (
    "Fleet first: a read-only lookup like this can run as an HQ harness session on a cheaper agent "
    "(" + how + ", see the hq-fleet skill), which HQ records and the owner can watch. "
    "If no harness fits, HQ is down, or the answer is needed right now, send this same launch again and it will go through."
)
print(json.dumps({"hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "deny",
    "permissionDecisionReason": reason,
}}))
'
exit 0
