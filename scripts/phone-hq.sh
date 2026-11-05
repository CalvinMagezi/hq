#!/usr/bin/env bash
# Read HQ's counts-only reports from a phone, over the bridge, without the person's chats ever
# leaving it. The phone computes the report; only the JSON counts come back.
#
#   scripts/phone-hq.sh signin <code>          # once: the code from Settings, Devices on the phone
#   scripts/phone-hq.sh adoption               # what `hq adoption --json` prints, from the phone
#   scripts/phone-hq.sh offers replay [--days 56]
#   scripts/phone-hq.sh signout                # forget the device token on this machine
#
# A release build of the app is not debuggable, so `adb run-as` cannot reach its files. This
# script signs this machine in as a device instead (the owner gets an inbox notice and can sign
# it out from Settings, Devices) and keeps that token in a file only this user can read.
#
# HQ_PHONE_URL is the phone's web server as this machine reaches it; the default is the bridge's
# forward on loopback (`hq bridge`, docs/PAIRING.md). HQ_PHONE_TOKEN_FILE moves the token file.
#
# Never use this to copy the database or chat history off the phone (docs/security/ADOPTION_DATA.md).
set -euo pipefail

URL="${HQ_PHONE_URL:-http://127.0.0.1:47480}"
TOKEN_FILE="${HQ_PHONE_TOKEN_FILE:-$HOME/.hq/phone-report-token}"

usage() {
  sed -n '5,8p' "$0" | sed 's/^# //' >&2
  exit 2
}

command -v curl >/dev/null || { echo "curl is not on PATH" >&2; exit 1; }

pretty() {
  if command -v jq >/dev/null; then jq .; else cat; echo; fi
}

token() {
  [ -s "$TOKEN_FILE" ] || { echo "not signed in: run '$0 signin <code>' with a code from the phone's Settings, Devices" >&2; exit 1; }
  cat "$TOKEN_FILE"
}

get() {
  curl -sSf -H "Authorization: Bearer $(token)" "$URL$1" | pretty
}

[ "$#" -gt 0 ] || usage
case "$1" in
  signin)
    [ "$#" -eq 2 ] || usage
    [[ "$2" =~ ^[A-Za-z0-9-]+$ ]] || { echo "a sign-in code is letters, digits and a dash" >&2; exit 2; }
    body=$(printf '{"code":"%s"}' "$2")
    reply=$(curl -sSf -X POST -H 'content-type: application/json' -H 'x-hq-client: web' \
      --data "$body" "$URL/api/auth/exchange")
    tok=$(printf '%s' "$reply" | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')
    [ -n "$tok" ] || { echo "the phone did not hand out a token" >&2; exit 1; }
    mkdir -p "$(dirname "$TOKEN_FILE")"
    (umask 077; printf '%s' "$tok" > "$TOKEN_FILE")
    echo "signed in; token kept in $TOKEN_FILE (sign it out from the phone's Settings, Devices)"
    ;;
  signout)
    rm -f "$TOKEN_FILE"
    echo "token removed from this machine; also sign it out on the phone (Settings, Devices)"
    ;;
  adoption)
    get /api/adoption
    ;;
  offers)
    [ "${2:-}" = "replay" ] || usage
    days=""
    if [ "${3:-}" = "--days" ]; then
      [[ "${4:-}" =~ ^[0-9]+$ ]] || usage
      days="?days=$4"
    fi
    get "/api/offers/replay$days"
    ;;
  *)
    usage
    ;;
esac
