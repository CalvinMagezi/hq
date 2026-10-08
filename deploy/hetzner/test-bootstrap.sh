#!/usr/bin/env bash
# Offline checks for deploy/hetzner: lint, dry run, and the rule that user-data holds no secrets.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
sample_ref="0123456789abcdef0123456789abcdef01234567"
fail() { echo "test-bootstrap.sh: $*" >&2; exit 1; }

command -v shellcheck > /dev/null || fail "install shellcheck"
shellcheck "$here"/bootstrap.sh "$here"/hq-join.sh "$here"/hcloud.sh "$here"/test-bootstrap.sh

plan="$(bash "$here/bootstrap.sh" --plan --repo example/hq --ref "$sample_ref" --hostname hq-test)"
for step in "Install HQ from signed releases" "Generate the web token" "not joined" "Restrict SSH to keys"; do
    grep -q "$step" <<< "$plan" || fail "--plan output is missing: $step"
done

placeholders="$(grep -o '@[A-Z0-9]*@' "$here/cloud-init.yaml" | sort -u | tr '\n' ' ')"
[ "$placeholders" = "@HOSTNAME@ @REF@ @REPO@ @SHA256@ " ] || fail "cloud-init.yaml placeholders changed: $placeholders"
grep -qiE 'token|key|password|authkey' <(grep -v '^#' "$here/cloud-init.yaml") && fail "cloud-init.yaml must not carry secrets"

for bad in "--ref nothex" "--hostname Bad_Name"; do
    # shellcheck disable=SC2086
    bash "$here/bootstrap.sh" --plan --repo example/hq --ref "$sample_ref" --hostname hq-test $bad > /dev/null 2>&1 && fail "accepted bad input: $bad"
done
echo "deploy/hetzner checks passed"
