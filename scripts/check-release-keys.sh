#!/usr/bin/env bash
# Every minisign public key embedded in a tracked file must be the release key in
# release/minisign.pub, except the throwaway test keys listed below by value. A copy that drifts
# (an installer, the npm package, the website) would trust a different signer, so CI fails on it.
# Each place known to embed the release key must also still carry it.
# Run from the repository root: bash scripts/check-release-keys.sh
set -euo pipefail

key=$(sed -n 2p release/minisign.pub | tr -d '\r')
[[ "$key" =~ ^RW[A-Za-z0-9+/]{54}$ ]] || { echo "release/minisign.pub has no key line" >&2; exit 1; }

# Test-only keys, by value. Their secret halves sit beside them in test fixtures, so they sign
# nothing real. Add one here only for a test key.
test_keys=(
  "RWQBec9Eeu7RQiMUD1cx2V5qHDo1rOjJxWe9Ekj5EOiaaRvN4eIL/uQJ" # crates/hq-cli/src/commands/update.rs tests
  "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3" # packages/agent-hq/test/release.test.js
  "RWSnPwucpJMePpy0Ef9OTTXJymAJd+o1ZkDtAIsj50skvofLEyaEDcwj" # scripts/windows/fixtures/test.pub
  "RWTl7TyjQCMCADrczLAFdThUsHMzu2PLdnz5t6xmWCM/yoqssxitkrF0" # scripts/windows/fixtures/other.pub
)

# Files that must embed the release key itself.
embedders=(
  apps/site/public/install.sh
  apps/site/public/install.ps1
  apps/site/src/pages/security.astro
  packages/agent-hq/lib/release.js
)

bad=0
# A minisign public key is 42 bytes starting "Ed", so its base64 is 56 characters starting RW[QRST],
# standing alone: not part of a longer base64 run (an inline image), though it may follow a "\n"
# escape inside a string literal.
key_re='(^|[^A-Za-z0-9+/]|\\n)RW[QRST][A-Za-z0-9+/]{53}([^A-Za-z0-9+/=]|$)'
while IFS= read -r hit; do
  file=${hit%%:*}
  rest=${hit#*:}
  line=${rest%%:*}
  found=$(printf '%s' "${rest#*:}" | grep -oE 'RW[QRST][A-Za-z0-9+/]{53}' | head -n 1)
  [ "$found" = "$key" ] && continue
  allowed=0
  for t in "${test_keys[@]}"; do [ "$found" = "$t" ] && allowed=1; done
  if [ "$allowed" -eq 0 ]; then
    echo "$file:$line: embeds a minisign key that is not release/minisign.pub: $found" >&2
    bad=1
  fi
done < <(git ls-files -z | xargs -0 grep -I -n -o -E "$key_re" -- 2>/dev/null || true)

for f in "${embedders[@]}"; do
  grep -qF "$key" "$f" || { echo "$f no longer embeds the release key" >&2; bad=1; }
done

[ "$bad" -eq 0 ] || exit 1
echo "every embedded minisign key is the release key or a listed test key"
