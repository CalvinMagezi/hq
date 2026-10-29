//! Where a coding session writes: its own git worktree, or the paths it owns in a folder it
//! shares (docs/plans/remote-driven-teams.md, phase 4). Two agents editing one working tree
//! overwrite each other, and one running `git stash` takes the other's changes with it, so
//! a second writer in a folder is refused unless the paths the two own do not overlap.
//! A worktree gives each its own tree on its own branch; a reviewer gets the commit it
//! reviews, detached. The host makes and removes worktrees; HQ records them here.

use super::*;
use crate::agent_host::{GitHead, WorktreeBase, WorktreeInfo};
use crate::util::arg_str_list;
use hq_db::session_checkouts::{self as checkouts, NewCheckout};

/// Most paths one session may own.
const MAX_OWNS: usize = 50;

/// What the caller asked for in `worktree`.
pub fn parse_worktree(args: &Value) -> Result<Option<WorktreeBase>> {
    let Some(v) = args.get("worktree").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let field = |k: &str| v.get(k).and_then(Value::as_str).map(str::trim);
    match (field("branch"), field("detached")) {
        (Some(b), None) if !b.is_empty() => Ok(Some(WorktreeBase::Branch(b.to_string()))),
        (None, Some(r)) if !r.is_empty() => Ok(Some(WorktreeBase::Detached(r.to_string()))),
        _ => bail!(
            "worktree takes {{\"branch\": \"<name>\"}} for a writer or {{\"detached\": \"<commit or ref>\"}} for a reviewer"
        ),
    }
}

/// `owns`: paths relative to the session's folder, without `..`. Trailing slashes and a
/// leading `./` are dropped, so `src/`, `./src` and `src` are one path.
pub fn parse_owns(args: &Value) -> Result<Vec<String>> {
    let raw = arg_str_list(args, "owns");
    if raw.len() > MAX_OWNS {
        bail!("owns takes at most {MAX_OWNS} paths");
    }
    let mut out = Vec::new();
    for p in raw {
        let clean = p
            .trim()
            .trim_start_matches("./")
            .trim_end_matches('/')
            .to_string();
        if clean.is_empty() || clean == "." {
            bail!(
                "owns paths are folders or files inside the session's folder, not the folder itself"
            );
        }
        if Path::new(&clean).is_absolute() || clean.split('/').any(|c| c == "..") {
            bail!("owns path '{p}' must be relative to the session's folder, without '..'");
        }
        if !out.contains(&clean) {
            out.push(clean);
        }
    }
    Ok(out)
}

/// Whether two writers' paths meet. No paths means the whole folder.
pub fn overlap(a: &[String], b: &[String]) -> bool {
    if a.is_empty() || b.is_empty() {
        return true;
    }
    let within = |x: &str, y: &str| x == y || x.starts_with(&format!("{y}/"));
    a.iter()
        .any(|x| b.iter().any(|y| within(x, y) || within(y, x)))
}

/// Refuses a writer whose paths overlap another running writer's in the same folder. Its own
/// parent is not counted: a session that delegated work in its folder waits on it. Nor is a
/// session still marked running that the host no longer has. Blocking: it may ask the host.
pub(super) fn check_writers(
    db: &Arc<Database>,
    host: &Host,
    folder: &str,
    parent: Option<&str>,
    owns: &[String],
) -> Result<()> {
    let (h, f) = (host.name().to_string(), folder.to_string());
    let writers = db.with_conn(move |c| checkouts::writers_in(c, &h, &f))?;
    let mut listed: Option<std::result::Result<Vec<AgentInfo>, _>> = None;
    for w in writers {
        if parent == Some(w.session_id.as_str()) || !overlap(&w.owns, owns) {
            continue;
        }
        // Gone from its host, as the supervisor reads it, the session is about to be marked
        // exited. A host that cannot say counts it as there.
        let agents = listed.get_or_insert_with(|| host.agents());
        if let Ok(agents) = agents
            && !agents
                .iter()
                .any(|a| a.name.as_deref() == Some(w.agent_name.as_str()) && !a.exited)
        {
            continue;
        }
        let theirs = if w.owns.is_empty() {
            "the whole folder".to_string()
        } else {
            w.owns.join(", ")
        };
        bail!(
            "session {} ('{}') already writes in {folder} ({theirs}). Two agents in one working tree overwrite each other. Start this one in its own worktree with worktree: {{\"branch\": \"<new branch>\"}} (a reviewer: {{\"detached\": \"<commit>\"}}), or give both sessions owns paths that do not overlap.",
            w.session_id,
            if w.label.is_empty() {
                "unlabelled"
            } else {
                &w.label
            },
        );
    }
    Ok(())
}

/// What a caller is told about a session's worktree.
pub(super) fn describe(w: &WorktreeInfo, base: &WorktreeBase) -> Value {
    let mut v = json!({ "path": w.path, "repo": w.repo, "head": w.head, "created": w.created });
    match base {
        WorktreeBase::Branch(b) => v["branch"] = json!(b),
        WorktreeBase::Detached(r) => v["detached"] = json!(r),
    }
    v
}

/// Records where a session that just started writes, and the commit it started at.
pub(super) fn record(
    db: &Arc<Database>,
    session_id: &str,
    folder: &str,
    worktree: Option<(&WorktreeInfo, &WorktreeBase)>,
    owns: &[String],
    head: Option<&str>,
) {
    let (branch, detached) = match worktree.map(|(_, b)| b) {
        Some(WorktreeBase::Branch(b)) => (Some(b.as_str()), None),
        Some(WorktreeBase::Detached(r)) => (None, Some(r.as_str())),
        None => (None, None),
    };
    let info = worktree.map(|(i, _)| i);
    let recorded = db.with_conn(|c| {
        checkouts::record(
            c,
            &NewCheckout {
                session_id,
                folder,
                worktree: info.map(|i| i.path.as_str()),
                repo: info.map(|i| i.repo.as_str()),
                branch,
                detached,
                owns,
                head: head.filter(|h| !h.is_empty()),
            },
        )
    });
    if let Err(e) = recorded {
        tracing::warn!(session = session_id, error = %e, "where the session writes was not recorded");
    }
}

/// Removes a stopped or exited session's worktree when it has nothing uncommitted. The branch
/// keeps every commit. Returns what to tell the caller, if it had a worktree.
pub fn release(db: &Arc<Database>, host: &Host, session_id: &str) -> Option<Value> {
    let id = session_id.to_string();
    let checkout = db
        .with_conn(move |c| checkouts::get(c, &id))
        .ok()
        .flatten()?;
    let path = checkout.worktree.clone()?;
    if checkout.removed_at.is_some() {
        return None;
    }
    match host.worktree_remove(&path) {
        Ok(r) if r.removed => {
            let id = session_id.to_string();
            let _ = db.with_conn(move |c| checkouts::mark_removed(c, &id));
            Some(json!({ "removed": path, "branch": checkout.branch }))
        }
        Ok(_) => Some(json!({
            "kept": path,
            "note": "it has changes nobody committed, so it was kept; commit or discard them there, then remove it with `git worktree remove`",
        })),
        Err(e) => {
            tracing::warn!(session = session_id, error = %e, "worktree not removed");
            Some(json!({ "kept": path, "note": format!("could not remove it: {e}") }))
        }
    }
}

/// The worktree a resumed session starts in again: made anew on its branch (or commit) when it
/// was removed at the stop, found as it was otherwise. `None` for a session without one.
pub(super) fn reopen(db: &Arc<Database>, host: &Host, session_id: &str) -> Result<Option<String>> {
    let id = session_id.to_string();
    let Some(checkout) = db.with_conn(move |c| checkouts::get(c, &id))? else {
        return Ok(None);
    };
    let (Some(_), Some(repo)) = (&checkout.worktree, &checkout.repo) else {
        return Ok(None);
    };
    let base = match (&checkout.branch, &checkout.detached) {
        (Some(b), _) => WorktreeBase::Branch(b.clone()),
        (None, Some(r)) => {
            WorktreeBase::Detached(checkout.last_head.clone().unwrap_or_else(|| r.clone()))
        }
        (None, None) => return Ok(None),
    };
    let info = host.worktree_add(repo, session_id, &base)?;
    record(
        db,
        session_id,
        &checkout.folder,
        Some((&info, &base)),
        &checkout.owns,
        checkout.last_head.as_deref().or(Some(info.head.as_str())),
    );
    Ok(Some(info.path))
}

/// Whether HQ keeps track of where `session_id` writes (a session started before this existed,
/// or not a coding session, has no record).
pub fn tracked(db: &Arc<Database>, session_id: &str) -> bool {
    let id = session_id.to_string();
    db.with_conn(move |c| checkouts::get(c, &id))
        .ok()
        .flatten()
        .is_some()
}

/// Notes the commit a session's folder is at after a finished turn. True when it is a commit
/// the session made since the last one noted: it is recorded as a `commit` event once.
pub fn note_commit(db: &Arc<Database>, session_id: &str, head: &GitHead) -> bool {
    let (id, sha) = (session_id.to_string(), head.sha.clone());
    let Ok(Some(before)) = db.with_conn(move |c| checkouts::note_head(c, &id, &sha)) else {
        return false;
    };
    // The first commit seen is where the session started, not something it made.
    if before.is_none() {
        return false;
    }
    let detail =
        json!({ "sha": head.sha, "subject": head.subject, "branch": head.branch }).to_string();
    let id = session_id.to_string();
    let _ = db
        .with_conn(move |c| registry::record_event(c, &id, "commit", "supervisor", Some(&detail)));
    true
}

/// "committed 1a2b3c4 (subject) on branch b": how a commit is named to whoever watches.
pub fn commit_line(head: &GitHead) -> String {
    let short: String = head.sha.chars().take(12).collect();
    match &head.branch {
        Some(b) => format!("committed {short} \"{}\" on branch {b}", head.subject),
        None => format!("committed {short} \"{}\"", head.subject),
    }
}
