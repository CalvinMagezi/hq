"""Spike E2, part 2: the daemon's file watcher and SQLite on private versus shared storage.

Usage: python3 e2b.py <adb serial> <forward port> <vault dir on the device> <label>
For one vault location it starts `hq web` and `hq start daemon` under Lite, writes a note from outside
the process, and times how long until /api/search finds it. Prints one JSON document.
"""
import json
import os
import random
import re
import subprocess
import sys
import tempfile
import time
import urllib.request

ADB = os.path.expanduser("~/Library/Android/sdk/platform-tools/adb")
SERIAL, PORT, VAULT, LABEL = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4]
HOME_DIR = "/data/local/tmp/hqhome"
TOKEN = "e2b-" + "".join(random.choice("abcdef0123456789") for _ in range(24))
POLL_LIMIT = 150
ENV = (
    f"HOME={HOME_DIR} HQ_PROFILE=lite HQ_VAULT_PATH={VAULT} HQ_CONFIG_PATH={HOME_DIR}/config.yaml "
    f"HQ_WEB_STATIC_DIR={HOME_DIR}/web HQ_WEB_AUTH_TOKEN={TOKEN}"
)


def adb(*args, timeout=300):
    out = subprocess.run([ADB, "-s", SERIAL, *args], capture_output=True, text=True, timeout=timeout)
    return out.stdout.replace("\r", "")


def sh(cmd, timeout=300):
    return adb("shell", cmd, timeout=timeout)


def spawn(args, log):
    subprocess.Popen(
        [ADB, "-s", SERIAL, "shell", f"cd /data/local/tmp && {ENV} exec ./hq {args} > {HOME_DIR}/{log} 2>&1 < /dev/null"],
        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )


def stop_all():
    for p in sh("pidof hq || true").split():
        sh(f"kill {p}")
    time.sleep(1.5)
    for p in sh("pidof hq || true").split():
        sh(f"kill -9 {p}")


def get(path, auth=True, timeout=10):
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}{path}")
    if auth:
        req.add_header("Authorization", f"Bearer {TOKEN}")
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return resp.status, resp.read().decode(errors="replace")


def wait_health():
    for _ in range(300):
        try:
            if get("/health", auth=False, timeout=2)[0] == 200:
                return True
        except Exception:
            time.sleep(0.2)
    return False


def procs():
    rows = {}
    for line in sh("ps -A -o PID,ARGS | grep '[.]/hq '").splitlines():
        m = re.match(r"\s*(\d+)\s+(.*)", line)
        if m:
            rows[int(m.group(1))] = m.group(2).strip()
    return rows


def rss(pid):
    m = re.search(r"VmRSS:\s+(\d+)", sh(f"cat /proc/{pid}/status"))
    return int(m.group(1)) if m else None


def time_to_find(token, where):
    body = f"---\ntitle: probe {token}\n---\n\nThe unique word is {token}.\n"
    path = f"{VAULT}/Notebooks/Projects/Project-00/probe-{token}.md"
    started = time.time()
    sh(f"cat > {path} <<'EOF'\n{body}EOF")
    while time.time() - started < POLL_LIMIT:
        try:
            code, text = get(f"/api/search?q={token}")
            if code == 200 and token in text:
                return round(time.time() - started, 1)
        except Exception:
            pass
        time.sleep(1)
    return None


def main():
    out = {"label": LABEL, "vault": VAULT, "serial": SERIAL}
    stop_all()
    sh(f"rm -rf {VAULT} {HOME_DIR}/.hq {HOME_DIR}/d.log {HOME_DIR}/w.log; mkdir -p {VAULT}")
    rng = random.Random(7)
    words = "alpha beta gamma delta vault note task search index agent memory sprint tunnel".split()
    with tempfile.TemporaryDirectory() as tmp:
        for i in range(200):
            d = os.path.join(tmp, "Notebooks", "Projects", f"Project-{i % 4:02d}")
            os.makedirs(d, exist_ok=True)
            with open(os.path.join(d, f"n{i:03d}.md"), "w") as f:
                f.write(f"---\ntitle: n{i}\n---\n" + "\n".join(" ".join(rng.choice(words) for _ in range(12)) for _ in range(60)) + "\n")
        sh(f"mkdir -p {VAULT}/Notebooks")
        adb("push", os.path.join(tmp, "Notebooks", "."), f"{VAULT}/Notebooks", timeout=600)
    adb("forward", f"tcp:{PORT}", "tcp:5678")
    spawn("web --no-open --port 5678", "w.log")
    out["web_up"] = wait_health()
    spawn("start daemon", "d.log")
    time.sleep(20)
    out["procs"] = {str(p): a[:60] for p, a in procs().items()}
    out["rss_kB"] = {str(p): rss(p) for p in procs()}
    out["daemon_log_tail"] = [l[:200] for l in sh(f"tail -6 {HOME_DIR}/d.log").splitlines()]
    out["doctor_watcher"] = [l.strip()[:200] for l in sh(f"cd /data/local/tmp && {ENV} ./hq doctor 2>&1 | grep -iE 'watch|index'").splitlines()][:6]
    out["seconds_until_new_note_is_searchable"] = time_to_find("zzq" + str(random.randint(10000, 99999)), VAULT)
    out["vault_files"] = sh(f"ls -a {VAULT}/_data 2>/dev/null").split()
    out["db_files"] = sh(f"ls -l {VAULT}/_data 2>/dev/null | awk '{{print $5, $NF}}'").split("\n")[:6]
    stop_all()
    print(json.dumps(out, indent=1))


main()
