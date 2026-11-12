//! What a session leaves for the next one: where the work stands, what to do next,
//! what is open, which files matter. Written when a session heartbeats or releases and
//! handed to whoever claims the task next. It is data from another session, shown as
//! such: never an instruction HQ itself is giving.

use super::leases::is_invisible;
use super::*;

/// Longest summary, next step or open-questions text kept; longer is cut.
pub const MAX_CHECKPOINT_TEXT_CHARS: usize = 2000;
/// Most files a checkpoint names.
pub const MAX_CHECKPOINT_FILES: usize = 50;
const MAX_FILE_CHARS: usize = 200;
/// Most skill names a checkpoint suggests.
pub const MAX_SUGGESTED_SKILLS: usize = 8;
const MAX_SKILL_NAME_CHARS: usize = 64;
/// Checkpoints kept per task; older ones are dropped as new ones arrive.
pub const MAX_CHECKPOINTS_PER_TASK: i64 = 50;

/// What a session says about where it got to.
#[derive(Debug, Clone, Default)]
pub struct Checkpoint {
    pub summary: String,
    pub next_step: String,
    pub open_questions: String,
    pub files: Vec<String>,
    /// Skill names the next session might load. A hint from another session, not an instruction.
    pub suggested_skills: Vec<String>,
}

impl Checkpoint {
    /// Nothing left once control and invisible characters are gone: a checkpoint that says nothing.
    pub fn is_empty(&self) -> bool {
        clean_text(&self.summary).is_empty()
            && clean_text(&self.next_step).is_empty()
            && clean_text(&self.open_questions).is_empty()
            && clean_files(&self.files).is_empty()
            && clean_skills(&self.suggested_skills).is_empty()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskCheckpoint {
    pub id: i64,
    pub task_id: String,
    pub work_session_id: Option<String>,
    pub actor: String,
    pub summary: String,
    pub next_step: String,
    pub open_questions: String,
    pub files: Vec<String>,
    pub suggested_skills: Vec<String>,
    #[serde(serialize_with = "crate::wire_time::utc")]
    pub created_at: String,
}

const CHECKPOINT_COLS: &str = "id, task_id, work_session_id, actor, summary, next_step, open_questions, files, created_at, suggested_skills";

fn row_to_checkpoint(r: &rusqlite::Row) -> rusqlite::Result<TaskCheckpoint> {
    let files: String = r.get(7)?;
    Ok(TaskCheckpoint {
        id: r.get(0)?,
        task_id: r.get(1)?,
        work_session_id: r.get(2)?,
        actor: r.get(3)?,
        summary: r.get(4)?,
        next_step: r.get(5)?,
        open_questions: r.get(6)?,
        files: serde_json::from_str(&files).unwrap_or_default(),
        created_at: r.get(8)?,
        suggested_skills: serde_json::from_str(&r.get::<_, String>(9)?).unwrap_or_default(),
    })
}

/// Text with control and invisible characters dropped (newlines kept), cut to the limit.
fn clean_text(text: &str) -> String {
    text.chars()
        .filter(|c| *c == '\n' || (!c.is_control() && !is_invisible(*c)))
        .take(MAX_CHECKPOINT_TEXT_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

fn clean_files(files: &[String]) -> Vec<String> {
    files
        .iter()
        .map(|f| {
            f.chars()
                .filter(|c| !c.is_control() && !is_invisible(*c))
                .take(MAX_FILE_CHARS)
                .collect::<String>()
        })
        .map(|f| f.trim().to_string())
        .filter(|f| !f.is_empty())
        .take(MAX_CHECKPOINT_FILES)
        .collect()
}

/// Skill names kept: short lowercase slugs (`a-z`, `0-9`, `-`, `_`, `.`), the rest dropped,
/// duplicates folded, at most `MAX_SUGGESTED_SKILLS`.
pub fn clean_skills(skills: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in skills {
        let name = raw.trim().to_lowercase();
        let ok = !name.is_empty()
            && name.chars().count() <= MAX_SKILL_NAME_CHARS
            && name.starts_with(|c: char| c.is_ascii_alphanumeric())
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if ok && !out.contains(&name) {
            out.push(name);
        }
        if out.len() == MAX_SUGGESTED_SKILLS {
            break;
        }
    }
    out
}

/// Records a checkpoint on a task, under a lease when there is one. A checkpoint with
/// nothing in it is refused. Only the latest `MAX_CHECKPOINTS_PER_TASK` are kept.
pub fn add_checkpoint(
    conn: &Connection,
    task_ref: &str,
    work_session_id: Option<&str>,
    actor: &str,
    checkpoint: &Checkpoint,
) -> Result<TaskCheckpoint> {
    let (summary, next_step, open_questions) = (
        clean_text(&checkpoint.summary),
        clean_text(&checkpoint.next_step),
        clean_text(&checkpoint.open_questions),
    );
    let files = clean_files(&checkpoint.files);
    let skills = clean_skills(&checkpoint.suggested_skills);
    if summary.is_empty()
        && next_step.is_empty()
        && open_questions.is_empty()
        && files.is_empty()
        && skills.is_empty()
    {
        anyhow::bail!(
            "a checkpoint needs a summary, a next step, open questions, files or suggested skills"
        );
    }
    in_write_tx(conn, |conn| {
        let task =
            get_task(conn, task_ref)?.ok_or_else(|| anyhow::anyhow!("no task '{task_ref}'"))?;
        conn.execute(
            "INSERT INTO task_checkpoints \
             (task_id, work_session_id, actor, summary, next_step, open_questions, files, suggested_skills) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                task.id,
                work_session_id,
                super::leases::clean_label(actor),
                summary,
                next_step,
                open_questions,
                serde_json::to_string(&files)?,
                serde_json::to_string(&skills)?
            ],
        )?;
        let id = conn.last_insert_rowid();
        conn.execute(
            "DELETE FROM task_checkpoints WHERE task_id = ?1 AND id NOT IN \
             (SELECT id FROM task_checkpoints WHERE task_id = ?1 ORDER BY id DESC LIMIT ?2)",
            params![task.id, MAX_CHECKPOINTS_PER_TASK],
        )?;
        changed_outside_tx(conn);
        Ok(conn.query_row(
            &format!("SELECT {CHECKPOINT_COLS} FROM task_checkpoints WHERE id = ?1"),
            params![id],
            row_to_checkpoint,
        )?)
    })
}

/// The newest checkpoint on a task, the one a new session resumes from.
pub fn latest_checkpoint(conn: &Connection, task_ref: &str) -> Result<Option<TaskCheckpoint>> {
    let Some(task) = get_task(conn, task_ref)? else {
        return Ok(None);
    };
    Ok(conn
        .query_row(
            &format!("SELECT {CHECKPOINT_COLS} FROM task_checkpoints WHERE task_id = ?1 ORDER BY id DESC LIMIT 1"),
            params![task.id],
            row_to_checkpoint,
        )
        .optional()?)
}

/// A task's checkpoints, newest first.
pub fn list_checkpoints(
    conn: &Connection,
    task_ref: &str,
    limit: usize,
) -> Result<Vec<TaskCheckpoint>> {
    let task = get_task(conn, task_ref)?.ok_or_else(|| anyhow::anyhow!("no task '{task_ref}'"))?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {CHECKPOINT_COLS} FROM task_checkpoints WHERE task_id = ?1 ORDER BY id DESC LIMIT {limit}"
    ))?;
    let rows = stmt
        .query_map(params![task.id], row_to_checkpoint)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
