//! The one system notice the hub posts: a helper has stopped and needs the person's OK.
//!
//! Only the decisions live here (who just became blocked, what the notice says, where it opens).
//! Posting it is the shell's job. The notice names a helper and never says what it wants.

use crate::agents::{AgentView, Need};
use crate::window::main_path_allowed;
use std::collections::HashSet;

/// The notice's title. The text of every string below is also in `apps/strings/en.yaml`.
pub const TITLE: &str = "AgentHQ · HQ Hub";
/// The notice's one action.
pub const OPEN_ACTION: &str = "Open";
/// The line under the title when the helper has no name the person would know.
pub const BODY_DETAIL: &str = "Open HQ to see what it wants to do.";

/// A notice to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
    /// The page of HQ's web app the Open action goes to.
    pub path: String,
}

/// The names of helpers that are blocked now and were not before. A name that stays blocked
/// across polls is reported once.
pub fn newly_blocked<'a>(before: &HashSet<String>, agents: &'a [AgentView]) -> Vec<&'a AgentView> {
    agents
        .iter()
        .filter(|a| a.need == Need::Blocked && !before.contains(&a.name))
        .collect()
}

/// The names that are blocked, for the next comparison.
pub fn blocked_names(agents: &[AgentView]) -> HashSet<String> {
    agents
        .iter()
        .filter(|a| a.need == Need::Blocked)
        .map(|a| a.name.clone())
        .collect()
}

/// What a helper that just stopped for the person's OK shows. The helper's name is its kind
/// (Claude, Codex) when known, because that is what the person calls it.
pub fn notice_for(agent: &AgentView) -> Notice {
    let who = match agent.agent.as_deref() {
        Some("claude") => "Claude".to_string(),
        Some("codex") => "Codex".to_string(),
        Some(other) if !other.is_empty() => other.to_string(),
        _ => agent.name.clone(),
    };
    Notice {
        title: TITLE.to_string(),
        body: format!("{who} needs your OK. {BODY_DETAIL}"),
        path: main_path_allowed(&format!("/helpers/{}", agent.name)).unwrap_or_else(|| "/".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(name: &str, kind: Option<&str>, need: Need) -> AgentView {
        AgentView {
            name: name.into(),
            agent: kind.map(Into::into),
            need,
            title: String::new(),
            cwd: String::new(),
            quiet_secs: 0,
        }
    }

    #[test]
    fn a_helper_is_announced_once_when_it_becomes_blocked() {
        let first = [
            agent("a", Some("claude"), Need::Blocked),
            agent("b", None, Need::Working),
        ];
        let before = HashSet::new();
        assert_eq!(newly_blocked(&before, &first).len(), 1);
        let seen = blocked_names(&first);
        assert!(newly_blocked(&seen, &first).is_empty());
        let later = [
            agent("a", Some("claude"), Need::Working),
            agent("b", None, Need::Blocked),
        ];
        let blocked: Vec<_> = newly_blocked(&seen, &later)
            .iter()
            .map(|a| a.name.clone())
            .collect();
        assert_eq!(blocked, ["b"]);
    }

    #[test]
    fn the_notice_names_the_helper_and_opens_its_page_and_says_nothing_of_the_request() {
        let n = notice_for(&agent("fix-7", Some("claude"), Need::Blocked));
        assert_eq!(n.title, "AgentHQ · HQ Hub");
        assert_eq!(
            n.body,
            "Claude needs your OK. Open HQ to see what it wants to do."
        );
        assert_eq!(n.path, "/helpers/fix-7");
    }

    #[test]
    fn a_name_that_is_not_a_safe_page_opens_the_home_page() {
        let n = notice_for(&agent("a/../b", None, Need::Blocked));
        assert_eq!(n.path, "/");
    }
}
