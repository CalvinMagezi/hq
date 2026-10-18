"""Spike E2: run Lite `hq web` on an Android emulator and measure it the way the Linux reference run was measured.

Usage: python3 e2.py <adb serial> <label> <forward port> <hq binary> <web dist dir>
Prints one JSON document on stdout at the end. Method notes are in docs/plans/native-bridge/spike-e/README.md.
"""
import json
import os
import random
import re
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request

ADB = os.path.expanduser("~/Library/Android/sdk/platform-tools/adb")
SERIAL, LABEL, PORT, HQ_BIN, WEB_DIST = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4], sys.argv[5]
HOME_DIR = "/data/local/tmp/hqhome"
TOKEN = "e2-" + "".join(random.choice("abcdef0123456789") for _ in range(24))
NOTES = 2000
SAMPLE_EVERY = 0.4
IDLE_AFTER_WORKLOAD = 45
ENV = (
    f"HOME={HOME_DIR} HQ_PROFILE=lite HQ_VAULT_PATH={HOME_DIR}/.vault HQ_CONFIG_PATH={HOME_DIR}/config.yaml "
    f"HQ_WEB_STATIC_DIR={HOME_DIR}/web HQ_WEB_AUTH_TOKEN={TOKEN}"
)


def adb(*args, timeout=300, check=True):
    out = subprocess.run([ADB, "-s", SERIAL, *args], capture_output=True, text=True, timeout=timeout)
    if check and out.returncode != 0:
        raise RuntimeError(f"adb {args} failed: {out.stderr[:300]}")
    return out.stdout.replace("\r", "")


def sh(cmd, timeout=300):
    return adb("shell", cmd, timeout=timeout, check=False)


def pids(name="hq"):
    return [int(p) for p in sh(f"pidof {name} || true").split()]


def status(pid):
    text = sh(f"cat /proc/{pid}/status 2>/dev/null")
    row = {}
    for key in ("VmRSS", "VmHWM", "Threads"):
        m = re.search(rf"^{key}:\s+(\d+)", text, re.M)
        row[key] = int(m.group(1)) if m else None
    return row


def pss_total(pid):
    text = sh(f"dumpsys meminfo {pid} 2>/dev/null | grep -E 'TOTAL PSS|TOTAL:' | head -2")
    m = re.search(r"TOTAL(?: PSS)?:\s+(\d+)", text)
    return int(m.group(1)) if m else None


def http(path, auth=True, timeout=30):
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}{path}")
    if auth:
        req.add_header("Authorization", f"Bearer {TOKEN}")
    started = time.time()
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        body = resp.read()
        return resp.status, len(body), time.time() - started


def start_server():
    subprocess.Popen(
        [ADB, "-s", SERIAL, "shell", f"cd /data/local/tmp && {ENV} exec ./hq web --no-open --port 5678 > {HOME_DIR}/hq.log 2>&1 < /dev/null"],
        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    started = time.time()
    while time.time() - started < 90:
        try:
            if http("/health", auth=False, timeout=2)[0] == 200:
                return time.time() - started
        except Exception:
            time.sleep(0.2)
    raise RuntimeError("server did not answer /health in 90 s: " + sh(f"tail -5 {HOME_DIR}/hq.log"))


def stop_all():
    for p in pids():
        sh(f"kill {p}")
    time.sleep(1)
    for p in pids():
        sh(f"kill -9 {p}")


def server_pid():
    found = pids()
    if not found:
        raise RuntimeError("hq is not running")
    return min(found)


def gen_notes(dest):
    rng = random.Random(46)
    words = [w for w in "alpha beta gamma delta vault note task search index agent memory sprint tunnel gateway phone android kernel socket buffer thread latency cache schema query result report review design".split()]
    total = 0
    for i in range(NOTES):
        proj = f"Project-{i % 40:02d}"
        os.makedirs(os.path.join(dest, "Notebooks", "Projects", proj), exist_ok=True)
        lines = [f"---\ntitle: Note {i}\ntags: [{rng.choice(words)}, {rng.choice(words)}]\n---", f"# Note {i} about {rng.choice(words)}"]
        for _ in range(rng.randint(50, 500)):
            lines.append(" ".join(rng.choice(words) for _ in range(rng.randint(8, 18))) + f" {rng.randint(0, 99999)}")
        text = "\n".join(lines) + "\n"
        total += len(text)
        with open(os.path.join(dest, "Notebooks", "Projects", proj, f"note-{i:04d}.md"), "w") as f:
            f.write(text)
    return total


class Sampler(threading.Thread):
    """Polls /proc/<pid>/status for every hq process and keeps the peak seen."""

    def __init__(self):
        super().__init__(daemon=True)
        self.stop_flag = False
        self.peaks = {}

    def run(self):
        while not self.stop_flag:
            try:
                for p in pids():
                    s = status(p)
                    cur = self.peaks.setdefault(p, {"VmHWM": 0, "VmRSS": 0})
                    cur["VmHWM"] = max(cur["VmHWM"], s["VmHWM"] or 0)
                    cur["VmRSS"] = max(cur["VmRSS"], s["VmRSS"] or 0)
            except Exception:
                pass
            time.sleep(SAMPLE_EVERY)


def snap(pid):
    s = status(pid)
    s["PSS_kB"] = pss_total(pid)
    return s


def main():
    out = {"label": LABEL, "serial": SERIAL}
    out["android"] = sh("getprop ro.build.version.release").strip() + " (api " + sh("getprop ro.build.version.sdk").strip() + ")"
    out["abi"] = sh("getprop ro.product.cpu.abi").strip()
    out["mem_total_kB"] = int(re.search(r"MemTotal:\s+(\d+)", sh("cat /proc/meminfo")).group(1))
    out["cpus"] = int(sh("grep -c ^processor /proc/cpuinfo").strip() or 0)
    out["webview"] = " ".join(sh("dumpsys webviewupdate | grep -iE 'Current WebView package|versionName' | head -2").split())
    stop_all()
    sh(f"mkdir -p {HOME_DIR} && rm -rf {HOME_DIR}/.vault {HOME_DIR}/.hq {HOME_DIR}/config.yaml {HOME_DIR}/hq.log {HOME_DIR}/web")
    have = sh("ls -l /data/local/tmp/hq 2>/dev/null").split()
    if not have or int(have[4]) != os.path.getsize(HQ_BIN):
        adb("push", HQ_BIN, "/data/local/tmp/hq", timeout=600)
    sh("chmod 755 /data/local/tmp/hq")
    adb("push", WEB_DIST, f"{HOME_DIR}/web", timeout=600)
    adb("forward", f"tcp:{PORT}", "tcp:5678")

    # Run 1: fresh home, empty vault.
    out["start_empty_s"] = round(start_server(), 2)
    pid = server_pid()
    out["run1_start"] = snap(pid)
    time.sleep(30)
    out["run1_30s"] = snap(pid)
    out["doctor_memory"] = [l.strip() for l in sh(f"cd /data/local/tmp && {ENV} ./hq doctor 2>&1 | grep -iE 'memory|ram|gb'", timeout=120).splitlines()][:6]
    out["cpu_idle"] = " ".join(sh(f"top -b -n 1 -p {pid} 2>/dev/null | tail -2").split())[:200]
    stop_all()

    # Run 2: the 2,000 note vault, started before indexing.
    with tempfile.TemporaryDirectory() as tmp:
        out["notes_bytes"] = gen_notes(tmp)
        started = time.time()
        adb("push", os.path.join(tmp, "Notebooks"), f"{HOME_DIR}/.vault/Notebooks", timeout=1200)
        out["push_notes_s"] = round(time.time() - started, 1)
    out["start_with_notes_s"] = round(start_server(), 2)
    pid = server_pid()
    out["run2_before_index"] = snap(pid)
    sampler = Sampler()
    sampler.start()
    started = time.time()
    rc = sh(f"cd /data/local/tmp && {ENV} ./hq reindex > {HOME_DIR}/reindex.log 2>&1; echo $?", timeout=1800).strip()
    out["reindex_s"] = round(time.time() - started, 1)
    out["reindex_rc"] = rc
    time.sleep(1)
    sampler.stop_flag = True
    sampler.join()
    out["reindex_peaks_kB"] = sampler.peaks
    out["vault_db_bytes"] = sh(f"ls -l {HOME_DIR}/.vault/_data/vault.db 2>/dev/null").split()[4:5]
    searches = {}
    for tag, q in (("first_a", "tunnel"), ("first_b", "memory+schema"), ("first_c", "android+kernel"), ("first_d", "latency")):
        try:
            code, size, secs = http(f"/api/search?q={q}")
            searches[tag] = {"status": code, "bytes": size, "ms": round(secs * 1000)}
        except Exception as e:
            searches[tag] = {"error": str(e)[:100]}
    try:
        code, size, secs = http("/api/search?q=tunnel")
        searches["warm_repeat"] = {"status": code, "bytes": size, "ms": round(secs * 1000)}
    except Exception as e:
        searches["warm_repeat"] = {"error": str(e)[:100]}
    out["searches"] = searches
    out["run2_after_search"] = snap(pid)
    time.sleep(IDLE_AFTER_WORKLOAD)
    out["run2_idle_45s"] = snap(pid)
    out["log_tail"] = sh(f"tail -3 {HOME_DIR}/hq.log").splitlines()[-3:]
    stop_all()
    print(json.dumps(out, indent=1))


main()
