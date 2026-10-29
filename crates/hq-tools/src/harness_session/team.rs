//! A remote client forming one team (docs/plans/remote-driven-teams.md, phase 4.4). A claude.ai
//! connection or a web chat that starts a lead and then its builder and reviewer used to get
//! three unrelated sessions: no roster, no messages between them, no stop-with-root, no team
//! limits. `team: <root session id>` on spawn and handoff starts the new session under that
//! root instead, with everything a delegated worker gets. It is not `from_session`, which is a
//! session naming itself as the delegator.

use super::*;
use crate::util::arg_str;

/// The root named in `team`, if any.
pub fn parse(args: &Value) -> Option<String> {
    Some(arg_str(args, "team").trim().to_string()).filter(|t| !t.is_empty())
}

/// Checks that the caller may add a session to `root`'s team and that the team has room.
/// Returns the root and the new session's depth. Only an owner grant (the owner's own key, a
/// web chat, a claude.ai connection on a team it started) may: an agent adds workers with
/// `agent_delegate`, and the handoff key cannot reach into a team.
pub fn join(db: &Arc<Database>, args: &Value, root: &str) -> Result<(String, i64)> {
    if CwdBound::of(args) == CwdBound::Handoff {
        bail!("the handoff key cannot add a session to a team; start it without team");
    }
    if caller_session(args).is_some() || spawned_session(args).is_some() {
        bail!("an agent session adds workers to its team with agent_delegate, not team");
    }
    connector_may_drive(db, args, root)?;
    let id = root.to_string();
    let row = db
        .with_conn(move |c| registry::get(c, &id))?
        .filter(|r| r.status == registry::STATUS_RUNNING)
        .ok_or_else(|| anyhow::anyhow!("no running session '{root}' to add a session to"))?;
    if let Some(parent) = &row.parent_session_id {
        let id = root.to_string();
        let top = db.with_conn(move |c| registry::tree_root(c, &id))?;
        bail!("session {root} is a worker under {parent}; name the team's root, {top}, as team");
    }
    let depth = crate::a2a::check_tree_limits(db, &row, &agent_host_config().teams)?;
    Ok((row.id, depth))
}

/// The writers a reviewer joining `root`'s team observes when it names none: every running
/// coding session in the team.
pub fn writers(db: &Arc<Database>, root: &str) -> Vec<String> {
    let id = root.to_string();
    db.with_conn(move |c| registry::tree_members(c, &id))
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.status == registry::STATUS_RUNNING && r.purpose == registry::PURPOSE_CODE)
        .map(|r| r.id)
        .collect()
}
