#!/usr/bin/env bash
# Install the repo's git hooks into this clone (worktrees share them).
# Safe to re-run. Currently installs scripts/pre-commit, which rustfmts staged
# Rust files and blocks commits that carry personal data.

set -euo pipefail

root=$(git rev-parse --show-toplevel)
hooks=$(git -C "$root" rev-parse --path-format=absolute --git-common-dir)/hooks
mkdir -p "$hooks"

target="$hooks/pre-commit"
if [ -e "$target" ] && [ ! -L "$target" ]; then
    echo "Leaving $target alone: it is a file, not our symlink. Move it aside and re-run." >&2
    exit 1
fi
ln -sf "$root/scripts/pre-commit" "$target"
echo "pre-commit hook installed: $target -> $root/scripts/pre-commit"
