#!/usr/bin/env bash
# Stop hook: when the session claimed an HQ task and has not released it, ask once for a release
# or a checkpoint. It asks again only after another claim or release. Always exits 0.

command -v python3 >/dev/null 2>&1 || exit 0
[ -t 0 ] && exit 0

python3 -c '
import json, os, sys, tempfile

CLAIMS = {"task_claim", "task_next"}
RELEASES = {"task_release"}

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
marks = os.path.join(tempfile.gettempdir(), "agent-hq-plugin")
os.makedirs(marks, exist_ok=True)
mark = os.path.join(marks, session + ".lease-checked")
seen = f"{claims}:{releases}"
try:
    with open(mark) as f:
        if f.read() == seen:
            sys.exit(0)
except OSError:
    pass
with open(mark, "w") as f:
    f.write(seen)

reason = (
    "This session claimed an HQ task and has not released it. If the work is done or paused, call task_release "
    "with an honest status (ready_for_review, blocked or to_do) and write anything other sessions need into the vault. "
    "If you are still working on it, send task_heartbeat with a checkpoint (summary, next_step) and carry on."
)
print(json.dumps({"decision": "block", "reason": reason}))
'
exit 0
