"""Spike E3 check 2: does the hq child survive Home, screen off, a recents swipe, am kill, force-stop and a kill -9 of the app?

Usage: python3 e3_survival.py <adb serial> <forward port>
Prints a timeline. The app is example.hq.spike (debuggable, so `run-as` can signal it).
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
HOME_WAIT = 60
SCREEN_OFF_WAIT = 300
T0 = time.time()


def adb(*args, timeout=60):
    out = subprocess.run([ADB, "-s", SERIAL, *args], capture_output=True, text=True, timeout=timeout)
    return out.stdout.replace("\r", "").strip()


def sh(cmd, timeout=60):
    return adb("shell", cmd, timeout=timeout)


def say(msg):
    print(f"[{int(time.time() - T0):4d}s] {msg}", flush=True)


def pids():
    child = sh("pidof libhq.so || true").split()
    app = sh(f"pidof {PKG} || true").split()
    return (child[0] if child else None), (app[0] if app else None)


def health():
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{PORT}/health", timeout=3) as r:
            return r.status
    except Exception as e:
        return type(e).__name__


def state(label):
    child, app = pids()
    say(f"{label}: hq child pid={child} app pid={app} health={health()}")
    return child, app


def wait_up(limit=60):
    started = time.time()
    while time.time() - started < limit:
        child, _ = pids()
        if child and health() == 200:
            return round(time.time() - started, 1)
        time.sleep(0.5)
    return None


def main():
    adb("forward", f"tcp:{PORT}", "tcp:5678")
    sh(f"am force-stop {PKG}")
    time.sleep(2)
    sh("input keyevent KEYCODE_WAKEUP")
    sh(f"am start -n {PKG}/.MainActivity")
    say(f"launch: up after {wait_up()} s")
    child0, app0 = state("running")

    sh("input keyevent KEYCODE_HOME")
    time.sleep(HOME_WAIT)
    state(f"after Home and {HOME_WAIT} s")

    sh("input keyevent KEYCODE_SLEEP")
    time.sleep(SCREEN_OFF_WAIT)
    state(f"after screen off for {SCREEN_OFF_WAIT} s")
    sh("input keyevent KEYCODE_WAKEUP")
    sh("wm dismiss-keyguard")
    time.sleep(2)

    sh(f"am start -n {PKG}/.MainActivity")
    time.sleep(3)
    sh("input keyevent KEYCODE_HOME")
    time.sleep(1)
    size = re.search(r"(\d+)x(\d+)", sh("wm size"))
    w, h = int(size.group(1)), int(size.group(2))
    sh("input keyevent KEYCODE_APP_SWITCH")
    time.sleep(2)
    sh(f"input swipe {w // 2} {int(h * 0.55)} {w // 2} {int(h * 0.05)} 150")
    time.sleep(5)
    child, app = state("after swiping the app away in recents")
    say("tasks: " + (sh(f"dumpsys activity activities | grep -c {PKG}") or "0") + " mentions of the package")
    say("service: " + " | ".join(l.strip() for l in sh(f"dumpsys activity services {PKG} | grep -E 'ServiceRecord|isForeground|foregroundId' | head -3").splitlines()))

    if not child:
        sh(f"am start -n {PKG}/.MainActivity")
        say(f"restarted by hand: up after {wait_up()} s")
    sh("input keyevent KEYCODE_HOME")
    time.sleep(1)
    say("am kill: " + (sh(f"am kill {PKG}") or "sent"))
    time.sleep(5)
    state("after am kill")

    child, app = pids()
    if not child:
        sh(f"am start -n {PKG}/.MainActivity")
        say(f"restarted by hand: up after {wait_up()} s")
    child, app = pids()
    say(f"run-as kill -9 of the app process {app}")
    sh(f"run-as {PKG} kill -9 {app}")
    started = time.time()
    time.sleep(2)
    back = wait_up(90)
    state(f"after kill -9 of the app (sticky restart back in {back} s)" if back else "after kill -9 of the app (no restart within 90 s)")

    child, app = pids()
    if child:
        say(f"run-as kill -9 of the hq child {child}")
        sh(f"run-as {PKG} kill -9 {child}")
        time.sleep(10)
        state("after kill -9 of the hq child only (the service does not respawn it)")

    sh(f"am start -n {PKG}/.MainActivity")
    wait_up(60)
    sh("input keyevent KEYCODE_HOME")
    say("am force-stop: " + (sh(f"am force-stop {PKG}") or "sent"))
    time.sleep(5)
    state("after force-stop")
    say("logcat (service and kill lines):")
    for line in adb("logcat", "-d", "-s", "HQSPIKE:V", timeout=60).splitlines():
        if any(k in line for k in ("onStartCommand", "onDestroy", "exited", "hq web started")):
            print("   ", line[:170])


main()
