#!/usr/bin/env bash
# Stop hook: when the session claimed an HQ task and has not released it, ask once for a release
# or a checkpoint. It asks again only after another claim or release. Always exits 0.

command -v python3 >/dev/null 2>&1 || exit 0
[ -t 0 ] && exit 0

python3 -c '
import json, os, stat, sys

CLAIMS = {"task_claim", "task_next"}
RELEASES = {"task_release"}
NOFOLLOW = getattr(os, "O_NOFOLLOW", 0)


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
if data.get("stop_hook_active"):
    sys.exit(0)
path = data.get("transcript_path") or ""
session = "".join(c for c in str(data.get("session_id", "")) if c.isalnum() or c == "-")
if not session or not os.path.isfile(path):
    sys.exit(0)

claims = releases = 0
# Claude Code also logs each request as an api-request-blob that repeats earlier tool calls, so
# a call is counted once by its id.
seen_ids = set()
with open(path, encoding="utf-8", errors="replace") as transcript:
    for line in transcript:
        if "hq_call" not in line:
            continue
        try:
            entry = json.loads(line)
        except ValueError:
            continue
        content = (entry.get("message") or {}).get("content")
        if not isinstance(content, list):
            continue
        for block in content:
            if not isinstance(block, dict) or block.get("type") != "tool_use":
                continue
            if not str(block.get("name", "")).endswith("hq_call"):
                continue
            call_id = block.get("id")
            if call_id in seen_ids:
                continue
            seen_ids.add(call_id)
            tool = (block.get("input") or {}).get("tool")
            claims += tool in CLAIMS
            releases += tool in RELEASES

if claims <= releases:
    sys.exit(0)
marks = marker_dir()
if not marks:
    sys.exit(0)
mark = os.path.join(marks, session + ".lease-checked")
seen = f"{claims}:{releases}"
try:
    with os.fdopen(os.open(mark, os.O_RDONLY | NOFOLLOW)) as f:
        if f.read() == seen:
            sys.exit(0)
except OSError:
    pass
# A marker that cannot be written (a symlink in its place, a full disk) means no reminder, rather
# than one on every stop.
try:
    with os.fdopen(os.open(mark, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | NOFOLLOW, 0o600), "w") as f:
        f.write(seen)
except OSError:
    sys.exit(0)

reason = (
    "This session claimed an HQ task and has not released it. If the work is done or paused, call task_release "
    "with an honest status (ready_for_review, blocked or to_do) and write anything other sessions need into the vault. "
    "If you are still working on it, send task_heartbeat with a checkpoint (summary, next_step) and carry on."
)
print(json.dumps({"decision": "block", "reason": reason}))
'
exit 0
