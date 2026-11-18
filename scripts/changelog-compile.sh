#!/usr/bin/env bash
# Folds changelog.d/<slug>.<kind>.md fragments into CHANGELOG.md's [Unreleased] section.
# Run it when a release is cut. `--check` validates fragments without writing; see
# changelog.d/README.md and scripts/changelog_compile.py for the options.
set -euo pipefail
exec python3 -I "$(cd "$(dirname "$0")" && pwd)/changelog_compile.py" "$@"
