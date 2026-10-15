//! What is going on in a task or an initiative, as one generated view.
//!
//! Nothing here is stored. Each call reads the task system, the session registry and the note
//! provenance records as they are now and puts them side by side, with the time of the freshest
//! thing each source knows and a list of the places where the links between them are missing.
//!
//! The view never decides anything. A task is complete when its status says so, and a note
//! edit, an exited session or a quiet lease says nothing about either. Who owns the work is
//! who holds its lease or is assigned, not whoever wrote last.

use anyhow::{Result, anyhow};
use hq_db::Database;
use hq_db::harness_sessions_registry::{self as sessions, HarnessSessionRow};
use hq_db::provenance::{self, Provenance, ReviewState};
use hq_db::tasks as t;
use hq_vault::VaultClient;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Notes listed for one task unless asked for fewer or more.
pub const DEFAULT_NOTES: usize = 20;
pub const MAX_NOTES: usize = 100;
/// Ended leases shown after the live ones.
const RECENT_LEASES: usize = 5;
/// Tasks an initiative view looks into; the rest are counted, not opened.
const INITIATIVE_TASKS: usize = 50;
/// Notes looked at per task inside an initiative view.
const INITIATIVE_NOTES_PER_TASK: usize = 10;
/// Gaps listed; the total is always reported.
const MAX_GAPS: usize = 50;
/// A harness session counts as running while the registry says so.
const SESSION_RUNNING: &str = "running";
/// Note types that mark a note as a decision or a finding. Only a note that declares one of these
/// counts; nothing is guessed from its text.
const DECISION_TYPES: &[&str] = &["decision", "finding"];
const PRIVATE_FOLDERS: &[&str] = &["_system", "_threads", "_data"];

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub notes_limit: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            notes_limit: DEFAULT_NOTES,
        }
    }
}

impl Options {
    pub fn with_notes_limit(limit: Option<u64>) -> Self {
        Self {
            notes_limit: limit.map_or(DEFAULT_NOTES, |n| (n as usize).clamp(1, MAX_NOTES)),
        }
    }
}

/// One note linked to a task, as the view reports it.
struct NoteView {
    path: String,
    /// The version on disk now, when it has a record.
    record: Option<Provenance>,
    /// How the note relates to authorship: `recorded`, `unattributed`, `changed_outside_tools`
    /// (a record exists for an earlier version only), `no_record` or `missing`.
    attribution: &'static str,
    /// `produced`, `origin` or `related` when the task links the note.
    link: Option<String>,
    /// The `type:` the note's frontmatter declares, if any.
    declared_type: Option<String>,
}

impl NoteView {
    fn state(&self) -> Option<ReviewState> {
        self.record.as_ref().map(|r| r.review_state)
    }

    fn json(&self) -> Value {
        let mut out = json!({
            "path": self.path,
            "attribution": self.attribution,
            "link": self.link,
            "type": self.declared_type,
        });
        if let Some(r) = &self.record {
            out["version_id"] = json!(r.id);
            out["written_at"] = json!(r.written_at);
            out["actor"] = json!(r.actor_id);
            out["source_kind"] = json!(r.source_kind.as_str());
            out["session"] = json!(r.session_id);
            out["task"] = json!(r.task_id);
            out["recorded_late"] = json!(r.recorded_late);
            out["review_state"] = json!(r.review_state.as_str());
            out["reviewed_by"] = json!(r.reviewed_by);
            out["reviewed_at"] = json!(r.reviewed_at);
            out["review_reason"] = json!(r.review_reason);
        }
        out
    }
}

fn is_private(path: &str) -> bool {
    std::path::Path::new(path)
        .components()
        .find_map(|c| match c {
            std::path::Component::Normal(p) => Some(p.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .is_some_and(|first| PRIVATE_FOLDERS.contains(&first.as_str()))
}

fn gap(kind: &str, detail: impl Into<String>, subject: Option<&str>) -> Value {
    json!({ "kind": kind, "detail": detail.into(), "subject": subject })
}

/// Read one note as it is now and match it to its provenance.
fn note_view(
    conn: &rusqlite::Connection,
    vault: &VaultClient,
    path: &str,
    link: Option<String>,
) -> Result<NoteView> {
    let full = vault.vault_path().join(path);
    let raw = std::fs::read_to_string(&full).ok();
    let Some(raw) = raw else {
        return Ok(NoteView {
            path: path.to_string(),
            record: None,
            attribution: "missing",
            link,
            declared_type: None,
        });
    };
    let hash = hq_db::search::content_hash(&raw);
    let record = provenance::current(conn, path, &hash)?;
    let attribution = match &record {
        Some(r) if r.is_unattributed() => "unattributed",
        Some(_) => "recorded",
        None if provenance::history(conn, path, 1)?.is_empty() => "no_record",
        None => "changed_outside_tools",
    };
    let declared_type = vault.read_note(path).ok().and_then(|n| {
        n.frontmatter
            .get("type")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
    });
    Ok(NoteView {
        path: path.to_string(),
        record,
        attribution,
        link,
        declared_type,
    })
}

/// The notes a task is tied to: the latest version each path has under this task, then the notes
/// it links that those leave out, up to `limit`.
fn task_notes(
    conn: &rusqlite::Connection,
    vault: &VaultClient,
    task: &t::Task,
    limit: usize,
) -> Result<(Vec<NoteView>, usize)> {
    let links: BTreeMap<String, String> = t::list_task_links(conn, &task.id)?
        .into_iter()
        .filter(|l| l.kind == t::LINK_VAULT_NOTE)
        .map(|l| (l.reference, l.direction))
        .collect();
    let mut paths: Vec<String> = Vec::new();
    let mut seen = BTreeSet::new();
    for rec in provenance::for_task(conn, &task.id, MAX_NOTES)? {
        if seen.insert(rec.path.clone()) {
            paths.push(rec.path);
        }
    }
    for path in links.keys() {
        if seen.insert(path.clone()) {
            paths.push(path.clone());
        }
    }
    paths.retain(|p| !is_private(p));
    let total = paths.len();
    paths.truncate(limit);
    let views = paths
        .iter()
        .map(|p| note_view(conn, vault, p, links.get(p).cloned()))
        .collect::<Result<Vec<_>>>()?;
    Ok((views, total))
}

fn session_json(row: &HarnessSessionRow, via: &BTreeSet<&'static str>) -> Value {
    json!({
        "id": row.id,
        "harness": row.harness,
        "agent": row.agent_name,
        "purpose": row.purpose,
        "status": row.status,
        "running": row.status == SESSION_RUNNING,
        "last_agent_status": row.last_agent_status,
        "last_seen_at": row.last_seen_at,
        "created_at": row.created_at,
        "goal": row.goal,
        "via": via,
    })
}

fn lease_json(l: &t::WorkSession) -> Value {
    json!({
        "lease_id": l.id,
        "actor": l.actor,
        "harness": l.harness,
        "branch": l.branch,
        "harness_session": l.harness_session_id,
        "started_at": l.started_at,
        "last_heartbeat_at": l.last_heartbeat_at,
        "ended_at": l.ended_at,
        "end_reason": l.end_reason,
        "live": l.ended_at.is_none(),
    })
}

#[derive(Default)]
struct Contributor {
    source_kinds: BTreeSet<&'static str>,
    notes: usize,
    leases: usize,
    last_active: Option<String>,
}

impl Contributor {
    fn touch(&mut self, at: &str) {
        if self.last_active.as_deref().is_none_or(|prev| at > prev) {
            self.last_active = Some(at.to_string());
        }
    }
}

fn contributors_json(map: &BTreeMap<String, Contributor>) -> Value {
    let mut rows: Vec<Value> = map
        .iter()
        .map(|(actor, c)| {
            json!({
                "actor": actor,
                "source_kinds": c.source_kinds,
                "notes": c.notes,
                "leases": c.leases,
                "last_active": c.last_active,
            })
        })
        .collect();
    rows.sort_by(|a, b| b["last_active"].as_str().cmp(&a["last_active"].as_str()));
    json!(rows)
}

fn newest<'a>(times: impl Iterator<Item = &'a str>) -> Option<String> {
    times.max().map(str::to_string)
}

/// Everything the view knows about one task, in the order the sources were read.
struct TaskView {
    json: Value,
    notes: Vec<NoteView>,
    contributors: BTreeMap<String, Contributor>,
    gaps: Vec<Value>,
}

fn build_task(
    conn: &rusqlite::Connection,
    vault: &VaultClient,
    task: &t::Task,
    notes_limit: usize,
) -> Result<TaskView> {
    let initiative = t::get_initiative(conn, &task.initiative_id)?;
    let space = match &initiative {
        Some(i) => t::get_space(conn, &i.space_id)?,
        None => None,
    };
    let checkpoint = t::latest_checkpoint(conn, &task.id)?;
    let leases = t::list_work_sessions(conn, &task.id, 20)?;
    let live_leases: Vec<&t::WorkSession> =
        leases.iter().filter(|l| l.ended_at.is_none()).collect();
    let ended_leases: Vec<&t::WorkSession> = leases
        .iter()
        .filter(|l| l.ended_at.is_some())
        .take(RECENT_LEASES)
        .collect();

    // Harness sessions tied to the task by a lease, by the mission it was launched for, or by a
    // link someone recorded.
    let mut by_id: BTreeMap<String, (Option<HarnessSessionRow>, BTreeSet<&'static str>)> =
        BTreeMap::new();
    for row in sessions::list_for_mission(conn, &task.id)? {
        let entry = by_id.entry(row.id.clone()).or_default();
        entry.1.insert("mission");
        entry.0 = Some(row);
    }
    let mut wanted: Vec<(String, &'static str)> = leases
        .iter()
        .filter_map(|l| l.harness_session_id.clone())
        .map(|id| (id, "lease"))
        .collect();
    wanted.extend(
        t::list_task_links(conn, &task.id)?
            .into_iter()
            .filter(|l| l.kind == t::LINK_SESSION)
            .map(|l| (l.reference, "link")),
    );
    for (id, via) in wanted {
        let entry = by_id.entry(id.clone()).or_default();
        entry.1.insert(via);
        if entry.0.is_none() {
            entry.0 = sessions::get(conn, &id)?;
        }
    }

    let (notes, notes_total) = task_notes(conn, vault, task, notes_limit)?;

    let mut gaps: Vec<Value> = Vec::new();
    let mut contributors: BTreeMap<String, Contributor> = BTreeMap::new();

    let mut session_rows = Vec::new();
    let mut running_sessions = 0;
    for (id, (row, via)) in &by_id {
        match row {
            Some(row) => {
                if row.status == SESSION_RUNNING {
                    running_sessions += 1;
                }
                session_rows.push(session_json(row, via));
            }
            None => {
                session_rows.push(json!({ "id": id, "registered": false, "via": via }));
                gaps.push(gap(
                    "session_not_registered",
                    "linked to this task but absent from the session registry",
                    Some(id),
                ));
            }
        }
    }

    for l in &leases {
        let c = contributors.entry(l.actor.clone()).or_default();
        c.leases += 1;
        c.touch(&l.last_heartbeat_at);
    }
    for n in &notes {
        match n.attribution {
            "missing" => gaps.push(gap(
                "note_missing",
                "the task links this note but the file is gone",
                Some(&n.path),
            )),
            "unattributed" | "no_record" => gaps.push(gap(
                "note_unattributed",
                "the version on disk has no known author",
                Some(&n.path),
            )),
            "changed_outside_tools" => gaps.push(gap(
                "note_changed_outside_tools",
                "the file was changed after its last recorded write, so its author is not known",
                Some(&n.path),
            )),
            _ => {}
        }
        let Some(r) = &n.record else { continue };
        if r.is_unattributed() {
            continue;
        }
        let c = contributors.entry(r.actor_id.clone()).or_default();
        c.notes += 1;
        c.source_kinds.insert(r.source_kind.as_str());
        c.touch(&r.written_at);
        if r.session_id.is_none() && r.source_kind == hq_db::provenance::SourceKind::Agent {
            gaps.push(gap(
                "note_without_session",
                "written by an agent but no session was recorded",
                Some(&n.path),
            ));
        }
        if r.task_id.as_deref().is_some_and(|id| id != task.id) {
            gaps.push(gap(
                "note_from_another_task",
                "the version on disk was written under a different task",
                Some(&n.path),
            ));
        }
    }
    if task.status == t::STATUS_IN_PROGRESS && live_leases.is_empty() && running_sessions == 0 {
        gaps.push(gap(
            "in_progress_without_a_holder",
            "the status is in_progress but no live lease or running session is attached",
            Some(&task.display_id),
        ));
    }
    if notes_total > notes.len() {
        gaps.push(gap(
            "notes_not_listed",
            format!(
                "{} more linked notes beyond the limit",
                notes_total - notes.len()
            ),
            None,
        ));
    }

    // Split what the notes declare themselves to be, by what has been decided about them.
    let mut decisions: BTreeMap<&'static str, Vec<Value>> = BTreeMap::new();
    for n in notes.iter().filter(|n| {
        n.declared_type
            .as_deref()
            .is_some_and(|t| DECISION_TYPES.contains(&t))
    }) {
        let state = n.state().map_or("unreviewed", ReviewState::as_str);
        decisions.entry(state).or_default().push(n.json());
    }
    let mut review_counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for n in &notes {
        if let Some(s) = n.state() {
            *review_counts.entry(s.as_str()).or_default() += 1;
        }
    }

    let mut open_questions = Vec::new();
    if let Some(cp) = &checkpoint
        && !cp.open_questions.trim().is_empty()
    {
        open_questions.push(json!({
            "text": cp.open_questions,
            "recorded_by": cp.actor,
            "at": cp.created_at,
            "source": "checkpoint",
        }));
    }
    let blocker = (task.status == t::STATUS_BLOCKED).then(|| {
        json!({
            "reason": task.blocked_reason,
            "waiting_on": task.waiting_on,
            "since": task.blocked_since,
        })
    });

    let freshness = json!({
        "task": task.updated_at,
        "checkpoint": checkpoint.as_ref().map(|c| c.created_at.clone()),
        "leases": newest(leases.iter().map(|l| l.last_heartbeat_at.as_str())),
        "sessions": newest(by_id.values().filter_map(|(r, _)| r.as_ref()?.last_seen_at.as_deref())),
        "notes": newest(notes.iter().filter_map(|n| n.record.as_ref()).map(|r| r.written_at.as_str())),
    });
    let missing_sources: Vec<&str> = freshness
        .as_object()
        .map(|o| {
            o.iter()
                .filter(|(_, v)| v.is_null())
                .map(|(k, _)| k.as_str())
                .collect()
        })
        .unwrap_or_default();

    let gaps_total = gaps.len();
    gaps.truncate(MAX_GAPS);

    let json = json!({
        "task": {
            "id": task.id,
            "display_id": task.display_id,
            "title": task.title,
            "status": task.status,
            "priority": task.priority,
            "assignees": task.assignees,
            "tags": task.tags,
            "parent": task.parent_task_id,
            "subtasks": { "total": task.subtask_count, "done": task.subtask_done },
            "depends_on": task.depends_on,
            "blocked_by": task.blocked_by,
            "long_horizon": task.long_horizon,
            "work_started_at": task.work_started_at,
            "first_ready_for_review_at": task.first_ready_for_review_at,
            "completed_at": task.completed_at,
            "updated_at": task.updated_at,
            "archived_at": task.archived_at,
            "initiative": initiative.as_ref().map(|i| json!({ "id": i.id, "name": i.name })),
            "space": space.as_ref().map(|s| json!({ "id": s.id, "slug": s.slug, "name": s.name })),
        },
        "awaiting_review": task.status == t::STATUS_READY_FOR_REVIEW,
        "blocker": blocker,
        "checkpoint": checkpoint.as_ref().map(|c| json!({
            "summary": c.summary,
            "next_step": c.next_step,
            "open_questions": c.open_questions,
            "files": c.files,
            "actor": c.actor,
            "at": c.created_at,
        })),
        "holders": live_leases.iter().map(|l| lease_json(l)).collect::<Vec<_>>(),
        "recent_leases": ended_leases.iter().map(|l| lease_json(l)).collect::<Vec<_>>(),
        "sessions": session_rows,
        "notes": notes.iter().map(NoteView::json).collect::<Vec<_>>(),
        "notes_total": notes_total,
        "review_counts": review_counts,
        "decisions_and_findings": decisions,
        "open_questions": open_questions,
        "contributors": contributors_json(&contributors),
        "gaps": gaps,
        "gaps_total": gaps_total,
        "freshness": freshness,
        "sources_with_nothing": missing_sources,
    });
    Ok(TaskView {
        json,
        notes,
        contributors,
        gaps,
    })
}

fn envelope(scope: &str, body: Value) -> Value {
    let mut out = body;
    out["scope"] = json!(scope);
    out["generated_at"] = json!(chrono::Utc::now().to_rfc3339());
    out["basis"] = json!(
        "Generated now from the task system, the session registry and note provenance. \
         Status, ownership and completion come from the task system only; a note edit or a \
         session exiting does not change any of them."
    );
    out
}

/// The view of one task.
pub fn task_activity(
    db: &Database,
    vault: &VaultClient,
    task_ref: &str,
    opts: Options,
) -> Result<Value> {
    db.with_conn(|conn| {
        let task = t::get_task(conn, task_ref)?.ok_or_else(|| anyhow!("no task '{task_ref}'"))?;
        let view = build_task(conn, vault, &task, opts.notes_limit)?;
        Ok(envelope("task", view.json))
    })
}

/// The view of an initiative: where each of its tasks stands, who is working, what is waiting on
/// review, and the gaps, with the newest notes across them.
pub fn initiative_activity(
    db: &Database,
    vault: &VaultClient,
    initiative_ref: &str,
    opts: Options,
) -> Result<Value> {
    db.with_conn(|conn| {
        let initiative = t::get_initiative(conn, initiative_ref)?
            .ok_or_else(|| anyhow!("no initiative '{initiative_ref}'"))?;
        let space = t::get_space(conn, &initiative.space_id)?;
        let tasks = t::list_tasks(
            conn,
            &t::TaskFilter {
                initiative_id: Some(initiative.id.clone()),
                limit: Some(INITIATIVE_TASKS),
                ..Default::default()
            },
        )?;
        let per_task_notes = opts.notes_limit.min(INITIATIVE_NOTES_PER_TASK);

        let mut status_counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut rows = Vec::new();
        let mut awaiting_review = Vec::new();
        let mut blocked = Vec::new();
        let mut contributors: BTreeMap<String, Contributor> = BTreeMap::new();
        let mut all_notes: Vec<(String, Value)> = Vec::new();
        let mut review_totals: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut gaps_total = 0;
        let mut gap_kinds: BTreeMap<String, usize> = BTreeMap::new();
        let mut newest_task_update: Option<String> = None;

        for task in &tasks {
            *status_counts.entry(task.status.clone()).or_default() += 1;
            let view = build_task(conn, vault, task, per_task_notes)?;
            let holders = view.json["holders"].as_array().cloned().unwrap_or_default();
            if task.status == t::STATUS_READY_FOR_REVIEW {
                awaiting_review.push(
                    json!({ "id": task.id, "display_id": task.display_id, "title": task.title }),
                );
            }
            if task.status == t::STATUS_BLOCKED {
                blocked.push(json!({
                    "id": task.id,
                    "display_id": task.display_id,
                    "title": task.title,
                    "reason": task.blocked_reason,
                    "waiting_on": task.waiting_on,
                }));
            }
            for g in &view.gaps {
                *gap_kinds
                    .entry(g["kind"].as_str().unwrap_or("other").to_string())
                    .or_default() += 1;
            }
            gaps_total += view.json["gaps_total"].as_u64().unwrap_or(0) as usize;
            for n in &view.notes {
                if let Some(s) = n.state() {
                    *review_totals.entry(s.as_str()).or_default() += 1;
                }
                if let Some(r) = &n.record {
                    let mut note = n.json();
                    note["task_display_id"] = json!(task.display_id);
                    all_notes.push((r.written_at.clone(), note));
                }
            }
            for (actor, c) in view.contributors {
                let into = contributors.entry(actor).or_default();
                into.notes += c.notes;
                into.leases += c.leases;
                into.source_kinds.extend(c.source_kinds);
                if let Some(at) = c.last_active {
                    into.touch(&at);
                }
            }
            if newest_task_update
                .as_deref()
                .is_none_or(|prev| task.updated_at.as_str() > prev)
            {
                newest_task_update = Some(task.updated_at.clone());
            }
            rows.push(json!({
                "id": task.id,
                "display_id": task.display_id,
                "title": task.title,
                "status": task.status,
                "priority": task.priority,
                "assignees": task.assignees,
                "blocked_by": task.blocked_by,
                "holders": holders.iter().map(|h| h["actor"].clone()).collect::<Vec<_>>(),
                "last_checkpoint_at": view.json["checkpoint"]["at"],
                "notes": view.json["notes_total"],
                "review_counts": view.json["review_counts"],
                "gaps": view.json["gaps_total"],
            }));
        }
        all_notes.sort_by(|a, b| b.0.cmp(&a.0));
        let recent_notes: Vec<Value> = all_notes
            .into_iter()
            .take(opts.notes_limit)
            .map(|(_, n)| n)
            .collect();

        Ok(envelope(
            "initiative",
            json!({
                "initiative": {
                    "id": initiative.id,
                    "name": initiative.name,
                    "prefix": initiative.id_prefix,
                    "space": space.map(|s| json!({ "id": s.id, "slug": s.slug, "name": s.name })),
                },
                "tasks_shown": tasks.len(),
                "status_counts": status_counts,
                "tasks": rows,
                "awaiting_review": awaiting_review,
                "blocked": blocked,
                "review_counts": review_totals,
                "recent_notes": recent_notes,
                "contributors": contributors_json(&contributors),
                "gaps_total": gaps_total,
                "gap_kinds": gap_kinds,
                "freshness": { "tasks": newest_task_update },
            }),
        ))
    })
}

/// `task_activity`: the generated view of a task or an initiative.
pub struct TaskActivityTool {
    pub(super) vault: std::sync::Arc<VaultClient>,
    pub(super) db: std::sync::Arc<Database>,
}

#[async_trait::async_trait]
impl crate::registry::HqTool for TaskActivityTool {
    fn name(&self) -> &str {
        "task_activity"
    }

    fn description(&self) -> &str {
        "What is going on in a task (`task_id`) or an initiative (`initiative_id`), generated now: \
         status, priority, assignees, blockers and the latest checkpoint from the task system; \
         linked harness sessions and live leases; recent notes with who wrote them, under which \
         task and session, and whether anyone has reviewed them; decisions and findings split by \
         review state; open questions only where someone recorded them; a freshness time per \
         source; and explicit gaps where a link is missing. Status, ownership and completion are \
         never inferred from note edits or sessions ending. Read-only."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": { "type": "string", "description": "Task id or display id (for example HQ-12)." },
                "initiative_id": { "type": "string", "description": "Initiative id, for the view across its tasks. Give this or task_id." },
                "notes_limit": { "type": "integer", "description": "Notes to list (default 20, at most 100)." }
            }
        })
    }

    fn category(&self) -> &str {
        "tasks"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value> {
        let text = |key: &str| {
            args.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let opts = Options::with_notes_limit(args.get("notes_limit").and_then(Value::as_u64));
        let (db, vault) = (self.db.clone(), self.vault.clone());
        match (text("task_id"), text("initiative_id")) {
            (Some(task), None) => {
                tokio::task::spawn_blocking(move || task_activity(&db, &vault, &task, opts)).await?
            }
            (None, Some(initiative)) => {
                tokio::task::spawn_blocking(move || {
                    initiative_activity(&db, &vault, &initiative, opts)
                })
                .await?
            }
            _ => Err(anyhow!("give exactly one of task_id or initiative_id")),
        }
    }
}

#[cfg(test)]
#[path = "activity_tests.rs"]
mod tests;
