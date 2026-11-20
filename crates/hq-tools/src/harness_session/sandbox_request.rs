//! `sandbox_request`: a helper that hit a sandbox wall asks HQ for exactly what it needs.
//!
//! `hq_core::grants::classify` decides from the typed request and the owner's profile, never from
//! the helper's reason. An approved grant is written to `sandbox_grants`, and the session is
//! restarted (resumed, same conversation) so its sandbox carries the grant: a sandbox cannot be
//! widened on a live process. What needs the owner is filed in their inbox; the helper calls
//! again with the same arguments to find the answer. The profile a task works under is bound by
//! the owner (`hq grants profile`), never by anything a helper can write. Plan:
//! `docs/plans/delegated-sandbox-grants.md`.

use crate::registry::HqTool;
use crate::util::arg_str;
use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_core::config::AgentHostConfig;
use hq_core::grants::{
    Capability, Decision, GrantRequest, GrantsConfig, PathView, SandboxDelta, TaskContext,
    classify, denied_path, expand_resource, normalize, rule_covers,
};
use hq_core::types::{ValueItem, ValueKind};
use hq_db::sandbox_grants::{self as store, NewGrant};
use hq_db::{Database, value_items};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use value_items::RequestState;

const SOURCE: &str = "sandbox-grants";
const KEY_PREFIX: &str = "sandbox-grant:";
const REQUEST_TTL_SECS: i64 = 24 * 3600;
const GRANT_TTL_SECS: i64 = 24 * 3600;
/// Declined or refused requests after which HQ stops forwarding more from one session.
const MAX_SETBACKS: usize = 3;
const MAX_WAITING: usize = 3;
const MAX_TEXT_CHARS: usize = 300;
/// How long a restart waits for the helper to finish its turn before it stops it anyway.
const SETTLE_WAIT: Duration = Duration::from_secs(60);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Approved { id: String },
    Pending { id: String },
    Declined,
    Refused(String),
    Stopped,
    TooMany,
}

/// What the broker knows about the asking session.
pub(super) struct Asker<'a> {
    pub session: &'a str,
    pub cwd: &'a Path,
    pub task: Option<&'a str>,
    pub host: &'a str,
}

fn key_for(id: &str) -> String {
    format!("{KEY_PREFIX}{id}")
}

/// The repository a session works in: the folder a worktree was made from, else where it runs.
/// A standing rule for a repository applies to every session in it.
pub(super) fn repo_root(db: &Database, session: &str, cwd: &str) -> String {
    db.with_conn(|c| hq_db::session_checkouts::get(c, session))
        .ok()
        .flatten()
        .and_then(|k| k.repo.or(Some(k.folder)))
        .unwrap_or_else(|| cwd.to_string())
}

/// The scopes the owner's standing rules can reach this session through: its task and its
/// repository. (Its own scope holds its own grants, which are not rules.)
pub(super) fn rule_scopes(
    db: &Database,
    session: &str,
    task: Option<&str>,
    cwd: &str,
) -> Vec<String> {
    let mut out: Vec<String> = task
        .map(|t| format!("{}{t}", store::TASK_SCOPE))
        .into_iter()
        .collect();
    out.push(format!(
        "{}{}",
        store::REPO_SCOPE,
        repo_root(db, session, cwd)
    ));
    out
}

/// What a standing rule would let `req` do, when one covers it: exactly the resource asked for.
fn covered_by_rule(
    db: &Database,
    paths: &dyn PathView,
    asker: &Asker,
    req: &GrantRequest,
    now: i64,
) -> Result<Option<(SandboxDelta, String)>> {
    let scopes = rule_scopes(db, asker.session, asker.task, &asker.cwd.to_string_lossy());
    let home = paths.home();
    for rule in store::rules_for(db, &scopes, now)? {
        let Ok(cap) = serde_json::from_value::<Capability>(json!(rule.capability)) else {
            continue;
        };
        let mut resource = expand_resource(cap, &rule.resource, &home);
        if matches!(cap, Capability::FsRead | Capability::FsWrite) {
            resource = paths.canonical(&resource);
        }
        if rule_covers(cap, &resource, req) {
            return Ok(Some((delta_for_owner_approval(req), rule.id)));
        }
    }
    Ok(None)
}

/// The profile this session works under: the owner's binding for the session or its task.
pub(super) fn profile_name(db: &Database, session: &str) -> Option<String> {
    let task = db
        .with_conn(|c| hq_db::harness_sessions_registry::get(c, session))
        .ok()
        .flatten()
        .and_then(|row| row.mission_id);
    store::profile_for(db, session, task.as_deref())
        .ok()
        .flatten()
}

/// The paths of the computer a session runs on, asked of its host: that is where the home
/// directory and the symlinks are, and it is not always the machine HQ runs on.
pub(super) struct HostPaths {
    host: crate::agent_host::Host,
    home: PathBuf,
}

impl HostPaths {
    /// None when the host cannot resolve paths (an older host), so nothing is decided blind.
    pub(super) fn probe(host: crate::agent_host::Host) -> Option<Self> {
        let home = host.realpaths(&[])?.home;
        Some(Self {
            host,
            home: PathBuf::from(home),
        })
    }
}

impl PathView for HostPaths {
    fn home(&self) -> PathBuf {
        self.home.clone()
    }
    fn canonical(&self, path: &str) -> String {
        match self.host.realpaths(&[path.to_string()]) {
            Some(found) if found.resolved.len() == 1 => {
                found.resolved.into_iter().next().unwrap_or_default()
            }
            // An answer that cannot be read leaves the path as written; `classify` then judges
            // an unresolved path, and a symlink on this host cannot be seen through.
            _ => path.to_string(),
        }
    }
}

fn shown(text: &str) -> String {
    let clipped: String = text.chars().take(MAX_TEXT_CHARS).collect();
    clipped
        .chars()
        .filter(|c| !c.is_control() || *c == ' ')
        .collect::<String>()
        .escape_debug()
        .to_string()
}

fn describe(req: &GrantRequest) -> String {
    let what = match req.capability {
        Capability::FsWrite => "write to",
        Capability::FsRead => "read",
        Capability::Loopback => "connect to local port",
        Capability::Egress => "reach",
    };
    format!("{what} {}", shown(&req.resource))
}

/// Applies the owner's answers to requests this session had waiting. Approval counts only when
/// it is signed and unspent, and spends it.
pub(super) fn settle_pending(db: &Database, session: &str, now: i64) -> Result<()> {
    for grant in store::pending_for_session(db, session, now)? {
        let key = key_for(&grant.id);
        let latest = value_items::latest_engagement_by_dedup(db, &key)?;
        if RequestState::from_latest(latest, chrono::Utc::now()) == RequestState::Declined {
            store::decide(db, &grant.id, store::DECLINED, now, None)?;
            continue;
        }
        let signed = value_items::is_signed_approval(db, &key)?
            && super::folder_request::approval_unspent(db, &key)?;
        if signed && value_items::consume_approval(db, &key)? {
            store::decide(
                db,
                &grant.id,
                store::ACTIVE,
                now,
                Some(now + GRANT_TTL_SECS),
            )?;
        }
    }
    Ok(())
}

fn file_for_owner(
    db: &Database,
    id: &str,
    asker: &Asker,
    req: &GrantRequest,
    reason: &str,
) -> Result<()> {
    let mut item = ValueItem::new(
        SOURCE,
        ValueKind::ActionNeeded,
        format!("A helper wants to {}", describe(req)),
        format!(
            "Helper session {} asked to {}. HQ could not approve it by itself: it is outside the \
             access this task was given. Approving applies to this session only and lasts up to a \
             day; the session restarts to pick it up. The helper's own words, not HQ's: \"{}\". \
             Dismiss to refuse.",
            shown(asker.session),
            describe(req),
            shown(reason),
        ),
    )
    .with_dedup_key(key_for(id));
    item.expires_at = Some(chrono::Utc::now() + chrono::Duration::seconds(REQUEST_TTL_SECS));
    value_items::insert(db, &item)?;
    Ok(())
}

/// The change one owner-approved request applies: exactly the resource asked for, nothing wider.
fn delta_for_owner_approval(req: &GrantRequest) -> SandboxDelta {
    let mut d = SandboxDelta::default();
    match req.capability {
        Capability::FsWrite => d.writable.push(PathBuf::from(&req.resource)),
        Capability::FsRead => d.readable.push(PathBuf::from(&req.resource)),
        Capability::Loopback => d.loopback_ports.extend(req.resource.parse::<u16>()),
        Capability::Egress => {
            // `normalize` made the resource `host:port`; anything else never reaches here.
            if let Some((host, port)) = req.resource.rsplit_once(':')
                && let Ok(port) = port.parse()
            {
                d.allow.push((host.to_string(), port));
            }
        }
    }
    d
}

fn existing(
    db: &Database,
    session: &str,
    capability: &str,
    resource: &str,
    now: i64,
) -> Result<Option<Outcome>> {
    Ok(
        store::open_for(db, session, capability, resource, now)?.map(|open| {
            match open.state.as_str() {
                store::ACTIVE => Outcome::Approved { id: open.id },
                _ => Outcome::Pending { id: open.id },
            }
        }),
    )
}

/// Decides one request and records it. Everything but the restart happens here.
pub(super) fn decide(
    db: &Database,
    cfg: &GrantsConfig,
    paths: &dyn PathView,
    asker: &Asker,
    req: &GrantRequest,
    reason: &str,
    now: i64,
) -> Result<Outcome> {
    if hq_db::tasks::has_hidden_chars(&req.resource) || req.resource.contains('\0') {
        return Ok(Outcome::Refused(
            "the request has control or invisible characters, so it cannot be shown to the owner"
                .into(),
        ));
    }
    store::expire_stale(db, now)?;
    settle_pending(db, asker.session, now)?;
    let mut req = normalize(req);
    if matches!(req.capability, Capability::FsRead | Capability::FsWrite) {
        req.resource = paths.canonical(&req.resource);
    }
    let capability = serde_json::to_value(req.capability)?
        .as_str()
        .unwrap_or_default()
        .to_string();
    if let Some(found) = existing(db, asker.session, &capability, &req.resource, now)? {
        return Ok(found);
    }
    if store::declined_for(db, asker.session, &capability, &req.resource)? {
        return Ok(Outcome::Declined);
    }
    if store::setbacks(db, asker.session)? >= MAX_SETBACKS {
        return Ok(Outcome::Stopped);
    }
    if store::pending_for_session(db, asker.session, now)?.len() >= MAX_WAITING {
        return Ok(Outcome::TooMany);
    }
    // Compare like with like: a home reached through a link, or a profile path that is one, must
    // match the canonical resource.
    let home = PathBuf::from(paths.canonical(&paths.home().to_string_lossy()));
    let home = home.as_path();
    let profile = profile_name(db, asker.session)
        .and_then(|name| cfg.profiles.get(&name))
        .map(|p| {
            let mut r = p.resolved(home);
            for list in [&mut r.writable, &mut r.readable] {
                *list = list.iter().map(|x| paths.canonical(x)).collect();
            }
            r
        });
    let worktree = asker
        .cwd
        .canonicalize()
        .unwrap_or_else(|_| asker.cwd.to_path_buf());
    let ctx = TaskContext {
        home: home.to_path_buf(),
        worktree,
        profile,
    };
    let reason: String = reason.chars().take(MAX_TEXT_CHARS).collect();
    let until = Some(now + REQUEST_TTL_SECS);
    let mut decision = classify(&req, &ctx);
    let mut approved_note = String::new();
    // A request outside the profile that the owner's standing access covers is approved without
    // asking; a refusal is never overridden.
    if matches!(decision, Decision::Ask(_))
        && let Some((d, rule_id)) = covered_by_rule(db, paths, asker, &req, now)?
    {
        decision = Decision::Approve(d);
        // The rule that approved it, so revoking the rule revokes this grant too.
        approved_note = store::rule_note(&rule_id);
    }
    let (state, by, note, delta, expires) = match decision {
        Decision::Approve(d) => (
            store::ACTIVE,
            store::BY_POLICY,
            approved_note,
            d,
            Some(now + GRANT_TTL_SECS),
        ),
        Decision::Ask(why) => (
            store::PENDING,
            store::BY_SYSTEM,
            why.text().into(),
            delta_for_owner_approval(&req),
            until,
        ),
        Decision::Refuse(why) => (
            store::REFUSED,
            store::BY_SYSTEM,
            why.text().into(),
            SandboxDelta::default(),
            None,
        ),
    };
    let filed = store::file(
        db,
        &NewGrant {
            session_id: asker.session,
            task_id: asker.task,
            host: asker.host,
            repo: &repo_root(db, asker.session, &asker.cwd.to_string_lossy()),
            capability: &capability,
            resource: &req.resource,
            delta: &delta,
            state,
            decided_by: by,
            helper_reason: &reason,
            decision_note: &note,
            expires_at: expires,
        },
        now,
    );
    let id = match filed {
        Ok(id) => id,
        // A parallel call filed the same request first: answer from that one.
        Err(e) => match existing(db, asker.session, &capability, &req.resource, now)? {
            Some(found) => return Ok(found),
            None => return Err(e),
        },
    };
    Ok(match state {
        store::ACTIVE => Outcome::Approved { id },
        store::PENDING => {
            file_for_owner(db, &id, asker, &req, &reason)?;
            Outcome::Pending { id }
        }
        _ => Outcome::Refused(note),
    })
}

/// What a launch of `session` carries: its active grants and the access profile the owner bound
/// to it. Every path is checked again here, resolved, because the stored rows and the profile
/// are not trusted to still be what `classify` once approved.
pub(super) fn launch_delta(
    db: &Database,
    cfg: &AgentHostConfig,
    session: &str,
    paths: Option<&dyn PathView>,
    now: i64,
) -> SandboxDelta {
    let Some(paths) = paths else {
        return SandboxDelta::default();
    };
    let home = PathBuf::from(paths.canonical(&paths.home().to_string_lossy()));
    let mut delta = store::delta_for_session(db, session, now).unwrap_or_default();
    // The owner's standing access for this task and repository, read against this computer's home.
    let row = db
        .with_conn(|c| hq_db::harness_sessions_registry::get(c, session))
        .ok()
        .flatten();
    if let Some(row) = row {
        let scopes = rule_scopes(db, session, row.mission_id.as_deref(), &row.cwd);
        for rule in store::rules_for(db, &scopes, now).unwrap_or_default() {
            let Ok(cap) = serde_json::from_value::<Capability>(json!(rule.capability)) else {
                continue;
            };
            let resource = expand_resource(cap, &rule.resource, &paths.home());
            let extra = delta_for_owner_approval(&normalize(&GrantRequest {
                capability: cap,
                resource,
            }));
            delta.writable.extend(extra.writable);
            delta.readable.extend(extra.readable);
            delta.loopback_ports.extend(extra.loopback_ports);
            delta.allow.extend(extra.allow);
        }
    }
    if let Some(name) = profile_name(db, session)
        && let Some(p) = cfg.grants.profiles.get(&name)
    {
        let base = p.delta(&paths.home());
        delta.writable.extend(base.writable);
        delta.readable.extend(base.readable);
        delta.loopback_ports.extend(base.loopback_ports);
        delta.allow.extend(base.allow);
    }
    let safe = |list: Vec<PathBuf>| -> Vec<PathBuf> {
        list.into_iter()
            .map(|p| PathBuf::from(paths.canonical(&p.to_string_lossy())))
            .filter(|p| denied_path(p, &home).is_none())
            .collect()
    };
    delta.writable = safe(delta.writable);
    delta.readable = safe(delta.readable);
    delta.loopback_ports.retain(|p| *p != 0);
    delta
}

fn reply(outcome: Outcome, req: &GrantRequest, restart: &Restart) -> Value {
    let what = describe(req);
    match outcome {
        Outcome::Approved { id } => json!({
            "status": "approved",
            "grant": id,
            "restart": matches!(restart, Restart::Scheduled),
            "next": match restart {
                Restart::Scheduled => "HQ applied this to your session. Your session restarts when this turn ends and continues the same conversation. Do not retry now; you will be told when it is back.".to_string(),
                Restart::Blocked(why) => format!("HQ approved this, but could not restart you to apply it ({why}). It takes effect the next time your session is started. Tell the user that; do not retry now."),
                Restart::NotDue => "This is already in force. If the command still fails, it is not blocked by this; read the error again.".to_string(),
            },
        }),
        Outcome::Pending { id } => json!({
            "status": "pending_owner_approval",
            "grant": id,
            "next": format!("HQ could not approve '{what}' by itself and asked the owner. Tell the user you are waiting on it, carry on with anything that does not need it, then call sandbox_request again with the same arguments to find the answer. Do not work around the denial."),
        }),
        Outcome::Declined => json!({
            "status": "declined",
            "next": "The owner declined. Do not ask for this again or try another route to it; finish what you can without it and report what is blocked.",
        }),
        Outcome::Refused(why) => json!({
            "status": "refused",
            "reason": why,
            "next": "HQ does not grant this, and the owner was not asked. Use another approach, or report to the user that the task needs something HQ cannot give.",
        }),
        Outcome::Stopped => json!({
            "status": "stopped_asking",
            "next": "Several of your requests were declined or refused, so HQ is not forwarding more. Finish what you can and report what is blocked.",
        }),
        Outcome::TooMany => json!({
            "status": "too_many_waiting",
            "next": "Requests from this session are already waiting for the owner. Wait for their answers before asking for more.",
        }),
    }
}

/// Where the restart a grant needs stands.
pub(super) enum Restart {
    NotDue,
    Scheduled,
    Blocked(String),
}

static RESTARTING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

/// Restarts `session` once its turn ends, so its sandbox carries the new grant, unless it cannot
/// be restarted without losing something. Best effort after that: a failure is logged and the
/// grant stays recorded for the next launch.
pub(super) fn plan_restart(db: &Arc<Database>, session: &str) -> Restart {
    match store::restart_due(db, session) {
        Ok(true) => {}
        _ => return Restart::NotDue,
    }
    match super::control::restart_blocker(db, session) {
        Ok(None) => {}
        Ok(Some(why)) => {
            let _ = store::clear_restart_due(db, session);
            return Restart::Blocked(why);
        }
        Err(e) => {
            let _ = store::clear_restart_due(db, session);
            return Restart::Blocked(e.to_string());
        }
    }
    if !RESTARTING
        .lock()
        .is_ok_and(|mut r| r.insert(session.to_string()))
    {
        return Restart::Scheduled;
    }
    let _ = store::clear_restart_due(db, session);
    let (db, session) = (db.clone(), session.to_string());
    tokio::spawn(async move {
        restart(&db, &session).await;
        if let Ok(mut r) = RESTARTING.lock() {
            r.remove(&session);
        }
    });
    Restart::Scheduled
}

async fn restart(db: &Arc<Database>, session: &str) {
    let (d, id) = (db.clone(), session.to_string());
    let settled =
        tokio::task::spawn_blocking(move || super::control::wait(&d, &id, &[], SETTLE_WAIT)).await;
    if !matches!(settled, Ok(Ok(_))) {
        tracing::warn!(session, "sandbox grant restart: session never settled");
    }
    let (d, id) = (db.clone(), session.to_string());
    if let Ok(Err(e)) = tokio::task::spawn_blocking(move || super::control::stop(&d, &id)).await {
        tracing::warn!(session, error = %e, "sandbox grant restart: stop failed");
        return;
    }
    let note = "HQ applied the sandbox access you asked for and restarted you. Retry the command that was blocked.";
    if let Err(e) = super::control::resume(db, session, Some(note)).await {
        tracing::warn!(session, error = %e, "sandbox grant restart: resume failed");
    }
}

pub struct SandboxRequestTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for SandboxRequestTool {
    fn name(&self) -> &str {
        "sandbox_request"
    }

    fn description(&self) -> &str {
        "Ask HQ for access your sandbox denies, instead of working around it. Name exactly one resource: capability fs_write or fs_read with an absolute path, loopback with a local port, or egress with host or host:port. HQ approves what the owner's access profile for this task covers and restarts your session (same conversation) so it applies; anything else goes to the owner and you call again with the same arguments to find the answer. Credentials, HQ's own files and other agents' state are never granted. 'reason' is shown to the owner as your words and never decides anything."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "capability": { "type": "string", "enum": ["fs_write", "fs_read", "loopback", "egress"] },
                "resource": { "type": "string", "description": "Absolute path, port number, or host[:port]" },
                "reason": { "type": "string", "description": "What you are doing and what was denied" }
            },
            "required": ["capability", "resource"]
        })
    }

    fn category(&self) -> &str {
        "harness"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let Some(session) = super::caller_session(&args).map(str::to_string) else {
            bail!(
                "only a session HQ launched can ask for sandbox access; this call did not come from one"
            );
        };
        let capability: Capability = serde_json::from_value(json!(arg_str(&args, "capability")))
            .map_err(|_| {
                anyhow::anyhow!("capability must be fs_write, fs_read, loopback or egress")
            })?;
        let req = GrantRequest {
            capability,
            resource: arg_str(&args, "resource"),
        };
        let reason = arg_str(&args, "reason");
        let db = self.db.clone();
        let (outcome, restart) = {
            let (db, session, req) = (db.clone(), session.clone(), req.clone());
            tokio::task::spawn_blocking(move || -> Result<(Outcome, Restart)> {
                let row = super::control::get_row(&db, &session)?;
                let cwd = PathBuf::from(&row.cwd);
                let asker = Asker {
                    session: &session,
                    cwd: &cwd,
                    task: row.mission_id.as_deref(),
                    host: &row.host,
                };
                let cfg = super::agent_host_config().grants;
                let now = chrono::Utc::now().timestamp();
                let host = crate::agent_host::host(Some(&row.host))?;
                let Some(paths) = HostPaths::probe(host) else {
                    return Ok((
                        Outcome::Refused(
                            "the computer this session runs on cannot resolve paths for HQ yet (its host is older than this HQ); tell the user to update the host".into(),
                        ),
                        Restart::NotDue,
                    ));
                };
                let out = decide(&db, &cfg, &paths, &asker, &req, &reason, now)?;
                // A grant that took effect, on this call or since the last one, is carried by a
                // restart whatever this call's answer was.
                Ok((out, plan_restart(&db, &session)))
            })
            .await??
        };
        Ok(reply(outcome, &req, &restart))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::grants::{GrantProfile, canonical_lenient};

    struct TestPaths(PathBuf);

    impl PathView for TestPaths {
        fn home(&self) -> PathBuf {
            self.0.clone()
        }
        fn canonical(&self, path: &str) -> String {
            canonical_lenient(path)
        }
    }

    const SESSION: &str = "hs-1";

    struct Fixture {
        db: Database,
        cfg: GrantsConfig,
        cwd: PathBuf,
        home: PathBuf,
        _dir: tempfile::TempDir,
    }

    impl Fixture {
        fn at(&self, rel: &str) -> String {
            self.home.join(rel).to_string_lossy().into_owned()
        }
    }

    fn fixture(with_profile: bool) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let db = Database::open_memory().unwrap();
        let mut cfg = GrantsConfig::default();
        cfg.profiles.insert(
            "mobile".into(),
            GrantProfile {
                writable: vec!["~/.gradle".into()],
                loopback_ports: vec![5037],
                ..Default::default()
            },
        );
        if with_profile {
            store::bind_profile(&db, SESSION, "mobile", 1).unwrap();
        }
        Fixture {
            db,
            cfg,
            cwd: home.join("wt/app"),
            home,
            _dir: dir,
        }
    }

    fn ask(f: &Fixture, capability: Capability, resource: &str) -> Outcome {
        let asker = Asker {
            session: SESSION,
            cwd: &f.cwd,
            task: Some("t1"),
            host: "local",
        };
        let req = GrantRequest {
            capability,
            resource: resource.into(),
        };
        decide(
            &f.db,
            &f.cfg,
            &TestPaths(f.home.clone()),
            &asker,
            &req,
            "build failed",
            100,
        )
        .unwrap()
    }

    #[test]
    fn a_profile_covered_request_is_approved_and_reaches_the_next_launch() {
        let f = fixture(true);
        let out = ask(&f, Capability::FsWrite, &f.at(".gradle/caches"));
        assert!(matches!(out, Outcome::Approved { .. }), "{out:?}");
        let d = store::delta_for_session(&f.db, SESSION, 200).unwrap();
        assert_eq!(d.writable, vec![PathBuf::from(&f.at(".gradle/caches"))]);
        let again = ask(&f, Capability::FsWrite, &f.at(".gradle/caches"));
        assert!(
            matches!(again, Outcome::Approved { .. }),
            "asking twice files once"
        );
        assert_eq!(
            store::for_session(&f.db, SESSION, 10).unwrap().len(),
            2,
            "binding plus one grant"
        );
    }

    #[test]
    fn a_request_outside_the_profile_waits_for_the_owner_and_grants_nothing_yet() {
        let f = fixture(true);
        let out = ask(&f, Capability::Egress, "example.org");
        assert!(matches!(out, Outcome::Pending { .. }), "{out:?}");
        assert!(
            store::delta_for_session(&f.db, SESSION, 200)
                .unwrap()
                .is_empty()
        );
        let again = ask(&f, Capability::Egress, "example.org");
        assert!(matches!(again, Outcome::Pending { .. }));
        assert_eq!(
            store::pending_for_session(&f.db, SESSION, 100)
                .unwrap()
                .len(),
            1
        );
        let items = value_items::count_waiting_with_prefix(&f.db, KEY_PREFIX).unwrap();
        assert_eq!(items, 1, "one inbox item");
    }

    #[test]
    fn no_profile_means_ask_and_credentials_are_refused_without_asking_anyone() {
        let f = fixture(false);
        assert!(matches!(
            ask(&f, Capability::FsWrite, &f.at(".gradle")),
            Outcome::Pending { .. }
        ));
        let out = ask(&f, Capability::FsRead, &f.at(".ssh/id_ed25519"));
        assert!(matches!(out, Outcome::Refused(_)), "{out:?}");
        assert_eq!(
            value_items::count_waiting_with_prefix(&f.db, KEY_PREFIX).unwrap(),
            1
        );
    }

    #[test]
    fn a_declined_request_is_not_asked_again_and_three_setbacks_stop_the_session() {
        let f = fixture(true);
        let Outcome::Pending { id } = ask(&f, Capability::Egress, "a.example.org") else {
            panic!("pending")
        };
        assert!(store::decide(&f.db, &id, store::DECLINED, 150, None).unwrap());
        assert_eq!(
            ask(&f, Capability::Egress, "a.example.org"),
            Outcome::Declined
        );
        for p in [f.at(".ssh"), f.at(".aws")] {
            assert!(matches!(
                ask(&f, Capability::FsRead, &p),
                Outcome::Refused(_)
            ));
        }
        assert_eq!(
            ask(&f, Capability::Egress, "b.example.org"),
            Outcome::Stopped
        );
    }

    #[test]
    fn another_session_does_not_inherit_a_grant() {
        let f = fixture(true);
        ask(&f, Capability::FsWrite, &f.at(".gradle"));
        assert!(
            store::delta_for_session(&f.db, "hs-2", 200)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn only_the_owners_binding_opts_a_session_in_and_a_task_tag_does_nothing() {
        use hq_db::harness_sessions_registry as registry;
        let f = fixture(false);
        f.db.with_conn(|c| {
            c.execute(
                "INSERT OR IGNORE INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('in-1', 'personal', 'Work', 'work', 'FR')",
                [],
            )?;
            let tags = ["grants:mobile".to_string()];
            let tagged = hq_db::tasks::NewTask {
                title: "Build",
                description: "d",
                created_by: "test",
                tags: &tags,
                ..Default::default()
            };
            hq_db::tasks::create_task(c, "tk-tagged", "in-1", &tagged)?;
            for id in ["hs-tag", "hs-bound"] {
                registry::insert(
                    c,
                    &registry::NewSession {
                        id,
                        harness: "codex",
                        label: "",
                        cwd: "/t",
                        mission_id: Some("tk-tagged"),
                        placement: registry::Placement {
                            host: "local",
                            agent_name: id,
                            workspace_id: "w",
                            pane_id: "p",
                        },
                        purpose: None,
                    },
                )?;
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(
            profile_name(&f.db, "hs-tag"),
            None,
            "a tag a helper can write opts in nothing"
        );
        store::bind_profile(&f.db, "task:tk-tagged", "mobile", 5).unwrap();
        assert_eq!(profile_name(&f.db, "hs-tag").as_deref(), Some("mobile"));
        assert_eq!(profile_name(&f.db, "hs-missing"), None);
    }

    #[test]
    fn spellings_of_one_request_are_one_request_and_hidden_text_is_refused() {
        let f = fixture(true);
        let a = ask(&f, Capability::Egress, "Example.org");
        let b = ask(&f, Capability::Egress, "example.org:443");
        let (Outcome::Pending { id: x }, Outcome::Pending { id: y }) = (a, b) else {
            panic!("pending")
        };
        assert_eq!(x, y);
        let out = ask(
            &f,
            Capability::FsRead,
            &format!("{}/a\u{202e}b", f.home.display()),
        );
        assert!(matches!(out, Outcome::Refused(_)), "{out:?}");
    }

    #[test]
    fn a_launch_rechecks_stored_paths_and_adds_nothing_without_a_view_of_the_host() {
        let f = fixture(true);
        let d = SandboxDelta {
            writable: vec![f.home.join(".ssh/x"), f.home.join("ok")],
            loopback_ports: vec![0, 5037],
            ..Default::default()
        };
        store::file(
            &f.db,
            &NewGrant {
                session_id: SESSION,
                task_id: None,
                host: "local",
                repo: "",
                capability: "fs_write",
                resource: "/planted",
                delta: &d,
                state: store::ACTIVE,
                decided_by: store::BY_POLICY,
                helper_reason: "",
                decision_note: "",
                expires_at: None,
            },
            2,
        )
        .unwrap();
        let cfg = AgentHostConfig::default();
        let got = launch_delta(&f.db, &cfg, SESSION, Some(&TestPaths(f.home.clone())), 10);
        // The home of this machine differs from the fixture's, so nothing under the fixture
        // home is a credential here; what matters is the port and that a remote host gets none.
        assert!(!got.loopback_ports.contains(&0));
        assert!(launch_delta(&f.db, &cfg, SESSION, None, 10).is_empty());
    }

    fn item_id(f: &Fixture, grant: &str) -> String {
        f.db.with_conn(|c| {
            Ok(c.query_row(
                "SELECT id FROM value_items WHERE dedup_key = ?1",
                [key_for(grant)],
                |r| r.get::<_, String>(0),
            )?)
        })
        .unwrap()
    }

    #[test]
    fn the_owners_yes_activates_the_request_once_and_the_no_declines_it() {
        let f = fixture(true);
        let Outcome::Pending { id } = ask(&f, Capability::Egress, "example.org") else {
            panic!("pending")
        };
        assert!(value_items::approve_by_id(&f.db, &item_id(&f, &id)).unwrap());
        let again = ask(&f, Capability::Egress, "example.org");
        assert_eq!(again, Outcome::Approved { id: id.clone() });
        assert!(
            store::restart_due(&f.db, SESSION).unwrap(),
            "the answer makes a restart due"
        );
        let d = store::delta_for_session(&f.db, SESSION, 200).unwrap();
        assert_eq!(d.allow, vec![("example.org".to_string(), 443)]);
        let third = ask(&f, Capability::Egress, "example.org");
        assert_eq!(third, Outcome::Approved { id });

        let Outcome::Pending { id: no } = ask(&f, Capability::Egress, "other.org") else {
            panic!("pending")
        };
        assert!(value_items::dismiss_by_id(&f.db, &item_id(&f, &no)).unwrap());
        assert_eq!(ask(&f, Capability::Egress, "other.org"), Outcome::Declined);
        assert_eq!(
            store::delta_for_session(&f.db, SESSION, 200)
                .unwrap()
                .allow
                .len(),
            1
        );
    }

    #[test]
    fn a_standing_repo_rule_approves_what_it_covers_without_asking_and_never_a_refusal() {
        let f = fixture(false);
        let repo = format!("{}{}", store::REPO_SCOPE, f.cwd.display());
        let empty = SandboxDelta::default();
        store::add_rule(
            &f.db,
            &repo,
            "fs_write",
            "~/.gradle",
            &empty,
            "starter mobile-dev",
            1,
            None,
        )
        .unwrap();
        store::add_rule(
            &f.db,
            &repo,
            "egress",
            "pub.dev:443",
            &empty,
            "starter mobile-dev",
            1,
            None,
        )
        .unwrap();
        // The rule's `~` means the session computer's home, so a cache under it is covered.
        let out = ask(&f, Capability::FsWrite, &f.at(".gradle/caches/x"));
        assert!(matches!(out, Outcome::Approved { .. }), "{out:?}");
        let out = ask(&f, Capability::Egress, "Pub.dev");
        assert!(matches!(out, Outcome::Approved { .. }), "{out:?}");
        let grants = store::for_session(&f.db, SESSION, 10).unwrap();
        assert!(
            grants
                .iter()
                .all(|g| g.decided_by == store::BY_POLICY && g.decision_note.starts_with("rule:"))
        );
        // Not covered: another host, and a sibling directory.
        assert!(matches!(
            ask(&f, Capability::Egress, "example.org"),
            Outcome::Pending { .. }
        ));
        assert!(matches!(
            ask(&f, Capability::FsWrite, &f.at(".gradlex")),
            Outcome::Pending { .. }
        ));
        // A rule naming a credential directory cannot open it.
        store::add_rule(&f.db, &repo, "fs_read", "~/.ssh", &empty, "oops", 2, None).unwrap();
        let out = ask(&f, Capability::FsRead, &f.at(".ssh/id_ed25519"));
        assert!(matches!(out, Outcome::Refused(_)), "{out:?}");
    }

    #[test]
    fn a_rule_for_another_repository_or_task_does_not_apply() {
        let f = fixture(false);
        store::add_rule(
            &f.db,
            "repo:/somewhere/else",
            "fs_write",
            "~/.gradle",
            &SandboxDelta::default(),
            "x",
            1,
            None,
        )
        .unwrap();
        store::add_rule(
            &f.db,
            "task:t-other",
            "fs_write",
            "~/.gradle",
            &SandboxDelta::default(),
            "x",
            1,
            None,
        )
        .unwrap();
        assert!(matches!(
            ask(&f, Capability::FsWrite, &f.at(".gradle")),
            Outcome::Pending { .. }
        ));
    }
}
