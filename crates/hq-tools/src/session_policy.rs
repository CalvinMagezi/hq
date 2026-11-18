//! Who may look at which session, and how many sessions a team may run. The checks the session
//! tools and the agent-to-agent tools share, kept below both so neither has to reach up into the
//! other.

use anyhow::{Result, bail};
use hq_core::config::TeamsConfig;
use hq_db::Database;
use hq_db::harness_sessions_registry::{self as registry, HarnessSessionRow};

/// Window, in minutes, for the per-hour limits on sessions started and messages sent.
pub const RATE_WINDOW_MINUTES: i64 = 60;

/// Checks that `parent` may get another child under the team's limits (depth, children per
/// session, running sessions per team, starts per hour), and returns the child's depth. Shared
/// by a delegation and a remote client adding a session to its team.
pub fn check_tree_limits(
    db: &Database,
    parent: &HarnessSessionRow,
    limits: &TeamsConfig,
) -> Result<i64> {
    let depth = parent.spawn_depth + 1;
    let max_depth = limits.depth_limit();
    if depth > max_depth {
        bail!(
            "this team allows {max_depth} levels of delegation below its lead (agent_host.teams.max_depth), and this session is already at the limit; do the work yourself or report back"
        );
    }
    let id = parent.id.clone();
    let (children, live, recent) = db.with_conn(move |c| {
        let root = registry::tree_root(c, &id)?;
        let live = registry::tree_members(c, &root)?
            .iter()
            .filter(|s| s.status == registry::STATUS_RUNNING)
            .count();
        Ok((
            registry::running_children(c, &id)?.len(),
            live,
            registry::tree_started_since(c, &root, RATE_WINDOW_MINUTES)?,
        ))
    })?;
    let max_children = limits.live_children();
    if children >= max_children {
        bail!(
            "this session already has {children} running children (at most {max_children}, agent_host.teams.max_live_children); wait for one to finish"
        );
    }
    let max_live = limits.live_per_tree();
    if live >= max_live {
        bail!(
            "this team already has {live} running sessions (at most {max_live}, agent_host.teams.max_live_per_tree); wait for one to finish"
        );
    }
    let per_hour = limits.spawns_per_hour();
    if recent >= per_hour {
        bail!(
            "rate limit: this team may start at most {per_hour} sessions per {RATE_WINDOW_MINUTES} minutes (agent_host.teams.max_spawns_per_hour)"
        );
    }
    Ok(depth)
}

/// Taken while a session is added to a team (a delegation, or a remote client's `team`), so
/// two starts cannot both pass the limits before either child exists.
pub(crate) static DELEGATION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Refuses a launched agent a look at any session outside its team (itself, its
/// parent and children, and everyone else under the same root).
pub fn check_session_access(
    c: &rusqlite::Connection,
    caller: Option<&str>,
    target: &str,
) -> Result<()> {
    let Some(session) = caller else { return Ok(()) };
    if session == target {
        return Ok(());
    }
    let row = registry::get(c, session)?;
    let is_parent = row.as_ref().and_then(|r| r.parent_session_id.as_deref()) == Some(target);
    let is_child = registry::running_children(c, session)?
        .iter()
        .any(|k| k.id == target);
    if is_parent || is_child || registry::same_team(c, session, target)? {
        return Ok(());
    }
    bail!("this session may only look at itself and the sessions of its own team")
}
