//! Maintenance of the derived entity index (FR-063).
//!
//! `_graph/*.md` concept pages are the source of truth. The `entity_nodes` and
//! `entity_edges` tables are a cache of them: rebuilt atomically, patched one
//! page at a time, reconciled against the vault, and marked degraded when any
//! of that fails so readers fall back instead of trusting a stale graph.
//!
//! A typed relation is one list line on a concept page:
//! `- criticizes: [[project-apollo]] (src: Notebooks/Meetings/review.md)`.
//! Any other wikilink becomes an untyped `linked` edge.

use crate::concept_pages::{canonical_key, canonical_slug};
use anyhow::Result;
use chrono::Utc;
use hq_core::types::Note;
use hq_db::Database;
use hq_vault::VaultClient;
use regex::Regex;
use rusqlite::{Connection, params};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

pub const GRAPH_DIR: &str = "_graph";
const ARCHIVE_PREFIX: &str = "_graph/_archive/";
pub const STATUS_OK: &str = "ok";
pub const STATUS_DEGRADED: &str = "degraded";
pub const CONF_SOURCED: &str = "sourced";
pub const CONF_STATED: &str = "stated";
pub const CONF_LINKED: &str = "linked";
pub const REL_LINKED: &str = "linked";

/// One relation read off a concept page.
#[derive(Debug, Clone, PartialEq)]
pub struct PageEdge {
    pub relation: String,
    pub target_slug: String,
    pub evidence_path: String,
    pub confidence: &'static str,
    pub weight: f64,
}

fn typed_line_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?m)^[ \t]*[-*][ \t]+([a-z][a-z_]{1,31}):[ \t]*\[\[([^\]|#]+)(?:[|#][^\]]*)?\]\](?:[ \t]*\(src:[ \t]*([^)]+)\))?[ \t]*$",
        )
        .expect("valid typed relation regex")
    })
}

fn wikilink_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[\[([^\]|#]+)").expect("valid wikilink regex"))
}

fn safe_vault_path(p: &str) -> bool {
    let p = p.trim();
    !p.is_empty() && !p.starts_with('/') && !p.contains("..") && p.ends_with(".md")
}

/// Read typed and plain edges from a page body. `page_path` is the fallback evidence.
pub fn parse_page_edges(page_path: &str, body: &str) -> Vec<PageEdge> {
    let mut edges = Vec::new();
    for cap in typed_line_re().captures_iter(body) {
        let src = cap
            .get(3)
            .map(|m| m.as_str().trim())
            .filter(|s| safe_vault_path(s));
        edges.push(PageEdge {
            relation: cap[1].to_string(),
            target_slug: canonical_slug(&cap[2]),
            evidence_path: src.unwrap_or(page_path).to_string(),
            confidence: if src.is_some() {
                CONF_SOURCED
            } else {
                CONF_STATED
            },
            weight: 1.0,
        });
    }
    let plain = typed_line_re().replace_all(body, "");
    let mut counts: HashMap<String, usize> = HashMap::new();
    for cap in wikilink_re().captures_iter(&plain) {
        *counts.entry(canonical_slug(&cap[1])).or_insert(0) += 1;
    }
    for (target_slug, n) in counts {
        edges.push(PageEdge {
            relation: REL_LINKED.to_string(),
            target_slug,
            evidence_path: page_path.to_string(),
            confidence: CONF_LINKED,
            weight: n as f64,
        });
    }
    edges
}

fn page_slug(rel_path: &str) -> String {
    rel_path
        .strip_prefix("_graph/")
        .unwrap_or(rel_path)
        .trim_end_matches(".md")
        .to_string()
}

/// `mtime_nanos:len`, cheap enough to check every page on every query.
fn stamp(vault: &VaultClient, rel_path: &str) -> Option<String> {
    let meta = std::fs::metadata(vault.vault_path().join(rel_path)).ok()?;
    let nanos = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(format!("{nanos}:{}", meta.len()))
}

pub fn list_pages(vault: &VaultClient) -> Result<Vec<String>> {
    Ok(vault
        .list_notes_recursive(GRAPH_DIR)?
        .into_iter()
        .filter(|p| !p.starts_with(ARCHIVE_PREFIX))
        .collect())
}

pub fn set_state(db: &Database, status: &str, error: Option<&str>) -> Result<()> {
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO entity_index_state (id, status, error, updated_at) VALUES (1, ?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET status = ?1, error = ?2, updated_at = ?3",
            params![status, error, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    })
}

/// Best effort: recording a failure must never mask the failure itself.
fn mark_degraded(db: &Database, err: &anyhow::Error) {
    let _ = set_state(db, STATUS_DEGRADED, Some(&err.to_string()));
}

pub fn index_status(db: &Database) -> Result<Option<(String, Option<String>)>> {
    db.with_conn(|conn| {
        Ok(conn
            .query_row(
                "SELECT status, error FROM entity_index_state WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok())
    })
}

fn upsert_node(conn: &Connection, note: &Note, page_path: &str, page_stamp: &str) -> Result<i64> {
    let entity_type = note
        .frontmatter
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let mentions = note
        .frontmatter
        .get("source_refs")
        .and_then(|v| v.as_sequence())
        .map_or(1, |s| s.len().max(1)) as i64;
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO entity_nodes (canonical, display_name, entity_type, mention_count, created_at, updated_at, page_path, page_stamp)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7)
         ON CONFLICT(canonical) DO UPDATE SET display_name = ?2, entity_type = ?3,
            mention_count = ?4, updated_at = ?5, page_path = ?6, page_stamp = ?7",
        params![canonical_key(&note.title), note.title, entity_type, mentions, now, page_path, page_stamp],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM entity_nodes WHERE canonical = ?1",
        [canonical_key(&note.title)],
        |r| r.get(0),
    )?)
}

fn insert_edges(
    conn: &Connection,
    vault: &VaultClient,
    source_id: i64,
    edges: &[PageEdge],
    slug_to_id: &HashMap<String, i64>,
) -> Result<usize> {
    let now = Utc::now().to_rfc3339();
    let mut written = 0usize;
    for edge in edges {
        let Some(&target_id) = slug_to_id.get(&edge.target_slug) else {
            continue;
        };
        if target_id == source_id {
            continue;
        }
        // Typed relations keep their direction; plain links stay undirected (min, max).
        let (s, t) = if edge.relation == REL_LINKED {
            (source_id.min(target_id), source_id.max(target_id))
        } else {
            (source_id, target_id)
        };
        let evidence_stamp = stamp(vault, &edge.evidence_path).map(|_| {
            std::fs::metadata(vault.vault_path().join(&edge.evidence_path))
                .and_then(|m| m.modified())
                .map(|t| chrono::DateTime::<Utc>::from(t).to_rfc3339())
                .unwrap_or_default()
        });
        conn.execute(
            "INSERT OR REPLACE INTO entity_edges
             (source_id, target_id, relationship, weight, source_memory_id, updated_at, evidence_path, evidence_updated_at, confidence)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8)",
            params![s, t, edge.relation, edge.weight, now, edge.evidence_path, evidence_stamp, edge.confidence],
        )?;
        written += 1;
    }
    Ok(written)
}

struct LoadedPage {
    rel_path: String,
    slug: String,
    note: Note,
    stamp: String,
}

fn load_pages(vault: &VaultClient) -> Result<Vec<LoadedPage>> {
    let mut pages = Vec::new();
    for rel_path in list_pages(vault)? {
        let note = vault.read_note(&rel_path)?;
        let page_stamp = stamp(vault, &rel_path).unwrap_or_default();
        pages.push(LoadedPage {
            slug: page_slug(&rel_path),
            rel_path,
            note,
            stamp: page_stamp,
        });
    }
    Ok(pages)
}

/// Replace the whole index from the vault in one transaction. Pages are read
/// first, so a vault read failure leaves the old rows untouched but degraded.
pub fn rebuild(db: &Database, vault: &VaultClient) -> Result<(usize, usize)> {
    let result = rebuild_inner(db, vault);
    match &result {
        Ok(_) => set_state(db, STATUS_OK, None)?,
        Err(e) => mark_degraded(db, e),
    }
    result
}

fn rebuild_inner(db: &Database, vault: &VaultClient) -> Result<(usize, usize)> {
    let pages = load_pages(vault)?;
    db.with_conn(|conn| {
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM entity_edges", [])?;
        tx.execute("DELETE FROM entity_nodes", [])?;
        let mut slug_to_id = HashMap::new();
        for page in &pages {
            let id = upsert_node(&tx, &page.note, &page.rel_path, &page.stamp)?;
            slug_to_id.insert(page.slug.clone(), id);
        }
        let mut edge_count = 0usize;
        for page in &pages {
            let edges = parse_page_edges(&page.rel_path, &page.note.content);
            edge_count += insert_edges(&tx, vault, slug_to_id[&page.slug], &edges, &slug_to_id)?;
        }
        tx.commit()?;
        Ok((pages.len(), edge_count))
    })
}

fn slug_map(conn: &Connection) -> Result<HashMap<String, i64>> {
    let mut stmt =
        conn.prepare("SELECT id, page_path FROM entity_nodes WHERE page_path IS NOT NULL")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    let mut map = HashMap::new();
    for row in rows {
        let (id, path) = row?;
        map.insert(page_slug(&path), id);
    }
    Ok(map)
}

fn delete_node(conn: &Connection, node_id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM entity_edges WHERE source_id = ?1 OR target_id = ?1",
        [node_id],
    )?;
    conn.execute("DELETE FROM entity_nodes WHERE id = ?1", [node_id])?;
    Ok(())
}

/// Idempotently apply one page event (create, edit, move target, or delete).
/// A failure marks the index degraded so readers stop trusting it.
pub fn reindex_page(db: &Database, vault: &VaultClient, rel_path: &str) -> Result<()> {
    let result = reindex_page_inner(db, vault, rel_path);
    if let Err(e) = &result {
        mark_degraded(db, e);
    }
    result
}

fn reindex_page_inner(db: &Database, vault: &VaultClient, rel_path: &str) -> Result<()> {
    let page = if vault.note_exists(rel_path) {
        Some((
            vault.read_note(rel_path)?,
            stamp(vault, rel_path).unwrap_or_default(),
        ))
    } else {
        None
    };
    let is_new = db.with_conn(|conn| {
        Ok(conn
            .query_row(
                "SELECT 1 FROM entity_nodes WHERE page_path = ?1",
                [rel_path],
                |_| Ok(()),
            )
            .is_err())
    })? && page.is_some();
    db.with_conn(|conn| {
        let tx = conn.unchecked_transaction()?;
        let existing: Option<i64> = tx
            .query_row("SELECT id FROM entity_nodes WHERE page_path = ?1", [rel_path], |r| r.get(0))
            .ok();
        let Some((note, page_stamp)) = page else {
            if let Some(id) = existing {
                delete_node(&tx, id)?;
            }
            tx.commit()?;
            return Ok(());
        };
        let id = upsert_node(&tx, &note, rel_path, &page_stamp)?;
        // Typed edges leave this node; plain links are stored (min, max), so they are found by evidence.
        tx.execute(
            "DELETE FROM entity_edges
             WHERE (source_id = ?1 AND relationship != ?2) OR (evidence_path = ?3 AND relationship = ?2)",
            params![id, REL_LINKED, rel_path],
        )?;
        let slugs = slug_map(&tx)?;
        insert_edges(&tx, vault, id, &parse_page_edges(rel_path, &note.content), &slugs)?;
        tx.commit()?;
        Ok(())
    })?;
    if is_new {
        reindex_referrers(db, vault, rel_path)?;
    }
    Ok(())
}

/// Pages that linked to a page before it existed have no edge yet; re-read them.
fn reindex_referrers(db: &Database, vault: &VaultClient, new_page: &str) -> Result<()> {
    let needle = format!("[[{}", page_slug(new_page));
    for path in list_pages(vault)? {
        if path == new_page {
            continue;
        }
        if vault.read_note(&path)?.content.contains(&needle) {
            reindex_page_inner(db, vault, &path)?;
        }
    }
    Ok(())
}

/// How the index compares with the vault right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexFreshness {
    Fresh,
    Degraded(String),
    Stale {
        changed: usize,
        missing_from_index: usize,
        gone_from_vault: usize,
    },
}

struct Drift {
    changed: Vec<String>,
    added: Vec<String>,
    removed: Vec<String>,
}

fn compute_drift(db: &Database, vault: &VaultClient) -> Result<Drift> {
    let indexed: HashMap<String, Option<String>> = db.with_conn(|conn| {
        let mut stmt = conn.prepare("SELECT page_path, page_stamp FROM entity_nodes")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
            ))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (path, stamp) = row?;
            map.insert(path.unwrap_or_default(), stamp);
        }
        Ok(map)
    })?;
    let pages = list_pages(vault)?;
    let on_disk: HashSet<&String> = pages.iter().collect();
    let mut drift = Drift {
        changed: vec![],
        added: vec![],
        removed: vec![],
    };
    for path in &pages {
        match indexed.get(path) {
            None => drift.added.push(path.clone()),
            Some(s) if s.as_deref() != stamp(vault, path).as_deref() => {
                drift.changed.push(path.clone())
            }
            Some(_) => {}
        }
    }
    for path in indexed.keys() {
        if !on_disk.contains(path) {
            drift.removed.push(path.clone());
        }
    }
    Ok(drift)
}

/// Cheap, read-only check used before every graph query.
/// Reason reported before the first build; `graph_paths::discover` builds on first use.
pub const NEVER_BUILT: &str = "the graph index has never been built";

pub fn check_freshness(db: &Database, vault: &VaultClient) -> IndexFreshness {
    match index_status(db) {
        Ok(Some((status, error))) if status == STATUS_DEGRADED => {
            return IndexFreshness::Degraded(
                error.unwrap_or_else(|| "a previous update failed".into()),
            );
        }
        Ok(Some(_)) => {}
        Ok(None) => return IndexFreshness::Degraded(NEVER_BUILT.into()),
        Err(e) => return IndexFreshness::Degraded(format!("index state unreadable: {e}")),
    }
    match compute_drift(db, vault) {
        Ok(d) if d.changed.is_empty() && d.added.is_empty() && d.removed.is_empty() => {
            IndexFreshness::Fresh
        }
        Ok(d) => IndexFreshness::Stale {
            changed: d.changed.len(),
            missing_from_index: d.added.len(),
            gone_from_vault: d.removed.len(),
        },
        Err(e) => IndexFreshness::Degraded(format!("vault unreadable: {e}")),
    }
}

/// Apply only the drift between vault and index. Returns the number of pages touched.
pub fn reconcile(db: &Database, vault: &VaultClient) -> Result<usize> {
    let drift = compute_drift(db, vault).inspect_err(|e| mark_degraded(db, e))?;
    let touched = drift.changed.len() + drift.added.len() + drift.removed.len();
    for path in drift.changed.iter().chain(&drift.added) {
        reindex_page(db, vault, path)?;
    }
    for path in &drift.removed {
        if path.is_empty() {
            db.with_conn(|conn| {
                conn.execute("DELETE FROM entity_edges WHERE source_id IN (SELECT id FROM entity_nodes WHERE page_path IS NULL) OR target_id IN (SELECT id FROM entity_nodes WHERE page_path IS NULL)", [])?;
                conn.execute("DELETE FROM entity_nodes WHERE page_path IS NULL", [])?;
                Ok(())
            })?;
        } else {
            reindex_page(db, vault, path)?;
        }
    }
    set_state(db, STATUS_OK, None)?;
    Ok(touched)
}
