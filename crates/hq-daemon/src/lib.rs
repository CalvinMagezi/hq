//! Daemon task bodies (event worker, value bus, email triage, embeddings,
//! memory consolidation, turn recovery). hq-cli's daemon loop schedules them.

pub mod agent_worker;
pub mod email_gate;
pub mod email_send;
pub mod email_triage;
pub mod embeddings;
pub mod family_confirm;
pub mod instance_lock;
pub mod machine_profile;
pub mod memory;
pub mod notif_gate;
pub mod task_state;
pub mod turn_reconciler;
pub mod user_model;
pub mod value_bus;

pub use email_triage::{EmailAction, parse_email_command, resolve_email_action};
pub use embeddings::process_embeddings;
pub use family_confirm::{FamilyAction, parse_family_command, resolve_family_action};
pub use machine_profile::spawn_machine_profile_loop;
pub use memory::run_memory_cycle;
pub use value_bus::{ValueAction, parse_value_command, record_engagement, run_value_bus_delivery};

/// Find the last `<keyword> <token>` in `text`, trying `commands` in order.
/// Returns the matched action, the 4 to 16 character alphanumeric token, and
/// the trimmed text before the keyword. ASCII lowercasing keeps byte offsets
/// valid for slicing the original text.
pub(crate) fn parse_token_command<A: Copy>(
    text: &str,
    commands: &[(&str, A)],
) -> Option<(A, String, String)> {
    let lower = text.to_ascii_lowercase();
    for &(kw, action) in commands {
        let Some(pos) = lower.rfind(kw) else { continue };
        let token: String = text[pos + kw.len()..]
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        if (4..=16).contains(&token.len()) {
            return Some((action, token, text[..pos].trim().to_string()));
        }
    }
    None
}
