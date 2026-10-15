//! What a task itself says it needs, for a context packet built from a task id.
//!
//! In priority order: the task's own record (description, acceptance criteria, status, owner,
//! dependencies, what its last session left open), then the notes it links, then the notes
//! written under it or by the sessions it links, then explicit open questions. Everything here
//! comes from the task system and the provenance store; nothing is inferred from the text of a
//! note, and nothing is claimed beyond what the records hold.

use anyhow::Result;
use hq_db::Database;
use hq_db::tasks as t;
use serde::{Deserialize, Serialize};

/// Notes written under the task, or by its linked sessions, to offer as sources.
const MAX_PRODUCED: usize = 6;
/// Non-note links (pull requests, commits, URLs, sessions, tasks) listed in the record.
const MAX_REFERENCES: usize = 8;
const MAX_ACCEPTANCE_ITEMS: usize = 6;
const MAX_ITEM_CHARS: usize = 240;
const MAX_OPEN_QUESTION_CHARS: usize = 400;
/// The record never takes more than this share of the packet budget.
const RECORD_SHARE_DIVISOR: usize = 2;

/// A link from the task to something that is not a note: listed for the reader, not read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskReference {
    pub kind: String,
    pub reference: String,
    pub label: String,
}

/// The task as the task system holds it. Task text is written by people and agents, so it is
/// reference data like a note: the packet fences it and never obeys it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: String,
    pub display_id: String,
    pub title: String,
    pub status: String,
    pub priority: Option<String>,
    pub assignees: Vec<String>,
    pub description: String,
    pub acceptance: Vec<String>,
    /// Display ids of the dependencies that are not yet complete.
    pub blocked_by: Vec<String>,
    pub blocked_reason: Option<String>,
    pub waiting_on: Option<String>,
    /// What the last session on the task recorded as open, when it recorded anything.
    pub open_questions: String,
    pub references: Vec<TaskReference>,
    pub updated_at: String,
}

fn cap(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

/// The acceptance criteria written into a description: the text after an `Acceptance` heading
/// or label, split into lines and `;`-separated items. Empty when the task states none.
pub fn acceptance_of(description: &str) -> Vec<String> {
    let mut lines = description.lines();
    let mut items: Vec<String> = Vec::new();
    while let Some(line) = lines.next() {
        let lower = line.trim().to_lowercase();
        if !lower.starts_with("acceptance") {
            continue;
        }
        if let Some((_, rest)) = line.split_once(':')
            && !rest.trim().is_empty()
        {
            items.push(rest.trim().to_string());
        }
        for next in lines.by_ref() {
            if next.trim().is_empty() {
                break;
            }
            items.push(next.trim().trim_start_matches(['-', '*', ' ']).to_string());
        }
        break;
    }
    items
        .into_iter()
        .flat_map(|item| {
            item.split(';')
                .map(|part| part.trim().to_string())
                .collect::<Vec<_>>()
        })
        .filter(|item| !item.is_empty())
        .map(|item| cap(&item, MAX_ITEM_CHARS))
        .take(MAX_ACCEPTANCE_ITEMS)
        .collect()
}

impl TaskRecord {
    /// The text that goes inside the fenced record. This is also what the budget is charged for.
    pub fn body(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("Title: {}\n", self.title));
        if !self.description.trim().is_empty() {
            out.push_str(&format!("Description: {}\n", self.description.trim()));
        }
        if !self.acceptance.is_empty() {
            out.push_str("Acceptance criteria:\n");
            for item in &self.acceptance {
                out.push_str(&format!("- {item}\n"));
            }
        }
        if !self.assignees.is_empty() {
            out.push_str(&format!("Assigned to: {}\n", self.assignees.join(", ")));
        }
        if !self.blocked_by.is_empty() {
            out.push_str(&format!(
                "Waiting on unfinished tasks: {}\n",
                self.blocked_by.join(", ")
            ));
        }
        if let Some(reason) = &self.blocked_reason {
            out.push_str(&format!("Blocked: {reason}\n"));
        }
        if let Some(waiting) = &self.waiting_on {
            out.push_str(&format!("Waiting on: {waiting}\n"));
        }
        if !self.open_questions.trim().is_empty() {
            out.push_str(&format!(
                "Open questions (as the last session recorded them): {}\n",
                self.open_questions.trim()
            ));
        }
        for r in &self.references {
            let label = if r.label.is_empty() {
                String::new()
            } else {
                format!(" ({})", r.label)
            };
            out.push_str(&format!("Linked {}: {}{label}\n", r.kind, r.reference));
        }
        out
    }

    /// How much of the packet budget the record uses.
    pub fn chars(&self) -> usize {
        self.body().chars().count()
    }

    /// Trim the record until it fits in `max` characters, least important parts first: the
    /// non-note links, then the open questions, then the acceptance list, then the description.
    pub fn fit(&mut self, max: usize) {
        while self.chars() > max && !self.references.is_empty() {
            self.references.pop();
        }
        if self.chars() > max {
            self.open_questions = cap(&self.open_questions, MAX_OPEN_QUESTION_CHARS / 4);
        }
        while self.chars() > max && self.acceptance.len() > 1 {
            self.acceptance.pop();
        }
        if self.chars() > max {
            let over = self.chars() - max;
            let keep = self.description.chars().count().saturating_sub(over);
            self.description = cap(&self.description, keep);
        }
        if self.chars() > max {
            // Last resort for a tiny budget: the title and status are what is left.
            self.acceptance.clear();
            self.open_questions.clear();
            self.description.clear();
        }
    }
}

/// A source worth reading because of the task, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSource {
    pub path: String,
    pub via: &'static str,
    pub why: String,
}

/// What the task contributes to a packet.
#[derive(Debug, Clone)]
pub struct TaskContext {
    pub record: TaskRecord,
    /// Linked notes first, then notes written under the task or by its linked sessions.
    pub sources: Vec<TaskSource>,
}

/// Read the task, its links, its checkpoint and the notes produced under it. `Ok(None)` when
/// there is no such task.
pub fn task_context(
    db: &Database,
    task_ref: &str,
    record_budget: usize,
) -> Result<Option<TaskContext>> {
    db.with_conn(|conn| {
        let Some(task) = t::get_task(conn, task_ref)? else {
            return Ok(None);
        };
        let links = t::list_task_links(conn, &task.id)?;
        let checkpoint = t::latest_checkpoint(conn, &task.id)?;

        let mut sources: Vec<TaskSource> = Vec::new();
        for link in links.iter().filter(|l| l.kind == t::LINK_VAULT_NOTE) {
            sources.push(TaskSource {
                path: link.reference.clone(),
                via: "task_link",
                why: format!("linked to {} as {}", task.display_id, link.direction),
            });
        }

        // Notes written under this task, and notes written by the sessions it is linked to.
        let mut produced = hq_db::provenance::for_task(conn, &task.id, MAX_PRODUCED)?;
        let mut sessions: Vec<String> = links
            .iter()
            .filter(|l| l.kind == t::LINK_SESSION)
            .map(|l| l.reference.clone())
            .collect();
        sessions.extend(
            t::list_work_sessions(conn, &task.id, 10)?
                .into_iter()
                .filter_map(|w| w.harness_session_id),
        );
        sessions.sort();
        sessions.dedup();
        for session in &sessions {
            produced.extend(hq_db::provenance::for_session(conn, session, MAX_PRODUCED)?);
        }
        produced.sort_by_key(|p| std::cmp::Reverse(p.id));
        for record in produced.into_iter().take(MAX_PRODUCED * 2) {
            if sources.iter().any(|s| s.path == record.path) {
                continue;
            }
            let who = if record.is_unattributed() {
                "an unknown writer".to_string()
            } else {
                record.actor_id.clone()
            };
            sources.push(TaskSource {
                path: record.path,
                via: "task_produced",
                why: format!("recorded as written by {who} under {}", task.display_id),
            });
        }

        let references = links
            .iter()
            .filter(|l| l.kind != t::LINK_VAULT_NOTE)
            .take(MAX_REFERENCES)
            .map(|l| TaskReference {
                kind: l.kind.clone(),
                reference: cap(&l.reference, MAX_ITEM_CHARS),
                label: cap(&l.label, MAX_ITEM_CHARS),
            })
            .collect();

        let mut record = TaskRecord {
            id: task.id.clone(),
            display_id: task.display_id.clone(),
            title: cap(&task.title, MAX_ITEM_CHARS),
            status: task.status.clone(),
            priority: task.priority.clone(),
            assignees: task.assignees.clone(),
            acceptance: acceptance_of(&task.description),
            description: task.description.clone(),
            blocked_by: task.blocked_by.clone(),
            blocked_reason: task.blocked_reason.clone(),
            waiting_on: task.waiting_on.clone(),
            open_questions: checkpoint
                .map(|c| cap(&c.open_questions, MAX_OPEN_QUESTION_CHARS))
                .unwrap_or_default(),
            references,
            updated_at: task.updated_at.clone(),
        };
        record.fit(record_budget);
        Ok(Some(TaskContext { record, sources }))
    })
}

/// The share of a packet budget the task record may use.
pub fn record_budget(budget_chars: usize) -> usize {
    budget_chars / RECORD_SHARE_DIVISOR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acceptance_is_read_from_a_labelled_line_or_a_heading() {
        let d = "Do the thing.\n\nAcceptance: tests pass; docs updated\n\nNotes.";
        assert_eq!(acceptance_of(d), ["tests pass", "docs updated"]);
        let d = "Do the thing.\nAcceptance criteria\n- first\n- second\n\nOther.";
        assert_eq!(acceptance_of(d), ["first", "second"]);
        assert!(acceptance_of("Nothing stated here.").is_empty());
    }

    #[test]
    fn the_record_is_trimmed_to_its_share_least_important_first() {
        let mut r = TaskRecord {
            id: "tk".into(),
            display_id: "WRK-1".into(),
            title: "A title".into(),
            status: "in_progress".into(),
            priority: None,
            assignees: vec!["relay".into()],
            description: "d".repeat(2000),
            acceptance: vec!["a".repeat(100), "b".repeat(100)],
            blocked_by: vec![],
            blocked_reason: None,
            waiting_on: None,
            open_questions: "q".repeat(300),
            references: vec![TaskReference {
                kind: "pr".into(),
                reference: "x".repeat(100),
                label: String::new(),
            }],
            updated_at: String::new(),
        };
        r.fit(400);
        assert!(r.chars() <= 400, "{}", r.chars());
        assert!(r.references.is_empty(), "links go first");
        assert!(r.body().contains("A title"));
    }
}
