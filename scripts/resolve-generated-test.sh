#!/usr/bin/env bash
# Tests resolve-generated.sh in throwaway git repositories. No network.
# Run: bash scripts/resolve-generated-test.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo_root=$(cd "$here/.." && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
fails=0
fail() { echo "FAIL: $*" >&2; fails=$((fails + 1)); }

git_init() {
    git init -q -b main "$1"
    git -C "$1" config user.email test@example.com
    git -C "$1" config user.name test
    git -C "$1" config commit.gpgsign false
}
has_markers() { grep -qE '^(<<<<<<<|>>>>>>>)' "$1"; }

# ---- Cargo.lock: two branches each add a crate; the lock conflicts, the manifests do not.
mk_cargo_repo() {
    git_init "$1"
    ( cd "$1"
      mkdir -p crates/base/src
      printf '[workspace]\nmembers = ["crates/*"]\nresolver = "2"\n' > Cargo.toml
      printf '[package]\nname = "base"\nversion = "0.1.0"\nedition = "2021"\n' > crates/base/Cargo.toml
      echo '' > crates/base/src/lib.rs
      cargo generate-lockfile --offline -q
      git add -A && git commit -q -m base
      for n in zed zee; do
          git checkout -q -b "add-$n" main
          mkdir -p "crates/$n/src"
          printf '[package]\nname = "%s"\nversion = "0.1.0"\nedition = "2021"\n' "$n" > "crates/$n/Cargo.toml"
          echo '' > "crates/$n/src/lib.rs"
          cargo metadata --format-version 1 --offline > /dev/null 2>&1
          git add -A && git commit -q -m "add $n"
      done
      git checkout -q add-zed )
}

# merge: bring add-zee into add-zed
r="$t/cargo-merge"; mk_cargo_repo "$r"
if ( cd "$r" && git merge -q --no-edit add-zee > /dev/null 2>&1 ); then
    echo "note: the lock merged without a conflict, so the merge case is not exercised"
else
    ( cd "$r" && git diff --name-only --diff-filter=U | grep -qx Cargo.lock ) || fail "merge: expected a Cargo.lock conflict"
    ( cd "$r" && bash "$here/resolve-generated.sh" > /dev/null ) || fail "merge: script failed"
    has_markers "$r/Cargo.lock" && fail "merge: markers left in Cargo.lock"
    ( cd "$r" && cargo metadata --format-version 1 --offline --locked > /dev/null 2>&1 ) || fail "merge: Cargo.lock does not match the manifests"
    for n in zed zee base; do grep -q "name = \"$n\"" "$r/Cargo.lock" || fail "merge: $n missing from Cargo.lock"; done
fi

# rebase: rebase add-zee onto add-zed
r="$t/cargo-rebase"; mk_cargo_repo "$r"
if ( cd "$r" && git checkout -q add-zee && git rebase -q add-zed > /dev/null 2>&1 ); then
    echo "note: the lock rebased without a conflict, so the rebase case is not exercised"
else
    ( cd "$r" && bash "$here/resolve-generated.sh" > /dev/null ) || fail "rebase: script failed"
    has_markers "$r/Cargo.lock" && fail "rebase: markers left in Cargo.lock"
    ( cd "$r" && cargo metadata --format-version 1 --offline --locked > /dev/null 2>&1 ) || fail "rebase: Cargo.lock does not match the manifests"
    for n in zed zee base; do grep -q "name = \"$n\"" "$r/Cargo.lock" || fail "rebase: $n missing from Cargo.lock"; done
fi

# nothing in conflict: the script says so and changes nothing
r="$t/clean"; mk_cargo_repo "$r"
out=$( cd "$r" && bash "$here/resolve-generated.sh" 2>&1 )
echo "$out" | grep -q 'no generated file is in conflict' || fail "clean: wrong message: $out"
[ -z "$(git -C "$r" status --porcelain)" ] || fail "clean: the script changed files"

# an unknown option is refused
( cd "$r" && bash "$here/resolve-generated.sh" --bogus > /dev/null 2>&1 ) && fail "an unknown option was accepted"

# ---- strings: the generated outputs conflict, the hand-written catalogue does not.
if command -v bun > /dev/null; then
    mk_strings_repo() {
        git_init "$1"
        ( cd "$1"
          mkdir -p scripts apps/strings apps/hq-web/src/strings apps/android/app/src/main/res/values
          cp -r "$repo_root/scripts/strings" scripts/strings
          cp "$repo_root/apps/strings/en.yaml" apps/strings/en.yaml
          bun scripts/strings/build.ts > /dev/null
          git add -A && git commit -q -m base )
    }
    r="$t/strings"; mk_strings_repo "$r"
    ( cd "$r"
      for side in a b; do
          git checkout -q -b "edit-$side" main
          # hand-edit the same line of both generated files differently
          sed -i "0,/^export/s//\/\/ edited on $side\nexport/" apps/hq-web/src/strings/en.gen.ts
          sed -i "0,/<resources>/s//<resources><!-- edited on $side -->/" apps/android/app/src/main/res/values/strings_catalogue.xml
          git add -A && git commit -q -m "edit $side"
      done
      git checkout -q edit-a )
    if ( cd "$r" && git merge -q --no-edit edit-b > /dev/null 2>&1 ); then
        echo "note: the strings outputs merged without a conflict, so that case is not exercised"
    else
        ( cd "$r" && bash "$here/resolve-generated.sh" > /dev/null ) || fail "strings: script failed"
        has_markers "$r/apps/hq-web/src/strings/en.gen.ts" && fail "strings: markers left in en.gen.ts"
        ( cd "$r" && bun scripts/strings/build.ts --check > /dev/null 2>&1 ) || fail "strings: outputs are not up to date with the catalogue"
    fi

    # the catalogue itself in conflict: refused, with the reason
    r="$t/strings-src"; mk_strings_repo "$r"
    ( cd "$r"
      for side in a b; do
          git checkout -q -b "src-$side" main
          sed -i "1s/^/# edited on $side\n/" apps/strings/en.yaml
          sed -i "0,/^export/s//\/\/ edited on $side\nexport/" apps/hq-web/src/strings/en.gen.ts
          git add -A && git commit -q -m "src $side"
      done
      git checkout -q src-a )
    if ( cd "$r" && git merge -q --no-edit src-b > /dev/null 2>&1 ); then
        echo "note: the catalogue merged without a conflict, so the refusal is not exercised"
    else
        out=$( cd "$r" && bash "$here/resolve-generated.sh" 2>&1 ) && fail "strings-src: should refuse when the catalogue is in conflict"
        echo "$out" | grep -q 'resolve apps/strings/en.yaml by hand' || fail "strings-src: wrong refusal: $out"
    fi
else
    echo "note: bun is not installed, so the strings cases were skipped"
fi

if [ "$fails" -gt 0 ]; then echo "$fails failed" >&2; exit 1; fi
echo "all passed"
