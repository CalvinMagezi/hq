#!/bin/bash
# install-launchagent.sh — Install/refresh the macOS LaunchAgent for `hq start all`.
#
# This is what makes `KeepAlive=true` actually work, which in turn lets
# `install-hq.sh` restart the daemon by simply killing it.
#
# Prefer running this WITHOUT sudo — LaunchAgents live in the per-user GUI domain.
# If you do sudo it anyway, the script
# detects the real user via $SUDO_USER, drops back to them for every launchctl
# call, and re-chowns the plist + log dir. Running as root with no SUDO_USER
# (e.g. `sudo -i` then invoking the script) will hard-fail rather than corrupt
# launchd state.
#
# Usage:
#   ./scripts/install-launchagent.sh                # install + bootstrap
#   ./scripts/install-launchagent.sh --uninstall    # bootout + remove plist
#   ./scripts/install-launchagent.sh --status       # show current launchd state
#   ./scripts/install-launchagent.sh --reload       # bootout + bootstrap (no plist regen)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TEMPLATE="$REPO_ROOT/scripts/com.agent-hq.hq-all.plist.template"
LABEL="com.agent-hq.hq-all"

# Resolve the *real* invoking user, even when this script is sudo'd.
# LaunchAgents live in the per-user GUI domain (gui/<uid>), which doesn't exist
# for root — bootstrapping into gui/0 fails with `125: Domain does not support
# specified action`, which is exactly the error someone gets if they sudo this.
INVOKING_USER="${SUDO_USER:-$(id -un)}"
INVOKING_UID="$(id -u "$INVOKING_USER")"
USER_HOME="$(/usr/bin/dscl . -read "/Users/$INVOKING_USER" NFSHomeDirectory 2>/dev/null \
    | awk '{print $2}')"
USER_HOME="${USER_HOME:-$HOME}"
DOMAIN="gui/$INVOKING_UID"
PLIST_DEST="$USER_HOME/Library/LaunchAgents/$LABEL.plist"
LOG_DIR="$USER_HOME/Library/Logs"
HQ_BIN="${HQ_BIN:-/usr/local/bin/hq}"

# Helper: run launchctl as the invoking user. Required because launchctl
# bootstrap/bootout/enable/kickstart all target the *caller's* per-user domain;
# from a sudo session we have to drop privileges back to the real user.
run_as_user() {
    if [[ "$(id -un)" == "$INVOKING_USER" ]]; then
        "$@"
    else
        sudo -u "$INVOKING_USER" "$@"
    fi
}

if [[ "$INVOKING_UID" == "0" ]]; then
    echo "Error: cannot install a per-user LaunchAgent for root (UID 0)." >&2
    echo "       Run this script as your normal user (without sudo)." >&2
    exit 1
fi

if [[ "${1:-}" == "--status" ]]; then
    echo "Plist: $PLIST_DEST"
    if [[ -f "$PLIST_DEST" ]]; then
        echo "  exists ($(stat -f '%Sm' "$PLIST_DEST"))"
    else
        echo "  missing"
    fi
    echo
    echo "launchctl print $DOMAIN/$LABEL:"
    run_as_user launchctl print "$DOMAIN/$LABEL" 2>&1 | head -25 || echo "  (job not registered)"
    echo
    echo "Process check (pgrep 'hq start all'):"
    pgrep -fla "hq start all" || echo "  (no process running)"
    exit 0
fi

if [[ "${1:-}" == "--uninstall" ]]; then
    echo "── Uninstalling $LABEL ──"
    run_as_user launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
    if [[ -f "$PLIST_DEST" ]]; then
        rm -f "$PLIST_DEST"
        echo "Removed $PLIST_DEST"
    fi
    echo "Done. The daemon will not auto-restart anymore."
    exit 0
fi

if [[ "${1:-}" == "--reload" ]]; then
    echo "── Reloading $LABEL (bootout + bootstrap) ──"
    run_as_user launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
    sleep 1
    if ! run_as_user launchctl bootstrap "$DOMAIN" "$PLIST_DEST" 2>/tmp/hq-bootstrap.err; then
        if ! run_as_user launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; then
            echo "Error: bootstrap failed and job is not registered:" >&2
            cat /tmp/hq-bootstrap.err >&2
            exit 1
        fi
    fi
    rm -f /tmp/hq-bootstrap.err
    run_as_user launchctl enable "$DOMAIN/$LABEL" 2>/dev/null || true
    # --reload IS the case where you DO want to force-restart, so -k is safe here.
    run_as_user launchctl kickstart -k "$DOMAIN/$LABEL" >/dev/null 2>&1 || true
    echo "Reloaded. Tail logs with: hq logs agent -f"
    exit 0
fi

# ─── Install / refresh ───────────────────────────────────────────
if [[ ! -f "$TEMPLATE" ]]; then
    echo "Error: template not found at $TEMPLATE" >&2
    exit 1
fi

if [[ ! -x "$HQ_BIN" ]]; then
    echo "Error: hq binary not executable at $HQ_BIN" >&2
    echo "Build + install it first:" >&2
    echo "  ./scripts/install-hq.sh   (after a one-time: sudo ./scripts/install-hq.sh --link)" >&2
    exit 1
fi

mkdir -p "$(dirname "$PLIST_DEST")"
mkdir -p "$LOG_DIR"

# Capture API keys from the current environment. If a key is unset, default to
# empty string so the placeholder is replaced (not left as a literal token).
GROQ_API_KEY="${GROQ_API_KEY:-}"
CEREBRAS_API_KEY="${CEREBRAS_API_KEY:-}"
ANTHROPIC_API_KEY="${ANTHROPIC_API_KEY:-}"
OPENROUTER_API_KEY="${OPENROUTER_API_KEY:-}"

# Substitute placeholders into the template using the *invoking user's* HOME,
# not root's, so paths inside the plist are correct under sudo too.
sed \
    -e "s|__HOME__|$USER_HOME|g" \
    -e "s|__HQ_BIN__|$HQ_BIN|g" \
    -e "s|__GROQ_API_KEY__|$GROQ_API_KEY|g" \
    -e "s|__CEREBRAS_API_KEY__|$CEREBRAS_API_KEY|g" \
    -e "s|__ANTHROPIC_API_KEY__|$ANTHROPIC_API_KEY|g" \
    -e "s|__OPENROUTER_API_KEY__|$OPENROUTER_API_KEY|g" \
    "$TEMPLATE" > "$PLIST_DEST"

# If we're running under sudo the new plist + log dir end up owned by root.
# Hand them back to the real user so launchctl can read them and the daemon
# can write its logs without permission-denied churn.
if [[ "$(id -un)" != "$INVOKING_USER" ]]; then
    chown "$INVOKING_USER" "$PLIST_DEST" 2>/dev/null || true
    chown -R "$INVOKING_USER" "$(dirname "$PLIST_DEST")" "$LOG_DIR" 2>/dev/null || true
fi

echo "Wrote $PLIST_DEST"

# Verify the plist parses before handing it to launchd.
if ! plutil -lint "$PLIST_DEST" >/dev/null; then
    echo "Error: generated plist is malformed; aborting." >&2
    exit 1
fi

# Bootout any existing instance so the new plist takes effect, then bootstrap.
# All four launchctl calls run as the invoking user so they target gui/<uid>,
# not gui/0 (which raises `125: Domain does not support specified action`).
run_as_user launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
sleep 1

# bootstrap can fail benignly if the job is already loaded from a stale state;
# only treat it as fatal when the job genuinely isn't registered afterwards.
if ! run_as_user launchctl bootstrap "$DOMAIN" "$PLIST_DEST" 2>/tmp/hq-bootstrap.err; then
    if ! run_as_user launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; then
        echo "Error: bootstrap failed and job is not registered:" >&2
        cat /tmp/hq-bootstrap.err >&2
        exit 1
    fi
    echo "(bootstrap reported a soft error, job is registered — continuing)"
fi
rm -f /tmp/hq-bootstrap.err

run_as_user launchctl enable "$DOMAIN/$LABEL" 2>/dev/null || true

# bootstrap + RunAtLoad=true already started the job. Use plain `kickstart`
# (NOT `kickstart -k`) so we only ensure-running rather than kill-and-restart;
# the -k variant would SIGTERM the daemon we just bootstrapped, and with
# KeepAlive=true the resulting churn just delays liveness pointlessly.
run_as_user launchctl kickstart "$DOMAIN/$LABEL" >/dev/null 2>&1 || true

# Confirm liveness via the health endpoint (truth source, not launchctl exit).
echo "Waiting for daemon to come up on http://localhost:5678/health ..."
for _ in {1..30}; do
    sleep 1
    if curl -sf http://localhost:5678/health >/dev/null 2>&1; then
        echo "Daemon up. $(curl -s http://localhost:5678/health)"
        break
    fi
done

if ! curl -sf http://localhost:5678/health >/dev/null 2>&1; then
    echo
    echo "⚠️  Daemon did not respond on /health within 30s."
    echo "   But the LaunchAgent IS registered. Inspect with:"
    echo "     ./scripts/install-launchagent.sh --status"
    echo "     hq logs agent -f"
fi

echo
echo "── LaunchAgent installed ──"
echo "  Label:       $LABEL"
echo "  Binary:      $HQ_BIN"
echo "  stdout log:  $LOG_DIR/hq-agent.log"
echo "  stderr log:  $LOG_DIR/hq-agent.error.log"
echo "  Read them with \`hq logs agent\` (plain \`hq logs\` reads the daemon target, hq-daemon.log)."
echo
echo "Verify with:"
echo "  ./scripts/install-launchagent.sh --status"
echo "  curl -sf http://localhost:5678/health"
