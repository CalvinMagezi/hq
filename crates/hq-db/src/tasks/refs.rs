//! Typed links from a task to a vault note, chat thread, session, commit, pull
//! request, URL or another task. Every `ref` is normalised per kind, so the same
//! thing is always the same row and a lookup from a note to its tasks is an index
//! hit. Links are data: a ref is checked for shape, never fetched or followed.

use super::*;

pub const LINK_VAULT_NOTE: &str = "vault_note";
pub const LINK_CHAT_THREAD: &str = "chat_thread";
pub const LINK_SESSION: &str = "session";
pub const LINK_COMMIT: &str = "commit";
pub const LINK_PR: &str = "pr";
pub const LINK_URL: &str = "url";
pub const LINK_TASK: &str = "task";
pub const LINK_KINDS: [&str; 7] = [
    LINK_VAULT_NOTE,
    LINK_CHAT_THREAD,
    LINK_SESSION,
    LINK_COMMIT,
    LINK_PR,
    LINK_URL,
    LINK_TASK,
];

pub const DIRECTION_ORIGIN: &str = "origin";
pub const DIRECTION_RELATED: &str = "related";
pub const DIRECTION_PRODUCED: &str = "produced";
const DIRECTIONS: [&str; 3] = [DIRECTION_ORIGIN, DIRECTION_RELATED, DIRECTION_PRODUCED];

/// Most links one task keeps, so a runaway agent cannot fill a task.
pub const MAX_LINKS_PER_TASK: i64 = 100;
const MAX_REF_CHARS: usize = 300;
const MAX_URL_CHARS: usize = 2000;
const MAX_ID_CHARS: usize = 128;
const MAX_LABEL_CHARS: usize = 200;
const MIN_SHA_LEN: usize = 7;
const MAX_SHA_LEN: usize = 40;
const MAX_PR_NUMBER_DIGITS: usize = 9;

#[derive(Debug, Clone, Serialize)]
pub struct TaskLink {
    pub id: i64,
    pub task_id: String,
    pub kind: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub label: String,
    pub direction: String,
    pub created_by: String,
    pub created_at: String,
    /// For a `task` link, the task it points at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linked_task: Option<LinkedTask>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkedTask {
    pub id: String,
    pub display_id: String,
    pub title: String,
    pub status: String,
}

const LINK_COLS: &str = "id, task_id, kind, ref, label, direction, created_by, created_at";

fn row_to_link(r: &rusqlite::Row) -> rusqlite::Result<TaskLink> {
    Ok(TaskLink {
        id: r.get(0)?,
        task_id: r.get(1)?,
        kind: r.get(2)?,
        reference: r.get(3)?,
        label: r.get(4)?,
        direction: r.get(5)?,
        created_by: r.get(6)?,
        created_at: r.get(7)?,
        linked_task: None,
    })
}

fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':')
}

fn is_sha(text: &str) -> bool {
    (MIN_SHA_LEN..=MAX_SHA_LEN).contains(&text.len()) && text.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_repo_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')
}

/// `owner/repo`, neither part empty or `.` or `..` (which a browser would resolve to
/// a different repository than the one written).
fn is_plain_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    repo.chars().all(is_repo_char) && [owner, name].iter().all(|p| !p.is_empty() && *p != "." && *p != "..")
}

/// A path inside the vault in one spelling: `./`, `a//b` and a trailing slash all
/// collapse, so the same note is always the same ref. Absolute paths, drive letters,
/// backslashes and `..` are refused.
fn vault_note_ref(text: &str) -> Result<String> {
    let drive_letter = text.len() >= 2 && text.as_bytes()[1] == b':' && text.as_bytes()[0].is_ascii_alphabetic();
    let parts: Vec<&str> = text.split('/').filter(|p| !p.is_empty() && *p != ".").collect();
    let bad = text.starts_with('/') || text.contains('\\') || drive_letter || parts.is_empty() || parts.contains(&"..");
    if bad {
        anyhow::bail!("vault_note ref must be a path inside the vault such as Notebooks/Projects/plan.md, got '{text}'");
    }
    Ok(parts.join("/"))
}

fn id_ref(kind: &str, text: &str) -> Result<String> {
    if text.is_empty() || text.chars().count() > MAX_ID_CHARS || !text.chars().all(is_id_char) {
        anyhow::bail!("{kind} ref must be an id of letters, digits and - _ . : (at most {MAX_ID_CHARS}), got '{text}'");
    }
    Ok(text.to_string())
}

fn commit_ref(text: &str) -> Result<String> {
    let (repo, sha) = match text.split_once('@') {
        Some((repo, sha)) => (Some(repo), sha),
        None => (None, text),
    };
    let repo_ok = repo.is_none_or(is_plain_repo);
    if !repo_ok || !is_sha(sha) {
        anyhow::bail!("commit ref must be a sha of 7 to 40 hex digits, optionally as repo@sha, got '{text}'");
    }
    Ok(match repo {
        Some(repo) => format!("{}@{}", repo.to_ascii_lowercase(), sha.to_ascii_lowercase()),
        None => sha.to_ascii_lowercase(),
    })
}

/// `owner/repo#123`, also accepted as a github.com pull request URL.
fn pr_ref(text: &str) -> Result<String> {
    let from_url = text
        .strip_prefix("https://github.com/")
        .and_then(|rest| {
            let parts: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
            match parts.as_slice() {
                [owner, repo, "pull", number] => Some(format!("{owner}/{repo}#{number}")),
                _ => None,
            }
        })
        .unwrap_or_else(|| text.to_string());
    let valid = from_url.split_once('#').is_some_and(|(repo, number)| {
        repo.split_once('/').is_some_and(|(_, name)| is_plain_repo(repo) && !name.contains('/'))
            && !number.is_empty()
            && number.len() <= MAX_PR_NUMBER_DIGITS
            && number.chars().all(|c| c.is_ascii_digit())
    });
    if !valid {
        anyhow::bail!("pr ref must be owner/repo#123 or a github.com pull request URL, got '{text}'");
    }
    // GitHub names are case-insensitive and 007 is 7, so each pull request is one ref.
    let (repo, number) = from_url.split_once('#').unwrap_or_default();
    let number = number.trim_start_matches('0');
    if number.is_empty() {
        anyhow::bail!("pr number must be at least 1, got '{text}'");
    }
    Ok(format!("{}#{number}", repo.to_ascii_lowercase()))
}

fn url_ref(text: &str) -> Result<String> {
    let scheme_ok = text.starts_with("https://") || text.starts_with("http://");
    if !scheme_ok
        || text.chars().count() > MAX_URL_CHARS
        || text.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        anyhow::bail!("url ref must be an http or https URL without spaces, got '{text}'");
    }
    Ok(text.to_string())
}

/// The kind, direction, label and stored ref a link would have, or why it is refused.
/// For a `task` link the ref becomes the other task's internal id.
pub fn normalize_link(
    conn: &Connection,
    kind: &str,
    reference: &str,
    direction: Option<&str>,
) -> Result<(String, String, String)> {
    if !LINK_KINDS.contains(&kind) {
        anyhow::bail!("unknown link kind '{kind}', expected one of {}", LINK_KINDS.join(", "));
    }
    let direction = direction.unwrap_or(DIRECTION_RELATED);
    if !DIRECTIONS.contains(&direction) {
        anyhow::bail!("unknown link direction '{direction}', expected one of {}", DIRECTIONS.join(", "));
    }
    let text = reference.trim();
    if text.is_empty() || text.chars().any(char::is_control) || (kind != LINK_URL && text.chars().count() > MAX_REF_CHARS) {
        anyhow::bail!("{kind} ref is empty, too long or contains control characters");
    }
    let stored = match kind {
        LINK_VAULT_NOTE => vault_note_ref(text)?,
        LINK_CHAT_THREAD | LINK_SESSION => id_ref(kind, text)?,
        LINK_COMMIT => commit_ref(text)?,
        LINK_PR => pr_ref(text)?,
        LINK_URL => url_ref(text)?,
        _ => get_task(conn, text)?
            .ok_or_else(|| anyhow::anyhow!("no task '{text}' to link to"))?
            .id,
    };
    Ok((kind.to_string(), stored, direction.to_string()))
}

fn clean_text(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(MAX_LABEL_CHARS).collect::<String>().trim().to_string()
}

fn with_linked_task(conn: &Connection, mut link: TaskLink) -> Result<TaskLink> {
    if link.kind == LINK_TASK {
        link.linked_task = get_task(conn, &link.reference)?.map(|t| LinkedTask {
            id: t.id,
            display_id: t.display_id,
            title: t.title,
            status: t.status,
        });
    }
    Ok(link)
}

/// Links `task_ref` to `reference`. Adding the same link again returns the one
/// that exists (the flag says whether a row was created).
pub fn add_task_link(
    conn: &Connection,
    task_ref: &str,
    kind: &str,
    reference: &str,
    label: &str,
    direction: Option<&str>,
    created_by: &str,
) -> Result<(TaskLink, bool)> {
    in_write_tx(conn, |conn| {
        let task = get_task(conn, task_ref)?.ok_or_else(|| anyhow::anyhow!("no task '{task_ref}'"))?;
        let (kind, stored, direction) = normalize_link(conn, kind, reference, direction)?;
        if kind == LINK_TASK && stored == task.id {
            anyhow::bail!("a task cannot link to itself");
        }
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM task_links WHERE task_id = ?1 AND kind = ?2 AND ref = ?3",
                params![task.id, kind, stored],
                |r| r.get(0),
            )
            .optional()?;
        let (id, created) = match existing {
            Some(id) => (id, false),
            None => {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM task_links WHERE task_id = ?1",
                    params![task.id],
                    |r| r.get(0),
                )?;
                if count >= MAX_LINKS_PER_TASK {
                    anyhow::bail!("{} already has {MAX_LINKS_PER_TASK} links", task.display_id);
                }
                conn.execute(
                    "INSERT INTO task_links (task_id, kind, ref, label, direction, created_by) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![task.id, kind, stored, clean_text(label), direction, clean_text(created_by)],
                )?;
                (conn.last_insert_rowid(), true)
            }
        };
        let link = conn.query_row(
            &format!("SELECT {LINK_COLS} FROM task_links WHERE id = ?1"),
            params![id],
            row_to_link,
        )?;
        if created {
            changed_outside_tx(conn);
        }
        Ok((with_linked_task(conn, link)?, created))
    })
}

/// Removes one link. Returns whether it existed.
pub fn remove_task_link(conn: &Connection, task_ref: &str, kind: &str, reference: &str) -> Result<bool> {
    in_write_tx(conn, |conn| {
        let task = get_task(conn, task_ref)?.ok_or_else(|| anyhow::anyhow!("no task '{task_ref}'"))?;
        let (kind, stored, _) = normalize_link(conn, kind, reference, None)?;
        let removed = conn.execute(
            "DELETE FROM task_links WHERE task_id = ?1 AND kind = ?2 AND ref = ?3",
            params![task.id, kind, stored],
        )?;
        if removed > 0 {
            changed_outside_tx(conn);
        }
        Ok(removed > 0)
    })
}

/// A task's links, origin first, then oldest first.
pub fn list_task_links(conn: &Connection, task_ref: &str) -> Result<Vec<TaskLink>> {
    let task = get_task(conn, task_ref)?.ok_or_else(|| anyhow::anyhow!("no task '{task_ref}'"))?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {LINK_COLS} FROM task_links WHERE task_id = ?1 \
         ORDER BY direction != 'origin', id"
    ))?;
    let links = stmt
        .query_map(params![task.id], row_to_link)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    links.into_iter().map(|l| with_linked_task(conn, l)).collect()
}

/// The tasks that link to a thing, for example every task a vault note started.
pub fn tasks_linked_to(conn: &Connection, kind: &str, reference: &str) -> Result<Vec<Task>> {
    let (kind, stored, _) = normalize_link(conn, kind, reference, None)?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {TASK_COLS} FROM tasks t JOIN task_links l ON l.task_id = t.id \
         WHERE l.kind = ?1 AND l.ref = ?2 ORDER BY t.updated_at DESC, t.id DESC LIMIT {MAX_LIST_LIMIT}"
    ))?;
    let mut tasks = stmt
        .query_map(params![kind, stored], row_to_task)?
        .collect::<rusqlite::Result<Vec<Task>>>()?;
    hydrate(conn, &mut tasks)?;
    Ok(tasks)
}
