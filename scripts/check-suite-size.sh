#!/usr/bin/env bash
# Fails if the HQ Suite adds more than its budget to the `hq` binary.
#
# Builds `hq` twice with the release-small profile, once without the `suite` feature and once
# with it, and compares the two. The budget is 6 MiB (docs/plans/hq-suite.md); set
# HQ_SUITE_BUDGET_BYTES to try another. Pass a target triple to measure the phone build, for
# example aarch64-linux-android (needs the Android NDK, as in .github/workflows/android.yml).
#
# Usage: scripts/check-suite-size.sh [target-triple]
# It uses its own target folder so it does not disturb a developer's build; set
# HQ_SUITE_SIZE_TARGET_DIR to put that somewhere with room (each build is several GB).
set -euo pipefail

budget=${HQ_SUITE_BUDGET_BYTES:-6291456}
target=${1:-}
dir=${HQ_SUITE_SIZE_TARGET_DIR:-target/suite-size}
args=(--profile release-small -p hq-cli --bin hq)
[ -n "$target" ] && args+=(--target "$target")

size_of() {
  if stat -c %s "$1" >/dev/null 2>&1; then stat -c %s "$1"; else stat -f %z "$1"; fi
}

bin_path() {
  if [ -n "$target" ]; then echo "$dir/$target/release-small/hq"; else echo "$dir/release-small/hq"; fi
}

build() {
  CARGO_TARGET_DIR="$dir" cargo build "${args[@]}" --no-default-features --features "$1" >&2
  size_of "$(bin_path)"
}

echo "building without the suite..." >&2
without=$(build "discord,telegram,bridge")
echo "building with the suite..." >&2
with=$(build "discord,telegram,bridge,suite")

delta=$((with - without))
printf 'without suite: %d bytes\nwith suite:    %d bytes\nsuite adds:    %d bytes (budget %d)\n' \
  "$without" "$with" "$delta" "$budget"
if [ "$delta" -gt "$budget" ]; then
  echo "the suite is over its size budget by $((delta - budget)) bytes" >&2
  exit 1
fi
