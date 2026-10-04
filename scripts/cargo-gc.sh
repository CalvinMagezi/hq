#!/bin/bash
# cargo-gc.sh — Garbage-collect Cargo build artifacts when they exceed a threshold.
#
# The debug profile accumulates stale test binaries and incremental compilation
# artifacts that Cargo never cleans up. On a 14-crate workspace with heavy deps
# (tokio, reqwest, serenity, teloxide, rusqlite), this can reach
# 100GB+ within weeks of active development.
#
# Usage:
#   ./scripts/cargo-gc.sh           # Clean if debug > 10GB (default)
#   ./scripts/cargo-gc.sh 20        # Clean if debug > 20GB
#   CARGO_GC_DRY_RUN=1 ./scripts/cargo-gc.sh  # Preview without cleaning

set -euo pipefail

THRESHOLD_GB="${1:-10}"
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET_DIR="${PROJECT_ROOT}/target"

if [ ! -d "$TARGET_DIR/debug" ]; then
    echo "No debug artifacts to clean."
    exit 0
fi

# Get size in GB (macOS du -sk gives KB)
SIZE_KB=$(du -sk "$TARGET_DIR/debug" 2>/dev/null | cut -f1 || echo "0")
SIZE_GB=$((SIZE_KB / 1024 / 1024))

echo "target/debug: ${SIZE_GB}GB (threshold: ${THRESHOLD_GB}GB)"

if [ "$SIZE_GB" -lt "$THRESHOLD_GB" ]; then
    echo "Under threshold. No cleanup needed."
    exit 0
fi

if [ "${CARGO_GC_DRY_RUN:-0}" = "1" ]; then
    echo "[DRY RUN] Would clean ${SIZE_GB}GB of debug artifacts."
    exit 0
fi

echo "Cleaning debug artifacts (${SIZE_GB}GB > ${THRESHOLD_GB}GB threshold)..."
cd "$PROJECT_ROOT"
cargo clean --profile dev 2>&1

echo "Done. Run 'cargo check' to rebuild essentials."
