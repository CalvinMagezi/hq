//! Relay adapter core — platform bridges, session state, and chat handling
//! for the Discord and Telegram relays.
//!
//! An earlier "unified relay framework" (`UnifiedBot`, `AgentBackend`,
//! `SessionOrchestrator`, the WebSocket `protocol` types) was never wired up
//! and is gone, and the WhatsApp `PlatformBridge` was retired on 2026-09-24.
//! The real entry points are `discord::run_discord_relay` and
//! `telegram::run_telegram_relay`, wired directly in `hq-cli`.

// The shared modules are crate-private and only the bridges call them, so a
// build with a bridge feature off leaves some of them unused.
#![cfg_attr(not(all(feature = "discord", feature = "telegram")), allow(dead_code))]

mod chat_commands;
mod heartbeat;
mod native_run;
pub mod relay_common;
mod session_info;
mod session_runner;
mod subagent_followup;
mod thread_sync;
mod watch_scheduler;

#[cfg(feature = "discord")]
pub mod discord;
#[cfg(feature = "discord")]
mod discord_channels;
#[cfg(feature = "discord")]
pub mod discord_helpers {
    pub use super::discord::build_status_embed;
    pub use super::discord::format_attachment_descriptor;
}
#[cfg(feature = "telegram")]
pub mod telegram;
