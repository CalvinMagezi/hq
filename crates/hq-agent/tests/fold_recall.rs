//! The context fold over a long chat (`docs/plans/context-fold.md`).
//!
//! The regression tests use a summarizer that keeps every `FACT:` line, so they check the
//! mechanics: nothing planted is lost through folds and distillation, the history stays bounded,
//! and the bytes in front of the newest block never change between turns (what lets a provider's
//! prompt cache keep working).
//!
//! `real_model_keeps_amounts_decisions_and_open_asks` runs the real summary rules against a
//! model and is opt-in, since it needs a configured provider and the network:
//! `cargo test -p hq-agent --test fold_recall -- --ignored`.

use anyhow::Result;
use async_trait::async_trait;
use hq_agent::fold::{self, LlmSummarizer, Summarizer, rules};
use hq_core::config::ContextFoldConfig;
use hq_db::Database;
use hq_db::chat::{add_message, create_thread};

struct KeepFacts;

#[async_trait]
impl Summarizer for KeepFacts {
    fn model(&self) -> String {
        "keep-facts".into()
    }
    async fn summarize(&self, _rules: &str, input: &str) -> Result<String> {
        let facts: Vec<String> = input
            .lines()
            .map(str::trim)
            .filter_map(|l| l.find("FACT:").map(|i| format!("- {}", &l[i..])))
            .collect();
        Ok(format!("Goal as of this block: test\n{}", facts.join("\n")))
    }
}

fn history(db: &Database, t: &str, cfg: &ContextFoldConfig) -> Vec<String> {
    let view = fold::load_view(db, t, cfg).unwrap();
    fold::block_messages(&view.blocks)
        .into_iter()
        .map(|m| format!("user: {}", m.content))
        .chain(
            view.tail
                .into_iter()
                .map(|m| format!("{}: {}", m.message.role, m.message.content)),
        )
        .collect()
}

fn small() -> ContextFoldConfig {
    ContextFoldConfig {
        tail_messages: 6,
        tail_tokens: 100_000,
        blocks_budget_tokens: 1_000,
        ..Default::default()
    }
}

#[tokio::test]
async fn two_hundred_turns_keep_every_fact_and_stay_bounded() {
    let db = Database::open_memory().unwrap();
    let t = db
        .with_conn(|c| Ok(create_thread(c, "t", "user", "user")?.thread_id))
        .unwrap();
    let cfg = small();
    let mut planted = Vec::new();
    for turn in 0..200 {
        let user = if turn % 7 == 0 {
            let fact = format!(
                "FACT: turn {turn} deposit UGX {} for the plan agreed with the bank, the school and the landlord after the long call",
                1_000 * turn
            );
            planted.push(fact.clone());
            format!("Please note this.\n{fact}")
        } else {
            format!("small talk {turn}")
        };
        db.with_conn(|c| {
            add_message(c, &t, "user", &user)?;
            add_message(c, &t, "assistant", &format!("ok {turn}"))?;
            Ok(())
        })
        .unwrap();
        fold::fold_thread(&db, &t, &cfg, &KeepFacts).await.unwrap();
    }
    let seen = history(&db, &t, &cfg).join("\n");
    for fact in &planted {
        assert!(seen.contains(fact.as_str()), "lost {fact}");
    }
    let view = fold::load_view(&db, &t, &cfg).unwrap();
    assert!(
        view.tail.len() <= cfg.target_messages() * 2,
        "{}",
        view.tail.len()
    );
    assert!(
        view.blocks.iter().any(|b| b.tier == 2),
        "200 turns should have distilled at least once"
    );
}

#[tokio::test]
async fn the_bytes_before_the_newest_block_never_change() {
    let db = Database::open_memory().unwrap();
    let t = db
        .with_conn(|c| Ok(create_thread(c, "t", "user", "user")?.thread_id))
        .unwrap();
    // No distillation here: that is the one rare step allowed to rewrite the front.
    let cfg = ContextFoldConfig {
        blocks_budget_tokens: 1_000_000,
        ..small()
    };
    let mut previous: Vec<String> = Vec::new();
    let mut folds = 0;
    for turn in 0..80 {
        db.with_conn(|c| {
            add_message(c, &t, "user", &format!("FACT: question {turn}"))?;
            add_message(c, &t, "assistant", &format!("answer {turn}"))?;
            Ok(())
        })
        .unwrap();
        let before = fold::load_view(&db, &t, &cfg).unwrap().blocks.len();
        fold::fold_thread(&db, &t, &cfg, &KeepFacts).await.unwrap();
        let now = history(&db, &t, &cfg);
        let blocks_now = fold::load_view(&db, &t, &cfg).unwrap().blocks.len();
        if blocks_now == before {
            // No fold: last turn's history is an exact prefix of this one's.
            assert_eq!(&now[..previous.len()], &previous[..], "turn {turn}");
        } else {
            // A fold: every block that existed is unchanged and still in front.
            folds += 1;
            assert_eq!(&now[..before], &previous[..before], "turn {turn}");
        }
        previous = now;
    }
    assert!(folds >= 5, "{folds}");
}

#[tokio::test]
async fn editing_an_old_message_unfolds_what_followed_it() {
    let db = Database::open_memory().unwrap();
    let t = db
        .with_conn(|c| Ok(create_thread(c, "t", "user", "user")?.thread_id))
        .unwrap();
    let cfg = small();
    let mut ids = Vec::new();
    for turn in 0..30 {
        let id = db
            .with_conn(|c| {
                let u = add_message(c, &t, "user", &format!("FACT: q{turn}"))?;
                add_message(c, &t, "assistant", &format!("a{turn}"))?;
                Ok(u.message_id)
            })
            .unwrap();
        ids.push(id);
        fold::fold_thread(&db, &t, &cfg, &KeepFacts).await.unwrap();
    }
    db.with_conn(|c| hq_db::chat::delete_messages_from(c, &t, &ids[5]))
        .unwrap();
    let seen = history(&db, &t, &cfg).join("\n");
    assert!(seen.contains("FACT: q4"));
    assert!(
        !seen.contains("FACT: q5"),
        "a deleted message must not live on in a block"
    );
    assert!(!seen.contains("FACT: q20"));
}

#[tokio::test]
#[ignore = "needs a configured model provider and the network"]
async fn real_model_keeps_amounts_decisions_and_open_asks() {
    let db = Database::open_memory().unwrap();
    let t = db
        .with_conn(|c| Ok(create_thread(c, "t", "user", "user")?.thread_id))
        .unwrap();
    let script = [
        (
            "user",
            "The school fees for term two are UGX 1,350,000, due on 2026-01-20.",
        ),
        ("assistant", "Noted: UGX 1,350,000 due 2026-01-20."),
        (
            "user",
            "Let's pay it in two parts, because my second salary lands on the 25th.",
        ),
        (
            "assistant",
            "Plan: two payments. I'll remind you on the 24th for the second one.",
        ),
        (
            "user",
            "Also send the landlord the revised budget when you can.",
        ),
        ("assistant", "Will do once you confirm the rent figure."),
        (
            "user",
            "Actually make the first payment UGX 700,000, not half.",
        ),
        (
            "assistant",
            "Updated: first payment UGX 700,000, the rest after the 25th.",
        ),
    ];
    db.with_conn(|c| {
        for (role, text) in script {
            add_message(c, &t, role, text)?;
        }
        for i in 0..12 {
            add_message(c, &t, "user", &format!("unrelated question {i}"))?;
            add_message(c, &t, "assistant", &format!("unrelated answer {i}"))?;
        }
        Ok(())
    })
    .unwrap();
    let cfg = ContextFoldConfig {
        tail_messages: 4,
        ..Default::default()
    };
    let summarizer = LlmSummarizer::from_config(&cfg);
    fold::fold_thread(&db, &t, &cfg, &summarizer).await.unwrap();
    let view = fold::load_view(&db, &t, &cfg).unwrap();
    let text = view
        .blocks
        .iter()
        .map(|b| b.summary.clone())
        .collect::<Vec<_>>()
        .join("\n");
    println!("{text}");
    for needle in ["1,350,000", "700,000", "25", "landlord", "Open objectives"] {
        assert!(text.contains(needle), "summary lost {needle}:\n{text}");
    }
    assert!(text.contains("Goal as of this block"));
    let _ = rules::TIER1_RULES;
}
