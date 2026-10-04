use super::*;
use crate::concept_pages::{add_relation, derive_entity_index};
use crate::context_packet::{ContextNeed, GapKind, build_packet};
use crate::graph_index::{self, IndexFreshness};
use tempfile::TempDir;

struct Fixture {
    _dir: TempDir,
    vault: VaultClient,
    db: Database,
}

fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let vault = VaultClient::new(dir.path().to_path_buf()).unwrap();
    let db = Database::open(&dir.path().join("test.db")).unwrap();
    Fixture {
        _dir: dir,
        vault,
        db,
    }
}

fn put(f: &Fixture, path: &str, body: &str) {
    let full = f.vault.vault_path().join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, body).unwrap();
}

fn query(seed: &str) -> GraphQuery {
    GraphQuery {
        seeds: vec![seed.to_string()],
        ..Default::default()
    }
}

fn paths(outcome: GraphOutcome) -> GraphResult {
    match outcome {
        GraphOutcome::Paths(r) => r,
        GraphOutcome::Unavailable { reason, .. } => panic!("graph unavailable: {reason}"),
    }
}

/// project <- criticism <- decision <- task, each hop backed by its own note.
fn chain_fixture() -> Fixture {
    let f = fixture();
    put(
        &f,
        "Notebooks/Meetings/review.md",
        "Jane Doe criticised Project Apollo for slow onboarding.",
    );
    put(
        &f,
        "Notebooks/Decisions/d1.md",
        "Decision Pause Rollout answers Jane Doe and her criticism.",
    );
    put(
        &f,
        "Notebooks/Tasks/t1.md",
        "Open task Fix Billing Rounding follows Decision Pause Rollout.",
    );
    add_relation(
        &f.vault,
        "Jane Doe",
        "criticizes",
        "Project Apollo",
        Some("Notebooks/Meetings/review.md"),
    )
    .unwrap();
    add_relation(
        &f.vault,
        "Decision Pause Rollout",
        "responds_to",
        "Jane Doe",
        Some("Notebooks/Decisions/d1.md"),
    )
    .unwrap();
    add_relation(
        &f.vault,
        "Fix Billing Rounding",
        "follows",
        "Decision Pause Rollout",
        Some("Notebooks/Tasks/t1.md"),
    )
    .unwrap();
    derive_entity_index(&f.db, &f.vault).unwrap();
    f
}

#[test]
fn multi_hop_chain_is_found_with_sources_that_direct_lookup_misses() {
    let f = chain_fixture();
    let result = paths(discover(&f.db, &f.vault, &query("Project Apollo")));
    let chain = result
        .paths
        .iter()
        .find(|p| p.nodes.last().unwrap().name == "Fix Billing Rounding")
        .expect("three-hop chain to the task");
    assert_eq!(chain.edges.len(), 3);
    assert_eq!(
        chain.describe(),
        "Project Apollo <-[criticizes]- Jane Doe <-[responds_to]- Decision Pause Rollout <-[follows]- Fix Billing Rounding"
    );
    for edge in &chain.edges {
        assert_eq!(edge.status, EdgeStatus::Verified);
        assert!(edge.evidence_path.starts_with("Notebooks/"));
        assert!(!edge.evidence_updated_at.is_empty());
    }

    f.db.with_conn(|c| {
        for (p, t, body) in [
            (
                "Notebooks/Meetings/review.md",
                "review",
                "Jane Doe criticised Project Apollo for slow onboarding.",
            ),
            (
                "Notebooks/Tasks/t1.md",
                "t1",
                "Open task Fix Billing Rounding follows Decision Pause Rollout.",
            ),
        ] {
            hq_db::search::index_note(c, p, t, body, "")?;
        }
        let hits = hq_db::search::keyword_search(c, "Apollo", 10)?;
        assert!(hits.iter().all(|h| h.note_path != "Notebooks/Tasks/t1.md"));
        Ok(())
    })
    .unwrap();
}

#[test]
fn page_only_edges_are_tentative_and_unsupported_edges_are_dropped() {
    let f = fixture();
    add_relation(&f.vault, "Alpha", "depends_on", "Beta", None).unwrap();
    add_relation(
        &f.vault,
        "Alpha",
        "blocks",
        "Gamma",
        Some("Notebooks/gone.md"),
    )
    .unwrap();
    derive_entity_index(&f.db, &f.vault).unwrap();
    let result = paths(discover(&f.db, &f.vault, &query("Alpha")));
    let to_beta = result
        .paths
        .iter()
        .find(|p| p.nodes.last().unwrap().name == "Beta")
        .unwrap();
    assert_eq!(to_beta.edges[0].status, EdgeStatus::Tentative);
    assert!(to_beta.describe().contains("depends_on?"));
    assert!(
        result
            .paths
            .iter()
            .all(|p| p.nodes.last().unwrap().name != "Gamma")
    );
    assert!(
        result
            .notes
            .iter()
            .any(|n| n.contains("missing or unreadable"))
    );
}

#[test]
fn source_that_stops_mentioning_the_entities_invalidates_the_edge() {
    let f = fixture();
    put(&f, "Notebooks/s.md", "Alpha relates to Beta here.");
    add_relation(
        &f.vault,
        "Alpha",
        "depends_on",
        "Beta",
        Some("Notebooks/s.md"),
    )
    .unwrap();
    derive_entity_index(&f.db, &f.vault).unwrap();
    assert_eq!(
        paths(discover(&f.db, &f.vault, &query("Alpha")))
            .paths
            .len(),
        1
    );
    put(&f, "Notebooks/s.md", "Rewritten, nothing relevant.");
    let result = paths(discover(&f.db, &f.vault, &query("Alpha")));
    assert!(result.paths.is_empty());
    assert!(
        result
            .notes
            .iter()
            .any(|n| n.contains("no longer mentions"))
    );
}

#[test]
fn cycles_terminate_and_hop_path_and_expansion_bounds_hold() {
    let f = fixture();
    for (a, b) in [("A", "B"), ("B", "C"), ("C", "A"), ("C", "D")] {
        put(&f, "Notebooks/e.md", "A B C D linked");
        add_relation(&f.vault, a, "related_to", b, Some("Notebooks/e.md")).unwrap();
    }
    derive_entity_index(&f.db, &f.vault).unwrap();

    let q = GraphQuery {
        seeds: vec!["A".into()],
        max_hops: 10,
        ..Default::default()
    };
    let result = paths(discover(&f.db, &f.vault, &q));
    for p in &result.paths {
        let names: HashSet<&String> = p.nodes.iter().map(|n| &n.name).collect();
        assert_eq!(names.len(), p.nodes.len(), "no node repeats in a path");
        assert!(p.edges.len() <= HARD_MAX_HOPS);
    }

    let one_hop = GraphQuery {
        seeds: vec!["A".into()],
        max_hops: 1,
        ..Default::default()
    };
    assert!(
        paths(discover(&f.db, &f.vault, &one_hop))
            .paths
            .iter()
            .all(|p| p.edges.len() == 1)
    );

    let capped = GraphQuery {
        seeds: vec!["A".into()],
        max_paths: 1,
        ..Default::default()
    };
    let r = paths(discover(&f.db, &f.vault, &capped));
    assert_eq!(r.paths.len(), 1);
    assert!(r.truncated);

    let starved = GraphQuery {
        seeds: vec!["A".into()],
        max_expansions: 1,
        ..Default::default()
    };
    assert!(paths(discover(&f.db, &f.vault, &starved)).truncated);

    let tiny = GraphQuery {
        seeds: vec!["A".into()],
        max_chars: 5,
        ..Default::default()
    };
    assert_eq!(paths(discover(&f.db, &f.vault, &tiny)).paths.len(), 1);
}

#[test]
fn superseded_and_contradicted_nodes_carry_caveats() {
    let f = fixture();
    put(&f, "Notebooks/e.md", "Plan v2 Plan v1 Claim X Claim Y");
    add_relation(
        &f.vault,
        "Plan v2",
        "supersedes",
        "Plan v1",
        Some("Notebooks/e.md"),
    )
    .unwrap();
    add_relation(
        &f.vault,
        "Claim X",
        "contradicts",
        "Claim Y",
        Some("Notebooks/e.md"),
    )
    .unwrap();
    add_relation(&f.vault, "Task", "follows", "Plan v1", None).unwrap();
    add_relation(&f.vault, "Task", "cites", "Claim Y", None).unwrap();
    derive_entity_index(&f.db, &f.vault).unwrap();
    let result = paths(discover(&f.db, &f.vault, &query("Task")));
    let joined: Vec<String> = result
        .paths
        .iter()
        .flat_map(|p| p.caveats.clone())
        .collect();
    assert!(
        joined
            .iter()
            .any(|c| c.contains("Plan v1 is superseded by Plan v2"))
    );
    assert!(
        joined
            .iter()
            .any(|c| c.contains("Claim Y conflicts with Claim X"))
    );
}

#[test]
fn edits_moves_and_deletes_never_leave_silently_trusted_edges() {
    let f = chain_fixture();
    assert_eq!(check_freshness_of(&f), IndexFreshness::Fresh);

    add_relation(&f.vault, "Project Apollo", "related_to", "Roadmap", None).unwrap();
    assert!(matches!(
        check_freshness_of(&f),
        IndexFreshness::Stale { .. }
    ));
    // A query reconciles the drift first, so a stale edge is never served and the query still works.
    assert!(matches!(
        discover(&f.db, &f.vault, &query("Project Apollo")),
        GraphOutcome::Paths(_)
    ));
    assert_eq!(check_freshness_of(&f), IndexFreshness::Fresh);
    graph_index::reindex_page(&f.db, &f.vault, "_graph/project-apollo.md").unwrap();
    graph_index::reindex_page(&f.db, &f.vault, "_graph/roadmap.md").unwrap();
    assert_eq!(check_freshness_of(&f), IndexFreshness::Fresh);
    let r = paths(discover(&f.db, &f.vault, &query("Project Apollo")));
    assert!(
        r.paths
            .iter()
            .any(|p| p.nodes.last().unwrap().name == "Roadmap")
    );

    // Idempotent: repeating the same event changes nothing.
    let before = edge_count(&f);
    graph_index::reindex_page(&f.db, &f.vault, "_graph/project-apollo.md").unwrap();
    assert_eq!(edge_count(&f), before);

    // Delete: the node and every edge touching it go away.
    std::fs::remove_file(f.vault.vault_path().join("_graph/jane-doe.md")).unwrap();
    graph_index::reindex_page(&f.db, &f.vault, "_graph/jane-doe.md").unwrap();
    let r = paths(discover(&f.db, &f.vault, &query("Project Apollo")));
    assert!(
        r.paths
            .iter()
            .all(|p| p.nodes.iter().all(|n| n.name != "Jane Doe"))
    );

    // Move: same page content under a new name.
    let from = f.vault.vault_path().join("_graph/roadmap.md");
    std::fs::rename(
        &from,
        f.vault.vault_path().join("_graph/product-roadmap.md"),
    )
    .unwrap();
    assert_ne!(check_freshness_of(&f), IndexFreshness::Fresh);
    graph_index::reconcile(&f.db, &f.vault).unwrap();
    assert_eq!(check_freshness_of(&f), IndexFreshness::Fresh);
}

fn check_freshness_of(f: &Fixture) -> IndexFreshness {
    graph_index::check_freshness(&f.db, &f.vault)
}

fn edge_count(f: &Fixture) -> i64 {
    f.db.with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM entity_edges", [], |r| r.get(0))?))
        .unwrap()
}

#[test]
fn failed_rebuild_keeps_old_rows_but_marks_the_index_degraded_until_recovery() {
    let f = chain_fixture();
    let before = edge_count(&f);
    put(&f, "_graph/broken.md", "placeholder");
    std::fs::write(
        f.vault.vault_path().join("_graph/broken.md"),
        [0xff, 0xfe, 0xfd],
    )
    .unwrap();

    assert!(graph_index::rebuild(&f.db, &f.vault).is_err());
    assert_eq!(
        edge_count(&f),
        before,
        "transaction never started, old rows intact"
    );
    match discover(&f.db, &f.vault, &query("Project Apollo")) {
        GraphOutcome::Unavailable { fallback, .. } => assert!(fallback.contains("vault_search")),
        GraphOutcome::Paths(_) => panic!("a degraded index must not be served"),
    }

    std::fs::remove_file(f.vault.vault_path().join("_graph/broken.md")).unwrap();
    graph_index::rebuild(&f.db, &f.vault).unwrap();
    assert!(
        !paths(discover(&f.db, &f.vault, &query("Project Apollo")))
            .paths
            .is_empty()
    );
}

#[test]
fn rebuild_recovers_a_wiped_index_from_the_vault() {
    let f = chain_fixture();
    f.db.with_conn(|c| {
        c.execute("DELETE FROM entity_edges", [])?;
        c.execute("DELETE FROM entity_nodes", [])?;
        Ok(())
    })
    .unwrap();
    assert_ne!(check_freshness_of(&f), IndexFreshness::Fresh);
    graph_index::reconcile(&f.db, &f.vault).unwrap();
    assert_eq!(check_freshness_of(&f), IndexFreshness::Fresh);
    assert!(
        !paths(discover(&f.db, &f.vault, &query("Project Apollo")))
            .paths
            .is_empty()
    );
}

#[test]
fn packets_use_graph_sources_and_degrade_to_search_when_the_graph_is_down() {
    let f = chain_fixture();
    let need = ContextNeed {
        graph_seeds: vec!["Project Apollo".into()],
        queries: vec!["apollo".into()],
        ..Default::default()
    };
    f.db.with_conn(|c| {
        hq_db::search::index_note(
            c,
            "Notebooks/Meetings/review.md",
            "review",
            "Jane Doe criticised Project Apollo",
            "",
        )
    })
    .unwrap();
    let now = chrono::Utc::now();
    let p = build_packet(&f.vault, Some(&f.db), "review apollo", &need, now);
    let task = p
        .entries
        .iter()
        .find(|e| e.path == "Notebooks/Tasks/t1.md")
        .expect("graph-only source");
    assert_eq!(task.via, "graph");
    assert!(task.relevance.contains("Fix Billing Rounding"));
    assert!(p.render().contains("via=\"graph\""));

    graph_index::set_state(&f.db, graph_index::STATUS_DEGRADED, Some("boom")).unwrap();
    let p = build_packet(&f.vault, Some(&f.db), "review apollo", &need, now);
    assert!(p.entries.iter().all(|e| e.path != "Notebooks/Tasks/t1.md"));
    assert!(
        p.entries
            .iter()
            .any(|e| e.path == "Notebooks/Meetings/review.md"),
        "search fallback"
    );
    assert!(
        p.gaps
            .iter()
            .any(|g| g.kind == GapKind::GraphUnavailable && g.detail.contains("boom"))
    );

    let none = build_packet(&f.vault, None, "x", &need, now);
    assert!(
        none.gaps
            .iter()
            .any(|g| g.kind == GapKind::GraphUnavailable)
    );
}

#[test]
fn the_first_query_builds_a_never_built_index_instead_of_reporting_it_unavailable() {
    let f = chain_fixture();
    f.db.with_conn(|c| {
        c.execute("DELETE FROM entity_index_state", [])?;
        Ok(())
    })
    .unwrap();
    assert!(matches!(
        check_freshness_of(&f),
        IndexFreshness::Degraded(_)
    ));
    assert!(matches!(
        discover(&f.db, &f.vault, &query("Project Apollo")),
        GraphOutcome::Paths(_)
    ));
    assert_eq!(check_freshness_of(&f), IndexFreshness::Fresh);
}

fn concept_fixture(cited: &[&str]) -> Fixture {
    let f = fixture();
    put(
        &f,
        "Notebooks/s1.md",
        "Alpha depends on Beta, per the spec.",
    );
    put(&f, "Notebooks/s2.md", "Beta was chosen by Alpha's owner.");
    add_relation(&f.vault, "Alpha", "depends_on", "Beta", None).unwrap();
    for src in cited {
        crate::concept_pages::upsert_concept_page(&f.vault, "Alpha", "unknown", &[], src).unwrap();
    }
    derive_entity_index(&f.db, &f.vault).unwrap();
    f
}

#[test]
fn natural_language_query_with_punctuation_still_builds_a_packet() {
    let f = fixture();
    put(&f, "Notebooks/k.md", "AcmeCorp is a talent platform.");
    f.db.with_conn(|c| {
        hq_db::search::index_note(
            c,
            "Notebooks/k.md",
            "k",
            "AcmeCorp is a talent platform.",
            "",
        )
    })
    .unwrap();
    let need = ContextNeed {
        queries: vec!["What is AcmeCorp?".to_string()],
        ..Default::default()
    };
    let p = build_packet(
        &f.vault,
        Some(&f.db),
        "What is AcmeCorp?",
        &need,
        chrono::Utc::now(),
    );
    assert!(p.entries.iter().any(|e| e.path == "Notebooks/k.md"));
    assert!(p.gaps.iter().all(|g| g.kind != GapKind::RetrievalFailed));
}

fn graph_need() -> ContextNeed {
    ContextNeed {
        graph_seeds: vec!["Alpha".to_string()],
        ..Default::default()
    }
}

#[test]
fn concept_page_evidence_is_replaced_by_the_source_notes_it_cites() {
    let f = concept_fixture(&["Notebooks/s1.md", "Notebooks/s2.md", "Notebooks/gone.md"]);
    let result = paths(discover(&f.db, &f.vault, &query("Alpha")));
    let path = result
        .paths
        .iter()
        .find(|p| p.nodes.last().unwrap().name == "Beta")
        .unwrap();
    assert_eq!(path.edges[0].status, EdgeStatus::Tentative);
    assert_eq!(
        path.evidence_paths(),
        vec!["Notebooks/s1.md", "Notebooks/s2.md"]
    );

    let p = build_packet(
        &f.vault,
        Some(&f.db),
        "alpha",
        &graph_need(),
        chrono::Utc::now(),
    );
    let graph_entries: Vec<_> = p.entries.iter().filter(|e| e.via == "graph").collect();
    assert_eq!(graph_entries.len(), 2);
    assert!(graph_entries.iter().all(|e| !e.path.starts_with("_graph/")));
    assert!(
        graph_entries
            .iter()
            .all(|e| e.relevance.contains("depends_on?"))
    );
}

#[test]
fn concept_page_without_evidence_is_reported_unsupported_and_never_packed() {
    let f = concept_fixture(&[]);
    let result = paths(discover(&f.db, &f.vault, &query("Alpha")));
    let path = result
        .paths
        .iter()
        .find(|p| p.nodes.last().unwrap().name == "Beta")
        .unwrap();
    assert!(path.has_unsupported_edge());
    assert!(path.evidence_paths().is_empty());
    assert!(path.caveats.iter().any(|c| c.contains("unsupported")));

    let p = build_packet(
        &f.vault,
        Some(&f.db),
        "alpha",
        &graph_need(),
        chrono::Utc::now(),
    );
    assert!(p.entries.iter().all(|e| !e.path.starts_with("_graph/")));
    assert!(p.gaps.iter().any(|g| g.detail.contains("unsupported")));
}
