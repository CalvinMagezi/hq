#!/usr/bin/env bash
# Runs an HQ image and checks what a user depends on: first-run token, /health,
# the web UI, token enforcement, and that the token survives a restart.
#
#   docker/smoke-test.sh IMAGE
#
# CONTAINER_CLI picks the runtime (default "docker", for example "sudo nerdctl").
# HOST_PORT is the loopback port to publish on (default 15678).
set -euo pipefail

image=${1:?usage: smoke-test.sh IMAGE}
read -ra cli <<< "${CONTAINER_CLI:-docker}"
port=${HOST_PORT:-15678}
name="hq-smoke-$$"
base="http://127.0.0.1:$port"
health_tries=60

fail() { echo "smoke-test: FAIL: $*" >&2; "${cli[@]}" logs "$name" 2>&1 | tail -n 40 >&2 || true; exit 1; }
pass() { echo "smoke-test: ok: $*"; }
cleanup() { "${cli[@]}" rm -f -v "$name" > /dev/null 2>&1 || true; }
trap cleanup EXIT

status() { curl --silent --output /dev/null --write-out '%{http_code}' --max-time 5 "$@" || true; }

wait_healthy() {
    for _ in $(seq "$health_tries"); do
        [ "$(status "$base/health")" = 200 ] && return 0
        sleep 1
    done
    return 1
}

"${cli[@]}" run -d --name "$name" -p "127.0.0.1:$port:5678" "$image" > /dev/null
wait_healthy || fail "/health did not return 200 within ${health_tries}s"
pass "/health is 200"

[ "$(status "$base/")" = 200 ] || fail "web UI at / is not 200"
pass "web UI at / is 200"

token=$("${cli[@]}" logs "$name" 2>&1 | sed -n 's/^ Web token: *//p' | head -n 1)
[[ "$token" =~ ^[0-9a-f]{64}$ ]] || fail "first-run banner did not show a 64 hex character token"
pass "first run printed a token"

[ "$(status "$base/api/ws-ticket" -X POST)" = 401 ] || fail "unauthenticated /api call was not refused with 401"
[ "$(status "$base/api/ws-ticket" -X POST -H 'Authorization: Bearer wrong')" = 401 ] || fail "wrong token was not refused"
[ "$(status "$base/api/ws-ticket" -X POST -H "Authorization: Bearer $token")" = 200 ] || fail "the generated token was refused"
pass "/api refuses no token and a wrong token, accepts the generated one"

lines_before=$("${cli[@]}" logs "$name" 2>&1 | wc -l)
"${cli[@]}" restart "$name" > /dev/null
wait_healthy || fail "/health did not return after a restart"
[ "$(status "$base/api/ws-ticket" -X POST -H "Authorization: Bearer $token")" = 200 ] || fail "token did not survive a restart"
pass "the same token works after a restart"

# Output of the second start only: the banner must not repeat the token.
if "${cli[@]}" logs "$name" 2>&1 | tail -n "+$((lines_before + 1))" | grep -q "$token"; then
    fail "the token was printed again after the first run"
fi
pass "the token is not printed again"

echo "smoke-test: all checks passed for $image"
