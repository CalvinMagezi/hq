//! Relations a fold finds become graph edges that cite the chat message saying them, and stop
//! counting once that message is gone (`docs/plans/context-fold.md`, phase 5).

use anyhow::Result;
use async_trait::async_trait;
use hq_agent::fold::{self, FoldOutcome, Summarizer};
use hq_core::config::ContextFoldConfig;
use hq_db::Database;
use hq_db::chat::{add_message, create_thread};
use hq_memory::graph_paths::{GraphOutcome, GraphQuery, discover};
use hq_vault::VaultClient;

/// States one real relation and one the messages never say.
struct States(String);

#[async_trait]
impl Summarizer for States {
    fn model(&self) -> String {
        "states".into()
    }
    async fn summarize(&self, _: &str, _: &str) -> Result<String> {
        Ok(format!(
            "Goal as of this block: open an account for the fees\n- chose Example Bank\n\nRELATIONS:\n\
             - decided_for: school fees -> Example Bank ({})\n\
             - decided_for: school fees -> Other Bank ({})\n\
             - invents_things: a -> b ({})",
            self.0, self.0, self.0
        ))
    }
}

fn edges(db: &Database, vault: &VaultClient) -> Vec<String> {
    let outcome = discover(
        db,
        vault,
        &GraphQuery {
            seeds: vec!["school fees".into()],
            ..Default::default()
        },
    );
    match outcome {
        GraphOutcome::Paths(r) => r
            .paths
            .iter()
            .flat_map(|p| p.edges.iter())
            .map(|e| format!("{} {} {} {}", e.from, e.relation, e.to, e.evidence_path))
            .collect(),
        GraphOutcome::Unavailable { reason, .. } => panic!("graph unavailable: {reason}"),
    }
}

#[tokio::test]
async fn a_folded_decision_is_a_graph_edge_until_its_message_goes() {
    let dir = tempfile::tempdir().unwrap();
    let vault = VaultClient::new(dir.path().to_path_buf()).unwrap();
    let db = Database::open(&dir.path().join("test.db")).unwrap();
    let (t, decided) = db
        .with_conn(|c| {
            let t = create_thread(c, "t", "user", "user")?.thread_id;
            let decided = add_message(
                c,
                &t,
                "user",
                "For the school fees we go with Example Bank, their fees are lower.",
            )?
            .message_id;
            add_message(c, &t, "assistant", "Noted.")?;
            for i in 0..12 {
                add_message(c, &t, "user", &format!("question {i}"))?;
                add_message(c, &t, "assistant", &format!("answer {i}"))?;
            }
            Ok((t, decided))
        })
        .unwrap();
    let cfg = ContextFoldConfig {
        tail_messages: 4,
        ..Default::default()
    };
    let summarizer = States(hq_db::thread_folds::message_ref(&decided));
    let done = fold::fold_thread(&db, &t, &cfg, &summarizer).await.unwrap();
    let relations: Vec<_> = done
        .iter()
        .flat_map(|d| match d {
            FoldOutcome::Folded { relations, .. } => relations.clone(),
            _ => Vec::new(),
        })
        .collect();
    assert_eq!(
        relations.len(),
        1,
        "only the relation the message states: {relations:?}"
    );

    // The stored summary keeps no RELATIONS section.
    let view = fold::load_view(&db, &t, &cfg).unwrap();
    assert!(!view.blocks[0].summary.contains("RELATIONS"));

    assert_eq!(fold::record_relations_in(&db, &vault, &done), 1);
    let found = edges(&db, &vault);
    assert!(
        found
            .iter()
            .any(|e| e.contains("decided_for") && e.contains("chat:")),
        "{found:?}"
    );

    // A context packet can carry the message itself and check a citation of it.
    let need = hq_memory::context_packet::ContextNeed {
        why: "what bank".into(),
        refs: vec![relations[0].evidence.clone()],
        ..Default::default()
    };
    let packet = hq_memory::context_packet::build_packet(
        &vault,
        Some(&db),
        "which bank for the fees",
        &need,
        chrono::Utc::now(),
    );
    let entry = packet
        .entries
        .iter()
        .find(|e| e.path.starts_with("chat:"))
        .expect("the chat message is in the packet");
    let cite = format!("Example Bank [{}]", entry.id);
    let report = hq_memory::context_packet::verify_citations_in(&cite, &packet, &vault, Some(&db));
    assert_eq!(report.verified, vec![entry.id.clone()]);

    db.with_conn(|c| hq_db::chat::delete_messages_from(c, &t, &decided))
        .unwrap();
    let report = hq_memory::context_packet::verify_citations_in(&cite, &packet, &vault, Some(&db));
    assert_eq!(
        report.changed,
        vec![entry.id.clone()],
        "the cited message is gone"
    );
    let found = edges(&db, &vault);
    assert!(
        !found.iter().any(|e| e.contains("decided_for")),
        "an edge whose message is gone must not count: {found:?}"
    );
}
