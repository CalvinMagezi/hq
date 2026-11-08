#!/usr/bin/env bash
# Checks where the plugin's hooks keep their marker files: in a 0700 directory of this user's,
# never through a symlink. Needs python3. Run: bash packages/claude-plugin/test/hooks-test.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
scripts="$here/../scripts"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fails=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1" >&2; fails=$((fails + 1)); fi; }
mode() { stat -c %a "$1" 2>/dev/null || stat -f %Lp "$1"; }

home="$work/home"
mkdir -p "$home"
run() { # run <script> <json> [extra env...]
  local s=$1 j=$2
  shift 2
  printf '%s' "$j" | env -i PATH="$PATH" HOME="$home" "$@" bash "$scripts/$s"
}
nudge_input() { printf '{"tool_name":"Agent","tool_input":{"subagent_type":"Explore"},"session_id":"%s"}' "$1"; }

# fleet-nudge: first launch is sent back, the second goes through, and the marker directory is private.
out=$(run fleet-nudge.sh "$(nudge_input s1)")
check "first Explore launch is nudged" '[[ "$out" == *permissionDecision*deny* ]]'
out=$(run fleet-nudge.sh "$(nudge_input s1)")
check "the same session is not nudged twice" '[ -z "$out" ]'
marks="$home/.cache/agent-hq-plugin"
check "markers go under ~/.cache when XDG_RUNTIME_DIR is unset" '[ -f "$marks/s1.fleet-nudged" ]'
check "the marker directory is 0700" '[ "$(mode "$marks")" = 700 ]'
check "nothing is written to the shared temp directory" '[ ! -e "${TMPDIR:-/tmp}/agent-hq-plugin/s1.fleet-nudged" ]'

# A symlink planted at a marker path is never followed.
ln -s "$work/planted-nudge" "$marks/s2.fleet-nudged"
out=$(run fleet-nudge.sh "$(nudge_input s2)")
check "a symlinked nudge marker is not written through" '[ ! -e "$work/planted-nudge" ]'
check "and the launch goes through" '[ -z "$out" ]'

# A marker directory that is itself a symlink is refused.
home2="$work/home2"
mkdir -p "$home2/.cache" "$work/elsewhere"
ln -s "$work/elsewhere" "$home2/.cache/agent-hq-plugin"
out=$(printf '%s' "$(nudge_input s3)" | env -i PATH="$PATH" HOME="$home2" bash "$scripts/fleet-nudge.sh")
check "a symlinked marker directory is refused" '[ -z "$(ls -A "$work/elsewhere")" ] && [ -z "$out" ]'

# A loose existing directory is tightened to 0700.
home3="$work/home3"
mkdir -p "$home3/.cache/agent-hq-plugin"
chmod 755 "$home3/.cache/agent-hq-plugin"
printf '%s' "$(nudge_input s4)" | env -i PATH="$PATH" HOME="$home3" bash "$scripts/fleet-nudge.sh" > /dev/null
check "an existing 0755 marker directory becomes 0700" '[ "$(mode "$home3/.cache/agent-hq-plugin")" = 700 ]'

# XDG_RUNTIME_DIR wins when it is set.
xdg="$work/xdg"
mkdir -m 700 "$xdg"
run fleet-nudge.sh "$(nudge_input s5)" XDG_RUNTIME_DIR="$xdg" > /dev/null
check "markers go under XDG_RUNTIME_DIR when it is set" '[ -f "$xdg/agent-hq-plugin/s5.fleet-nudged" ]'

# lease-check: a claimed, unreleased task is reminded once, and a planted symlink is not followed.
transcript="$work/transcript.jsonl"
printf '%s\n' '{"message":{"content":[{"type":"tool_use","id":"t1","name":"mcp__hq__hq_call","input":{"tool":"task_claim"}}]}}' > "$transcript"
stop_input() { printf '{"transcript_path":"%s","session_id":"%s"}' "$transcript" "$1"; }
out=$(run lease-check.sh "$(stop_input l1)")
check "an unreleased claim asks for a release" '[[ "$out" == *'"'"'"decision": "block"'"'"'* ]]'
out=$(run lease-check.sh "$(stop_input l1)")
check "and asks only once" '[ -z "$out" ]'
check "the lease marker is 0600" '[ "$(mode "$marks/l1.lease-checked")" = 600 ]'
printf 'keep' > "$work/planted-lease"
ln -s "$work/planted-lease" "$marks/l2.lease-checked"
out=$(run lease-check.sh "$(stop_input l2)")
check "a symlinked lease marker is not written through" '[ "$(cat "$work/planted-lease")" = keep ]'
check "and no reminder is sent without a marker" '[ -z "$out" ]'

[ "$fails" -eq 0 ] || { echo "$fails check(s) failed" >&2; exit 1; }
echo "plugin hook markers: all checks passed"
