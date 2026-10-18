# Drive an isolated hq host over its control socket: python3 drive.py <run-dir> <workdir>
import json, socket, sys, time
run, work = sys.argv[1], sys.argv[2]
tok = open(f"{run}/operator.token").read().strip()
s = socket.socket(socket.AF_UNIX); s.connect(f"{run}/host.sock"); f = s.makefile("rw")
n = 0
def call(method, params=None):
    global n; n += 1
    f.write(json.dumps({"id": n, "method": method, "params": params or {}}) + "\n"); f.flush()
    return json.loads(f.readline())
r = call("hello", {"protocol_version": 1, "token": tok}); assert "result" in r, r
script = ("echo pane-ok; echo user=$(id -un); "
          "echo ssh:$(cat $HOME/.ssh/id_ed25519 2>&1 | head -c 60); "
          "echo hqdir:$(ls $HOME/.hq 2>&1 | head -c 60); "
          "echo egress-denied:$(curl -s -m 8 -o /dev/null -w '%{http_code}' https://example.com 2>&1); echo egress-allowed:$(curl -s -m 8 -o /dev/null -w '%{http_code}' https://api.anthropic.com 2>&1); "
          "echo docs:$(ls $HOME/Documents 2>&1 | head -c 60)")
r = call("agent.spawn", {"name": "probe", "argv": ["/bin/sh", "-c", script], "cwd": work,
                         "agent": "shell", "sandbox": {"mode": "process", "allow": [{"host": "api.anthropic.com", "ports": [443]}]}, "rows": 24, "cols": 100})
print("spawn:", json.dumps(r)[:300])
if "result" in r:
    print("wait:", json.dumps(call("agent.wait", {"name": "probe", "until": "exit", "timeout_ms": 30000}))[:200])
    print(call("agent.read", {"name": "probe", "source": "recent_unwrapped"}).get("result", {}).get("text", ""))
if "result" in r:
    print("egress decisions:", json.dumps(call("agent.egress", {"name": "probe"}).get("result")))
