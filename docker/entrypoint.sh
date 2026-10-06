#!/usr/bin/env bash
# Container entrypoint: scaffold HQ under /data on first run, make sure the web
# token exists, then exec the command (default: hq start all).
#
# Anything that is not `hq start ...` is exec'd untouched, so
# `docker run IMAGE hq --version` and `docker run -it IMAGE bash` stay cheap.
set -euo pipefail

DATA_DIR=${HQ_DATA_DIR:-/data}
TOKEN_FILE="$DATA_DIR/web-auth.env"
TOKEN_VAR=HQ_WEB_AUTH_TOKEN
TOKEN_BYTES=32
VERSION_MARKER="$DATA_DIR/.hq-version"
WEB_PORT=5678

# hq install records provider keys it finds in the environment. Keys belong to the
# container's environment, not to a file on the volume, so they are stripped again.
KEY_FIELDS_IN_CONFIG='^(openrouter|anthropic|google_ai)_api_key:'

log() { printf 'hq-docker: %s\n' "$*" >&2; }

if [ "${1:-}" != "hq" ] || [ "${2:-}" != "start" ]; then
    exec "$@"
fi

if [ ! -w "$DATA_DIR" ]; then
    log "$DATA_DIR is not writable by uid $(id -u). For a bind mount run: chown -R 10001:10001 <host dir>"
    exit 1
fi

mkdir -p "$HOME" "$HQ_VAULT_PATH"

# The config file is the "first run finished" marker, so it is only moved into place once
# install succeeded and the keys are stripped. An interrupted first run just starts over.
if [ ! -f "$HQ_CONFIG_PATH" ]; then
    log "first run: scaffolding the vault in $HQ_VAULT_PATH"
    staging=$(mktemp -d "$DATA_DIR/.install.XXXXXX")
    trap 'rm -rf "$staging"' EXIT
    HQ_CONFIG_PATH="$staging/config.yaml" hq install --non-interactive --vault-path "$HQ_VAULT_PATH" \
        > "$staging/install.log" 2>&1 || { cat "$staging/install.log" >&2; exit 1; }
    sed -i -E "/$KEY_FIELDS_IN_CONFIG/d" "$staging/config.yaml"
    mv "$staging/config.yaml" "$HQ_CONFIG_PATH"
    rm -rf "$staging"
    trap - EXIT
    hq --version > "$VERSION_MARKER"
fi

# A new image is the update unit, so this does what `hq update` does after a swap: refresh
# the shipped system files and guides, leaving MEMORY.md, PREFERENCES.md and the config alone.
# Its failure is logged, not fatal, as in the updater.
running_version=$(hq --version)
if [ "$(cat "$VERSION_MARKER" 2> /dev/null || true)" != "$running_version" ]; then
    log "image changed ($running_version): upgrading the vault system files"
    if hq install --upgrade --vault-path "$HQ_VAULT_PATH" > /dev/null 2>&1; then
        printf '%s\n' "$running_version" > "$VERSION_MARKER"
    else
        log "hq install --upgrade failed; HQ will start anyway and retry on the next start"
    fi
fi

new_token=""
if [ -n "${!TOKEN_VAR:-}" ]; then
    log "web token taken from the $TOKEN_VAR environment variable"
elif [ -s "$TOKEN_FILE" ]; then
    # Parsed, not sourced, so the file can never run code.
    token=$(sed -n "s/^$TOKEN_VAR=//p" "$TOKEN_FILE" | head -n 1)
    [ -n "$token" ] || { log "$TOKEN_FILE has no $TOKEN_VAR line; delete it to generate a new token"; exit 1; }
    export "$TOKEN_VAR=$token"
    log "web token loaded from $TOKEN_FILE"
else
    new_token=$(od -An -tx1 -N"$TOKEN_BYTES" /dev/urandom | tr -d ' \n')
    ( umask 077 && printf '%s=%s\n' "$TOKEN_VAR" "$new_token" > "$TOKEN_FILE" )
    chmod 600 "$TOKEN_FILE"
    export "$TOKEN_VAR=$new_token"
fi

if [ -n "$new_token" ]; then
    cat >&2 <<EOF

================================================================================
 HQ web UI:   http://localhost:$WEB_PORT/#token=$new_token
 Web token:   $new_token

 This is the only time the token is printed. Open the link above once, or paste
 the token when asked. It is saved in the data volume ($TOKEN_FILE,
 mode 600). To read it later:
   docker exec <container> cat $TOKEN_FILE
 To rotate it: delete that file and restart the container.
 Everything under /api and /ws is refused without it.
================================================================================

EOF
fi

log "starting: $*"
exec "$@"
