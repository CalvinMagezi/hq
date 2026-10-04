//! Embedded vault content for `hq install`.
//!
//! All content is compiled into the binary via `include_str!()` so it ships
//! with every release and is always available, even offline.

// ── System files ────────────────────────────────────────────────────────────

pub const SOUL: &str = include_str!("soul.md");
pub const MEMORY: &str = include_str!("memory.md");
pub const PREFERENCES: &str = include_str!("preferences.md");
pub const HEARTBEAT: &str = include_str!("heartbeat.md");
pub const CONFIG_SYSTEM: &str = include_str!("config_system.md");

// ── Guide files ─────────────────────────────────────────────────────────────

pub const GUIDE_GETTING_STARTED: &str = include_str!("guide_getting_started.md");
pub const GUIDE_VAULT_STRUCTURE: &str = include_str!("guide_vault_structure.md");
pub const GUIDE_AGENT_PRINCIPLES: &str = include_str!("guide_agent_principles.md");
pub const GUIDE_TOOL_SETUP: &str = include_str!("guide_tool_setup.md");
pub const GUIDE_WORKFLOWS: &str = include_str!("guide_workflows.md");
pub const GUIDE_MEMORY_SYSTEM: &str = include_str!("guide_memory_system.md");

/// All system files as (relative_path, content) pairs.
pub fn system_files() -> Vec<(&'static str, &'static str)> {
    vec![
        ("_system/SOUL.md", SOUL),
        ("_system/MEMORY.md", MEMORY),
        ("_system/PREFERENCES.md", PREFERENCES),
        ("_system/HEARTBEAT.md", HEARTBEAT),
        ("_system/CONFIG.md", CONFIG_SYSTEM),
    ]
}

/// All guide files as (relative_path, content) pairs.
pub fn guide_files() -> Vec<(&'static str, &'static str)> {
    vec![
        ("_system/guides/getting-started.md", GUIDE_GETTING_STARTED),
        ("_system/guides/vault-structure.md", GUIDE_VAULT_STRUCTURE),
        ("_system/guides/agent-principles.md", GUIDE_AGENT_PRINCIPLES),
        ("_system/guides/tool-setup.md", GUIDE_TOOL_SETUP),
        ("_system/guides/workflows.md", GUIDE_WORKFLOWS),
        ("_system/guides/memory-system.md", GUIDE_MEMORY_SYSTEM),
    ]
}

/// CAPABILITIES.md template with placeholders for detected values.
#[allow(clippy::too_many_arguments)]
pub fn capabilities_template(
    os: &str,
    arch: &str,
    shell: &str,
    openrouter: &str,
    anthropic: &str,
    google_ai: &str,
    gws_cli: &str,
    drawit_cli: &str,
    mcp_status: &str,
) -> String {
    format!(
        r#"---
noteType: system-file
fileName: capabilities
version: 1
pinned: false
---
# HQ Capabilities

Auto-detected during `hq install`. Re-run `hq install --upgrade` to refresh.

## Platform

| Property | Value |
|----------|-------|
| OS | {os} |
| Arch | {arch} |
| Shell | {shell} |

## LLM Access

| Provider | Status |
|----------|--------|
| OpenRouter | {openrouter} |
| Anthropic | {anthropic} |
| Google AI | {google_ai} |

## Agent Router

All agent sessions run through HQ's built-in LLM router. External harnesses have been retired.

## Integrations

| Tool | Status |
|------|--------|
| gws CLI | {gws_cli} |
| DrawIt | {drawit_cli} |
| MCP Server | {mcp_status} |

## Features

| Feature | Status |
|---------|--------|
| Vault | ready |
| Jobs | ready |
| Delegation | ready (via built-in LLM router) |
| Multi-agent | ready |
| Relay (Discord/Telegram) | not configured |
| Sync | not configured |
| Daemon | not started |
"#
    )
}

/// ONBOARD.md initial state with all steps pending.
pub fn onboard_template() -> &'static str {
    r#"---
noteType: system-file
fileName: onboard
version: 1
---
# Onboarding Progress

Run `hq onboard` to walk through each step interactively.
Run `hq onboard --step N` to jump to a specific step.
Run `hq onboard --reset` to start fresh.

| Step | Name | Status | Completed |
|------|------|--------|-----------|
| 1 | API Keys | pending | — |
| 2 | Agent Harnesses | pending | — |
| 3 | Google Workspace | pending | — |
| 4 | MCP Server | pending | — |
| 5 | Relay Setup | pending | — |
| 6 | Personalization | pending | — |
"#
}
