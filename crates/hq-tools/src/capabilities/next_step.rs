//! One optional next step for a task the turn just touched. The task is compared with invented,
//! labelled examples (`assets/next_step_examples.jsonl`) using the on-device embedder, and a
//! capability is offered only when its nearest examples all agree. No language model is involved,
//! nothing runs without a tap, and nothing here leaves the device.

use std::collections::HashSet;
use std::sync::LazyLock;

use anyhow::Result;
use hq_db::offers::TouchedTask;
use hq_db::search::cosine_similarity;
use rusqlite::Connection;
use serde::Deserialize;
use tokio::sync::OnceCell;

use super::offers::{Pacing, SAME_OFFER_GAP_SECS, TurnOffer, line_for};
use crate::vault::QueryEmbedder;

const EXAMPLES: &str = include_str!("../../assets/next_step_examples.jsonl");
/// All of these nearest examples must name the same capability.
const NEIGHBOURS: usize = 3;
/// The closest example must be at least this similar, or the task is unlike anything known.
const MIN_SIMILARITY: f32 = 0.7;
/// Enough of a task's description to judge it; the bank's notes are one or two sentences.
const NOTE_CHARS: usize = 240;
/// A long title would crowd the composer.
const TITLE_CHARS: usize = 80;
/// Only a turn that worked on a task has a task to suggest a step for.
const TASK_TOOLS: [&str; 3] = ["task_create", "task_create_many", "task_update"];

/// Capability, the kind stored in `offer_log`, and what Yes puts in the composer.
const STEPS: [(&str, &str, &str); 10] = [
    (
        "track-todo",
        "next_track_todo",
        "Give \"{task}\" a due date.",
    ),
    (
        "tasks-from-list",
        "next_tasks_from_list",
        "Split \"{task}\" into separate tasks.",
    ),
    (
        "read-photo",
        "next_read_photo",
        "Read the text in this photo for \"{task}\".",
    ),
    (
        "file-to-note",
        "next_file_to_note",
        "Keep this file as a note I can search for \"{task}\".",
    ),
    (
        "note-to-file",
        "next_note_to_file",
        "Make the note for \"{task}\" into a PDF I can send.",
    ),
    (
        "remember-people",
        "next_remember_people",
        "For \"{task}\", remember that ",
    ),
    (
        "look-up",
        "next_look_up",
        "Look up what I need for \"{task}\".",
    ),
    (
        "email",
        "next_email",
        "Find the emails about \"{task}\" and draft a reply.",
    ),
    (
        "projects",
        "next_projects",
        "Start a project for \"{task}\".",
    ),
    (
        "helpers",
        "next_helpers",
        "Send a helper to work on \"{task}\".",
    ),
];

#[derive(Deserialize)]
struct Example {
    t: String,
    c: Option<String>,
}

static BANK: LazyLock<Vec<Example>> = LazyLock::new(|| {
    EXAMPLES
        .lines()
        .map(|l| serde_json::from_str(l).expect("next_step_examples.jsonl is valid"))
        .collect()
});
static VECTORS: OnceCell<Vec<Vec<f32>>> = OnceCell::const_new();

/// The catalog entry behind a kind stored in `offer_log`, for reports.
pub fn capability_of(kind: &str) -> Option<&'static str> {
    STEPS.iter().find(|s| s.1 == kind).map(|s| s.0)
}

/// The task to weigh, when this turn worked on one and no offer was shown too recently.
pub fn task_to_weigh(
    conn: &Connection,
    turn_tools: &[String],
    asked_at: i64,
    now: i64,
) -> Result<Option<TouchedTask>> {
    if !turn_tools.iter().any(|t| TASK_TOOLS.contains(&t.as_str())) {
        return Ok(None);
    }
    if !Pacing::from_last_shown(&hq_db::offers::last_shown(conn)?).any_allows(now) {
        return Ok(None);
    }
    hq_db::offers::task_touched_since(conn, asked_at)
}

/// A task written the way the examples are.
pub fn query_text(task: &TouchedTask) -> String {
    let note: String = task.description.chars().take(NOTE_CHARS).collect();
    let note = note.trim();
    let mut text = format!("Task: {}\nState: {}", task.title, task.status);
    if !note.is_empty() {
        text.push_str("\nNote: ");
        text.push_str(note);
    }
    text
}

/// Embeds the example bank once. It can wait on the model's first download.
pub async fn warm(embed: &QueryEmbedder) -> Result<()> {
    VECTORS
        .get_or_try_init(|| async {
            let mut all = Vec::with_capacity(BANK.len());
            for example in BANK.iter() {
                all.push(embed(example.t.clone()).await?);
            }
            anyhow::Ok(all)
        })
        .await?;
    Ok(())
}

/// The capability the nearest examples agree on, or `None`. Any embedding failure means no
/// suggestion. Until the bank is ready the answer is `None` too, so a reply never waits for it.
pub async fn nearest_capability(embed: &QueryEmbedder, text: &str) -> Option<&'static str> {
    let Some(vectors) = VECTORS.get() else {
        let embed = embed.clone();
        tokio::spawn(async move {
            if let Err(e) = warm(&embed).await {
                tracing::debug!("next step: example bank unavailable: {e}");
            }
        });
        return None;
    };
    let query = embed(text.to_string())
        .await
        .inspect_err(|e| tracing::debug!("next step: could not embed the task: {e}"))
        .ok()?;
    let labels: Vec<Option<&'static str>> = BANK.iter().map(|e| e.c.as_deref()).collect();
    nearest(&query, vectors, &labels)
}

fn nearest(
    query: &[f32],
    vectors: &[Vec<f32>],
    labels: &[Option<&'static str>],
) -> Option<&'static str> {
    let mut scored: Vec<(f32, Option<&'static str>)> = vectors
        .iter()
        .zip(labels)
        .map(|(v, label)| (cosine_similarity(query, v), *label))
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let top = scored.get(..NEIGHBOURS)?;
    if top[0].0 < MIN_SIMILARITY {
        return None;
    }
    let first = top[0].1?;
    top.iter().all(|(_, c)| *c == Some(first)).then_some(first)
}

/// The offer for `capability`, unless it is not ready here, muted, shown too recently, or something
/// this turn already did.
pub fn offer_for(
    conn: &Connection,
    capability: &str,
    title: &str,
    ready: &HashSet<String>,
    turn_tools: &[String],
    now: i64,
) -> Result<Option<TurnOffer>> {
    let Some((_, kind, ask)) = STEPS.iter().find(|s| s.0 == capability) else {
        return Ok(None);
    };
    let Some(cap) = super::catalog().iter().find(|c| c.id == capability) else {
        return Ok(None);
    };
    let done = cap.tools.iter().any(|t| turn_tools.contains(t));
    if done || !ready.contains(capability) || hq_db::offers::muted(conn, now)?.contains(*kind) {
        return Ok(None);
    }
    let last = hq_db::offers::last_shown(conn)?;
    if last
        .get(*kind)
        .is_some_and(|t| now - t < SAME_OFFER_GAP_SECS)
    {
        return Ok(None);
    }
    let short: String = title.chars().take(TITLE_CHARS).collect();
    Ok(Some(TurnOffer {
        kind,
        line: line_for(&cap.says),
        reply: ask.replace("{task}", short.trim()),
        send: false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::Database;
    use hq_db::offers::{Answer, answer, record_shown};

    const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;

    fn ready() -> HashSet<String> {
        STEPS.iter().map(|s| s.0.to_string()).collect()
    }

    fn tools(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn every_example_names_a_step_and_every_step_is_in_the_catalog() {
        for example in BANK.iter() {
            if let Some(c) = &example.c {
                assert!(STEPS.iter().any(|s| s.0 == c), "no step for {c}");
            }
        }
        for (capability, _, _) in STEPS {
            assert!(
                crate::capabilities::catalog()
                    .iter()
                    .any(|c| c.id == capability),
                "{capability} is not in the catalog"
            );
        }
    }

    #[test]
    fn only_a_unanimous_close_match_suggests_anything() {
        let close = vec![1.0, 0.0];
        let vectors = vec![close.clone(), close.clone(), close.clone(), vec![0.0, 1.0]];
        let look = Some("look-up");
        assert_eq!(nearest(&close, &vectors, &[look, look, look, None]), look);
        assert_eq!(
            nearest(&close, &vectors, &[look, look, Some("email"), None]),
            None,
            "two of three is not agreement"
        );
        assert_eq!(nearest(&close, &vectors, &[None, None, None, look]), None);
        assert_eq!(
            nearest(&[0.5, -0.9], &vectors, &[look, look, look, look]),
            None,
            "unlike anything known"
        );
        assert_eq!(nearest(&close, &vectors[..2], &[look, look]), None);
    }

    #[test]
    fn a_suggestion_only_ever_fills_the_composer() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            for (capability, kind, _) in STEPS {
                let o = offer_for(c, capability, "Pay the roofer", &ready(), &[], NOW)?
                    .expect("an offer");
                assert_eq!(o.kind, kind);
                assert!(!o.send, "{kind} must wait for the person to send it");
                assert!(o.reply.contains("Pay the roofer") && !o.reply.contains("{task}"));
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_suggestion_respects_readiness_what_the_turn_did_mutes_and_spacing() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            let offer = |ready: &HashSet<String>, done: &[String], at: i64| {
                offer_for(c, "look-up", "Price cement", ready, done, at)
            };
            assert!(
                offer(&HashSet::new(), &[], NOW)?.is_none(),
                "not ready here"
            );
            assert!(
                offer(&ready(), &tools(&["web_search"]), NOW)?.is_none(),
                "already done"
            );

            record_shown(c, "m1", "next_look_up", NOW)?;
            assert!(offer(&ready(), &[], NOW + 1)?.is_none(), "too soon");
            assert!(offer(&ready(), &[], NOW + SAME_OFFER_GAP_SECS)?.is_some());

            answer(c, "m1", Answer::NotNow, NOW + 1)?;
            record_shown(c, "m2", "next_look_up", NOW + 2)?;
            answer(c, "m2", Answer::NotNow, NOW + 3)?;
            assert!(offer(&ready(), &[], NOW + 4 * DAY)?.is_none(), "muted");
            Ok(())
        })
        .unwrap();
    }

    #[tokio::test]
    async fn no_embedder_means_no_suggestion_and_no_error() {
        let failing: QueryEmbedder =
            std::sync::Arc::new(|_| Box::pin(async { anyhow::bail!("no model here") }));
        assert_eq!(
            nearest_capability(&failing, "Task: x\nState: to_do").await,
            None
        );
        assert!(warm(&failing).await.is_err());
    }

    #[test]
    fn a_next_step_also_spaces_out_the_rule_based_offers() {
        use crate::capabilities::offers::{ANY_OFFER_GAP_SECS, Detector};

        let shown = std::collections::HashMap::from([("next_email".to_string(), NOW)]);
        let pacing = Pacing::from_last_shown(&shown);
        assert!(!pacing.allows(Detector::PastedList, NOW + 1));
        assert!(pacing.allows(Detector::PastedList, NOW + ANY_OFFER_GAP_SECS));
    }

    #[test]
    fn only_a_turn_that_touched_a_task_is_weighed_and_any_recent_offer_holds_it_back() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            c.execute_batch(
                "PRAGMA foreign_keys = OFF;
                 INSERT INTO tasks (id, initiative_id, display_id, title, description, status, updated_at)
                 VALUES ('a', 'i', 'A-1', 'Order rebar', 'Before the pour', 'to_do', datetime(1800000000, 'unixepoch'));",
            )?;
            let task_turn = tools(&["task_create"]);
            assert!(task_to_weigh(c, &tools(&["web_search"]), NOW, NOW)?.is_none());
            assert!(task_to_weigh(c, &task_turn, NOW + 1, NOW + 1)?.is_none(), "older than the turn");
            let task = task_to_weigh(c, &task_turn, NOW, NOW)?.expect("the task");
            assert_eq!(query_text(&task), "Task: Order rebar\nState: to_do\nNote: Before the pour");

            record_shown(c, "m1", "pasted_list", NOW - 60)?;
            assert!(task_to_weigh(c, &task_turn, NOW, NOW)?.is_none(), "an offer shown today");
            Ok(())
        })
        .unwrap();
    }
}
