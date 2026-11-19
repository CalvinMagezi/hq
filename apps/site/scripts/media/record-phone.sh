#!/usr/bin/env bash
# Records the reference phone's screen and pulls the file. One fixed low-end device keeps every clip
# looking the same and shows what people actually have.
#   scripts/media/record-phone.sh <out.mp4> [seconds]
# Needs adb and a phone with USB debugging on. scrcpy is used when installed (it records at the
# phone's own frame rate and can show touches); otherwise the phone's screenrecord runs for a fixed time.
set -euo pipefail
out="${1:?usage: record-phone.sh <out.mp4> [seconds]}"
seconds="${2:-60}"
command -v adb >/dev/null || { echo "adb is not installed (brew install android-platform-tools)" >&2; exit 1; }
adb get-state >/dev/null 2>&1 || { echo "no phone connected over adb" >&2; exit 1; }

# A clean status bar: 12:00, full battery and signal, no notification icons.
adb shell settings put global sysui_demo_allowed 1
adb shell am broadcast -a com.android.systemui.demo -e command enter >/dev/null
adb shell am broadcast -a com.android.systemui.demo -e command clock -e hhmm 1200 >/dev/null
adb shell am broadcast -a com.android.systemui.demo -e command battery -e level 100 -e plugged false >/dev/null
adb shell am broadcast -a com.android.systemui.demo -e command notifications -e visible false >/dev/null
adb shell settings put system show_touches 1
cleanup() {
  adb shell am broadcast -a com.android.systemui.demo -e command exit >/dev/null || true
  adb shell settings put system show_touches 0 || true
}
trap cleanup EXIT

if command -v scrcpy >/dev/null; then
  echo "Recording with scrcpy. Do the flow on the phone, then press Ctrl+C here."
  scrcpy --no-audio --record "$out" --max-size 1280 || true
else
  echo "Recording for $seconds seconds with screenrecord. Do the flow on the phone now."
  adb shell screenrecord --time-limit "$seconds" --size 720x1280 /sdcard/hq-guide.mp4
  adb pull /sdcard/hq-guide.mp4 "$out"
  adb shell rm /sdcard/hq-guide.mp4
fi
echo "Saved $out. Next: edit it, then encode with scripts/media/encode-media.mjs."
