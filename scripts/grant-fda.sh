#!/usr/bin/env bash
# grant-fda.sh — One-time permanent FDA grant for hq via macOS PPPC configuration profile.
#
# WHY: Full Disk Access must be granted via System Settings or a PPPC config profile.
# A config profile keyed by binary path grants FDA permanently — no re-prompting when
# the binary is reinstalled to the same location.
#
# Run ONCE while at the laptop:
#   ./scripts/grant-fda.sh
#
# Password-free installs are a separate one-time step, `sudo ./scripts/install-hq.sh --link`,
# which makes /usr/local/bin/hq a symlink to ~/bin/hq so no sudoers rule is needed.
#
# After approving the profile in System Settings → Privacy & Security → Profiles,
# hq has permanent FDA at its install paths. No more prompts.

set -euo pipefail

INVOKING_USER="${SUDO_USER:-$(id -un)}"
USER_HOME="$(/usr/bin/dscl . -read "/Users/$INVOKING_USER" NFSHomeDirectory 2>/dev/null | awk '{print $2}')"
USER_HOME="${USER_HOME:-$HOME}"

HQ_SYSTEM_BIN="/usr/local/bin/hq"
HQ_USER_BIN="$USER_HOME/bin/hq"

# ─── 1. Generate PPPC profile with path-based FDA grants ──────
PROFILE_PATH="/tmp/hq-fda-policy.mobileconfig"
UUID1="$(uuidgen)"
UUID2="$(uuidgen)"

# Build array of binary grants — include user bin only if it exists
GRANTS=""
for BIN_PATH in "$HQ_SYSTEM_BIN" "$HQ_USER_BIN"; do
    GRANTS="${GRANTS}
                    <dict>
                        <key>Allowed</key>
                        <true/>
                        <key>Comment</key>
                        <string>Agent HQ binary at ${BIN_PATH}</string>
                        <key>Identifier</key>
                        <string>${BIN_PATH}</string>
                        <key>IdentifierType</key>
                        <string>path</string>
                        <key>StaticCode</key>
                        <false/>
                    </dict>"
done

cat > "$PROFILE_PATH" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>PayloadContent</key>
    <array>
        <dict>
            <key>PayloadDescription</key>
            <string>Grants Full Disk Access to hq binary for autonomous operation</string>
            <key>PayloadDisplayName</key>
            <string>Agent HQ FDA Policy</string>
            <key>PayloadIdentifier</key>
            <string>com.agent-hq.fda-policy.tcc</string>
            <key>PayloadType</key>
            <string>com.apple.TCC.configuration-profile-policy</string>
            <key>PayloadUUID</key>
            <string>${UUID1}</string>
            <key>PayloadVersion</key>
            <integer>1</integer>
            <key>Services</key>
            <dict>
                <key>SystemPolicyAllFiles</key>
                <array>${GRANTS}
                </array>
            </dict>
        </dict>
    </array>
    <key>PayloadDescription</key>
    <string>Pre-authorizes Agent HQ binary for Full Disk Access. Grants are path-based and survive binary updates at the same location.</string>
    <key>PayloadDisplayName</key>
    <string>Agent HQ - Full Disk Access</string>
    <key>PayloadIdentifier</key>
    <string>com.agent-hq.fda-policy</string>
    <key>PayloadOrganization</key>
    <string>Agent HQ</string>
    <key>PayloadRemovalDisallowed</key>
    <false/>
    <key>PayloadType</key>
    <string>Configuration</string>
    <key>PayloadUUID</key>
    <string>${UUID2}</string>
    <key>PayloadVersion</key>
    <integer>1</integer>
</dict>
</plist>
EOF

echo "Profile generated: $PROFILE_PATH"
echo "Grants FDA to: $HQ_SYSTEM_BIN and $HQ_USER_BIN"

# ─── 2. Install the profile ───────────────────────────────────
echo ""
echo "Opening profile in System Settings..."
echo ""
echo "  → In System Settings: Privacy & Security → Profiles"
echo "  → Click the 'Agent HQ - Full Disk Access' profile → Install"
echo ""

sudo -u "$INVOKING_USER" open "$PROFILE_PATH"

echo "Press Enter once you have clicked 'Install' in System Settings"
read -r

# ─── 3. Verify ────────────────────────────────────────────────
if sudo -u "$INVOKING_USER" profiles list 2>/dev/null | grep -qi "agent-hq\|Agent HQ"; then
    echo ""
    echo "Done. Profile installed — hq has permanent Full Disk Access."
    echo "No more FDA prompts after installs or rebuilds."
else
    echo ""
    echo "Profile may still be pending. Check System Settings → Privacy & Security → Profiles."
    echo "Once installed, FDA prompts will stop permanently."
fi
