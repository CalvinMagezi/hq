#!/usr/bin/env bash
# Fails when a retired project name comes back into the tree. HQ runs its own
# agent host; the name of the project it replaced is true only where credit is
# owed (NOTICE, provenance records, the rule files derived from it), in release
# history, in migrations that already ran, and in the one compatibility alias
# that lets configs written before the rename keep loading.
set -euo pipefail
export LC_ALL=C

# Written in two pieces so this file does not match itself.
WORD="her""dr"

ALLOWED='^(NOTICE|CHANGELOG\.md|docs/provenance/|crates/hq-db/sql/|crates/hq-db/src/migrations\.rs|crates/hq-db/src/harness_sessions_registry/tests\.rs|crates/hq-host/src/detect/|crates/hq-host/src/lib\.rs|crates/hq-core/src/config/(mod|tests)\.rs|crates/hq-tools/src/agent_host/pairing(\.rs|/)|docs/AGENT_HOST\.md|scripts/retired-names-check\.sh)'

bad=0
while IFS= read -r file; do
  if [[ "$file" =~ $ALLOWED ]]; then continue; fi
  if grep -I -i -q "$WORD" -- "$file" 2>/dev/null; then
    grep -I -i -n "$WORD" -- "$file" | head -3 | sed "s|^|retired-names-check: $file:|" >&2
    bad=1
  fi
done < <(git ls-files)

if [ "$bad" = 0 ]; then echo "retired-names-check: ok"; else
  echo "retired-names-check: the name above is retired; use the host's own wording (see docs/AGENT_HOST.md)" >&2
fi
exit "$bad"
