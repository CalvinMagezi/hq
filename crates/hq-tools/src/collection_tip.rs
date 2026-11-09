//! Once a week, one thing the owner saved that matches what they are working on.
//!
//! Chosen by keyword on the device: no model sees the tasks or the collection, and the item is
//! marked device-only, so it never goes through a push service.

use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, Utc};
use hq_core::types::{COLLECTION_TIP_SOURCE, SearchResult, ValueItem, ValueKind};
use hq_db::Database;
use hq_db::link_captures::{record_tip, tipped_paths};
use hq_db::tasks::{TaskFilter, list_tasks};
use std::collections::HashSet;

use crate::link_capture::SAVED_ITEM_TAG;

const OPEN_STATUSES: [&str; 2] = ["in_progress", "to_do"];
const TASKS_PER_STATUS: usize = 20;
const TASK_RECENCY_DAYS: i64 = 14;
/// Deep enough that notes which were never saved do not crowd out the ones that were.
const HITS_PER_TASK: usize = 100;
const MIN_WORD_CHARS: usize = 4;
/// One shared word is usually chance. Two is the first count that reads as a real match.
const MIN_SHARED_WORDS: usize = 2;
const MAX_TASK_TITLE_CHARS: usize = 120;
const STOP_WORDS: &[&str] = &[
    "about", "after", "again", "from", "have", "into", "make", "more", "need", "note", "notes",
    "only", "over", "some", "task", "that", "them", "then", "they", "this", "what", "when", "with",
    "work", "your",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tip {
    pub path: String,
    pub task: String,
}

fn words(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= MIN_WORD_CHARS && !STOP_WORDS.contains(&w.as_str()))
        .collect()
}

fn shared_words(task: &HashSet<String>, hit: &SearchResult) -> usize {
    let snippet = hit.snippet.replace("<mark>", "").replace("</mark>", "");
    let text = format!("{} {snippet}", hit.title);
    words(&text).intersection(task).count()
}

/// The saved note sharing the most words with an open task, skipping ones already brought up.
pub fn pick(
    tasks: &[String],
    mut search: impl FnMut(&str) -> Vec<SearchResult>,
    already: &HashSet<String>,
) -> Option<Tip> {
    let mut best: Option<(usize, Tip)> = None;
    for task in tasks {
        let wanted = words(task);
        if wanted.len() < MIN_SHARED_WORDS {
            continue;
        }
        let query = wanted.iter().cloned().collect::<Vec<_>>().join(" ");
        for hit in search(&query) {
            let saved = hit.tags.iter().any(|t| t == SAVED_ITEM_TAG);
            if !saved || already.contains(&hit.note_path) {
                continue;
            }
            let score = shared_words(&wanted, &hit);
            if score >= MIN_SHARED_WORDS && best.as_ref().is_none_or(|(b, _)| score > *b) {
                let tip = Tip {
                    path: hit.note_path,
                    task: task.clone(),
                };
                best = Some((score, tip));
            }
        }
    }
    best.map(|(_, tip)| tip)
}

/// The note's file name as plain words, since a shared page can choose its own title.
fn plain_name(path: &str) -> String {
    let stem = path
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".md");
    let kept: String = stem
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '\''))
        .collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A web app path to the note, each byte outside the plain path characters percent-encoded.
fn note_link(path: &str) -> String {
    let mut link = String::from("/notes/");
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'-' | b'_') {
            link.push(b as char);
        } else {
            link.push_str(&format!("%{b:02X}"));
        }
    }
    link
}

fn week_key(now: DateTime<Utc>) -> String {
    let week = now.iso_week();
    format!("collection-tip-{}-W{:02}", week.year(), week.week())
}

fn tip_item(tip: &Tip, now: DateTime<Utc>) -> ValueItem {
    let task: String = tip.task.chars().take(MAX_TASK_TITLE_CHARS).collect();
    ValueItem::new(
        COLLECTION_TIP_SOURCE,
        ValueKind::Fyi,
        format!("From your collection: {}", plain_name(&tip.path)),
        format!("You saved this, and it may help with \"{task}\"."),
    )
    .with_dedup_key(week_key(now))
    .with_link(note_link(&tip.path))
}

fn open_task_titles(db: &Database, now: DateTime<Utc>) -> Result<Vec<String>> {
    let since = (now - Duration::days(TASK_RECENCY_DAYS))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let mut titles = Vec::new();
    for status in OPEN_STATUSES {
        let filter = TaskFilter {
            status: Some(status.to_string()),
            limit: Some(TASKS_PER_STATUS),
            updated_since: Some(since.clone()),
            ..TaskFilter::default()
        };
        let found = db.with_conn(|c| list_tasks(c, &filter))?;
        titles.extend(found.into_iter().map(|t| t.title));
    }
    Ok(titles)
}

/// Puts this week's tip in the inbox. Returns whether there was one worth showing.
pub fn run(db: &Database, now: DateTime<Utc>) -> Result<bool> {
    // The daemon also runs this after every restart. Checked before picking, or a second run in
    // the week would mark a note as shown that the owner never saw.
    if hq_db::value_items::find_active_by_dedup(db, ValueKind::Fyi, &week_key(now))?.is_some() {
        return Ok(false);
    }
    let tasks = open_task_titles(db, now)?;
    let already = db.with_conn(tipped_paths)?;
    let search = |query: &str| {
        db.with_conn(|c| hq_db::search::keyword_search(c, query, HITS_PER_TASK))
            .inspect_err(|e| tracing::warn!(error = %e, "collection tip: search failed"))
            .unwrap_or_default()
    };
    let Some(tip) = pick(&tasks, search, &already) else {
        return Ok(false);
    };
    hq_db::value_items::emit(db, &tip_item(&tip, now))?;
    db.with_conn(|c| record_tip(c, &tip.path, now))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::types::MatchType;

    fn hit(path: &str, title: &str, tags: &[&str]) -> SearchResult {
        SearchResult {
            note_path: path.to_string(),
            title: title.to_string(),
            notebook: "Inbox".to_string(),
            snippet: String::new(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            relevance: 1.0,
            match_type: MatchType::Keyword,
        }
    }

    const TASK: &str = "Compare vector databases for search";

    #[test]
    fn it_picks_a_saved_note_that_shares_real_words() {
        let hits = vec![
            hit("a.md", "Vector databases compared", &[SAVED_ITEM_TAG]),
            hit("b.md", "Vector databases compared", &["other"]),
            hit("c.md", "Vector quilting", &[SAVED_ITEM_TAG]),
        ];
        let tip = pick(&[TASK.to_string()], |_| hits.clone(), &HashSet::new()).unwrap();
        assert_eq!(tip.path, "a.md");
        assert_eq!(tip.task, TASK);
    }

    #[test]
    fn a_note_already_shown_or_a_weak_match_is_not_picked() {
        let hits = vec![hit("a.md", "Vector databases", &[SAVED_ITEM_TAG])];
        let seen = HashSet::from(["a.md".to_string()]);
        assert_eq!(pick(&[TASK.to_string()], |_| hits.clone(), &seen), None);
        let weak = vec![hit("c.md", "Vector quilting", &[SAVED_ITEM_TAG])];
        assert_eq!(
            pick(&[TASK.to_string()], |_| weak.clone(), &HashSet::new()),
            None
        );
    }

    #[test]
    fn the_item_stays_on_the_device_and_a_hostile_name_is_plain() {
        let tip = Tip {
            path: "Notebooks/Inbox/![x](https:evil) a b.md".to_string(),
            task: TASK.to_string(),
        };
        let item = tip_item(&tip, Utc::now());
        assert!(item.is_device_only());
        assert_eq!(item.title, "From your collection: xhttpsevil a b");
        assert_eq!(
            item.link.as_deref(),
            Some("/notes/Notebooks/Inbox/%21%5Bx%5D%28https%3Aevil%29%20a%20b.md")
        );
    }

    #[test]
    fn a_week_with_a_match_writes_one_item_and_never_repeats_the_note() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            hq_db::search::index_note(
                c,
                "Notebooks/Inbox/vectors.md",
                "Vector databases compared",
                "Search engines and vector databases.",
                SAVED_ITEM_TAG,
            )
        })
        .unwrap();
        db.with_conn(|c| {
            hq_db::tasks::create_initiative(c, "in-1", "personal", None, "A", "a", "A")?;
            let new = hq_db::tasks::NewTask {
                title: TASK,
                created_by: "test",
                ..Default::default()
            };
            hq_db::tasks::create_task(c, "tk-1", "in-1", &new)
        })
        .unwrap();
        assert!(run(&db, Utc::now()).unwrap());
        db.with_conn(|c| {
            hq_db::search::index_note(
                c,
                "Notebooks/Inbox/second.md",
                "Vector databases again",
                "More vector databases.",
                SAVED_ITEM_TAG,
            )
        })
        .unwrap();
        assert!(!run(&db, Utc::now()).unwrap());
        let shown = db.with_conn(tipped_paths).unwrap();
        assert_eq!(
            shown.len(),
            1,
            "a later run this week must not use up another note"
        );
    }
}
