#!/usr/bin/env bash
# Recreates the throwaway worktree for Spike E and applies the patches. Usage: prepare.sh <path to a clone of the hq repo> [destination]
set -euo pipefail
repo=${1:?path to a clone of the hq repo}
dest=${2:-../hq-android-scratch}
base=${BASE:-f81d703}
here=$(cd "$(dirname "$0")" && pwd)
git -C "$repo" worktree add --detach "$dest" "$base"
for patch in "$here"/patches/*.patch; do
  git -C "$dest" apply "$patch"
done
echo "worktree ready at $dest on $base plus $(ls "$here"/patches | wc -l | tr -d ' ') patch(es)"
