//! Relay adapters — Discord and Telegram bot implementations.
//! Implementations live in hq-relay; this module re-exports the entry points.


#[cfg(feature = "discord")]
pub use hq_relay::discord::run_discord_relay;
#[cfg(feature = "telegram")]
pub use hq_relay::telegram::run_telegram_relay;
