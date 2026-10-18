# python3 hostcall.py <run-dir> <method> [json-params]: one call to an isolated host, prints the result
import json, socket, sys
run, method = sys.argv[1], sys.argv[2]
params = json.loads(sys.argv[3]) if len(sys.argv) > 3 else {}
tok = open(f"{run}/operator.token").read().strip()
s = socket.socket(socket.AF_UNIX); s.connect(f"{run}/host.sock"); f = s.makefile("rw")
def call(m, p):
    f.write(json.dumps({"id": 1, "method": m, "params": p}) + "\n"); f.flush(); return json.loads(f.readline())
assert "result" in call("hello", {"protocol_version": 1, "token": tok})
r = call(method, params)
print(json.dumps(r.get("result", r.get("error"))))
