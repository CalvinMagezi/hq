//! Telegram relay: teloxide bot with media handling and native HQ dispatch.
//!
//! Public surface: `run_telegram_relay` starts the bot and `send_message` lets
//! daemon tasks push notifications without holding relay session state.

mod access;
mod callbacks;
mod caller_context;
mod commands;
mod dispatch;
mod mailbox;
mod media;
mod send;
mod session;

pub use send::send_message;
pub use session::run_telegram_relay;
