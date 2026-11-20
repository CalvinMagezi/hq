//! `sandbox_request`: a helper that hit a sandbox wall asks HQ for exactly what it needs.
//!
//! `hq_core::grants::classify` decides from the typed request and the owner's profile, never from
//! the helper's reason. An approved grant is written to `sandbox_grants`, and the session is
//! restarted (resumed, same conversation) so its sandbox carries the grant: a sandbox cannot be
//! widened on a live process. What needs the owner is filed in their inbox; the helper calls
//! again with the same arguments to find the answer. Plan: `docs/plans/delegated-sandbox-grants.md`.

use crate::registry::HqTool;
use crate::util::arg_str;
use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_core::config::AgentHostConfig;
use hq_core::grants::{
    Capability, Decision, GrantRequest, GrantsConfig, SandboxDelta, TaskContext, classify,
};
use hq_core::types::{ValueItem, ValueKind};
use hq_db::sandbox_grants::{self as store, NewGrant};
use hq_db::{Database, value_items};
use serde_json::{Value, json};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use value_items::RequestState;

const SOURCE: &str = "sandbox-grants";
const KEY_PREFIX: &str = "sandbox-grant:";
const REQUEST_TTL_HOURS: i64 = 24;
const GRANT_TTL_SECS: i64 = 24 * 3600;
/// Declined or refused requests after which HQ stops forwarding more from one session.
const MAX_SETBACKS: usize = 3;
const MAX_WAITING: usize = 3;
const MAX_TEXT_CHARS: usize = 300;
/// How long a restart waits for the helper to finish its turn before it stops it anyway.
const SETTLE_WAIT: Duration = Duration::from_secs(60);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Approved { id: String, new: bool },
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

/// The tag that opts a task into an access profile: `grants:mobile` names the profile `mobile`.
const PROFILE_TAG_PREFIX: &str = "grants:";

/// The profile this session works under: one bound to it directly, else the one its task's
/// `grants:<name>` tag names. `None` means it opted into nothing, so every request asks.
pub(super) fn profile_name(db: &Database, session: &str) -> Option<String> {
    if let Ok(Some(bound)) = store::profile_for_session(db, session) {
        return Some(bound);
    }
    let task = db
        .with_conn(|c| hq_db::harness_sessions_registry::get(c, session))
        .ok()
        .flatten()?
        .mission_id?;
    let task = db
        .with_conn(|c| hq_db::tasks::get_task(c, &task))
        .ok()
        .flatten()?;
    task.tags
        .iter()
        .find_map(|t| t.strip_prefix(PROFILE_TAG_PREFIX))
        .map(str::to_string)
}

fn key_for(id: &str) -> String {
    format!("{KEY_PREFIX}{id}")
}

/// `path` with its longest existing ancestor resolved through symlinks, so a link cannot carry a
/// grant outside the directory it names. A path with `..` is returned as written, for `classify`
/// to refuse.
fn canonical_lenient(path: &str) -> String {
    let p = PathBuf::from(path);
    if !p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir)) {
        return path.to_string();
    }
    let mut tail = Vec::new();
    let mut at = p.as_path();
    loop {
        if let Ok(real) = at.canonicalize() {
            let mut out = real;
            out.extend(tail.iter().rev());
            return out.to_string_lossy().into_owned();
        }
        let (Some(name), Some(parent)) = (at.file_name(), at.parent()) else {
            return path.to_string();
        };
        tail.push(name.to_os_string());
        at = parent;
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

/// Applies the owner's answers to requests this session had waiting. Returns the ids that
/// became active, which is what makes a restart due.
pub(super) fn settle_pending(db: &Database, session: &str) -> Result<Vec<String>> {
    let mut activated = Vec::new();
    for grant in store::pending_for_session(db, session)? {
        let key = key_for(&grant.id);
        let latest = value_items::latest_engagement_by_dedup(db, &key)?;
        let now = chrono::Utc::now();
        match RequestState::from_latest(latest, now) {
            RequestState::Declined => {
                store::decide(db, &grant.id, store::DECLINED, now.timestamp())?;
            }
            RequestState::Ask | RequestState::Wait => {
                let signed = value_items::is_signed_approval(db, &key)?;
                if signed && super::folder_request::approval_unspent(db, &key)? {
                    value_items::consume_approval(db, &key)?;
                    if store::decide(db, &grant.id, store::ACTIVE, now.timestamp())? {
                        activated.push(grant.id);
                    }
                }
            }
        }
    }
    Ok(activated)
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
    item.expires_at = Some(chrono::Utc::now() + chrono::Duration::hours(REQUEST_TTL_HOURS));
    value_items::insert(db, &item)?;
    Ok(())
}

/// Decides one request and records it. Everything but the restart happens here.
pub(super) fn decide(
    db: &Database,
    cfg: &GrantsConfig,
    home: &Path,
    asker: &Asker,
    req: &GrantRequest,
    reason: &str,
    now: i64,
) -> Result<Outcome> {
    let activated = settle_pending(db, asker.session)?;
    let capability = serde_json::to_value(req.capability)?
        .as_str()
        .unwrap_or_default()
        .to_string();
    let mut req = req.clone();
    if matches!(req.capability, Capability::FsRead | Capability::FsWrite) {
        req.resource = canonical_lenient(&req.resource);
    }
    if let Some(open) = store::open_for(db, asker.session, &capability, &req.resource)? {
        return Ok(match open.state.as_str() {
            store::ACTIVE => Outcome::Approved {
                new: activated.contains(&open.id),
                id: open.id,
            },
            _ => Outcome::Pending { id: open.id },
        });
    }
    if store::declined_for(db, asker.session, &capability, &req.resource)? {
        return Ok(Outcome::Declined);
    }
    if store::setbacks(db, asker.session)? >= MAX_SETBACKS {
        return Ok(Outcome::Stopped);
    }
    if store::pending_for_session(db, asker.session)?.len() >= MAX_WAITING {
        return Ok(Outcome::TooMany);
    }
    // Compare like with like: a home reached through a link, or a profile path that is one, must
    // match the canonical resource.
    let home = PathBuf::from(canonical_lenient(&home.to_string_lossy()));
    let home = home.as_path();
    let profile = profile_name(db, asker.session)
        .and_then(|name| cfg.profiles.get(&name))
        .map(|p| {
            let mut r = p.resolved(home);
            for list in [&mut r.writable, &mut r.readable] {
                *list = list.iter().map(|x| canonical_lenient(x)).collect();
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
    let (state, by, note, delta, expires) = match classify(&req, &ctx) {
        Decision::Approve(d) => (
            store::ACTIVE,
            store::BY_POLICY,
            String::new(),
            d,
            Some(now + GRANT_TTL_SECS),
        ),
        Decision::Ask(why) => (
            store::PENDING,
            store::BY_SYSTEM,
            why.text().into(),
            delta_for_owner_approval(&req),
            Some(now + GRANT_TTL_SECS),
        ),
        Decision::Refuse(why) => (
            store::REFUSED,
            store::BY_SYSTEM,
            why.text().into(),
            SandboxDelta::default(),
            None,
        ),
    };
    let id = store::file(
        db,
        &NewGrant {
            session_id: asker.session,
            task_id: asker.task,
            host: asker.host,
            repo: &asker.cwd.to_string_lossy(),
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
    )?;
    Ok(match state {
        store::ACTIVE => Outcome::Approved { id, new: true },
        store::PENDING => {
            file_for_owner(db, &id, asker, &req, &reason)?;
            Outcome::Pending { id }
        }
        _ => Outcome::Refused(note),
    })
}

/// The change one owner-approved request applies: exactly the resource asked for, nothing wider.
fn delta_for_owner_approval(req: &GrantRequest) -> SandboxDelta {
    let mut d = SandboxDelta::default();
    match req.capability {
        Capability::FsWrite => d.writable.push(PathBuf::from(&req.resource)),
        Capability::FsRead => d.readable.push(PathBuf::from(&req.resource)),
        Capability::Loopback => d.loopback_ports.extend(req.resource.trim().parse::<u16>()),
        Capability::Egress => {
            let (host, port) = req
                .resource
                .rsplit_once(':')
                .and_then(|(h, p)| Some((h.to_string(), p.parse().ok()?)))
                .unwrap_or_else(|| (req.resource.clone(), 443));
            d.allow.push((host.to_ascii_lowercase(), port));
        }
    }
    d
}

/// What a launch of `session` carries: its active grants and the access profile it opted into.
pub(super) fn launch_delta(
    db: &Database,
    cfg: &AgentHostConfig,
    session: &str,
    now: i64,
) -> SandboxDelta {
    let mut delta = store::delta_for_session(db, session, now).unwrap_or_default();
    let home = dirs::home_dir();
    let profile = profile_name(db, session);
    if let (Some(name), Some(home)) = (profile, home)
        && let Some(p) = cfg.grants.profiles.get(&name)
    {
        let base = p.delta(&home);
        delta.writable.extend(base.writable);
        delta.readable.extend(base.readable);
        delta.loopback_ports.extend(base.loopback_ports);
        delta.allow.extend(base.allow);
    }
    delta
}

fn reply(outcome: Outcome, req: &GrantRequest) -> Value {
    let what = describe(req);
    match outcome {
        Outcome::Approved { id, new } => json!({
            "status": "approved",
            "grant": id,
            "restart": new,
            "next": if new {
                "HQ applied this to your session. Your session restarts when this turn ends and continues the same conversation. Do not retry now; you will be told when it is back."
            } else {
                "This is already in force. If the command still fails, it is not blocked by this; read the error again."
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
            "next": "HQ never grants this, and the owner was not asked. Use another approach, or report to the user that the task needs something HQ cannot give.",
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

/// Restarts `session` once its turn ends, so its sandbox carries the new grant. Best effort: a
/// failure is logged and the grant stays recorded for the next launch.
fn schedule_restart(db: Arc<Database>, session: String) {
    tokio::spawn(async move {
        let (d, id) = (db.clone(), session.clone());
        let settled =
            tokio::task::spawn_blocking(move || super::control::wait(&d, &id, &[], SETTLE_WAIT))
                .await;
        if !matches!(settled, Ok(Ok(_))) {
            tracing::warn!(session = %session, "sandbox grant restart: session never settled");
        }
        let (d, id) = (db.clone(), session.clone());
        if let Ok(Err(e)) = tokio::task::spawn_blocking(move || super::control::stop(&d, &id)).await
        {
            tracing::warn!(session = %session, error = %e, "sandbox grant restart: stop failed");
            return;
        }
        let note = "HQ applied the sandbox access you asked for and restarted you. Retry the command that was blocked.";
        if let Err(e) = super::control::resume(&db, &session, Some(note)).await {
            tracing::warn!(session = %session, error = %e, "sandbox grant restart: resume failed");
        }
    });
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
        let Some(home) = dirs::home_dir() else {
            bail!("HQ cannot tell which home directory this host uses");
        };
        let db = self.db.clone();
        let (outcome, req) = {
            let (db, session, req) = (db.clone(), session.clone(), req.clone());
            tokio::task::spawn_blocking(move || -> Result<(Outcome, GrantRequest)> {
                let row = super::control::get_row(&db, &session)?;
                let task = row.mission_id.clone();
                let cwd = PathBuf::from(&row.cwd);
                let asker = Asker {
                    session: &session,
                    cwd: &cwd,
                    task: task.as_deref(),
                    host: &row.host,
                };
                let cfg = super::agent_host_config().grants;
                let out = decide(
                    &db,
                    &cfg,
                    &home,
                    &asker,
                    &req,
                    &reason,
                    chrono::Utc::now().timestamp(),
                )?;
                Ok((out, req))
            })
            .await??
        };
        if matches!(outcome, Outcome::Approved { new: true, .. }) {
            schedule_restart(db, session);
        }
        Ok(reply(outcome, &req))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::grants::GrantProfile;

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
            let d = SandboxDelta::default();
            store::file(
                &db,
                &NewGrant {
                    session_id: SESSION,
                    task_id: None,
                    host: "local",
                    repo: "",
                    capability: store::PROFILE,
                    resource: "mobile",
                    delta: &d,
                    state: store::ACTIVE,
                    decided_by: store::BY_OWNER,
                    helper_reason: "",
                    decision_note: "",
                    expires_at: None,
                },
                1,
            )
            .unwrap();
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
        decide(&f.db, &f.cfg, &f.home, &asker, &req, "build failed", 100).unwrap()
    }

    #[test]
    fn a_profile_covered_request_is_approved_and_reaches_the_next_launch() {
        let f = fixture(true);
        let out = ask(&f, Capability::FsWrite, &f.at(".gradle/caches"));
        assert!(
            matches!(out, Outcome::Approved { new: true, .. }),
            "{out:?}"
        );
        let d = store::delta_for_session(&f.db, SESSION, 200).unwrap();
        assert_eq!(d.writable, vec![PathBuf::from(&f.at(".gradle/caches"))]);
        let again = ask(&f, Capability::FsWrite, &f.at(".gradle/caches"));
        assert!(
            matches!(again, Outcome::Approved { new: false, .. }),
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
        assert_eq!(store::pending_for_session(&f.db, SESSION).unwrap().len(), 1);
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
        assert!(store::decide(&f.db, &id, store::DECLINED, 150).unwrap());
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
    fn a_symlink_cannot_carry_a_grant_outside_the_listed_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
        let got = canonical_lenient(&format!("{}/link/new/dir", root.display()));
        assert_eq!(got, format!("{}/real/new/dir", root.display()));
        assert_eq!(
            canonical_lenient("/a/../b"),
            "/a/../b",
            "left for classify to refuse"
        );
    }

    #[test]
    fn a_task_tag_opts_its_session_into_a_profile_and_no_tag_means_none() {
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
            let plain = hq_db::tasks::NewTask {
                title: "Plain",
                description: "d",
                created_by: "test",
                ..Default::default()
            };
            hq_db::tasks::create_task(c, "tk-plain", "in-1", &plain)?;
            for (id, task) in [("hs-tag", "tk-tagged"), ("hs-plain", "tk-plain")] {
                registry::insert(
                    c,
                    &registry::NewSession {
                        id,
                        harness: "codex",
                        label: "",
                        cwd: "/t",
                        mission_id: Some(task),
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
        assert_eq!(profile_name(&f.db, "hs-tag").as_deref(), Some("mobile"));
        assert_eq!(profile_name(&f.db, "hs-plain"), None);
        assert_eq!(profile_name(&f.db, "hs-missing"), None);
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
        assert_eq!(
            again,
            Outcome::Approved {
                id: id.clone(),
                new: true
            }
        );
        let d = store::delta_for_session(&f.db, SESSION, 200).unwrap();
        assert_eq!(d.allow, vec![("example.org".to_string(), 443)]);
        let third = ask(&f, Capability::Egress, "example.org");
        assert_eq!(
            third,
            Outcome::Approved { id, new: false },
            "the restart is due once"
        );

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
}
