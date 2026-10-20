#!/usr/bin/env bash
# Fails unless every LOAD segment of an Android binary is aligned to at least 16 KB (2**14), the
# page size Google Play requires of native code in apps that target Android 15 or newer.
# Usage: check-android-alignment.sh <binary> [llvm-objdump]
set -euo pipefail

bin=${1:?binary to check}
objdump=${2:-llvm-objdump}

headers=$("$objdump" -p "$bin")
if ! printf '%s\n' "$headers" | awk '
  /^ *LOAD / {
    n = $NF
    sub(/.*\*\*/, "", n)
    segments++
    if (n + 0 < 14) { print "segment not aligned to 16 KB: " $0 > "/dev/stderr"; short = 1 }
  }
  END { exit (segments > 0 && !short) ? 0 : 1 }
'; then
  echo "$bin: a LOAD segment is aligned below 2**14, or none was found" >&2
  exit 1
fi
echo "$bin: every LOAD segment is aligned to 16 KB or more"
