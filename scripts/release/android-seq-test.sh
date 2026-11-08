#!/usr/bin/env bash
# Tests android-seq.sh against a fake `gh` that keeps tags in a file. No network.
# Run: bash scripts/release/android-seq-test.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT

mkdir "$t/bin"
cat > "$t/bin/gh" <<'FAKE'
#!/usr/bin/env bash
# fake gh: `api --paginate repos/R/git/matching-refs/tags/P --jq ...` lists $REFS lines starting
# with refs/tags/P; `api repos/R/git/refs -f ref=X -f sha=Y --jq .ref` appends X.
[ -z "${GH_FAIL:-}" ] || { echo "fake gh: HTTP 502" >&2; exit 1; }
[ "$1" = api ] || exit 9
shift
[ "$1" = --paginate ] && shift
case "$1" in
    */git/matching-refs/tags/*) p=${1#*/git/matching-refs/tags/}; grep "^refs/tags/$p" "$REFS" || true ;;
    */git/refs)
        ref=""
        while [ $# -gt 0 ]; do case "$1" in -f) case "$2" in ref=*) ref=${2#ref=} ;; esac; shift 2 ;; *) shift ;; esac; done
        grep -qx "$ref" "$REFS" && { echo "fake gh: Reference already exists" >&2; exit 1; }
        echo "$ref" >> "$REFS"; echo "$ref" ;;
    *) exit 9 ;;
esac
FAKE
chmod +x "$t/bin/gh"
export PATH="$t/bin:$PATH" REFS="$t/refs"
: > "$REFS"
s() { bash "$here/android-seq.sh" "$@"; }
fails=0
ok() { if "$@" > /dev/null 2>&1; then echo "ok   $*"; else echo "FAIL $*" >&2; fails=$((fails + 1)); fi; }
no() { if "$@" > /dev/null 2>&1; then echo "FAIL (should refuse) $*" >&2; fails=$((fails + 1)); else echo "ok   refused: $*"; fi; }
sha=$(printf '0%.0s' $(seq 40))

[ "$(s floor --repo a/b)" = 0 ] && echo "ok   no tags means floor 0" || { echo "FAIL floor with no tags" >&2; fails=$((fails + 1)); }
ok s check --repo a/b --seq 5
ok s record --repo a/b --seq 1700000000 --sha "$sha"
ok s record --repo a/b --seq 1700000500 --sha "$sha"
printf '%s\n' refs/tags/android-stable-seq-junk refs/tags/android-stable-seq-12x >> "$REFS"
[ "$(s floor --repo a/b 2> /dev/null)" = 1700000500 ] && echo "ok   floor is the highest well-formed tag" || { echo "FAIL floor" >&2; fails=$((fails + 1)); }
ok s check --repo a/b --seq 1700000500
ok s check --repo a/b --seq 1800000000
no s check --repo a/b --seq 1700000499
no s check --repo a/b --seq 0
no s record --repo a/b --seq 1700000500 --sha "$sha"
no s record --repo a/b --seq 12 --sha main
no s check --repo a/b --seq -1
no s check --repo a/b --seq ""
no s check --repo 'a/b;x' --seq 1
no s check --repo a/b --seq
GH_FAIL=1 no s check --repo a/b --seq 1800000000
GH_FAIL=1 no s floor --repo a/b

[ "$fails" -eq 0 ] || { echo "$fails check(s) failed" >&2; exit 1; }
echo "android-seq.sh: all checks passed"
