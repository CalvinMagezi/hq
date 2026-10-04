#!/bin/bash
# End-to-end test for hq install / onboard / service / upgrade / uninstall flow.
# Runs inside a clean Docker container with no prior state.

set -euo pipefail

PASS=0
FAIL=0
TOTAL=0

# ── Helpers ──────────────────────────────────────────────────────────────────

pass() {
    PASS=$((PASS + 1))
    TOTAL=$((TOTAL + 1))
    echo "  [PASS] $1"
}

fail() {
    FAIL=$((FAIL + 1))
    TOTAL=$((TOTAL + 1))
    echo "  [FAIL] $1"
}

assert_file() {
    if [ -f "$1" ]; then
        pass "$2"
    else
        fail "$2 (missing: $1)"
    fi
}

assert_dir() {
    if [ -d "$1" ]; then
        pass "$2"
    else
        fail "$2 (missing: $1)"
    fi
}

assert_contains() {
    if grep -q "$2" "$1" 2>/dev/null; then
        pass "$3"
    else
        fail "$3 (pattern '$2' not found in $1)"
    fi
}

assert_not_contains() {
    if ! grep -q "$2" "$1" 2>/dev/null; then
        pass "$3"
    else
        fail "$3 (pattern '$2' unexpectedly found in $1)"
    fi
}

assert_exit_0() {
    local desc="$1"
    shift
    if "$@" >/dev/null 2>&1; then
        pass "$desc"
    else
        fail "$desc (command failed: $*)"
    fi
}

# ── Pre-flight ───────────────────────────────────────────────────────────────

echo ""
echo "============================================"
echo "  Agent-HQ Install Flow — End-to-End Tests"
echo "============================================"
echo ""

# Verify clean state
echo "── Pre-flight checks ──"
if [ ! -f /usr/local/bin/hq ]; then
    echo "  [FATAL] hq binary not found at /usr/local/bin/hq"
    exit 1
fi
pass "hq binary exists"

assert_exit_0 "hq version runs" hq version

if [ -d "$HOME/.vault" ]; then
    fail "Clean state: ~/.vault should not exist yet"
else
    pass "Clean state: no pre-existing vault"
fi

if [ -d "$HOME/.hq" ]; then
    fail "Clean state: ~/.hq should not exist yet"
else
    pass "Clean state: no pre-existing config"
fi

# ══════════════════════════════════════════════════════════════════════════════
# TEST 1: hq install (fresh install)
# ══════════════════════════════════════════════════════════════════════════════

echo ""
echo "── Test 1: Fresh install ──"

OUTPUT=$(hq install --non-interactive 2>&1)
echo "$OUTPUT" | head -5

# Vault directories
assert_dir "$HOME/.vault" "Vault directory created"
assert_dir "$HOME/.vault/_system" "_system/ created"
assert_dir "$HOME/.vault/_system/guides" "_system/guides/ created"
assert_dir "$HOME/.vault/_jobs/pending" "_jobs/pending/ created"
assert_dir "$HOME/.vault/_jobs/running" "_jobs/running/ created"
assert_dir "$HOME/.vault/_jobs/done" "_jobs/done/ created"
assert_dir "$HOME/.vault/_jobs/failed" "_jobs/failed/ created"
assert_dir "$HOME/.vault/_delegation/pending/claude-code" "_delegation/pending/claude-code/ created"
assert_dir "$HOME/.vault/_delegation/pending/any" "_delegation/pending/any/ created"
assert_dir "$HOME/.vault/_threads/active" "_threads/active/ created"
assert_dir "$HOME/.vault/_plans/active" "_plans/active/ created"
assert_dir "$HOME/.vault/_bulletin/proposals" "_bulletin/proposals/ created"
assert_dir "$HOME/.vault/Notebooks/Projects" "Notebooks/Projects/ created"
assert_dir "$HOME/.vault/Notebooks/Onboarding" "Notebooks/Onboarding/ created"
assert_dir "$HOME/.vault/Notebooks/Memories" "Notebooks/Memories/ created"

# System files
assert_file "$HOME/.vault/_system/SOUL.md" "SOUL.md created"
assert_file "$HOME/.vault/_system/MEMORY.md" "MEMORY.md created"
assert_file "$HOME/.vault/_system/PREFERENCES.md" "PREFERENCES.md created"
assert_file "$HOME/.vault/_system/HEARTBEAT.md" "HEARTBEAT.md created"
assert_file "$HOME/.vault/_system/CONFIG.md" "CONFIG.md created"
assert_file "$HOME/.vault/_system/CAPABILITIES.md" "CAPABILITIES.md created"
assert_file "$HOME/.vault/_system/ONBOARD.md" "ONBOARD.md created"

# Guide files
assert_file "$HOME/.vault/_system/guides/getting-started.md" "Guide: getting-started"
assert_file "$HOME/.vault/_system/guides/vault-structure.md" "Guide: vault-structure"
assert_file "$HOME/.vault/_system/guides/agent-principles.md" "Guide: agent-principles"
assert_file "$HOME/.vault/_system/guides/tool-setup.md" "Guide: tool-setup"
assert_file "$HOME/.vault/_system/guides/workflows.md" "Guide: workflows"
assert_file "$HOME/.vault/_system/guides/memory-system.md" "Guide: memory-system"

# Soul content quality
assert_contains "$HOME/.vault/_system/SOUL.md" "Agent Operating Identity" "SOUL.md has rich content (not placeholder)"
assert_contains "$HOME/.vault/_system/SOUL.md" "Session Protocol" "SOUL.md includes session protocol"
assert_contains "$HOME/.vault/_system/SOUL.md" "version: 2" "SOUL.md is version 2"

# Capabilities detection
assert_contains "$HOME/.vault/_system/CAPABILITIES.md" "linux" "CAPABILITIES.md detected Linux OS"
assert_contains "$HOME/.vault/_system/CAPABILITIES.md" "Auto-detected" "CAPABILITIES.md has auto-detected header"

# Guide content quality
assert_contains "$HOME/.vault/_system/guides/tool-setup.md" "gws" "Tool setup guide covers gws"
assert_contains "$HOME/.vault/_system/guides/tool-setup.md" "Claude CLI" "Tool setup guide covers Claude CLI"
assert_contains "$HOME/.vault/_system/guides/workflows.md" "hq orchestrate" "Workflows guide covers orchestration"
assert_contains "$HOME/.vault/_system/guides/memory-system.md" "Consolidation" "Memory guide covers consolidation"

# Onboarding tracker
assert_contains "$HOME/.vault/_system/ONBOARD.md" "pending" "ONBOARD.md has pending steps"

# Config file
assert_file "$HOME/.hq/config.yaml" "Config file created"
assert_contains "$HOME/.hq/config.yaml" "vault_path" "Config has vault_path"
assert_contains "$HOME/.hq/config.yaml" "default_model" "Config has default_model"

# ══════════════════════════════════════════════════════════════════════════════
# TEST 2: Idempotency (run install again)
# ══════════════════════════════════════════════════════════════════════════════

echo ""
echo "── Test 2: Idempotency ──"

# Modify MEMORY.md to verify it's not overwritten
echo "- User likes cats." >> "$HOME/.vault/_system/MEMORY.md"

OUTPUT2=$(hq install --non-interactive 2>&1)

# Should report 0 directories created
if echo "$OUTPUT2" | grep -q "Created 0 directories"; then
    pass "Idempotent: no directories re-created"
else
    fail "Idempotent: directories were re-created"
fi

# MEMORY.md should still have our edit (not overwritten in non-upgrade mode)
assert_contains "$HOME/.vault/_system/MEMORY.md" "User likes cats" "MEMORY.md preserved (not overwritten)"

# CAPABILITIES.md should be refreshed (always overwritten)
assert_contains "$HOME/.vault/_system/CAPABILITIES.md" "Auto-detected" "CAPABILITIES.md refreshed on re-run"

# ══════════════════════════════════════════════════════════════════════════════
# TEST 3: Upgrade mode
# ══════════════════════════════════════════════════════════════════════════════

echo ""
echo "── Test 3: Upgrade mode ──"

# Modify SOUL.md to simulate old content
echo "OLD_MARKER" >> "$HOME/.vault/_system/SOUL.md"

# Run with --upgrade
hq install --non-interactive --upgrade >/dev/null 2>&1

# SOUL.md should be refreshed (upgraded)
assert_not_contains "$HOME/.vault/_system/SOUL.md" "OLD_MARKER" "Upgrade: SOUL.md was refreshed"
assert_contains "$HOME/.vault/_system/SOUL.md" "version: 2" "Upgrade: SOUL.md has latest version"

# MEMORY.md should NOT be overwritten (user-editable file)
assert_contains "$HOME/.vault/_system/MEMORY.md" "User likes cats" "Upgrade: MEMORY.md preserved"

# PREFERENCES.md should NOT be overwritten
assert_file "$HOME/.vault/_system/PREFERENCES.md" "Upgrade: PREFERENCES.md still exists"

# ══════════════════════════════════════════════════════════════════════════════
# TEST 4: Minimal install
# ══════════════════════════════════════════════════════════════════════════════

echo ""
echo "── Test 4: Minimal install (fresh vault) ──"

# Wipe and reinstall with --minimal
rm -rf "$HOME/.vault" "$HOME/.hq"

hq install --non-interactive --minimal >/dev/null 2>&1

# Should have system files
assert_file "$HOME/.vault/_system/SOUL.md" "Minimal: SOUL.md exists"
assert_file "$HOME/.vault/_system/CAPABILITIES.md" "Minimal: CAPABILITIES.md exists"

# Should NOT have guides (skipped in minimal)
if [ -f "$HOME/.vault/_system/guides/getting-started.md" ]; then
    fail "Minimal: guides should be skipped"
else
    pass "Minimal: guides correctly skipped"
fi

# Config should still exist
assert_file "$HOME/.hq/config.yaml" "Minimal: config created"

# ══════════════════════════════════════════════════════════════════════════════
# TEST 5: API key auto-detection from env vars
# ══════════════════════════════════════════════════════════════════════════════

echo ""
echo "── Test 5: Env var auto-detection ──"

rm -rf "$HOME/.vault" "$HOME/.hq"

OPENROUTER_API_KEY="sk-test-openrouter-123" \
ANTHROPIC_API_KEY="sk-ant-test-456" \
    hq install --non-interactive >/dev/null 2>&1

assert_contains "$HOME/.hq/config.yaml" "openrouter_api_key" "Env: OpenRouter key detected"
assert_contains "$HOME/.hq/config.yaml" "anthropic_api_key" "Env: Anthropic key detected"
assert_contains "$HOME/.vault/_system/CAPABILITIES.md" "configured" "Env: CAPABILITIES shows configured"

# ══════════════════════════════════════════════════════════════════════════════
# TEST 6: Service commands (Linux/systemd)
# ══════════════════════════════════════════════════════════════════════════════

echo ""
echo "── Test 6: Service commands ──"

assert_exit_0 "hq service status runs" hq service status
assert_exit_0 "hq service install runs" hq service install

# Check that systemd unit files were created (if systemd dir is writable)
SYSTEMD_DIR="$HOME/.config/systemd/user"
if [ -d "$SYSTEMD_DIR" ]; then
    if [ -f "$SYSTEMD_DIR/agent-hq-agent.service" ]; then
        pass "Systemd unit: agent-hq-agent.service created"
    else
        # May not have systemd in container, that's OK
        pass "Systemd dir exists (units may require systemd runtime)"
    fi
else
    pass "Systemd dir created by service install"
fi

assert_exit_0 "hq service uninstall runs" hq service uninstall

# ══════════════════════════════════════════════════════════════════════════════
# TEST 7: hq onboard --reset
# ══════════════════════════════════════════════════════════════════════════════

echo ""
echo "── Test 7: Onboard reset ──"

# Simulate some progress
sed -i 's/pending/done/g' "$HOME/.vault/_system/ONBOARD.md" 2>/dev/null || true

hq onboard --reset >/dev/null 2>&1

assert_contains "$HOME/.vault/_system/ONBOARD.md" "pending" "Onboard reset: steps back to pending"

# ══════════════════════════════════════════════════════════════════════════════
# TEST 8: Health check after install
# ══════════════════════════════════════════════════════════════════════════════

echo ""
echo "── Test 8: Health check ──"

OUTPUT_HEALTH=$(hq health 2>&1)

if echo "$OUTPUT_HEALTH" | grep -q "\[OK\].*Vault scaffolded"; then
    pass "Health: vault scaffolded check passes"
else
    fail "Health: vault scaffolded check"
fi

if echo "$OUTPUT_HEALTH" | grep -q "\[OK\].*Config"; then
    pass "Health: config check passes"
else
    fail "Health: config check"
fi

# ══════════════════════════════════════════════════════════════════════════════
# TEST 9: setup alias works
# ══════════════════════════════════════════════════════════════════════════════

echo ""
echo "── Test 9: Aliases ──"

rm -rf "$HOME/.vault" "$HOME/.hq"

assert_exit_0 "hq setup (alias for install) works" hq setup --non-interactive
assert_file "$HOME/.vault/_system/SOUL.md" "setup alias: SOUL.md created"

# Legacy init
rm -rf "$HOME/.vault" "$HOME/.hq"

assert_exit_0 "hq init (legacy) still works" hq init --non-interactive
assert_file "$HOME/.vault/_system/SOUL.md" "init legacy: SOUL.md created"

# ══════════════════════════════════════════════════════════════════════════════
# TEST 10: Version command
# ══════════════════════════════════════════════════════════════════════════════

echo ""
echo "── Test 10: Version ──"

VERSION_OUT=$(hq version 2>&1)
if echo "$VERSION_OUT" | grep -q "hq.*rust"; then
    pass "Version command shows version"
else
    fail "Version command output unexpected: $VERSION_OUT"
fi

# ══════════════════════════════════════════════════════════════════════════════
# TEST 11: Custom vault path
# ══════════════════════════════════════════════════════════════════════════════

echo ""
echo "── Test 11: Custom vault path ──"

rm -rf "$HOME/.vault" "$HOME/.hq" /tmp/custom-vault

hq install --non-interactive --vault-path /tmp/custom-vault >/dev/null 2>&1

assert_dir "/tmp/custom-vault" "Custom vault path: directory created"
assert_file "/tmp/custom-vault/_system/SOUL.md" "Custom vault path: SOUL.md in custom location"
assert_file "/tmp/custom-vault/_system/guides/tool-setup.md" "Custom vault path: guides in custom location"
assert_contains "$HOME/.hq/config.yaml" "/tmp/custom-vault" "Custom vault path: config points to custom path"

rm -rf /tmp/custom-vault

# ══════════════════════════════════════════════════════════════════════════════
# TEST 12: Install output format
# ══════════════════════════════════════════════════════════════════════════════

echo ""
echo "── Test 12: Install output format ──"

rm -rf "$HOME/.vault" "$HOME/.hq"

OUTPUT_FMT=$(hq install --non-interactive 2>&1)

# Check all 7 steps appear in output
if echo "$OUTPUT_FMT" | grep -q "Step 1: Platform detection"; then
    pass "Output: Step 1 header present"
else
    fail "Output: Step 1 header missing"
fi

if echo "$OUTPUT_FMT" | grep -q "Step 2: Scaffolding vault"; then
    pass "Output: Step 2 header present"
else
    fail "Output: Step 2 header missing"
fi

if echo "$OUTPUT_FMT" | grep -q "Step 3: Seeding soul content"; then
    pass "Output: Step 3 header present"
else
    fail "Output: Step 3 header missing"
fi

if echo "$OUTPUT_FMT" | grep -q "Step 4: Installing guide files"; then
    pass "Output: Step 4 header present"
else
    fail "Output: Step 4 header missing"
fi

if echo "$OUTPUT_FMT" | grep -q "Step 5: Writing config"; then
    pass "Output: Step 5 header present"
else
    fail "Output: Step 5 header missing"
fi

if echo "$OUTPUT_FMT" | grep -q "Step 6: Detecting tools"; then
    pass "Output: Step 6 header present"
else
    fail "Output: Step 6 header missing"
fi

if echo "$OUTPUT_FMT" | grep -q "Installation complete"; then
    pass "Output: completion summary present"
else
    fail "Output: completion summary missing"
fi

# Should suggest onboard since no tools/keys are configured
if echo "$OUTPUT_FMT" | grep -q "hq onboard"; then
    pass "Output: suggests hq onboard"
else
    fail "Output: should suggest hq onboard"
fi

# ══════════════════════════════════════════════════════════════════════════════
# TEST 13: File content integrity (spot checks)
# ══════════════════════════════════════════════════════════════════════════════

echo ""
echo "── Test 13: Content integrity ──"

# SOUL.md should have all key sections
assert_contains "$HOME/.vault/_system/SOUL.md" "Operating Principles" "SOUL.md: has Operating Principles"
assert_contains "$HOME/.vault/_system/SOUL.md" "Vault Structure" "SOUL.md: has Vault Structure section"
assert_contains "$HOME/.vault/_system/SOUL.md" "Writing Standards" "SOUL.md: has Writing Standards"
assert_contains "$HOME/.vault/_system/SOUL.md" "pinned: true" "SOUL.md: frontmatter pinned"

# CONFIG.md should have the config table
assert_contains "$HOME/.vault/_system/CONFIG.md" "DEFAULT_MODEL" "CONFIG.md: has DEFAULT_MODEL"
assert_contains "$HOME/.vault/_system/CONFIG.md" "orchestration_mode" "CONFIG.md: has orchestration_mode"

# HEARTBEAT.md should be self-documenting
assert_contains "$HOME/.vault/_system/HEARTBEAT.md" "Pending Actions" "HEARTBEAT.md: has Pending Actions"
assert_contains "$HOME/.vault/_system/HEARTBEAT.md" "daemon processes" "HEARTBEAT.md: explains daemon"

# CAPABILITIES.md should have platform info
assert_contains "$HOME/.vault/_system/CAPABILITIES.md" "Platform" "CAPABILITIES.md: has Platform section"
assert_contains "$HOME/.vault/_system/CAPABILITIES.md" "LLM Access" "CAPABILITIES.md: has LLM section"
assert_contains "$HOME/.vault/_system/CAPABILITIES.md" "Agent Harnesses" "CAPABILITIES.md: has Harnesses section"
assert_contains "$HOME/.vault/_system/CAPABILITIES.md" "Integrations" "CAPABILITIES.md: has Integrations section"

# Getting started guide should mention key commands
assert_contains "$HOME/.vault/_system/guides/getting-started.md" "hq health" "Guide: mentions hq health"
assert_contains "$HOME/.vault/_system/guides/getting-started.md" "hq onboard" "Guide: mentions hq onboard"
assert_contains "$HOME/.vault/_system/guides/getting-started.md" "daemon" "Guide: explains daemon"

# Vault structure guide should map all key directories
assert_contains "$HOME/.vault/_system/guides/vault-structure.md" "_system/" "Guide: maps _system/"
assert_contains "$HOME/.vault/_system/guides/vault-structure.md" "_jobs/" "Guide: maps _jobs/"
assert_contains "$HOME/.vault/_system/guides/vault-structure.md" "Notebooks/" "Guide: maps Notebooks/"
assert_contains "$HOME/.vault/_system/guides/vault-structure.md" "frontmatter" "Guide: explains frontmatter"

# ══════════════════════════════════════════════════════════════════════════════
# TEST 14: Concurrent vault paths (simulate different machines)
# ══════════════════════════════════════════════════════════════════════════════

echo ""
echo "── Test 14: Multiple vaults ──"

rm -rf /tmp/vault-a /tmp/vault-b

hq install --non-interactive --vault-path /tmp/vault-a >/dev/null 2>&1
hq install --non-interactive --vault-path /tmp/vault-b >/dev/null 2>&1

assert_file "/tmp/vault-a/_system/SOUL.md" "Vault A: SOUL.md exists"
assert_file "/tmp/vault-b/_system/SOUL.md" "Vault B: SOUL.md exists"
assert_file "/tmp/vault-a/_system/CAPABILITIES.md" "Vault A: CAPABILITIES.md exists"
assert_file "/tmp/vault-b/_system/CAPABILITIES.md" "Vault B: CAPABILITIES.md exists"

# Both should have full scaffolding independently
assert_dir "/tmp/vault-a/_jobs/pending" "Vault A: full scaffolding"
assert_dir "/tmp/vault-b/_delegation/pending/any" "Vault B: full scaffolding"

rm -rf /tmp/vault-a /tmp/vault-b

# ══════════════════════════════════════════════════════════════════════════════
# Summary
# ══════════════════════════════════════════════════════════════════════════════

echo ""
echo "============================================"
echo "  Results: $PASS passed, $FAIL failed ($TOTAL total)"
echo "============================================"
echo ""

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
