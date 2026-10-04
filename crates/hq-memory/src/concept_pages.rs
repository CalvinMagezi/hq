//! Concept pages: markdown-first replacement for the hidden `entity_graph`
//! SQLite tables. `entity_nodes`/`entity_edges` become a rebuildable cache
//! derived from these pages via `derive_entity_index`, not the source of
//! truth — the pages themselves are ground truth.

use crate::entity_graph;
use anyhow::Result;
use hq_core::types::Note;
use hq_db::Database;
use hq_vault::VaultClient;
use std::collections::HashMap;

pub fn canonical_slug(name: &str) -> String {
    let lower = name.to_lowercase();
    let trimmed = lower.trim();
    let mut slug = String::with_capacity(trimmed.len());
    let mut last_was_dash = false;
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_was_dash = false;
        } else if !last_was_dash && !slug.is_empty() {
            slug.push('-');
            last_was_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "unnamed".to_string()
    } else {
        slug
    }
}

pub fn concept_page_rel_path(name: &str) -> String {
    format!("_graph/{}.md", canonical_slug(name))
}

/// The dedup key stored in `entity_nodes.canonical` — byte-for-byte the same
/// normalization `entity_graph::resolve_entity` already uses. Deliberately
/// NOT the same as `canonical_slug`: every existing reader (spreading_activation)
/// looks entities up by this scheme (spaces preserved),
/// so using the filesystem slug here would silently break multi-word lookups.
pub fn canonical_key(name: &str) -> String {
    name.to_lowercase().trim().to_string()
}

pub struct UpsertOutcome {
    pub path: String,
    pub created: bool,
}

fn frontmatter_string_list(note: &Note, key: &str) -> Vec<String> {
    note.frontmatter
        .get(key)
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn yaml_string_list(items: &[String]) -> serde_yaml::Value {
    serde_yaml::Value::Sequence(
        items
            .iter()
            .map(|s| serde_yaml::Value::String(s.clone()))
            .collect(),
    )
}

pub fn upsert_concept_page(
    vault: &VaultClient,
    name: &str,
    entity_type: &str,
    related: &[String],
    source_ref: &str,
) -> Result<UpsertOutcome> {
    let rel_path = concept_page_rel_path(name);
    let now = chrono::Utc::now().to_rfc3339();

    if vault.note_exists(&rel_path) {
        let mut note = vault.read_note(&rel_path)?;

        let mut refs = frontmatter_string_list(&note, "source_refs");
        if !refs.contains(&source_ref.to_string()) {
            refs.push(source_ref.to_string());
        }
        note.frontmatter
            .insert("source_refs".to_string(), yaml_string_list(&refs));
        note.frontmatter.insert(
            "updated".to_string(),
            serde_yaml::Value::String(now.clone()),
        );

        for r in related {
            let link = format!("[[{}]]", canonical_slug(r));
            if !note.content.contains(&link) {
                note.content
                    .push_str(&format!("\n\nAlso related to {link}."));
            }
        }

        vault.write_note(&rel_path, &note)?;
        Ok(UpsertOutcome {
            path: rel_path,
            created: false,
        })
    } else {
        let mut frontmatter = HashMap::new();
        frontmatter.insert(
            "type".to_string(),
            serde_yaml::Value::String(entity_type.to_string()),
        );
        frontmatter.insert("aliases".to_string(), yaml_string_list(&[]));
        frontmatter.insert(
            "created".to_string(),
            serde_yaml::Value::String(now.clone()),
        );
        frontmatter.insert("updated".to_string(), serde_yaml::Value::String(now));
        frontmatter.insert(
            "source_refs".to_string(),
            yaml_string_list(&[source_ref.to_string()]),
        );

        let mut content = format!("# {name}\n");
        if !related.is_empty() {
            let links: Vec<String> = related
                .iter()
                .map(|r| format!("[[{}]]", canonical_slug(r)))
                .collect();
            content.push_str(&format!("\nRelated to {}.\n", links.join(", ")));
        }

        let note = Note {
            title: name.to_string(),
            content,
            path: rel_path.clone(),
            frontmatter,
            note_type: None,
            tags: vec![],
            pinned: false,
            source: None,
            embedding_status: None,
            created_at: None,
            updated_at: None,
            modified_at: chrono::Utc::now(),
        };

        vault.write_note(&rel_path, &note)?;
        Ok(UpsertOutcome {
            path: rel_path,
            created: true,
        })
    }
}

pub fn archive_concept_page(vault: &VaultClient, name: &str, reason: &str) -> Result<()> {
    let rel_path = concept_page_rel_path(name);
    if !vault.note_exists(&rel_path) {
        return Ok(());
    }
    let note = vault.read_note(&rel_path)?;
    let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let slug = canonical_slug(name);
    let archive_rel_path = format!("_graph/_archive/{slug}-{timestamp}.md");

    let archived = Note {
        title: note.title.clone(),
        content: format!("<!-- archived: {reason} -->\n\n{}", note.content),
        path: archive_rel_path.clone(),
        frontmatter: note.frontmatter.clone(),
        note_type: None,
        tags: vec![],
        pinned: false,
        source: None,
        embedding_status: None,
        created_at: None,
        updated_at: None,
        modified_at: chrono::Utc::now(),
    };
    vault.write_note(&archive_rel_path, &archived)?;
    Ok(())
}

pub struct DeriveStats {
    pub nodes: usize,
    pub edges: usize,
}

/// Marker file proving the one-time `entity_graph` -> `_graph/` migration has
/// already run. Lives under `_system/`, not `_graph/`, specifically so it is
/// never subject to `_graph/`-scoped archival/pruning logic (see
/// `MemoryForgetter::archive_stale_concept_pages`), and follows this
/// codebase's existing convention for state marker files (e.g.
/// `_system/.telegram-auth-chat`).
const ENTITY_GRAPH_MIGRATION_MARKER: &str = "_system/.entity_graph_migrated";

/// One-time, idempotent migration: materializes any pre-existing
/// `entity_nodes`/`entity_edges` rows (written by the old direct-write path
/// in `ingester.rs`/`consolidator.rs`/`dream.rs`) into `_graph/` concept
/// pages. Called from `derive_entity_index` only when the
/// `ENTITY_GRAPH_MIGRATION_MARKER` sentinel is absent, so a real vault's
/// accumulated entity data survives the first truncate-and-rebuild after this
/// migration lands. Gating on the sentinel rather than "is `_graph/` empty
/// right now" matters: `_graph/` can legitimately become empty again long
/// after migration (e.g. the forgetter archives every remaining page in one
/// cycle), and re-running the backfill at that point would resurrect pages
/// that were deliberately archived.
pub fn backfill_concept_pages_from_entity_graph(
    db: &Database,
    vault: &VaultClient,
) -> Result<usize> {
    let nodes: Vec<(i64, String, String, String)> = db.with_conn(|conn| {
        let mut stmt =
            conn.prepare("SELECT id, canonical, display_name, entity_type FROM entity_nodes")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok::<_, anyhow::Error>(out)
    })?;

    if nodes.is_empty() {
        return Ok(0);
    }

    let id_to_name: HashMap<i64, String> = nodes
        .iter()
        .map(|(id, _, display_name, _)| (*id, display_name.clone()))
        .collect();

    for (id, _canonical, display_name, entity_type) in &nodes {
        upsert_concept_page(
            vault,
            display_name,
            entity_type,
            &[],
            "backfill:entity_graph",
        )?;

        let edges = entity_graph::get_edges_from(db, *id)?;
        // Edges are undirected but stored once with source_id < target_id
        // (entity_graph::upsert_edge canonicalizes direction). Only emit the
        // wikilink from the source side, otherwise both endpoints would link
        // to each other and derive_entity_index would double-count the edge.
        let related: Vec<String> = edges
            .iter()
            .filter(|edge| edge.source_id == *id)
            .filter_map(|edge| id_to_name.get(&edge.target_id).cloned())
            .collect();

        if !related.is_empty() {
            upsert_concept_page(
                vault,
                display_name,
                entity_type,
                &related,
                "backfill:entity_graph",
            )?;
        }
    }

    Ok(nodes.len())
}

pub fn derive_entity_index(db: &Database, vault: &VaultClient) -> Result<DeriveStats> {
    let marker_path = vault.vault_path().join(ENTITY_GRAPH_MIGRATION_MARKER);
    if !marker_path.exists() {
        backfill_concept_pages_from_entity_graph(db, vault)?;
        if let Some(parent) = marker_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&marker_path, chrono::Utc::now().to_rfc3339())?;
    }

    let (nodes, edges) = crate::graph_index::rebuild(db, vault)?;
    Ok(DeriveStats { nodes, edges })
}

/// Append a typed relation line to `from`'s page, creating either page if needed.
/// `evidence` is the vault-relative note that supports the claim.
pub fn add_relation(
    vault: &VaultClient,
    from: &str,
    relation: &str,
    to: &str,
    evidence: Option<&str>,
) -> Result<()> {
    let valid = relation.len() >= 2
        && relation.starts_with(|c: char| c.is_ascii_lowercase())
        && relation.chars().all(|c| c.is_ascii_lowercase() || c == '_');
    if !valid {
        anyhow::bail!("relation must be lowercase letters and underscores: {relation:?}");
    }
    upsert_concept_page(vault, to, "unknown", &[], "relation")?;
    let outcome = upsert_concept_page(vault, from, "unknown", &[], "relation")?;
    let mut note = vault.read_note(&outcome.path)?;
    let mut line = format!("- {relation}: [[{}]]", canonical_slug(to));
    if let Some(src) = evidence {
        line.push_str(&format!(" (src: {src})"));
    }
    if !note.content.contains(&line) {
        note.content.push_str(&format!("\n{line}\n"));
        vault.write_note(&outcome.path, &note)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::Database;
    use std::path::PathBuf;

    fn temp_vault() -> (PathBuf, VaultClient) {
        let dir = std::env::temp_dir().join(format!(
            "hq-concept-pages-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let vault = VaultClient::new(dir.clone()).unwrap();
        (dir, vault)
    }

    #[test]
    fn canonical_slug_matches_entity_graph_dedup_semantics() {
        // Same normalization entity_graph::resolve_entity uses (lowercase+trim),
        // plus filesystem-safe substitution. "Rust " and "rust" must collide.
        assert_eq!(canonical_slug("Rust "), canonical_slug("rust"));
        assert_eq!(canonical_slug("Agent HQ"), "agent-hq");
        assert_eq!(canonical_slug("  "), "unnamed");
    }

    #[test]
    fn derive_entity_index_reproduces_direct_write_path_for_a_co_occurring_pair() {
        // Characterization: today, ingester.rs resolves two co-occurring entity
        // names directly via entity_graph::resolve_entity + upsert_edge("co_occurs").
        // Capture what that produces, then prove the page-write + derive path
        // yields an equivalent entity_nodes/entity_edges state.
        let db = Database::open_memory().unwrap();

        // --- OLD (direct) path, captured as the target shape ---
        let old_a = crate::entity_graph::resolve_entity(&db, "Rust").unwrap();
        let old_b = crate::entity_graph::resolve_entity(&db, "Agent HQ").unwrap();
        crate::entity_graph::upsert_edge(&db, old_a, old_b, "co_occurs", None).unwrap();
        let old_node_count: i64 = db
            .with_conn(|conn| {
                conn.query_row("SELECT COUNT(*) FROM entity_nodes", [], |r| r.get(0))
                    .map_err(Into::into)
            })
            .unwrap();
        let old_edge_count: i64 = db
            .with_conn(|conn| {
                conn.query_row("SELECT COUNT(*) FROM entity_edges", [], |r| r.get(0))
                    .map_err(Into::into)
            })
            .unwrap();
        assert_eq!(old_node_count, 2);
        assert_eq!(old_edge_count, 1);

        // Reset to a clean DB for the NEW path so the two are compared fairly.
        let db2 = Database::open_memory().unwrap();
        let (_dir, vault) = temp_vault();

        // --- NEW (page-write + derive) path ---
        upsert_concept_page(
            &vault,
            "Rust",
            "tool",
            &["Agent HQ".to_string()],
            "memory:1",
        )
        .unwrap();
        upsert_concept_page(&vault, "Agent HQ", "tool", &[], "memory:1").unwrap();

        let stats = derive_entity_index(&db2, &vault).unwrap();
        assert_eq!(stats.nodes, 2);
        assert_eq!(stats.edges, 1, "one wikilink from rust.md -> agent-hq.md");

        let new_node_count: i64 = db2
            .with_conn(|conn| {
                conn.query_row("SELECT COUNT(*) FROM entity_nodes", [], |r| r.get(0))
                    .map_err(Into::into)
            })
            .unwrap();
        let new_edge_count: i64 = db2
            .with_conn(|conn| {
                conn.query_row("SELECT COUNT(*) FROM entity_edges", [], |r| r.get(0))
                    .map_err(Into::into)
            })
            .unwrap();
        assert_eq!(
            new_node_count, old_node_count,
            "same node count as direct-write path"
        );
        assert_eq!(
            new_edge_count, old_edge_count,
            "same edge count as direct-write path"
        );

        // The derived canonical keys must match what resolve_entity would have used,
        // so spreading_activation (a reader, unchanged) can still find these nodes
        // by the same seed name a caller already uses today. Deliberately checked
        // for BOTH fixture entities, not just "Rust" — "rust" happens to be
        // identical whether you lowercase-trim it or slug it, so it alone would
        // not catch a canonical/slug mismatch. "Agent HQ" is the case that does:
        // its canonical_key is "agent hq" (space) but its canonical_slug is
        // "agent-hq" (dash) — these must NOT be conflated in entity_nodes.canonical.
        let canonical_rust: String = db2
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT canonical FROM entity_nodes WHERE canonical = ?1",
                    ["rust"],
                    |r| r.get(0),
                )
                .map_err(Into::into)
            })
            .unwrap();
        assert_eq!(canonical_rust, "rust");

        let canonical_agent_hq: String = db2
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT canonical FROM entity_nodes WHERE canonical = ?1",
                    ["agent hq"],
                    |r| r.get(0),
                )
                .map_err(Into::into)
            })
            .unwrap();
        assert_eq!(
            canonical_agent_hq, "agent hq",
            "canonical must be canonical_key(\"Agent HQ\") = \"agent hq\" (space), \
             NOT canonical_slug(\"Agent HQ\") = \"agent-hq\" (dash) — a slug-based \
             canonical would silently break spreading_activation \
             lookups for every multi-word entity name"
        );
    }

    #[test]
    fn derive_entity_index_backfills_existing_entity_graph_before_first_truncate() {
        let db = Database::open_memory().unwrap();

        // Simulate pre-existing entity graph data from the OLD direct-write system,
        // written before this branch's _graph/ pages existed at all.
        let old_a = crate::entity_graph::resolve_entity(&db, "Legacy Entity A").unwrap();
        let old_b = crate::entity_graph::resolve_entity(&db, "Legacy Entity B").unwrap();
        crate::entity_graph::upsert_edge(&db, old_a, old_b, "co_occurs", None).unwrap();

        let (_dir, vault) = temp_vault();
        // _graph/ is empty at this point — no concept pages exist yet.

        // The first derive_entity_index call must NOT silently wipe the legacy data;
        // it must backfill pages from it first, then derive from those pages.
        let stats = derive_entity_index(&db, &vault).unwrap();

        assert_eq!(
            stats.nodes, 2,
            "backfilled legacy entities must survive the first derive"
        );
        assert_eq!(
            stats.edges, 1,
            "backfilled legacy edge must survive the first derive"
        );

        // Confirm concept pages were actually created on disk, not just re-inserted into SQLite.
        assert!(vault.note_exists(&concept_page_rel_path("Legacy Entity A")));
        assert!(vault.note_exists(&concept_page_rel_path("Legacy Entity B")));

        // Confirm entity_nodes still resolves the ORIGINAL multi-word canonical keys correctly
        // (not slugs) after going through the backfill+derive round-trip.
        let canonical: String = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT canonical FROM entity_nodes WHERE canonical = ?1",
                    ["legacy entity a"],
                    |r| r.get(0),
                )
                .map_err(Into::into)
            })
            .unwrap();
        assert_eq!(canonical, "legacy entity a");
    }

    #[test]
    fn upsert_concept_page_creates_then_updates_without_duplicating_source_refs() {
        let (_dir, vault) = temp_vault();
        let first = upsert_concept_page(&vault, "Rust", "tool", &[], "memory:1").unwrap();
        assert!(first.created);

        let second = upsert_concept_page(&vault, "Rust", "tool", &[], "memory:1").unwrap();
        assert!(!second.created);

        let note = vault.read_note(&concept_page_rel_path("Rust")).unwrap();
        let refs = note
            .frontmatter
            .get("source_refs")
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(
            refs.len(),
            1,
            "duplicate source_ref must not be appended twice"
        );
    }

    #[test]
    fn archive_concept_page_copies_content_before_any_caller_modifies_the_live_page() {
        let (_dir, vault) = temp_vault();
        upsert_concept_page(&vault, "Stale Concept", "concept", &[], "memory:1").unwrap();

        archive_concept_page(&vault, "Stale Concept", "superseded by merge").unwrap();

        let archive_dir = vault.vault_path().join("_graph").join("_archive");
        let entries: Vec<_> = std::fs::read_dir(&archive_dir).unwrap().collect();
        assert_eq!(entries.len(), 1, "exactly one archive file written");
        let archived_content =
            std::fs::read_to_string(entries.into_iter().next().unwrap().unwrap().path()).unwrap();
        assert!(archived_content.contains("superseded by merge"));
        assert!(archived_content.contains("Stale Concept"));

        // Live page must still exist and be unchanged by archiving alone.
        assert!(vault.note_exists(&concept_page_rel_path("Stale Concept")));
    }

    #[test]
    fn derive_entity_index_does_not_resurrect_pages_after_they_are_legitimately_archived() {
        let db = Database::open_memory().unwrap();

        // Simulate legacy entity graph data, same as the backfill test.
        let old_a = crate::entity_graph::resolve_entity(&db, "Legacy Entity A").unwrap();
        let old_b = crate::entity_graph::resolve_entity(&db, "Legacy Entity B").unwrap();
        crate::entity_graph::upsert_edge(&db, old_a, old_b, "co_occurs", None).unwrap();

        let (_dir, vault) = temp_vault();

        // First derive call: backfill runs, migration marker gets written, pages exist.
        let first_stats = derive_entity_index(&db, &vault).unwrap();
        assert_eq!(first_stats.nodes, 2);

        // Simulate the forgetter legitimately archiving every live page (e.g. both
        // became stale and disconnected in the same cycle — a realistic outcome).
        crate::concept_pages::archive_concept_page(&vault, "Legacy Entity A", "test archival")
            .unwrap();
        crate::concept_pages::archive_concept_page(&vault, "Legacy Entity B", "test archival")
            .unwrap();
        std::fs::remove_file(
            vault
                .vault_path()
                .join(concept_page_rel_path("Legacy Entity A")),
        )
        .unwrap();
        std::fs::remove_file(
            vault
                .vault_path()
                .join(concept_page_rel_path("Legacy Entity B")),
        )
        .unwrap();

        // _graph/ is now empty again, same precondition as the very first call —
        // but the migration already happened once. The second derive call must NOT
        // resurrect the archived pages.
        let second_stats = derive_entity_index(&db, &vault).unwrap();
        assert_eq!(
            second_stats.nodes, 0,
            "archived pages must not be resurrected by a second derive_entity_index call"
        );
        assert!(
            !vault.note_exists(&concept_page_rel_path("Legacy Entity A")),
            "legitimately archived page must stay archived, not reappear"
        );
    }
}
