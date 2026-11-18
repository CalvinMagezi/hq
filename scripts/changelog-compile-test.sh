#!/usr/bin/env bash
# Tests changelog-compile.sh against throwaway folders. No network.
# Run: bash scripts/changelog-compile-test.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
fails=0

run() { bash "$here/changelog-compile.sh" --root "$root" "$@"; }
new_root() {
    root=$(mktemp -d -p "$t")
    mkdir "$root/changelog.d"
    cat > "$root/CHANGELOG.md" <<'MD'
# Changelog

## [Unreleased]

### Added

- **Old added entry.** Stays below new ones.

### Fixed

- **Old fixed entry.**

### Added

- **A second Added block.**

## [1.0.0] - 2026-01-01

### Added

- **Shipped.** Never touched.
MD
}
ok() { if "$@"; then :; else echo "FAIL: $*" >&2; fails=$((fails + 1)); fi; }
no() { if "$@" > /dev/null 2>&1; then echo "FAIL (should refuse): $*" >&2; fails=$((fails + 1)); fi; }
has() { grep -qF -- "$2" "$1" || { echo "FAIL: '$2' missing from $1" >&2; fails=$((fails + 1)); }; }
lacks() { ! grep -qF -- "$2" "$1" || { echo "FAIL: '$2' should not be in $1" >&2; fails=$((fails + 1)); }; }
line_of() { grep -nF -- "$2" "$1" | head -1 | cut -d: -f1; }
before() { # before FILE A B: A comes before B
    local a b; a=$(line_of "$1" "$2"); b=$(line_of "$1" "$3")
    [ -n "$a" ] && [ -n "$b" ] && [ "$a" -lt "$b" ] || { echo "FAIL: '$2' not before '$3'" >&2; fails=$((fails + 1)); }
}

# 1. A fragment goes first in the first matching section, and the file is deleted.
new_root
printf '**New thing.** It does a thing.\n' > "$root/changelog.d/new-thing.added.md"
ok run
has "$root/CHANGELOG.md" '- **New thing.** It does a thing.'
before "$root/CHANGELOG.md" '**New thing.**' '**Old added entry.**'
before "$root/CHANGELOG.md" '**New thing.**' '## [1.0.0]'
[ ! -e "$root/changelog.d/new-thing.added.md" ] || { echo "FAIL: fragment not deleted" >&2; fails=$((fails + 1)); }

# 2. A kind with no section gets one at the top of [Unreleased]; released sections are untouched.
new_root
printf '**Closed a hole.** Details.\n' > "$root/changelog.d/hole.security.md"
ok run
has "$root/CHANGELOG.md" '### Security'
before "$root/CHANGELOG.md" '### Security' '### Added'
before "$root/CHANGELOG.md" '**Closed a hole.**' '**Old added entry.**'
sed -n '/^## \[1.0.0\]/,$p' "$root/CHANGELOG.md" > "$t/released.after"
printf '## [1.0.0] - 2026-01-01\n\n### Added\n\n- **Shipped.** Never touched.\n' | diff -q - "$t/released.after" > /dev/null \
    || { echo "FAIL: a released section changed" >&2; fails=$((fails + 1)); }

# 3. Several kinds, several fragments: name order, one bullet each, continuation lines indented.
new_root
printf '**B.** Second.\n' > "$root/changelog.d/bbb.fixed.md"
printf '**A.** First.\n\nA second paragraph.\n' > "$root/changelog.d/aaa.fixed.md"
printf '**C.** Changed.\n' > "$root/changelog.d/ccc.changed.md"
ok run
before "$root/CHANGELOG.md" '**A.** First.' '**B.** Second.'
before "$root/CHANGELOG.md" '**B.** Second.' '**Old fixed entry.**'
has "$root/CHANGELOG.md" '  A second paragraph.'
has "$root/CHANGELOG.md" '### Changed'

# 4. --keep folds but leaves the file; --dry-run writes nothing.
new_root
printf '**Kept.** Text.\n' > "$root/changelog.d/kept.added.md"
cp "$root/CHANGELOG.md" "$t/before"
ok run --dry-run
cmp -s "$root/CHANGELOG.md" "$t/before" || { echo "FAIL: --dry-run wrote" >&2; fails=$((fails + 1)); }
ok run --keep
has "$root/CHANGELOG.md" '**Kept.**'
[ -e "$root/changelog.d/kept.added.md" ] || { echo "FAIL: --keep deleted the file" >&2; fails=$((fails + 1)); }

# 5. Nothing to fold is not an error and changes nothing; a second run folds nothing twice.
new_root
cp "$root/CHANGELOG.md" "$t/before"
ok run
cmp -s "$root/CHANGELOG.md" "$t/before" || { echo "FAIL: empty run changed the file" >&2; fails=$((fails + 1)); }
printf '**Once.** Text.\n' > "$root/changelog.d/once.added.md"
ok run
ok run
[ "$(grep -cF '**Once.**' "$root/CHANGELOG.md")" = 1 ] || { echo "FAIL: folded twice" >&2; fails=$((fails + 1)); }

# 6. README.md in the folder is not a fragment.
new_root
printf '# readme\n' > "$root/changelog.d/README.md"
ok run --check

# 7. Bad fragments are refused, and nothing is written when any one is bad.
for bad in 'Bad Name.added.md' 'noslug.md' 'x.unknownkind.md' 'x.added.txt' 'UPPER.added.md'; do
    new_root
    printf '**Text.**\n' > "$root/changelog.d/$bad"
    no run --check
done
new_root
: > "$root/changelog.d/empty.added.md"
no run --check
new_root
printf -- '- **Starts with a dash.**\n' > "$root/changelog.d/dash.added.md"
no run --check
new_root
printf '**Good.** Text.\n' > "$root/changelog.d/good.added.md"
: > "$root/changelog.d/bad.added.md"
cp "$root/CHANGELOG.md" "$t/before"
no run
cmp -s "$root/CHANGELOG.md" "$t/before" || { echo "FAIL: wrote despite a bad fragment" >&2; fails=$((fails + 1)); }
[ -e "$root/changelog.d/good.added.md" ] || { echo "FAIL: deleted a fragment despite a bad one" >&2; fails=$((fails + 1)); }

# 8. A changelog with no [Unreleased] heading is refused.
new_root
printf '# Changelog\n\n## [1.0.0]\n' > "$root/CHANGELOG.md"
printf '**X.** Y.\n' > "$root/changelog.d/x.added.md"
no run

if [ "$fails" -gt 0 ]; then echo "$fails failed" >&2; exit 1; fi
echo "all passed"
