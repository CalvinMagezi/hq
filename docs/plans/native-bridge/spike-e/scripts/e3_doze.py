"""Spike E3 check 5: Doze and App Standby with a foreground service.

Usage: python3 e3_doze.py <adb serial> <forward port>
The service probes http://10.0.2.2:8099/ping every 5 s (a loopback-only server on the Mac) and logs NETPROBE lines;
this script forces Doze and counts how many probes succeed in each phase. /health on the hq child is read over
`adb forward`, which is loopback inside the device and is not affected by Doze.
"""
import os
import re
import subprocess
import sys
import time
import urllib.request

ADB = os.path.expanduser("~/Library/Android/sdk/platform-tools/adb")
SERIAL, PORT = sys.argv[1], int(sys.argv[2])
PKG = "example.hq.spike"
PHASE_SECONDS = 60
T0 = time.time()


def adb(*args, timeout=120):
    out = subprocess.run([ADB, "-s", SERIAL, *args], capture_output=True, text=True, timeout=timeout)
    return out.stdout.replace("\r", "").strip()


def sh(cmd, timeout=120):
    return adb("shell", cmd, timeout=timeout)


def say(msg):
    print(f"[{int(time.time() - T0):4d}s] {msg}", flush=True)


def health():
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{PORT}/health", timeout=3) as r:
            return r.status
    except Exception as e:
        return type(e).__name__


def probes_since(mark):
    ok = fail = 0
    for line in adb("logcat", "-d", "-s", "HQSPIKE:V", timeout=60).splitlines():
        if "NETPROBE" not in line:
            continue
        m = re.match(r"(\d\d-\d\d \d\d:\d\d:\d\d)", line)
        if not m or m.group(1) < mark:
            continue
        if "NETPROBE ok" in line:
            ok += 1
        elif "NETPROBE FAIL" in line:
            fail += 1
    return ok, fail


def phase(name, seconds=PHASE_SECONDS):
    mark = sh("date '+%m-%d %H:%M:%S'")
    time.sleep(seconds)
    ok, fail = probes_since(mark)
    say(f"{name}: {ok} probes ok, {fail} failed in {seconds} s; hq /health={health()}; deep idle={sh('cmd deviceidle get deep')}; bucket={sh(f'am get-standby-bucket {PKG}')}")


def main():
    adb("forward", f"tcp:{PORT}", "tcp:5678")
    sh(f"am force-stop {PKG}")
    time.sleep(2)
    adb("logcat", "-c")
    sh(f"am start -n {PKG}/.MainActivity --ei netprobe 12")
    time.sleep(6)
    sh("input keyevent KEYCODE_HOME")
    phase("baseline, screen on and plugged in", 30)

    sh("dumpsys battery unplug")
    sh("input keyevent KEYCODE_SLEEP")
    sh("cmd deviceidle enable deep")
    say("force-idle: " + sh("cmd deviceidle force-idle deep"))
    phase("forced deep Doze, screen off")

    say("whitelist: " + sh(f"cmd deviceidle whitelist +{PKG}"))
    phase("forced Doze with the app on the battery optimization allow list")
    sh(f"cmd deviceidle whitelist -{PKG}")

    sh("cmd deviceidle unforce")
    sh("dumpsys battery reset")
    sh("input keyevent KEYCODE_WAKEUP")
    sh("dumpsys battery unplug")
    sh(f"am set-standby-bucket {PKG} rare")
    phase("App Standby bucket rare, not in Doze")
    sh(f"am set-standby-bucket {PKG} active")
    sh("dumpsys battery reset")
    say("done")


main()
