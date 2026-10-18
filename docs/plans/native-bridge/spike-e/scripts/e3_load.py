"""Spike E3 check 4: a mixed workload with hq children of the app, watched for the phantom process killer.

Usage: python3 e3_load.py <adb serial> <apk> <notes.tgz> <minutes>
The service itself spawns `hq reindex` every 10 s (a child of the app process) and runs four searches against
the `hq web` child. This script only installs, seeds the vault, starts it and watches.
"""
import os
import re
import subprocess
import sys
import time

ADB = os.path.expanduser("~/Library/Android/sdk/platform-tools/adb")
SERIAL, APK, NOTES, MINUTES = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
PKG = "example.hq.spike"
SAMPLE_EVERY = 30
T0 = time.time()


def adb(*args, timeout=120):
    out = subprocess.run([ADB, "-s", SERIAL, *args], capture_output=True, text=True, timeout=timeout)
    return out.stdout.replace("\r", "").strip()


def sh(cmd, timeout=120):
    return adb("shell", cmd, timeout=timeout)


def say(msg):
    print(f"[{int(time.time() - T0):5d}s] {msg}", flush=True)


def main():
    adb("install", "-r", "-g", APK)
    sh(f"pm grant {PKG} android.permission.POST_NOTIFICATIONS")
    sh(f"am force-stop {PKG}")
    sh(f"am start -n {PKG}/.MainActivity")
    time.sleep(8)
    sh(f"am force-stop {PKG}")
    adb("push", NOTES, "/data/local/tmp/notes.tgz", timeout=300)
    sh(f"run-as {PKG} sh -c 'mkdir -p files/hqhome/.vault && cd files/hqhome/.vault && tar xzf /data/local/tmp/notes.tgz'", timeout=300)
    say("seeded vault files: " + sh(f"run-as {PKG} sh -c 'find files/hqhome/.vault -name \"*.md\" | wc -l'"))
    say("phantom settings: monitor=" + sh("settings get global settings_enable_monitor_phantom_procs") +
        " " + " ".join(sh("/system/bin/dumpsys activity settings | grep -i phantom").split()))
    adb("logcat", "-c")
    sh(f"am start -n {PKG}/.MainActivity --ei load {MINUTES}")
    time.sleep(5)
    sh("input keyevent KEYCODE_HOME")
    end = time.time() + MINUTES * 60 + 30
    while time.time() < end:
        child = sh("pidof libhq.so || true").split()
        app = sh(f"pidof {PKG} || true")
        rss = ""
        if child:
            m = re.search(r"VmRSS:\s+(\d+)", sh(f"cat /proc/{child[0]}/status"))
            rss = f"rss={m.group(1)} kB" if m else ""
        say(f"hq processes={len(child)} pids={','.join(child)} app={app} {rss}")
        time.sleep(SAMPLE_EVERY)
    say("--- LOAD lines (first, last)")
    lines = [l for l in adb("logcat", "-d", "-s", "HQSPIKE:V", timeout=120).splitlines() if "LOAD" in l]
    for l in lines[:2] + lines[-3:]:
        print("   ", l[:200])
    say(f"--- LOAD iterations logged: {len([l for l in lines if 'LOAD it=' in l])}")
    say("--- system lines about the package being killed or trimmed")
    log = adb("logcat", "-d", timeout=120).splitlines()
    for l in log:
        low = l.lower()
        if ("phantom" in low or "excessive" in low or "killing" in low or "force stopping" in low) and ("hq" in low or PKG in low):
            print("   ", l[:220])
    say("done")


main()
