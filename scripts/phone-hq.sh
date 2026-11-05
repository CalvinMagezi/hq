#!/usr/bin/env bash
# Run one `hq` command on an Android phone over adb, inside the app's own sandbox, against the
# phone's own vault. Meant for the reports that print counts only:
#
#   scripts/phone-hq.sh adoption
#   scripts/phone-hq.sh offers replay --days 56
#   scripts/phone-hq.sh adoption --json > adoption.json
#
# The command runs on the phone with the phone's binary; only its output crosses the cable.
# Do not use this to copy the database or chat history off the phone (docs/security/ADOPTION_DATA.md).
#
# Needs: adb, one phone connected with USB debugging, the HQ app installed and running (the
# binary is found through the running hq process). The app build is debuggable, which is what
# lets `run-as` reach its files (apps/android/README.md). HQ_APP_ID overrides the app id for a
# side-by-side test build.
set -euo pipefail

APP_ID="${HQ_APP_ID:-online.agenthq.app}"

if [ "$#" -eq 0 ]; then
  echo "usage: $0 <hq arguments>   e.g. $0 adoption" >&2
  exit 2
fi
ADB="${HQ_ADB:-adb}"
command -v "$ADB" >/dev/null || { echo "adb is not on PATH" >&2; exit 1; }

# Single-quote a string for a POSIX shell.
sq() { printf "'%s'" "${1//\'/\'\\\'\'}"; }

args=""
for a in "$@"; do
  args+=" $(sq "$a")"
done

# The binary lives in the app's native library directory; the running hq process names it.
remote='cd files || exit 1; pid=$(pidof libhq.so | cut -d" " -f1); [ -n "$pid" ] || { echo "hq is not running in the app; open the app first" >&2; exit 1; }; bin=$(readlink /proc/$pid/exe); HOME=$PWD/hqhome HQ_PROFILE=mobile HQ_VAULT_PATH=$PWD/hqhome/.vault HQ_CONFIG_PATH=$PWD/hqhome/config.yaml TMPDIR=$PWD/../cache exec "$bin"'"$args"

# adb hands one string to the phone's shell, which passes `remote` to run-as as one argument.
exec "$ADB" shell "run-as $APP_ID sh -c $(sq "$remote")"
