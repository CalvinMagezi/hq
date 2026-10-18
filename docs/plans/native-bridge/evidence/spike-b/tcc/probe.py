# python3 probe.py <run-dir> <cwd> [pane-name] [seconds-to-keep-alive]: sandboxed pane lists ~/Documents/HQ and reports how long it took
import json, socket, sys, time, os
run, cwd = sys.argv[1], sys.argv[2]
name = sys.argv[3] if len(sys.argv) > 3 else "probe"
tok = open(f"{run}/operator.token").read().strip()
s = socket.socket(socket.AF_UNIX); s.connect(f"{run}/host.sock"); f = s.makefile("rw")
n = 0
def call(m, p=None):
    global n; n += 1
    f.write(json.dumps({"id": n, "method": m, "params": p or {}}) + "\n"); f.flush()
    return json.loads(f.readline())
assert "result" in call("hello", {"protocol_version": 1, "token": tok})
script = "echo entries=$(ls $HOME/Documents/HQ 2>&1 | wc -l); ls $HOME/Documents/HQ >/dev/null 2>&1; echo rc=$?"
t0 = time.time()
r = call("agent.spawn", {"name": name, "argv": ["/bin/sh", "-c", script], "cwd": cwd, "agent": "shell",
                         "sandbox": {"mode": "process"}, "rows": 24, "cols": 100})
if "error" in r: print("spawn error:", r["error"]); sys.exit(1)
w = call("agent.wait", {"name": name, "until": "exit", "timeout_ms": 110000})
print(f"elapsed={time.time()-t0:.1f}s wait={json.dumps(w.get('result', w.get('error')))[:80]}")
print(call("agent.read", {"name": name, "source": "recent_unwrapped"}).get("result", {}).get("text", "").strip())
