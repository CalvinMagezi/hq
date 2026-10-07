//! What an agent says about itself through its hooks, and how that is weighed
//! against what its screen shows.

use crate::detect::AgentState;
use std::time::Duration;

/// A hook says the agent is working but the screen reads idle and nothing has
/// been printed this long: the "finished" hook was lost (an interrupted turn
/// fires none), so the screen wins.
pub const STALE_WORKING_AFTER: Duration = Duration::from_secs(15);

/// The state an event tells us, or None when it says nothing about state.
pub fn state_for(event: &str, notification_type: Option<&str>) -> Option<AgentState> {
    match event {
        "SessionStart" | "Stop" => Some(AgentState::Idle),
        "UserPromptSubmit" => Some(AgentState::Working),
        "Notification" => match notification_type {
            Some("permission_prompt") => Some(AgentState::Blocked),
            Some("idle_prompt") => Some(AgentState::Idle),
            _ => None,
        },
        _ => None,
    }
}

/// The state to report: the hook's word, except where the screen is more
/// reliable. A dialog on screen is always blocked, a reported block ends when
/// the dialog is gone (approving one fires no hook), and a "working" nobody
/// has printed anything for is stale.
pub fn combine(
    reported: Option<AgentState>,
    screen: Option<AgentState>,
    quiet: Duration,
) -> (Option<AgentState>, bool) {
    let Some(hook) = reported else {
        return (screen, false);
    };
    match (hook, screen) {
        (_, Some(AgentState::Blocked)) => (screen, false),
        (AgentState::Blocked, Some(_)) => (screen, false),
        (AgentState::Working, Some(AgentState::Idle)) if quiet >= STALE_WORKING_AFTER => {
            (screen, false)
        }
        _ => (Some(hook), true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use AgentState::*;

    const QUIET: Duration = Duration::from_secs(60);
    const BUSY: Duration = Duration::from_secs(1);

    #[test]
    fn events_map_to_states() {
        assert_eq!(state_for("UserPromptSubmit", None), Some(Working));
        assert_eq!(state_for("Stop", None), Some(Idle));
        assert_eq!(state_for("SessionStart", None), Some(Idle));
        assert_eq!(
            state_for("Notification", Some("permission_prompt")),
            Some(Blocked)
        );
        assert_eq!(state_for("Notification", Some("idle_prompt")), Some(Idle));
        assert_eq!(state_for("Notification", Some("auth_success")), None);
        assert_eq!(state_for("PreToolUse", None), None);
    }

    #[test]
    fn without_a_report_the_screen_decides() {
        assert_eq!(combine(None, Some(Working), BUSY), (Some(Working), false));
        assert_eq!(combine(None, None, BUSY), (None, false));
    }

    #[test]
    fn a_report_wins_over_a_screen_that_disagrees_mildly() {
        assert_eq!(
            combine(Some(Working), Some(Idle), BUSY),
            (Some(Working), true)
        );
        assert_eq!(combine(Some(Idle), Some(Working), BUSY), (Some(Idle), true));
        assert_eq!(combine(Some(Idle), None, BUSY), (Some(Idle), true));
    }

    #[test]
    fn a_dialog_on_screen_is_always_blocked() {
        assert_eq!(
            combine(Some(Idle), Some(Blocked), BUSY),
            (Some(Blocked), false)
        );
        assert_eq!(
            combine(Some(Working), Some(Blocked), QUIET),
            (Some(Blocked), false)
        );
    }

    #[test]
    fn a_stale_working_report_gives_way_to_a_quiet_idle_screen() {
        assert_eq!(
            combine(Some(Working), Some(Idle), QUIET),
            (Some(Idle), false)
        );
        // A screen with no opinion cannot overrule the hook.
        assert_eq!(combine(Some(Working), None, QUIET), (Some(Working), true));
    }

    #[test]
    fn a_reported_block_ends_when_the_dialog_leaves_the_screen() {
        // Approving a dialog fires no hook, so the screen has to end the block.
        assert_eq!(
            combine(Some(Blocked), Some(Working), BUSY),
            (Some(Working), false)
        );
        assert_eq!(
            combine(Some(Blocked), Some(Idle), BUSY),
            (Some(Idle), false)
        );
        assert_eq!(combine(Some(Blocked), None, BUSY), (Some(Blocked), true));
    }
}
