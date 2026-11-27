//! Who a vault write came from, and recording it.
//!
//! A tool learns its caller only from reserved `_hq_*` arguments that the runtime sets and a
//! caller cannot: the MCP gateway sets `_hq_connector` and `_hq_caller_session`, and a native
//! agent session sets the `_hq_native_*` markers after dropping any the model sent
//! (`hq_agent::session`). [`resolve`] turns those into a [`Writer`]. Anything it cannot establish
//! stays unknown. It never infers an author from the time, the file name or which session happened
//! to be active.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hq_db::Database;
use hq_db::provenance::{Attribution, SourceKind};
use serde_json::Value;
use tracing::debug;

use crate::harness_session::{caller_session, connector_actor, connector_label};

/// Set by a native agent session, never by the model: the session the call belongs to.
pub const NATIVE_SESSION_ARG: &str = "_hq_native_session";
/// Set by a native agent session: the agent's name.
pub const NATIVE_AGENT_ARG: &str = "_hq_native_agent";
/// Set by a native agent session: ids of the task leases it was handed, newest last. Whether any
/// is still live, and on which task, is read from the database when a write is recorded.
pub const NATIVE_LEASES_ARG: &str = "_hq_native_leases";
/// Everything a native session sets starts with this; the gateway drops all of it from callers.
pub const NATIVE_ARG_PREFIX: &str = "_hq_native_";

/// What is known about who is writing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Writer {
    pub actor_id: Option<String>,
    pub source_kind: SourceKind,
    pub session_id: Option<String>,
    pub task_id: Option<String>,
}

impl Writer {
    pub fn unknown() -> Self {
        Self {
            actor_id: None,
            source_kind: SourceKind::Unknown,
            session_id: None,
            task_id: None,
        }
    }

    /// The owner, writing through the web app or a command at a terminal.
    pub fn owner() -> Self {
        Self {
            actor_id: Some("owner".into()),
            source_kind: SourceKind::Human,
            session_id: None,
            task_id: None,
        }
    }

    /// A daemon job, named.
    pub fn system(job: &str) -> Self {
        Self {
            actor_id: Some(format!("system:{job}")),
            source_kind: SourceKind::System,
            session_id: None,
            task_id: None,
        }
    }

    fn attribution(&self) -> Attribution<'_> {
        Attribution {
            actor_id: self.actor_id.as_deref(),
            source_kind: Some(self.source_kind),
            session_id: self.session_id.as_deref(),
            task_id: self.task_id.as_deref(),
        }
    }
}

fn text_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// The task a spawned harness session works on: the lease it holds, else the mission it was
/// launched for. If the two name different tasks, nothing can be said.
fn harness_session_task(
    conn: &rusqlite::Connection,
    session_id: &str,
    mission: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let leased = hq_db::tasks::lease_for_session(conn, session_id)?.map(|l| l.task_id);
    // A mission id can match no task (it comes from the retired mission engine).
    let mission = match mission {
        Some(id) => hq_db::tasks::get_task(conn, id)?.map(|t| t.id),
        None => None,
    };
    Ok(match (leased, mission) {
        (Some(a), Some(b)) if a != b => None,
        (Some(a), _) => Some(a),
        (None, b) => b,
    })
}

fn from_harness_session(db: &Database, session_id: &str) -> Writer {
    let session = session_id.to_string();
    let found = db.with_conn(move |conn| {
        let row = hq_db::harness_sessions_registry::get(conn, &session)?;
        let mission = row.as_ref().and_then(|r| r.mission_id.clone());
        let task = harness_session_task(conn, &session, mission.as_deref())?;
        Ok((row, task))
    });
    let (row, task_id) = found.unwrap_or((None, None));
    let actor_id = row.map(|r| {
        if r.agent_name.trim().is_empty() {
            r.harness
        } else {
            r.agent_name
        }
    });
    Writer {
        actor_id,
        source_kind: SourceKind::Agent,
        session_id: Some(session_id.to_string()),
        task_id,
    }
}

/// The one task the live leases among `lease_ids` are on, or `None` if none is live or they are
/// on different tasks.
fn native_task(
    conn: &rusqlite::Connection,
    lease_ids: &[String],
) -> anyhow::Result<Option<String>> {
    let mut tasks = BTreeSet::new();
    for id in lease_ids {
        if let Some(lease) = hq_db::tasks::lease_by_id(conn, id)?
            && lease.ended_at.is_none()
        {
            tasks.insert(lease.task_id);
        }
    }
    Ok(if tasks.len() == 1 {
        tasks.into_iter().next()
    } else {
        None
    })
}

/// Arguments for a tool this tool calls on the caller's behalf: whatever `_hq_*` keys `inner`
/// carries are the caller's own words and are dropped, and the ones the runtime attested on the
/// call to this tool (`outer`) are passed on, so a tool nested one level down still sees who
/// the real caller is.
pub fn pass_on_attestation(outer: &Value, inner: &mut Value) {
    let Some(map) = inner.as_object_mut() else {
        return;
    };
    map.retain(|key, _| !key.starts_with("_hq_"));
    let Some(attested) = outer.as_object() else {
        return;
    };
    for (key, value) in attested {
        let runtime_set = key.starts_with(NATIVE_ARG_PREFIX)
            || key == crate::harness_session::CONNECTOR_ARG
            || key == crate::harness_session::CALLER_SESSION_ARG;
        if runtime_set {
            map.insert(key.clone(), value.clone());
        }
    }
}

/// The one task the live leases among `lease_ids` are on, or `None` if none is live or they are
/// on different tasks. For a session that holds leases and has to say what it is working on.
pub fn task_for_leases(db: &Database, lease_ids: &[String]) -> Option<String> {
    if lease_ids.is_empty() {
        return None;
    }
    let ids = lease_ids.to_vec();
    db.with_conn(move |conn| native_task(conn, &ids))
        .ok()
        .flatten()
}

fn lease_ids(args: &Value) -> Vec<String> {
    args.get(NATIVE_LEASES_ARG)
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The one task the calling native session holds a live lease on, from the leases the runtime
/// attested on this call. The model cannot name it: `_hq_*` keys it sends are dropped.
pub fn leased_task(args: &Value, db: &Database) -> Option<String> {
    task_for_leases(db, &lease_ids(args))
}

/// Work out who made the tool call whose arguments are `args`.
pub fn resolve(args: &Value, db: &Database) -> Writer {
    if let Some(label) = connector_label(args) {
        return Writer {
            actor_id: Some(connector_actor(label)),
            source_kind: SourceKind::Agent,
            session_id: None,
            task_id: None,
        };
    }
    if let Some(session) = caller_session(args) {
        return from_harness_session(db, session);
    }
    if let Some(session) = text_arg(args, NATIVE_SESSION_ARG) {
        let leases = lease_ids(args);
        let task_id = if leases.is_empty() {
            None
        } else {
            db.with_conn(move |conn| native_task(conn, &leases))
                .ok()
                .flatten()
        };
        return Writer {
            actor_id: text_arg(args, NATIVE_AGENT_ARG).map(str::to_string),
            source_kind: SourceKind::Agent,
            session_id: Some(session.to_string()),
            task_id,
        };
    }
    Writer::unknown()
}

/// The hash of each of some notes at one moment.
#[derive(Debug, Default, Clone)]
pub struct Fingerprints(HashMap<String, String>);

/// Record, off the async thread, that the tool call whose arguments are `args` wrote the note.
/// Does nothing when there is no database to record in.
pub async fn record_tool_write(
    db: Option<&Arc<Database>>,
    vault_path: &Path,
    rel: &str,
    args: &Value,
) {
    let Some(db) = db else { return };
    let recorder = Recorder::new(db.clone(), vault_path.to_path_buf());
    let (rel, args) = (rel.to_string(), args.clone());
    let _ = tokio::task::spawn_blocking(move || recorder.after_write(&rel, &args)).await;
}

/// Keep, off the async thread, the text the tool call whose arguments are `args` is about to
/// replace. Call it before the write. Does nothing when there is no database.
pub async fn keep_before_tool_write(
    db: Option<&Arc<Database>>,
    vault_path: &Path,
    rel: &str,
    args: &Value,
) {
    let Some(db) = db else { return };
    let recorder = Recorder::new(db.clone(), vault_path.to_path_buf());
    let (rel, args) = (rel.to_string(), args.clone());
    let _ = tokio::task::spawn_blocking(move || recorder.before_write(&rel, &args)).await;
}

/// Run `write` under the per-note lock, keeping any earlier text beforehand, discarding the
/// kept row if `write` fails, and recording provenance on success.
///
/// Must be called from synchronous code (for example inside `spawn_blocking`) so the
/// [`hq_vault::note_lock::NoteGuard`] is never held across an `.await`.
pub fn locked_tool_write<T, F>(
    db: Option<&Arc<Database>>,
    vault_path: &Path,
    rel: &str,
    args: &Value,
    write: F,
) -> anyhow::Result<T>
where
    F: FnOnce() -> anyhow::Result<T>,
{
    let _held = hq_vault::note_lock::lock_note(rel);
    let recorder = db.map(|db| Recorder::new(db.clone(), vault_path.to_path_buf()));
    if let Some(ref rec) = recorder {
        rec.before_write(rel, args);
    }
    match write() {
        Ok(value) => {
            if let Some(ref rec) = recorder {
                rec.after_write(rel, args);
            }
            Ok(value)
        }
        Err(e) => {
            if let Some(ref rec) = recorder {
                rec.discard_kept(rel, args);
            }
            Err(e)
        }
    }
}

/// Records writes. Best effort: a failure here is logged and never fails the write it describes.
#[derive(Clone)]
pub struct Recorder {
    db: Arc<Database>,
    vault_path: PathBuf,
}

impl Recorder {
    pub fn new(db: Arc<Database>, vault_path: PathBuf) -> Self {
        Self { db, vault_path }
    }

    /// A note was written by the tool call whose arguments are `args`.
    pub fn after_write(&self, rel: &str, args: &Value) {
        let writer = resolve(args, &self.db);
        self.record(rel, &writer);
    }

    /// A note was written by `writer`, which the caller already knows (the web app, a command).
    pub fn record(&self, rel: &str, writer: &Writer) {
        // Hash the file as it is now: that is the version this record describes.
        let raw = match std::fs::read_to_string(self.vault_path.join(rel)) {
            Ok(raw) => raw,
            Err(e) => {
                debug!(path = rel, error = %e, "provenance: cannot read the note, not recording");
                return;
            }
        };
        let hash = hq_db::search::content_hash(&raw);
        let result = self.db.with_conn(|conn| {
            hq_db::provenance::record(conn, rel, &hash, &writer.attribution())?;
            hq_db::provenance::clear_deleted(conn, rel)?;
            if writer.source_kind == SourceKind::Agent {
                hq_db::note_keep::tie_or_drop(conn, rel, &hash)?;
            } else {
                // The owner (or anyone who is not HQ) wrote it: what HQ's change replaced may be
                // older than this, so it is never offered again.
                hq_db::note_keep::remove(conn, rel)?;
            }
            if let Some(task) = writer.task_id.as_deref() {
                // Idempotent: the same link added again returns the existing one. A task already
                // at its link limit just does not get another.
                let by = writer.actor_id.as_deref().unwrap_or("provenance");
                if let Err(e) = hq_db::tasks::add_task_link(
                    conn,
                    task,
                    hq_db::tasks::LINK_VAULT_NOTE,
                    rel,
                    "written during this task",
                    Some(hq_db::tasks::DIRECTION_PRODUCED),
                    by,
                ) {
                    debug!(path = rel, error = %e, "provenance: could not link the note to its task");
                }
            }
            Ok(())
        });
        if let Err(e) = result {
            tracing::warn!(path = rel, error = %e, "provenance: could not record the write");
        }
    }

    /// The hash of each note as it is now, taken before something that may rewrite them.
    pub fn fingerprint(&self, rels: &[String]) -> Fingerprints {
        Fingerprints(
            rels.iter()
                .filter_map(|rel| self.hash_of(rel).map(|hash| (rel.clone(), hash)))
                .collect(),
        )
    }

    fn hash_of(&self, rel: &str) -> Option<String> {
        std::fs::read_to_string(self.vault_path.join(rel))
            .ok()
            .map(|raw| hq_db::search::content_hash(&raw))
    }

    /// A note moved from `from` to `to`. Its records follow it. Moving also rewrites the links in
    /// the notes that referred to it (`referrers`); each of those whose content really changed
    /// is a new version this call made, and the rest are left alone. `before` is the
    /// fingerprint of `from` and `referrers` taken before the move.
    pub fn after_move(
        &self,
        from: &str,
        to: &str,
        referrers: &[String],
        before: &Fingerprints,
        args: &Value,
    ) {
        let writer = resolve(args, &self.db);
        let (from_owned, to_owned) = (from.to_string(), to.to_string());
        if let Err(e) = self
            .db
            .with_conn(move |conn| hq_db::provenance::move_path(conn, &from_owned, &to_owned))
        {
            tracing::warn!(from, to, error = %e, "provenance: could not carry records to the new path");
        }
        // The moved note is the same text unless it linked to itself.
        if self.hash_of(to) != before.0.get(from).cloned() {
            self.record(to, &writer);
        }
        for rel in referrers {
            if rel != to && self.hash_of(rel) != before.0.get(rel).cloned() {
                self.record(rel, &writer);
            }
        }
    }

    /// A folder moved from `from_root` to `to_root`. Moving rewrote links in the notes that
    /// pointed into it and in moved notes that reached out of it (`rewritten`, by the paths they
    /// have now). Each whose content really changed is a new version this call made; the rest
    /// are left alone. `before` is the fingerprint, taken before the move, of those notes by
    /// their old paths.
    pub fn after_folder_move(
        &self,
        from_root: &str,
        to_root: &str,
        rewritten: &[String],
        before: &Fingerprints,
        args: &Value,
    ) {
        let writer = resolve(args, &self.db);
        let prefix = format!("{to_root}/");
        for rel in rewritten {
            let old = match rel.strip_prefix(&prefix) {
                Some(rest) => format!("{from_root}/{rest}"),
                None => rel.clone(),
            };
            if self.hash_of(rel) != before.0.get(&old).cloned() {
                self.record(rel, &writer);
            }
        }
    }

    /// A note was deleted: keep who wrote it for a retention period, and its earlier text goes.
    pub fn after_delete(&self, rel: &str) {
        let rel_owned = rel.to_string();
        let marked = self.db.with_conn(move |conn| {
            hq_db::note_keep::remove(conn, &rel_owned)?;
            hq_db::provenance::mark_deleted(conn, &rel_owned)
        });
        if let Err(e) = marked {
            tracing::warn!(path = rel, error = %e, "provenance: could not mark the note deleted");
        }
    }

    /// Before the tool call whose arguments are `args` changes the note: if HQ is the caller, keep
    /// the text it is about to replace so the owner can put it back. Text HQ itself wrote (and the
    /// owner has not kept) is not kept over the owner's, so the kept text stays the owner's
    /// version from before HQ's first change. The path comes from the model, so it is checked
    /// here, at the one place that reads it. Best effort, like the rest of this.
    pub fn before_write(&self, rel: &str, args: &Value) {
        if resolve(args, &self.db).source_kind != SourceKind::Agent {
            return;
        }
        let raw = match self.read_for_keeping(rel) {
            Kept::Text(raw) => raw,
            Kept::TooBig => {
                let rel = rel.to_string();
                let _ = self
                    .db
                    .with_conn(move |c| hq_db::note_keep::remove(c, &rel));
                return;
            }
            Kept::Nothing => return,
        };
        let hash = hq_db::search::content_hash(&raw);
        let kept = self.db.with_conn(|conn| {
            // Text the owner kept counts as theirs again.
            let ours = hq_db::provenance::current(conn, rel, &hash)?.is_some_and(|p| {
                p.source_kind == SourceKind::Agent
                    && p.review_state != hq_db::provenance::ReviewState::Accepted
            });
            if ours {
                hq_db::note_keep::arm(conn, rel)?;
                return Ok(());
            }
            hq_db::note_keep::keep(conn, rel, &raw)?;
            Ok(())
        });
        if let Err(e) = kept {
            tracing::warn!(path = rel, error = %e, "provenance: could not keep the earlier text");
        }
    }

    /// The write whose earlier text `before_write` may have kept failed: drop the unarmed kept
    /// row so a later non-keeping write cannot tie it to an older text.
    pub fn discard_kept(&self, rel: &str, args: &Value) {
        if resolve(args, &self.db).source_kind != SourceKind::Agent {
            return;
        }
        let rel_owned = rel.to_string();
        if let Err(e) = self
            .db
            .with_conn(move |conn| hq_db::note_keep::remove(conn, &rel_owned))
        {
            tracing::warn!(
                path = rel,
                error = %e,
                "provenance: could not discard kept text after failed write"
            );
        }
    }

    /// The note's text if it is a regular `.md` file the note tools may write, reached through no
    /// symlink, read through a bound so a huge or endless file costs nothing.
    fn read_for_keeping(&self, rel: &str) -> Kept {
        use std::io::Read;
        if !rel.ends_with(".md")
            || crate::write_policy::validate_write_path(&self.vault_path, rel).is_err()
        {
            return Kept::Nothing;
        }
        let Ok(parsed) = hq_vault::rel_path::RelPath::parse(rel) else {
            return Kept::Nothing;
        };
        let mut at = self.vault_path.clone();
        for part in parsed.normalized().split('/') {
            at.push(part);
            match std::fs::symlink_metadata(&at) {
                Ok(m) if !m.file_type().is_symlink() => {}
                _ => return Kept::Nothing,
            }
        }
        // A FIFO or device would block or never end on open, so only a regular file is opened,
        // and without waiting in case it was swapped for one since this check.
        if !std::fs::symlink_metadata(&at).is_ok_and(|m| m.is_file()) {
            return Kept::Nothing;
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK);
        }
        let Ok(file) = options.open(&at) else {
            return Kept::Nothing;
        };
        // The file may have been swapped since the check; the handle says what it really is.
        if !file.metadata().is_ok_and(|m| m.is_file()) {
            return Kept::Nothing;
        }
        let limit = hq_db::note_keep::MAX_PREV_BYTES as u64 + 1;
        let mut bytes = Vec::new();
        if file.take(limit).read_to_end(&mut bytes).is_err() {
            return Kept::Nothing;
        }
        if bytes.len() > hq_db::note_keep::MAX_PREV_BYTES {
            return Kept::TooBig;
        }
        String::from_utf8(bytes).map_or(Kept::Nothing, Kept::Text)
    }
}

/// What reading a note to keep it found.
enum Kept {
    Text(String),
    /// Over the size cap: nothing is kept, and a text kept earlier no longer describes the note.
    TooBig,
    Nothing,
}

#[cfg(test)]
mod tests;

/// The seam office tools write through: the per-note lock, the kept earlier text and the record.
pub struct DbWriteSeam {
    pub db: Arc<Database>,
    pub vault_path: PathBuf,
}

impl hq_tool_api::office::NoteWriteSeam for DbWriteSeam {
    fn write(
        &self,
        rel: &str,
        args: &Value,
        write: &mut dyn FnMut() -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        locked_tool_write(Some(&self.db), &self.vault_path, rel, args, write)
    }

    fn created(&self, rel: &str, args: &Value) {
        Recorder::new(self.db.clone(), self.vault_path.clone()).after_write(rel, args);
    }
}

/// A seam over `db` for the office tools, or none when there is no database.
pub fn office_seam(
    db: Option<&Arc<Database>>,
    vault_path: &Path,
) -> Option<hq_tool_api::office::SharedSeam> {
    db.map(|db| {
        Arc::new(DbWriteSeam {
            db: db.clone(),
            vault_path: vault_path.to_path_buf(),
        }) as hq_tool_api::office::SharedSeam
    })
}
