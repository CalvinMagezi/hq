#!/usr/bin/env bash
# Tests changed-areas.sh against throwaway git repositories. No network.
# Run: bash scripts/ci/changed-areas-test.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
script=$here/changed-areas.sh
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT

git init -q "$t/repo"
cd "$t/repo"
git config user.email test@example.com
git config user.name test
git config commit.gpgsign false
echo base > seed
git add -A
git commit -q -m base
base=$(git rev-parse HEAD)

fails=0
n=0
# check "<expected rust,web_app,android,site,installer,workflows>" <path>...
# Writes each path (creating folders), commits on top of base, runs the script, then resets.
check() {
    local want=$1; shift
    local p
    for p in "$@"; do
        mkdir -p "$(dirname "$p")"
        echo "change $n" > "$p"
    done
    git add -A
    git commit -q -m "case $n"
    local out="$t/out.$n"
    : > "$out"
    BASE=$base HEAD=$(git rev-parse HEAD) EVENT_NAME=${EVENT_NAME:-pull_request} GITHUB_OUTPUT=$out bash "$script" 2> /dev/null
    local got
    got=$(sed 's/^[a-z_]*=//' "$out" | paste -sd, -)
    if [ "$got" = "$want" ]; then
        echo "ok   $want  <- $*"
    else
        echo "FAIL want $want got $got  <- $*" >&2
        fails=$((fails + 1))
    fi
    git reset -q --hard "$base"
    git clean -qfd
    n=$((n + 1))
}

T=true F=false
#     rust web_app android site installer workflows
check "$F,$F,$F,$F,$F,$F" docs/ARCHITECTURE.md                      # docs only: nothing runs
check "$F,$F,$F,$F,$F,$F" CHANGELOG.md LICENSE NOTICE               # root prose
check "$T,$F,$F,$F,$F,$F" README.md                                 # a Rust test reads it
check "$T,$F,$F,$F,$F,$F" docs/DESKTOP.md                           # a Rust test reads it
check "$T,$F,$T,$F,$F,$F" crates/hq-core/src/lib.rs                 # Rust, and the Android build
check "$T,$F,$T,$F,$F,$F" Cargo.lock
check "$T,$F,$F,$F,$F,$F" deny.toml .cargo/config.toml .config/nextest.toml
check "$T,$T,$F,$F,$F,$F" apps/hq-web/src/app.tsx
check "$T,$T,$T,$F,$F,$F" apps/strings/en.json
check "$F,$F,$T,$F,$F,$F" apps/android/app/build.gradle.kts
check "$F,$F,$F,$T,$F,$F" apps/site/src/index.ts apps/deploy/src/main.ts
check "$F,$F,$F,$F,$T,$F" packages/agent-hq/bin/agent-hq.js
check "$T,$F,$F,$T,$T,$F" scripts/windows/install.ps1
check "$T,$F,$F,$F,$F,$F" scripts/personal-data-scan.sh
check "$F,$F,$F,$F,$F,$T" .github/workflows/android.yml
check "$F,$F,$F,$F,$F,$F" .github/CODEOWNERS
check "$T,$T,$T,$T,$T,$T" .github/workflows/ci.yml                  # a CI change runs all of CI
check "$T,$T,$T,$T,$T,$T" some/unlisted/path.txt                    # unknown path: fail open
check "$T,$T,$T,$T,$T,$T" weird-root-file                           # unknown root file
check "$T,$T,$T,$T,$T,$T" docs/ARCHITECTURE.md some/unlisted/x      # one unknown path wins
check "$T,$F,$T,$F,$F,$F" docs/ARCHITECTURE.md crates/hq-db/x.rs    # a mixed change takes the union

# A deleted file counts, and so does the old name of a renamed one.
git rm -q seed
git commit -q -m delete
out=$t/out.del; : > "$out"
BASE=$base HEAD=$(git rev-parse HEAD) EVENT_NAME=pull_request GITHUB_OUTPUT=$out bash "$script" 2> /dev/null
if [ "$(sed 's/^[a-z_]*=//' "$out" | paste -sd, -)" = "$T,$T,$T,$T,$T,$T" ]; then
    echo "ok   deleting an unlisted file fails open"
else
    echo "FAIL deleting an unlisted file" >&2; fails=$((fails + 1))
fi
git reset -q --hard "$base"

# Fail open: not a pull request, missing or null base, a base git does not know, an empty diff.
mkdir -p docs; echo "x" > docs/only.md; git add -A; git commit -q -m docs; head=$(git rev-parse HEAD)
open() {
    local label=$1; shift
    local out=$t/out.open; : > "$out"
    env "$@" GITHUB_OUTPUT="$out" bash "$script" 2> /dev/null || true
    if [ "$(sed 's/^[a-z_]*=//' "$out" | paste -sd, -)" = "$T,$T,$T,$T,$T,$T" ]; then
        echo "ok   fail open: $label"
    else
        echo "FAIL fail open: $label" >&2; fails=$((fails + 1))
    fi
}
open "push event"        BASE="$base" HEAD="$head" EVENT_NAME=push
open "no event"          BASE="$base" HEAD="$head" EVENT_NAME=
open "empty base"        BASE= HEAD="$head" EVENT_NAME=pull_request
open "null base"         BASE=0000000000000000000000000000000000000000 HEAD="$head" EVENT_NAME=pull_request
open "unknown base"      BASE=1111111111111111111111111111111111111111 HEAD="$head" EVENT_NAME=pull_request
open "empty diff"        BASE="$head" HEAD="$head" EVENT_NAME=pull_request

if [ "$fails" -gt 0 ]; then echo "$fails failed" >&2; exit 1; fi
echo "all passed"
