#!/bin/bash
# install-hq.sh — Install the hq binary and restart the daemon. No privileges
# needed once `--link` has been run once.
#
# /usr/local/bin is root-owned, so writing the binary there needs sudo every
# single time — which means an agent that rebuilds cannot finish the job and
# the daemon silently keeps running the old binary. `--link` replaces that
# file with a symlink to ~/bin/hq (one sudo, once, ever). After that every
# install is an unprivileged write to ~/bin/hq that /usr/local/bin/hq resolves
# through, so the launchd plist, the MCP configs, and the three hardcoded
# /usr/local/bin/hq paths in the Rust sources all keep working untouched.
#
# A sudoers NOPASSWD entry for this script would also work, but it grants
# standing passwordless root to a file anything with repo write access can
# edit. The symlink needs no standing privilege at all.
#
# Usage (from repo root):
#   ./scripts/install-hq.sh              # build if needed, install, restart daemon
#   ./scripts/install-hq.sh --no-build   # install the existing release binary
#   ./scripts/install-hq.sh --check      # report installed vs built version
#   sudo ./scripts/install-hq.sh --link  # one-time: point /usr/local/bin/hq at ~/bin/hq

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BINARY="$REPO_ROOT/target/release/hq"
DEST="${HQ_INSTALL_DEST:-/usr/local/bin/hq}"

# Resolve the real user FIRST. Under sudo, $HOME is root's, so every
# user-scoped path below has to hang off $USER_HOME rather than $HOME —
# otherwise `--link` would point /usr/local/bin/hq at /var/root/bin/hq and
# take the whole install down with it.
INVOKING_USER="${SUDO_USER:-$(id -un)}"
INVOKING_UID="$(id -u "$INVOKING_USER")"
USER_HOME="$(/usr/bin/dscl . -read "/Users/$INVOKING_USER" NFSHomeDirectory 2>/dev/null \
    | awk '{print $2}')"
USER_HOME="${USER_HOME:-$HOME}"

USER_BIN="$USER_HOME/bin/hq"
CERT_NAME="HQ Local Code Signing"
KEYCHAIN_PATH="$USER_HOME/.hq/signing.keychain"
KEYCHAIN_PASS="hq-$(hostname -s)-signing"
P12_PASS="hq-p12-import"  # unused at install time, kept for reference

# LaunchAgent label (matches scripts/com.agent-hq.hq-all.plist.template).
LAUNCH_LABEL="com.agent-hq.hq-all"
LAUNCH_DOMAIN="gui/$INVOKING_UID"
LAUNCH_PLIST="$USER_HOME/Library/LaunchAgents/$LAUNCH_LABEL.plist"

# Helper: run a command as the invoking user (needed when this script is sudo'd).
run_as_user() {
    if [[ "$(id -un)" == "$INVOKING_USER" ]]; then
        "$@"
    else
        sudo -u "$INVOKING_USER" "$@"
    fi
}

# True when $DEST resolves to $USER_BIN, i.e. --link has been run.
dest_is_linked() {
    [[ -L "$DEST" && "$(readlink "$DEST")" == "$USER_BIN" ]]
}

# The version string does not bump between builds, so it cannot tell a stale
# install from a fresh one. Content hash can — but only of the *source*
# binary: `sign_binary` rewrites each installed copy afterwards, so an
# installed file never hashes equal to the artifact it came from. Every
# install therefore stamps the source hash it was cut from.
STAMP_DIR="$USER_HOME/.hq/install-stamps"

short_hash() {
    [[ -f "$1" ]] || { echo "absent"; return; }
    shasum -a 256 "$1" 2>/dev/null | cut -c1-12
}

stamp_file() {
    echo "$STAMP_DIR/$(echo "$1" | tr '/' '_')"
}

record_stamp() {
    mkdir -p "$STAMP_DIR"
    short_hash "$BINARY" > "$(stamp_file "$1")"
    # A stamp written as root would block every later unprivileged install.
    chown -R "$INVOKING_USER" "$STAMP_DIR" 2>/dev/null || true
}

if [[ "${1:-}" == "--check" ]]; then
    built="$(short_hash "$BINARY")"
    printf 'built       %s  %s\n' "$built" "$BINARY"
    for p in "$USER_BIN" "$DEST"; do
        if [[ ! -e "$p" ]]; then
            printf '%-11s %s\n' "MISSING" "$p"
            continue
        fi
        # A symlinked $DEST is the same file as $USER_BIN by construction.
        if [[ "$p" == "$DEST" ]] && dest_is_linked; then
            printf '%-11s -> %s  %s\n' "linked" "$USER_BIN" "$p"
            continue
        fi
        from="$(cat "$(stamp_file "$p")" 2>/dev/null || echo unknown)"
        case "$from" in
            "$built") state="current" ;;
            unknown)  state="UNKNOWN" ;;
            *)        state="STALE" ;;
        esac
        printf '%-11s built from %s  %s  (%s)\n' "$state" "$from" "$p" \
            "$("$p" --version 2>/dev/null || echo '-')"
    done
    if dest_is_linked; then
        echo "linked      yes — installs need no privileges"
    else
        echo "linked      no  — run 'sudo ./scripts/install-hq.sh --link' once to drop the sudo requirement"
    fi
    exit 0
fi

# ─── One-time: make $DEST a symlink to the user-owned binary ──
if [[ "${1:-}" == "--link" ]]; then
    # Gate on write access rather than on being root: that is the condition
    # that actually matters, and it stays true under sudo.
    if [[ ! -w "$(dirname "$DEST")" ]]; then
        echo "Error: $(dirname "$DEST") is not writable by $(id -un)." >&2
        echo "Run: sudo $0 --link" >&2
        exit 1
    fi
    if dest_is_linked; then
        echo "Already linked: $DEST -> $USER_BIN"
        exit 0
    fi
    mkdir -p "$(dirname "$USER_BIN")"
    # Seed the target so the symlink is never dangling, preferring a fresh
    # build and falling back to whatever is currently installed.
    if [[ -f "$BINARY" ]]; then
        install -m 755 "$BINARY" "$USER_BIN"
    elif [[ -f "$DEST" && ! -L "$DEST" ]]; then
        install -m 755 "$DEST" "$USER_BIN"
    fi
    chown "$INVOKING_USER" "$USER_BIN" 2>/dev/null || true
    ln -sfn "$USER_BIN" "$DEST"
    echo "Linked $DEST -> $USER_BIN"
    echo "Future installs need no sudo: ./scripts/install-hq.sh"
    exit 0
fi

# ─── Build ────────────────────────────────────────────────────
if [[ "${1:-}" != "--no-build" ]]; then
    echo "Building release binary..."
    (cd "$REPO_ROOT" && cargo build --release -p hq-cli)
fi

if [[ ! -f "$BINARY" ]]; then
    echo "Error: release binary not found at $BINARY" >&2
    echo "Run: cargo build --release -p hq-cli" >&2
    exit 1
fi

# ─── Sign function ────────────────────────────────────────────
sign_binary() {
    local path="$1"
    # Check without -v: self-signed certs show as CSSMERR_TP_NOT_TRUSTED but still
    # produce a stable CSREQ (identifier + cert hash). TCC uses that hash for FDA
    # persistence — it doesn't require a CA-trusted cert.
    if [[ -f "$KEYCHAIN_PATH" ]] && \
       security find-identity -p codesigning "$KEYCHAIN_PATH" 2>/dev/null | grep -q "$CERT_NAME"; then
        # Stable cert — FDA grants survive this rebuild
        security unlock-keychain -p "$KEYCHAIN_PASS" "$KEYCHAIN_PATH" 2>/dev/null || true
        if codesign -f -s "$CERT_NAME" \
            --keychain "$KEYCHAIN_PATH" \
            --identifier "com.agent-hq.hq" \
            "$path" 2>/dev/null; then
            # Refresh saved CSREQ so grant-fda.sh always has the current value
            codesign -dr - "$path" 2>&1 \
                | grep "designated =>" \
                | sed 's/.*=> //' \
                > "$USER_HOME/.hq/signing-csreq.txt" 2>/dev/null || true
            return 0
        fi
    fi
    # Fallback: ad-hoc (FDA may need re-granting after this)
    echo "Warning: stable cert not found. Run ./scripts/setup-signing.sh for permanent FDA." >&2
    codesign -f -s - --identifier "com.agent-hq.hq" "$path" 2>/dev/null || true
    return 1
}

# ─── Install ──────────────────────────────────────────────────
STABLE_SIGN=false

mkdir -p "$(dirname "$USER_BIN")"
install -m 755 "$BINARY" "$USER_BIN"
sign_binary "$USER_BIN" && STABLE_SIGN=true
record_stamp "$USER_BIN"
echo "Installed $USER_BIN ($(du -sh "$USER_BIN" | cut -f1))"

# With the symlink in place, $DEST already resolves to the binary just
# written. Without it, $DEST needs a privileged copy — do it if we happen to
# have the rights, and say so plainly if we don't rather than exiting dirty.
if dest_is_linked; then
    echo "Installed $DEST -> $USER_BIN (symlink, no copy needed)"
elif [[ -w "$(dirname "$DEST")" || $EUID -eq 0 ]]; then
    install -m 755 "$BINARY" "$DEST"
    sign_binary "$DEST" && STABLE_SIGN=true
    record_stamp "$DEST"
    echo "Installed $DEST ($(du -sh "$DEST" | cut -f1))"
else
    echo ""
    echo "⚠️  $DEST is root-owned and was NOT updated — it still holds the old binary."
    echo "   The launchd daemon and the MCP configs both point there, so they are stale."
    echo "   Fix it once and this warning never returns:"
    echo "     sudo $0 --link"
    echo ""
fi

# ─── Stale MCP stdio server warning ───────────────────────────
# `hq mcp-serve` runs as a long-lived stdio subprocess spawned once by each
# MCP client (Claude Code, Cursor, etc.) at their own startup. Overwriting
# the binary file above does not affect a process already holding it open —
# it keeps running the pre-update code in memory until the client
# restarts/reconnects. We don't kill these automatically: doing so could
# sever a live session's own MCP connection (including the session running
# this install) with no guaranteed automatic reconnect.
STALE_MCP_PIDS=$(pgrep -f "hq mcp-serve" || true)
if [[ -n "$STALE_MCP_PIDS" ]]; then
    echo ""
    echo "⚠️  ${STALE_MCP_PIDS//$'\n'/, } — 'hq mcp-serve' still running from before this install."
    echo "   These MCP clients are on the OLD binary until reconnected:"
    while read -r pid; do
        [[ -n "$pid" ]] && ps -p "$pid" -o pid=,args= 2>/dev/null | sed 's/^/     /'
    done <<< "$STALE_MCP_PIDS"
    echo "   Restart the MCP client (e.g. Claude Code) to pick up this build."
fi

# ─── FDA check ────────────────────────────────────────────────
if ! $STABLE_SIGN; then
    echo ""
    echo "⚠️  Using ad-hoc signature — FDA may need re-granting."
    echo "   Run ./scripts/setup-signing.sh first for a permanent fix."
fi

# ─── Restart daemon ───────────────────────────────────────────
# We support three states:
#   (A) launchd-managed: bootout+kickstart via launchctl so KeepAlive cycles cleanly
#   (B) manually started: kill the pgrep'd PID then re-spawn detached as the user
#   (C) not running: just spawn it for the first time
#
# This eliminates the old failure mode where killing the daemon and waiting 20s
# for launchd to restart it silently failed because no LaunchAgent was registered.

LAUNCHD_REGISTERED=false
if run_as_user launchctl print "$LAUNCH_DOMAIN/$LAUNCH_LABEL" >/dev/null 2>&1; then
    LAUNCHD_REGISTERED=true
fi

wait_for_health() {
    for _ in {1..20}; do
        sleep 1
        if curl -sf http://localhost:5678/health >/dev/null 2>&1; then
            echo "Daemon up. $(curl -s http://localhost:5678/health)"
            verify_daemon_binary
            return 0
        fi
    done
    return 1
}

# A healthy daemon proves something is listening, not that it is running the
# code just built — the original symptom here was a green health check in
# front of a seven-hour-old orphan that held port 5678 and the daemon lock,
# so the launchd-managed process bailed on startup every time.
#
# Identify the daemon by who owns the port, not by `pgrep | head -1`: with a
# duplicate running, pgrep can return a transient loser while the orphan is
# the one actually answering, which is exactly how this passed while broken.
verify_daemon_binary() {
    local pid running_bin others
    pid="$(lsof -nP -iTCP:5678 -sTCP:LISTEN -t 2>/dev/null | head -1 || true)"
    [[ -n "$pid" ]] || pid="$(pgrep -f 'hq start all' | head -1 || true)"
    [[ -n "$pid" ]] || return 0
    running_bin="$(ps -p "$pid" -o comm= 2>/dev/null || true)"
    [[ -n "$running_bin" ]] || return 0

    # A second daemon means one of them lost the lock and is inert.
    others="$(pgrep -f 'hq start all' | grep -v "^${pid}$" || true)"
    if [[ -n "$others" ]]; then
        echo ""
        echo "⚠️  More than one 'hq start all' is running: serving=$pid, also ${others//$'\n'/, }"
        echo "   Only one can hold port 5678 and the daemon lock; the rest do nothing."
        echo "   Kill the strays:  kill ${others//$'\n'/ }"
    fi

    local from
    from="$(cat "$(stamp_file "$running_bin")" 2>/dev/null || echo unknown)"
    if [[ "$from" == "$(short_hash "$BINARY")" ]] \
       || { [[ "$running_bin" == "$DEST" ]] && dest_is_linked; }; then
        echo "Daemon (pid $pid) is running this build."
    else
        echo ""
        echo "⚠️  Daemon (pid $pid) is running $running_bin, which is NOT this build."
        echo "   Everything you just compiled is inert until that path is updated."
        echo "   Run once:  sudo $0 --link"
    fi
}

if $LAUNCHD_REGISTERED; then
    echo "Reloading launchd job $LAUNCH_LABEL..."
    run_as_user launchctl bootout "$LAUNCH_DOMAIN/$LAUNCH_LABEL" 2>/dev/null || true
    sleep 1
    run_as_user launchctl bootstrap "$LAUNCH_DOMAIN" "$LAUNCH_PLIST" 2>/dev/null || true
    run_as_user launchctl enable "$LAUNCH_DOMAIN/$LAUNCH_LABEL" 2>/dev/null || true
    run_as_user launchctl kickstart -k "$LAUNCH_DOMAIN/$LAUNCH_LABEL" >/dev/null 2>&1 || true

    if wait_for_health; then
        exit 0
    fi
    echo "⚠️  Daemon not responding after 20s — check $USER_HOME/Library/Logs/hq-agent.log"
    echo "   If stuck on open(), FDA needs re-granting:"
    echo "   System Settings → Privacy & Security → Full Disk Access → re-enable hq"
    exit 1
fi

# No launchd job — fall back to manual restart so the script never silently fails.
echo "(launchd job '$LAUNCH_LABEL' is not registered — falling back to direct restart)"
echo "   Install it once with: ./scripts/install-launchagent.sh"
echo

# Every matching pid, not just the first: `head -1` here previously left any
# second daemon (itself a straggler from a prior restart that failed the same
# way) running forever. Each subsequent install killed one and spawned
# another, so the count only ever grew — observed twice, both times ending
# with two daemons racing for port 5678 and the older one winning, silently
# keeping the owner on a stale build.
RUNNING_PIDS=$(pgrep -f "hq start all" || true)
if [[ -n "$RUNNING_PIDS" ]]; then
    echo "Stopping running hq daemon(s) (pid(s) $(echo "$RUNNING_PIDS" | tr '\n' ' '))..."
    for pid in $RUNNING_PIDS; do
        kill "$pid" 2>/dev/null || true
    done
    for _ in {1..10}; do
        sleep 1
        pgrep -f "hq start all" >/dev/null 2>&1 || break
    done
    # Force-kill anything still alive after 10s.
    for pid in $(pgrep -f "hq start all" || true); do
        kill -9 "$pid" 2>/dev/null || true
    done
fi

echo "Spawning fresh daemon as $INVOKING_USER..."
LOG="$USER_HOME/Library/Logs/hq-agent.log"
ERRLOG="$USER_HOME/Library/Logs/hq-agent.error.log"
mkdir -p "$(dirname "$LOG")"
# nohup + setsid-style backgrounding via &; sudo -u runs as the real user so the
# child inherits their HOME / launchd session, not root's.
run_as_user bash -c "nohup '$DEST' start all >>'$LOG' 2>>'$ERRLOG' </dev/null &"

if wait_for_health; then
    exit 0
fi
echo "⚠️  Daemon not responding after 20s — check $LOG"
echo "   Tail it with: tail -f $LOG"
exit 1
