#!/bin/bash
# setup-searxng.sh — stand up an optional self-hosted SearxNG instance for
# HQ's web_search tool. web_search works without it (built-in engine pool);
# set `searxng_url: http://127.0.0.1:8080` in ~/.hq/config.yaml to have it
# tried first.
#
# Idempotent: safe to re-run. Leaves an existing container running rather
# than recreating it, and won't regenerate settings.yml's secret key once
# it exists.
#
# Usage:
#   ./scripts/setup-searxng.sh          # install + start (or leave running)
#   ./scripts/setup-searxng.sh --check  # report status only, no changes

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONTAINER_NAME="hq-searxng"
IMAGE="searxng/searxng:latest"
PORT=8080
SEARXNG_DIR="$HOME/.hq/searxng"
SETTINGS_FILE="$SEARXNG_DIR/settings.yml"
SETTINGS_TEMPLATE="$REPO_ROOT/scripts/searxng/settings.yml"
BASE_URL="http://localhost:$PORT"

CHECK_ONLY=false
if [[ "${1:-}" == "--check" ]]; then
    CHECK_ONLY=true
fi

log() { echo "[setup-searxng] $*"; }

check_health() {
    curl -sf --max-time 5 "$BASE_URL/search?q=test&format=json" 2>/dev/null | grep -q '"results"'
}

if $CHECK_ONLY; then
    if check_health; then
        log "healthy — $BASE_URL responding with JSON results"
        exit 0
    fi
    log "not responding at $BASE_URL"
    exit 1
fi

if ! command -v docker >/dev/null 2>&1; then
    log "docker not found on PATH. Install Docker Desktop first: https://www.docker.com/products/docker-desktop/"
    exit 1
fi

if ! docker ps >/dev/null 2>&1; then
    # `docker ps` fails fast (socket ENOENT) when the daemon is down; `docker
    # info` was tried first but hangs for ~8s retrying the Server: section
    # instead of erroring, which made this check useless as a fast guard.
    log "Docker daemon not reachable. Start Docker Desktop and re-run."
    exit 1
fi

mkdir -p "$SEARXNG_DIR"

if [[ ! -f "$SETTINGS_FILE" ]]; then
    log "generating $SETTINGS_FILE"
    SECRET_KEY="$(openssl rand -hex 32)"
    sed "s/REPLACED_AT_INSTALL_TIME/$SECRET_KEY/" "$SETTINGS_TEMPLATE" > "$SETTINGS_FILE"
else
    log "reusing existing $SETTINGS_FILE (secret key unchanged)"
fi

# A container created before the loopback-only fix below is bound to
# 0.0.0.0 for good — `docker start` can't change a binding set at `docker
# run` time. Recreate rather than reuse when that's the case, so a stale
# container from an older run doesn't keep exposing the port silently.
if docker ps -a --format '{{.Names}}' | grep -qx "$CONTAINER_NAME" \
    && ! docker port "$CONTAINER_NAME" 8080/tcp 2>/dev/null | grep -q '^127\.0\.0\.1:'; then
    log "$CONTAINER_NAME exists but is not bound to loopback only — recreating"
    docker rm -f "$CONTAINER_NAME" >/dev/null
fi

if docker ps --format '{{.Names}}' | grep -qx "$CONTAINER_NAME"; then
    log "$CONTAINER_NAME already running"
else
    if docker ps -a --format '{{.Names}}' | grep -qx "$CONTAINER_NAME"; then
        log "starting existing (stopped) $CONTAINER_NAME container"
        docker start "$CONTAINER_NAME" >/dev/null
    else
        log "pulling $IMAGE"
        docker pull "$IMAGE" >/dev/null
        log "creating $CONTAINER_NAME on port $PORT (loopback only)"
        # Bind to 127.0.0.1, not 0.0.0.0: settings.yml deliberately disables
        # SearxNG's request limiter on the assumption this instance is
        # local-only, and Docker's own iptables rules bypass ufw/firewall
        # INPUT chains for published ports — a bare `-p $PORT:8080` on a
        # host with a public IP exposes an unauthenticated search proxy to
        # the entire internet regardless of any firewall's "deny incoming"
        # default. Confirmed exposed this way on first standup here.
        docker run -d \
            --name "$CONTAINER_NAME" \
            --restart unless-stopped \
            -p "127.0.0.1:$PORT:8080" \
            -v "$SETTINGS_FILE:/etc/searxng/settings.yml:ro" \
            -e "SEARXNG_BASE_URL=$BASE_URL/" \
            "$IMAGE" >/dev/null
    fi
fi

log "waiting for $BASE_URL to come up"
for _ in $(seq 1 30); do
    if check_health; then
        log "ready — set searxng_url: $BASE_URL in ~/.hq/config.yaml to have HQ's web_search try it first"
        exit 0
    fi
    sleep 1
done

log "container started but $BASE_URL isn't returning JSON results yet."
log "check logs with: docker logs $CONTAINER_NAME"
exit 1
