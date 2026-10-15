//! Reviewing what was written: `note_review` sets a version's review state, `note_provenance`
//! reads who wrote a note, when, and what has been decided about it.
//!
//! Review state changes only through `note_review`. Writing, reading or searching a note never
//! touches it, and a new version of a note starts unreviewed whatever the previous one was.

use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_db::Database;
use hq_db::provenance::{self, Provenance, ReviewEvent, ReviewState, SourceKind};
use hq_vault::VaultClient;
use serde_json::{Value, json};
use std::sync::Arc;

use super::{names_private_folder, validate_path, validate_path_in_vault};
use crate::harness_session::{CONNECTOR_ACTOR, spawned_session};
use crate::provenance::{Writer, resolve};
use crate::registry::HqTool;

/// How many versions `note_provenance` returns unless asked for fewer or more.
const DEFAULT_VERSIONS: usize = 20;
const MAX_VERSIONS: usize = 100;
/// The name given when a call carries no attested identity.
const UNATTESTED: &str = "unattested";

/// Who is making a review, as far as the runtime can tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reviewer {
    /// Recorded with the decision.
    pub label: String,
    /// Whether this reviewer may raise trust (accept, or put a version back to unreviewed).
    /// An agent working in a session may flag a problem but may not vouch for work.
    pub may_raise_trust: bool,
    /// The reviewer's own session, for the rule that no one reviews their own work.
    session: Option<String>,
    /// Whether the rule applies at all: an agent in a session of its own.
    self_review_barred: bool,
}

impl Reviewer {
    /// Work out who is reviewing from the attested arguments of the call.
    pub fn of_call(args: &Value, db: &Database) -> Self {
        Self::of_writer(&resolve(args, db), spawned_session(args).is_some())
    }

    /// `spawned` is a call that says it comes from a session HQ launched. That marker is only
    /// ever read as a restriction.
    pub fn of_writer(writer: &Writer, spawned: bool) -> Self {
        let connector = writer
            .actor_id
            .as_deref()
            .is_some_and(|a| a.starts_with(CONNECTOR_ACTOR));
        let may_raise_trust = match writer.source_kind {
            SourceKind::Human => true,
            // A call with no attested identity holds the owner's key and nothing says otherwise,
            // unless it announced itself as running in a session HQ launched. A connector's call
            // reaches here only after the owner approved that exact call.
            SourceKind::Unknown => !spawned,
            SourceKind::Agent => connector,
            SourceKind::System | SourceKind::Imported | SourceKind::Generated => false,
        };
        Self {
            label: writer
                .actor_id
                .clone()
                .unwrap_or_else(|| UNATTESTED.to_string()),
            may_raise_trust,
            session: writer.session_id.clone(),
            self_review_barred: writer.source_kind == SourceKind::Agent && !connector,
        }
    }

    /// The owner, through the web app.
    pub fn owner() -> Self {
        Self::of_writer(&Writer::owner(), false)
    }

    fn wrote(&self, record: &Provenance) -> bool {
        record.actor_id == self.label
            || self
                .session
                .as_deref()
                .is_some_and(|s| record.session_id.as_deref() == Some(s))
    }
}

/// What a review of one version came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reviewed {
    pub record: Provenance,
    pub previous: ReviewState,
    pub history: Vec<ReviewEvent>,
}

/// Settle `state` on the version `id` of `path`, or on the version on disk now when `id` is
/// `None`. The one place a review decision is checked and applied.
pub fn review_note(
    db: &Database,
    vault: &VaultClient,
    path: &str,
    id: Option<i64>,
    state: ReviewState,
    reason: Option<&str>,
    reviewer: &Reviewer,
) -> Result<Reviewed> {
    validate_path(path)?;
    if names_private_folder(path) {
        bail!("{path} is not a note: private folders carry no review");
    }
    let reason = reason.map(str::trim).filter(|r| !r.is_empty());
    if matches!(state, ReviewState::Disputed | ReviewState::Superseded) && reason.is_none() {
        bail!("a {} verdict needs a reason", state.as_str());
    }
    if !reviewer.may_raise_trust && matches!(state, ReviewState::Accepted | ReviewState::Unreviewed)
    {
        bail!(
            "{} cannot set {}: an agent can mark a version reviewed, disputed or superseded, \
             but accepting work, or reopening a decision, is the owner's",
            reviewer.label,
            state.as_str()
        );
    }

    let version = match id {
        Some(id) => db
            .with_conn(|c| provenance::get(c, id))?
            .filter(|p| p.path == path)
            .ok_or_else(|| anyhow::anyhow!("record {id} is not a version of {path}"))?,
        None => {
            let hash = hash_on_disk(vault, path).ok_or_else(|| {
                anyhow::anyhow!("{path} cannot be read; pass the id of the version to review")
            })?;
            db.with_conn(|c| provenance::current(c, path, &hash))?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "the version of {path} on disk has no provenance record (it was changed \
                         outside the tools), so there is nothing to review; pass the id of an \
                         earlier version from note_provenance"
                    )
                })?
        }
    };
    if reviewer.self_review_barred && reviewer.wrote(&version) {
        bail!(
            "{} wrote this version and cannot review it; someone else has to",
            reviewer.label
        );
    }

    let label = reviewer.label.clone();
    let reason_owned = reason.map(str::to_string);
    let (record, history) = db.with_conn(move |c| {
        provenance::set_review_state(c, version.id, state, &label, reason_owned.as_deref())?;
        let record = provenance::get(c, version.id)?
            .ok_or_else(|| anyhow::anyhow!("record {} vanished", version.id))?;
        let history = provenance::review_history(c, version.id)?;
        Ok((record, history))
    })?;
    Ok(Reviewed {
        record,
        previous: version.review_state,
        history,
    })
}

fn hash_on_disk(vault: &VaultClient, path: &str) -> Option<String> {
    validate_path_in_vault(vault.vault_path(), path).ok()?;
    std::fs::read_to_string(vault.vault_path().join(path))
        .ok()
        .map(|raw| hq_db::search::content_hash(&raw))
}

fn event_json(e: &ReviewEvent) -> Value {
    json!({
        "state": e.state.as_str(),
        "reviewer": e.reviewer,
        "reason": e.reason,
        "at": e.at,
    })
}

/// One version of a note as the tools report it.
pub fn version_json(p: &Provenance, is_current: bool, events: &[ReviewEvent]) -> Value {
    json!({
        "id": p.id,
        "content_hash": p.content_hash,
        "is_current": is_current,
        "actor": p.actor_id,
        "source_kind": p.source_kind.as_str(),
        "session": p.session_id,
        "task": p.task_id,
        "written_at": p.written_at,
        "unattributed": p.is_unattributed(),
        "recorded_late": p.recorded_late,
        "deleted": p.deleted_at.is_some(),
        "review_state": p.review_state.as_str(),
        "reviewed_by": p.reviewed_by,
        "reviewed_at": p.reviewed_at,
        "review_reason": p.review_reason,
        "review_history": events.iter().map(event_json).collect::<Vec<_>>(),
    })
}

pub struct NoteReviewTool {
    vault: Arc<VaultClient>,
    db: Arc<Database>,
}

impl NoteReviewTool {
    pub fn new(vault: Arc<VaultClient>, db: Arc<Database>) -> Self {
        Self { vault, db }
    }
}

#[async_trait]
impl HqTool for NoteReviewTool {
    fn name(&self) -> &str {
        "note_review"
    }

    fn description(&self) -> &str {
        "Record a decision about one version of a vault note: reviewed, accepted, disputed, \
         superseded, or unreviewed to reopen it. This is the only way review state changes; \
         writing or reading a note never does. Disputed and superseded need a reason. Defaults to \
         the version on disk now; pass `id` (from note_provenance) to judge an earlier one. An \
         agent in a session may mark reviewed, disputed or superseded but not accept, and cannot \
         review a version it wrote itself. The decision, its reviewer and reason are kept."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Vault-relative note path." },
                "state": {
                    "type": "string",
                    "enum": ["reviewed", "accepted", "disputed", "superseded", "unreviewed"]
                },
                "reason": { "type": "string", "description": "Why. Required for disputed and superseded." },
                "id": { "type": "integer", "description": "A version id from note_provenance; omit for the version on disk." }
            },
            "required": ["path", "state"]
        })
    }

    fn category(&self) -> &str {
        "vault"
    }

    fn is_read_only(&self) -> bool {
        false
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        let state_text = args
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(state) = ReviewState::parse(state_text) else {
            bail!(
                "state must be one of reviewed, accepted, disputed, superseded, unreviewed (got '{state_text}')"
            );
        };
        let id = args.get("id").and_then(Value::as_i64);
        let reason = args
            .get("reason")
            .and_then(Value::as_str)
            .map(str::to_string);
        let reviewer = Reviewer::of_call(&args, &self.db);

        let (db, vault) = (self.db.clone(), self.vault.clone());
        let reviewed = tokio::task::spawn_blocking(move || {
            review_note(&db, &vault, &path, id, state, reason.as_deref(), &reviewer)
        })
        .await??;

        let mut out = version_json(&reviewed.record, true, &reviewed.history);
        out["path"] = json!(reviewed.record.path);
        out["previous_state"] = json!(reviewed.previous.as_str());
        Ok(out)
    }
}

pub struct NoteProvenanceTool {
    vault: Arc<VaultClient>,
    db: Arc<Database>,
}

impl NoteProvenanceTool {
    pub fn new(vault: Arc<VaultClient>, db: Arc<Database>) -> Self {
        Self { vault, db }
    }
}

#[async_trait]
impl HqTool for NoteProvenanceTool {
    fn name(&self) -> &str {
        "note_provenance"
    }

    fn description(&self) -> &str {
        "Who wrote a vault note and what has been decided about it, newest version first: actor, \
         source kind, session, task, when, and each version's review state with its reviewer, \
         reason and history. `current_version_recorded` is false when the file on disk was changed \
         outside the tools, in which case its author is not known."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Vault-relative note path." },
                "limit": { "type": "integer", "description": "Versions to return (default 20, at most 100)." }
            },
            "required": ["path"]
        })
    }

    fn category(&self) -> &str {
        "vault"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        validate_path(&path)?;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map_or(DEFAULT_VERSIONS, |n| (n as usize).clamp(1, MAX_VERSIONS));
        let on_disk = hash_on_disk(&self.vault, &path);

        let path_for_db = path.clone();
        let versions = self.db.with_conn(move |c| {
            let records = provenance::history(c, &path_for_db, limit)?;
            records
                .into_iter()
                .map(|p| {
                    let events = provenance::review_history(c, p.id)?;
                    Ok((p, events))
                })
                .collect::<Result<Vec<_>>>()
        })?;

        let recorded = match on_disk.clone() {
            Some(hash) => {
                let path = path.clone();
                self.db
                    .with_conn(move |c| provenance::current(c, &path, &hash))?
                    .is_some()
            }
            None => false,
        };
        let listed: Vec<Value> = versions
            .iter()
            .map(|(p, events)| version_json(p, on_disk.as_deref() == Some(&p.content_hash), events))
            .collect();
        Ok(json!({
            "path": path,
            "exists": on_disk.is_some(),
            "current_version_recorded": recorded,
            "versions": listed,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness_session::{CONNECTOR_ARG, SPAWNED_SESSION_ARG, connector_actor};
    use crate::provenance::{NATIVE_AGENT_ARG, NATIVE_SESSION_ARG, Recorder, Writer as ProvWriter};

    struct Fx {
        db: Arc<Database>,
        vault: Arc<VaultClient>,
        dir: tempfile::TempDir,
    }

    impl Fx {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join("Notebooks")).unwrap();
            Self {
                db: Arc::new(Database::open_memory().unwrap()),
                vault: Arc::new(VaultClient::new(dir.path().to_path_buf()).unwrap()),
                dir,
            }
        }

        /// Write `body` to `rel` as `args` would have, recording it.
        fn write_as(&self, rel: &str, body: &str, args: &Value) -> String {
            std::fs::write(self.dir.path().join(rel), body).unwrap();
            Recorder::new(self.db.clone(), self.dir.path().to_path_buf()).after_write(rel, args);
            hq_db::search::content_hash(body)
        }

        fn write_as_owner(&self, rel: &str, body: &str) -> String {
            std::fs::write(self.dir.path().join(rel), body).unwrap();
            Recorder::new(self.db.clone(), self.dir.path().to_path_buf())
                .record(rel, &ProvWriter::owner());
            hq_db::search::content_hash(body)
        }

        fn review(
            &self,
            rel: &str,
            id: Option<i64>,
            state: ReviewState,
            reason: Option<&str>,
            who: &Reviewer,
        ) -> Result<Reviewed> {
            review_note(&self.db, &self.vault, rel, id, state, reason, who)
        }

        fn record_of(&self, rel: &str, hash: &str) -> Provenance {
            self.db
                .with_conn(|c| provenance::current(c, rel, hash))
                .unwrap()
                .unwrap()
        }
    }

    fn agent_call(session: &str, agent: &str) -> Value {
        json!({ NATIVE_SESSION_ARG: session, NATIVE_AGENT_ARG: agent })
    }

    fn reviewer_for(fx: &Fx, args: &Value) -> Reviewer {
        Reviewer::of_call(args, &fx.db)
    }

    const NOTE: &str = "Notebooks/a.md";

    #[test]
    fn the_owner_accepts_the_version_on_disk_and_it_shows_who_and_why() {
        let fx = Fx::new();
        let hash = fx.write_as(NOTE, "v1", &agent_call("s-1", "builder"));

        let done = fx
            .review(
                NOTE,
                None,
                ReviewState::Accepted,
                Some("checked against the source"),
                &Reviewer::owner(),
            )
            .unwrap();

        assert_eq!(done.previous, ReviewState::Unreviewed);
        assert_eq!(done.record.review_state, ReviewState::Accepted);
        assert_eq!(done.record.reviewed_by.as_deref(), Some("owner"));
        assert_eq!(
            done.record.review_reason.as_deref(),
            Some("checked against the source")
        );
        assert_eq!(done.history.len(), 1);
        assert_eq!(
            fx.record_of(NOTE, &hash).review_state,
            ReviewState::Accepted
        );
    }

    #[test]
    fn every_decision_is_kept_in_order() {
        let fx = Fx::new();
        fx.write_as(NOTE, "v1", &agent_call("s-1", "builder"));
        let owner = Reviewer::owner();
        fx.review(NOTE, None, ReviewState::Accepted, None, &owner)
            .unwrap();
        fx.review(
            NOTE,
            None,
            ReviewState::Disputed,
            Some("a figure is wrong"),
            &owner,
        )
        .unwrap();
        let last = fx
            .review(
                NOTE,
                None,
                ReviewState::Superseded,
                Some("replaced by the 2026 note"),
                &owner,
            )
            .unwrap();

        let states: Vec<_> = last.history.iter().map(|e| e.state).collect();
        assert_eq!(
            states,
            [
                ReviewState::Accepted,
                ReviewState::Disputed,
                ReviewState::Superseded
            ]
        );
        assert_eq!(last.history[1].reason.as_deref(), Some("a figure is wrong"));
        assert_eq!(last.previous, ReviewState::Disputed);
    }

    #[test]
    fn a_new_version_starts_unreviewed_and_the_old_one_keeps_its_verdict() {
        let fx = Fx::new();
        let first = fx.write_as(NOTE, "v1", &agent_call("s-1", "builder"));
        fx.review(NOTE, None, ReviewState::Accepted, None, &Reviewer::owner())
            .unwrap();

        let second = fx.write_as(NOTE, "v2", &agent_call("s-1", "builder"));

        assert_eq!(
            fx.record_of(NOTE, &second).review_state,
            ReviewState::Unreviewed
        );
        assert_eq!(
            fx.record_of(NOTE, &first).review_state,
            ReviewState::Accepted
        );
    }

    #[test]
    fn writing_the_same_text_again_does_not_reset_or_change_a_verdict() {
        let fx = Fx::new();
        let hash = fx.write_as(NOTE, "same", &agent_call("s-1", "builder"));
        fx.review(
            NOTE,
            None,
            ReviewState::Disputed,
            Some("doubtful"),
            &Reviewer::owner(),
        )
        .unwrap();

        fx.write_as(NOTE, "same", &agent_call("s-2", "other"));

        let rec = fx.record_of(NOTE, &hash);
        assert_eq!(rec.review_state, ReviewState::Disputed);
        assert_eq!(rec.actor_id, "builder", "the first writer keeps the credit");
    }

    #[test]
    fn an_agent_in_a_session_cannot_accept_or_reopen_but_can_flag() {
        let fx = Fx::new();
        fx.write_as(NOTE, "v1", &agent_call("s-1", "builder"));
        let reviewer = reviewer_for(&fx, &agent_call("s-2", "checker"));

        let err = fx
            .review(NOTE, None, ReviewState::Accepted, None, &reviewer)
            .unwrap_err()
            .to_string();
        assert!(err.contains("checker cannot set accepted"), "{err}");
        assert!(
            fx.review(NOTE, None, ReviewState::Unreviewed, None, &reviewer)
                .is_err()
        );
        // Nothing was recorded by the refused calls.
        let events = fx
            .db
            .with_conn(|c| provenance::review_history(c, provenance::history(c, NOTE, 1)?[0].id))
            .unwrap();
        assert!(events.is_empty());

        fx.review(NOTE, None, ReviewState::Reviewed, None, &reviewer)
            .unwrap();
        let flagged = fx
            .review(
                NOTE,
                None,
                ReviewState::Disputed,
                Some("contradicts the ledger"),
                &reviewer,
            )
            .unwrap();
        assert_eq!(flagged.record.reviewed_by.as_deref(), Some("checker"));
    }

    #[test]
    fn nobody_reviews_their_own_version_by_name_or_by_session() {
        let fx = Fx::new();
        fx.write_as(NOTE, "v1", &agent_call("s-1", "builder"));

        // Same agent, new session.
        let same_name = reviewer_for(&fx, &agent_call("s-9", "builder"));
        let err = fx
            .review(NOTE, None, ReviewState::Reviewed, None, &same_name)
            .unwrap_err()
            .to_string();
        assert!(err.contains("cannot review it"), "{err}");

        // Different name, same session.
        let same_session = reviewer_for(&fx, &agent_call("s-1", "renamed"));
        assert!(
            fx.review(NOTE, None, ReviewState::Disputed, Some("x"), &same_session)
                .is_err()
        );

        // A different agent in a different session may.
        let other = reviewer_for(&fx, &agent_call("s-2", "checker"));
        assert!(
            fx.review(NOTE, None, ReviewState::Reviewed, None, &other)
                .is_ok()
        );
    }

    #[test]
    fn disputed_and_superseded_need_a_reason_the_others_do_not() {
        let fx = Fx::new();
        fx.write_as(NOTE, "v1", &agent_call("s-1", "builder"));
        let owner = Reviewer::owner();
        for state in [ReviewState::Disputed, ReviewState::Superseded] {
            assert!(fx.review(NOTE, None, state, None, &owner).is_err());
            assert!(fx.review(NOTE, None, state, Some("   "), &owner).is_err());
        }
        assert!(
            fx.review(NOTE, None, ReviewState::Accepted, None, &owner)
                .is_ok()
        );
        assert!(
            fx.review(NOTE, None, ReviewState::Reviewed, None, &owner)
                .is_ok()
        );
    }

    #[test]
    fn a_connector_call_may_accept_because_the_owner_approved_it_at_the_gateway() {
        let fx = Fx::new();
        fx.write_as(NOTE, "v1", &agent_call("s-1", "builder"));
        let reviewer = reviewer_for(&fx, &json!({ CONNECTOR_ARG: "Claude" }));
        assert!(reviewer.may_raise_trust);
        assert_eq!(reviewer.label, connector_actor("Claude"));
        let done = fx
            .review(NOTE, None, ReviewState::Accepted, None, &reviewer)
            .unwrap();
        assert_eq!(done.record.reviewed_by, Some(connector_actor("Claude")));
    }

    #[test]
    fn a_call_with_no_identity_holds_the_owners_key_unless_it_says_it_runs_in_a_session() {
        let fx = Fx::new();
        let bare = reviewer_for(&fx, &json!({}));
        assert!(bare.may_raise_trust);
        assert_eq!(
            bare.label, UNATTESTED,
            "the history does not claim it was the owner"
        );

        let spawned = reviewer_for(&fx, &json!({ SPAWNED_SESSION_ARG: "hs-1" }));
        assert!(!spawned.may_raise_trust);
    }

    #[test]
    fn the_scheduled_jobs_and_imports_never_review() {
        let system = Reviewer::of_writer(&ProvWriter::system("cleanup"), false);
        assert!(!system.may_raise_trust);
    }

    #[test]
    fn a_version_changed_outside_the_tools_has_nothing_to_review_but_an_earlier_one_can_be() {
        let fx = Fx::new();
        let hash = fx.write_as(NOTE, "recorded", &agent_call("s-1", "builder"));
        let earlier = fx.record_of(NOTE, &hash);
        // Edited in an editor: no record for what is on disk now.
        std::fs::write(fx.dir.path().join(NOTE), "edited by hand").unwrap();

        let owner = Reviewer::owner();
        let err = fx
            .review(NOTE, None, ReviewState::Accepted, None, &owner)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no provenance record"), "{err}");

        let done = fx
            .review(
                NOTE,
                Some(earlier.id),
                ReviewState::Disputed,
                Some("since overwritten"),
                &owner,
            )
            .unwrap();
        assert_eq!(done.record.id, earlier.id);
    }

    #[test]
    fn a_record_of_another_note_unknown_states_and_private_paths_are_refused() {
        let fx = Fx::new();
        fx.write_as(NOTE, "v1", &agent_call("s-1", "builder"));
        let other = fx.write_as("Notebooks/b.md", "v1", &agent_call("s-1", "builder"));
        let other_id = fx.record_of("Notebooks/b.md", &other).id;
        let owner = Reviewer::owner();

        assert!(
            fx.review(NOTE, Some(other_id), ReviewState::Reviewed, None, &owner)
                .is_err()
        );
        assert!(
            fx.review(NOTE, Some(9999), ReviewState::Reviewed, None, &owner)
                .is_err()
        );
        assert!(
            fx.review("_system/SOUL.md", None, ReviewState::Reviewed, None, &owner)
                .is_err()
        );
        assert!(
            fx.review("../outside.md", None, ReviewState::Reviewed, None, &owner)
                .is_err()
        );
        assert!(
            fx.review(
                "Notebooks/missing.md",
                None,
                ReviewState::Reviewed,
                None,
                &owner
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn the_tools_refuse_an_agent_that_tries_to_accept_and_report_what_they_recorded() {
        let fx = Fx::new();
        fx.write_as(NOTE, "v1", &agent_call("s-1", "builder"));
        let review = NoteReviewTool::new(fx.vault.clone(), fx.db.clone());
        let view = NoteProvenanceTool::new(fx.vault.clone(), fx.db.clone());

        let mut forged = json!({ "path": NOTE, "state": "accepted" });
        forged
            .as_object_mut()
            .unwrap()
            .extend(agent_call("s-2", "checker").as_object().unwrap().clone());
        assert!(review.execute(forged).await.is_err());

        let mut flag = json!({ "path": NOTE, "state": "disputed", "reason": "no source" });
        flag.as_object_mut()
            .unwrap()
            .extend(agent_call("s-2", "checker").as_object().unwrap().clone());
        let out = review.execute(flag).await.unwrap();
        assert_eq!(out["review_state"], "disputed");
        assert_eq!(out["previous_state"], "unreviewed");
        assert_eq!(out["reviewed_by"], "checker");

        let seen = view.execute(json!({ "path": NOTE })).await.unwrap();
        assert_eq!(seen["exists"], true);
        assert_eq!(seen["current_version_recorded"], true);
        let v = &seen["versions"][0];
        assert_eq!(v["is_current"], true);
        assert_eq!(v["actor"], "builder");
        assert_eq!(v["review_state"], "disputed");
        assert_eq!(v["review_history"][0]["reason"], "no source");

        // Edited outside the tools: the view says so instead of crediting the earlier writer.
        std::fs::write(fx.dir.path().join(NOTE), "hand edit").unwrap();
        let seen = view.execute(json!({ "path": NOTE })).await.unwrap();
        assert_eq!(seen["current_version_recorded"], false);
        assert_eq!(seen["versions"][0]["is_current"], false);
    }

    #[tokio::test]
    async fn a_bad_state_is_named_in_the_error() {
        let fx = Fx::new();
        let review = NoteReviewTool::new(fx.vault.clone(), fx.db.clone());
        let err = review
            .execute(json!({ "path": NOTE, "state": "approved" }))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("approved"), "{err}");
    }

    #[test]
    fn a_read_of_the_provenance_history_changes_nothing() {
        let fx = Fx::new();
        let hash = fx.write_as_owner(NOTE, "v1");
        let before = fx.record_of(NOTE, &hash);
        let view = NoteProvenanceTool::new(fx.vault.clone(), fx.db.clone());
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(view.execute(json!({ "path": NOTE }))).unwrap();
        assert_eq!(fx.record_of(NOTE, &hash), before);
    }
}
