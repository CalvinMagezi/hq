//! Who started the turn a session is working on, so an automated caller cannot stop a turn the
//! owner started. Kept in memory: after a restart nothing is known and nothing is refused, which
//! is how stopping worked before.

use super::answer::Answerer;
use anyhow::{Result, bail};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Most sessions remembered; past it the table starts over rather than grow without bound.
const MAX_TRACKED: usize = 512;
/// The keys that stop a running turn.
const STOP_KEYS: &[&str] = &["esc", "escape", "ctrl+c"];

/// Session id to whether the owner started its latest turn.
fn table() -> &'static Mutex<HashMap<String, bool>> {
    static TABLE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn locked() -> std::sync::MutexGuard<'static, HashMap<String, bool>> {
    table().lock().unwrap_or_else(|p| p.into_inner())
}

/// Records who gave `session_id` its latest prompt.
pub(super) fn note_prompt(session_id: &str, who: Answerer) {
    let mut table = locked();
    if table.len() >= MAX_TRACKED && !table.contains_key(session_id) {
        table.clear();
    }
    table.insert(session_id.to_string(), who == Answerer::Owner);
}

/// Refuses an automated caller's stop keys while the session works on a turn the owner started.
pub(super) fn check_stop(
    session_id: &str,
    who: Answerer,
    keys: &[String],
    working: bool,
) -> Result<()> {
    let Answerer::Automated(by) = who else {
        return Ok(());
    };
    let stops = !keys.is_empty()
        && keys
            .iter()
            .all(|k| STOP_KEYS.contains(&k.to_ascii_lowercase().as_str()));
    if !(stops && working) {
        return Ok(());
    }
    if locked().get(session_id) == Some(&true) {
        bail!(
            "session {session_id} is working on a turn the owner started, so {by} may not stop it. Wait for it to finish, or ask the owner."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn esc() -> Vec<String> {
        vec!["Esc".to_string()]
    }

    #[test]
    fn an_agent_may_not_stop_a_turn_the_owner_started() {
        note_prompt("hs-owner-turn", Answerer::Owner);
        let refused = check_stop(
            "hs-owner-turn",
            Answerer::Automated("an agent session"),
            &esc(),
            true,
        );
        assert!(refused.unwrap_err().to_string().contains("owner started"));
    }

    #[test]
    fn an_agent_may_stop_its_own_or_an_idle_session() {
        note_prompt("hs-agent-turn", Answerer::Automated("an agent session"));
        let agent = Answerer::Automated("an agent session");
        assert!(check_stop("hs-agent-turn", agent, &esc(), true).is_ok());
        note_prompt("hs-owner-idle", Answerer::Owner);
        assert!(check_stop("hs-owner-idle", agent, &esc(), false).is_ok());
    }

    #[test]
    fn the_owner_other_keys_and_unknown_sessions_are_never_refused() {
        note_prompt("hs-owner-keys", Answerer::Owner);
        let agent = Answerer::Automated("an agent session");
        assert!(check_stop("hs-owner-keys", Answerer::Owner, &esc(), true).is_ok());
        assert!(check_stop("hs-owner-keys", Answerer::Policy, &esc(), true).is_ok());
        assert!(check_stop("hs-owner-keys", agent, &["down".to_string()], true).is_ok());
        assert!(check_stop("hs-never-seen", agent, &esc(), true).is_ok());
    }

    #[test]
    fn a_later_agent_prompt_hands_the_turn_to_the_agent() {
        let agent = Answerer::Automated("an agent session");
        note_prompt("hs-handover", Answerer::Owner);
        note_prompt("hs-handover", agent);
        assert!(check_stop("hs-handover", agent, &esc(), true).is_ok());
    }
}
