//! Bounded, source-backed multi-hop discovery over the entity index (FR-063).
//!
//! The graph only suggests where to look. Every edge on a returned path is
//! re-checked against the note that supports it, edges that fail the check are
//! dropped, and weakly supported ones are labelled tentative. Callers fetch the
//! source passages themselves before presenting any claim.

use crate::concept_pages::canonical_key;
use crate::graph_index::{
    CONF_LINKED, CONF_SOURCED, GRAPH_DIR, IndexFreshness, NEVER_BUILT, check_freshness,
};
use anyhow::Result;
use hq_core::types::Note;
use hq_db::Database;
use hq_vault::VaultClient;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const DEFAULT_MAX_HOPS: usize = 3;
pub const HARD_MAX_HOPS: usize = 4;
pub const DEFAULT_MAX_PATHS: usize = 5;
pub const DEFAULT_MAX_EXPANSIONS: usize = 500;
pub const DEFAULT_MAX_CHARS: usize = 1500;
const VERIFY_ATTEMPTS_PER_PATH: usize = 3;
const FACTOR_SOURCED: f64 = 1.0;
const FACTOR_STATED: f64 = 0.8;
const FACTOR_LINKED: f64 = 0.4;
const GENERATED_PREFIX: &str = "_graph/";
const CITATION_KEYS: &[&str] = &["source_refs", "evidence"];
pub const UNSUPPORTED_CAVEAT: &str = "unsupported: an edge rests only on a generated concept page that cites no readable source note";
const MAX_SOURCES_PER_EDGE: usize = 3;
const REL_SUPERSEDES: &str = "supersedes";
const REL_CONTRADICTS: &str = "contradicts";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphQuery {
    pub seeds: Vec<String>,
    pub max_hops: usize,
    pub max_paths: usize,
    /// Upper bound on node expansions, so cyclic or dense data always terminates.
    pub max_expansions: usize,
    /// Upper bound on the rendered size of all returned paths.
    pub max_chars: usize,
}

impl Default for GraphQuery {
    fn default() -> Self {
        Self {
            seeds: Vec::new(),
            max_hops: DEFAULT_MAX_HOPS,
            max_paths: DEFAULT_MAX_PATHS,
            max_expansions: DEFAULT_MAX_EXPANSIONS,
            max_chars: DEFAULT_MAX_CHARS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeStatus {
    /// An external note was re-read and still mentions both endpoints.
    Verified,
    /// Stated or linked on a concept page only, with no independent source note.
    Tentative,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathNode {
    pub name: String,
    pub page_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathEdge {
    pub from: String,
    pub to: String,
    pub relation: String,
    pub evidence_path: String,
    /// Evidence note modified time, read at query time.
    pub evidence_updated_at: String,
    /// Original notes a generated concept page cites for this edge, each re-read and
    /// still naming an endpoint. Empty unless `evidence_path` is a `_graph/` page.
    #[serde(default)]
    pub sources: Vec<String>,
    pub status: EdgeStatus,
    /// True when the path walks the edge against its stored direction.
    pub reversed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPath {
    pub nodes: Vec<PathNode>,
    pub edges: Vec<PathEdge>,
    pub score: f64,
    pub caveats: Vec<String>,
}

impl GraphPath {
    /// One line a reader can audit: `A <-[criticizes]- B -[follows]-> C`.
    pub fn describe(&self) -> String {
        let mut out = self.nodes[0].name.clone();
        for (edge, node) in self.edges.iter().zip(self.nodes.iter().skip(1)) {
            let label = match edge.status {
                EdgeStatus::Verified => edge.relation.clone(),
                EdgeStatus::Tentative => format!("{}?", edge.relation),
            };
            let arrow = if edge.reversed {
                format!(" <-[{label}]- ")
            } else {
                format!(" -[{label}]-> ")
            };
            out.push_str(&arrow);
            out.push_str(&node.name);
        }
        out
    }

    /// Notes safe to show as evidence. A generated `_graph/` page is never one of
    /// them: its cited source notes stand in for it, or nothing does.
    pub fn evidence_paths(&self) -> Vec<String> {
        let mut seen = Vec::new();
        for edge in &self.edges {
            let notes: &[String] = if is_generated(&edge.evidence_path) {
                &edge.sources
            } else {
                std::slice::from_ref(&edge.evidence_path)
            };
            for note in notes {
                if !seen.contains(note) {
                    seen.push(note.clone());
                }
            }
        }
        seen
    }

    /// True when some edge rests only on a generated page that cites no usable source.
    pub fn has_unsupported_edge(&self) -> bool {
        self.edges
            .iter()
            .any(|e| is_generated(&e.evidence_path) && e.sources.is_empty())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphResult {
    pub paths: Vec<GraphPath>,
    pub truncated: bool,
    /// Unresolved seeds, dropped paths and other things a reader should know.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum GraphOutcome {
    Paths(GraphResult),
    /// The index cannot be trusted right now; callers fall back to vault search.
    Unavailable {
        reason: String,
        fallback: String,
    },
}

#[derive(Debug, Clone)]
struct RawEdge {
    source_id: i64,
    target_id: i64,
    relation: String,
    evidence_path: Option<String>,
    confidence: String,
}

#[derive(Debug, Clone)]
struct NodeInfo {
    name: String,
    page_path: Option<String>,
}

fn node_info(db: &Database, id: i64) -> Result<NodeInfo> {
    db.with_conn(|conn| {
        Ok(conn.query_row(
            "SELECT display_name, page_path FROM entity_nodes WHERE id = ?1",
            [id],
            |r| {
                Ok(NodeInfo {
                    name: r.get(0)?,
                    page_path: r.get(1)?,
                })
            },
        )?)
    })
}

fn edges_of(db: &Database, id: i64) -> Result<Vec<RawEdge>> {
    db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT source_id, target_id, relationship, evidence_path, confidence
             FROM entity_edges WHERE source_id = ?1 OR target_id = ?1",
        )?;
        let rows = stmt.query_map([id], |r| {
            Ok(RawEdge {
                source_id: r.get(0)?,
                target_id: r.get(1)?,
                relation: r.get(2)?,
                evidence_path: r.get(3)?,
                confidence: r.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    })
}

fn resolve_seed(db: &Database, seed: &str) -> Option<i64> {
    db.with_conn(|conn| {
        Ok(conn
            .query_row(
                "SELECT id FROM entity_nodes WHERE canonical = ?1",
                [canonical_key(seed)],
                |r| r.get(0),
            )
            .ok())
    })
    .ok()
    .flatten()
}

fn edge_factor(confidence: &str) -> f64 {
    match confidence {
        c if c == CONF_SOURCED => FACTOR_SOURCED,
        c if c == CONF_LINKED => FACTOR_LINKED,
        _ => FACTOR_STATED,
    }
}

/// Nodes visited so far plus the edges taken, each with whether it was walked backwards.
type PathState = (Vec<i64>, Vec<(RawEdge, bool)>);

struct Candidate {
    nodes: Vec<i64>,
    edges: Vec<(RawEdge, bool)>,
    score: f64,
}

/// Depth-first over simple paths. A node never repeats within one path, and
/// `max_expansions` bounds the total work, so cycles cannot loop.
fn enumerate(
    db: &Database,
    seed: i64,
    q: &GraphQuery,
    truncated: &mut bool,
) -> Result<Vec<Candidate>> {
    let max_hops = q.max_hops.clamp(1, HARD_MAX_HOPS);
    let mut best: HashMap<i64, Candidate> = HashMap::new();
    let mut stack: Vec<PathState> = vec![(vec![seed], vec![])];
    let mut expansions = 0usize;
    while let Some((nodes, edges)) = stack.pop() {
        if edges.len() >= max_hops {
            continue;
        }
        if expansions >= q.max_expansions {
            *truncated = true;
            break;
        }
        expansions += 1;
        let here = *nodes.last().expect("path is never empty");
        for edge in edges_of(db, here)? {
            let (next, reversed) = if edge.source_id == here {
                (edge.target_id, false)
            } else {
                (edge.source_id, true)
            };
            if nodes.contains(&next) {
                continue;
            }
            let mut n2 = nodes.clone();
            n2.push(next);
            let mut e2 = edges.clone();
            e2.push((edge, reversed));
            let product: f64 = e2.iter().map(|(e, _)| edge_factor(&e.confidence)).product();
            let score = product / e2.len() as f64;
            let better = best.get(&next).is_none_or(|c| score > c.score);
            if better {
                best.insert(
                    next,
                    Candidate {
                        nodes: n2.clone(),
                        edges: e2.clone(),
                        score,
                    },
                );
            }
            stack.push((n2, e2));
        }
    }
    Ok(best.into_values().collect())
}

/// Re-read the note behind one edge. `Err` carries the reason it was dropped.
fn verify_edge(
    vault: &VaultClient,
    edge: &RawEdge,
    from: &NodeInfo,
    to: &NodeInfo,
) -> std::result::Result<(EdgeStatus, String, Vec<String>), String> {
    let Some(evidence) = &edge.evidence_path else {
        return Err("edge has no supporting note".to_string());
    };
    let note = vault
        .read_note(evidence)
        .map_err(|_| format!("supporting note {evidence} is missing or unreadable"))?;
    let modified = note.modified_at.to_rfc3339();
    let body = note.content.to_lowercase();
    if evidence.starts_with(GRAPH_DIR) {
        let other = if page_slug_of(evidence) == page_slug_opt(&from.page_path) {
            to
        } else {
            from
        };
        let needle = format!("[[{}", page_slug_opt(&other.page_path));
        if !body.contains(&needle) {
            return Err(format!("{evidence} no longer links to {}", other.name));
        }
        let sources = cited_sources(vault, &note, from, to);
        return Ok((EdgeStatus::Tentative, modified, sources));
    }
    if body.contains(&from.name.to_lowercase()) && body.contains(&to.name.to_lowercase()) {
        Ok((EdgeStatus::Verified, modified, Vec::new()))
    } else {
        Err(format!(
            "{evidence} no longer mentions both {} and {}",
            from.name, to.name
        ))
    }
}

fn is_generated(path: &str) -> bool {
    path.starts_with(GENERATED_PREFIX)
}

/// Source notes a concept page cites in frontmatter (`source_refs` or `evidence`),
/// kept only if they are ordinary vault notes that still exist and name an endpoint.
fn cited_sources(vault: &VaultClient, page: &Note, from: &NodeInfo, to: &NodeInfo) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for key in CITATION_KEYS {
        let Some(value) = page.frontmatter.get(*key) else {
            continue;
        };
        let items: Vec<&str> = match value {
            serde_yaml::Value::String(s) => vec![s.as_str()],
            serde_yaml::Value::Sequence(seq) => seq.iter().filter_map(|v| v.as_str()).collect(),
            _ => Vec::new(),
        };
        for item in items {
            let item = item.trim();
            let plausible = item.ends_with(".md")
                && !item.starts_with('/')
                && !item.contains("..")
                && !is_generated(item);
            if !plausible || out.iter().any(|o| o == item) {
                continue;
            }
            let Ok(note) = vault.read_note(item) else {
                continue;
            };
            let body = note.content.to_lowercase();
            if body.contains(&from.name.to_lowercase()) || body.contains(&to.name.to_lowercase()) {
                out.push(item.to_string());
            }
            if out.len() >= MAX_SOURCES_PER_EDGE {
                return out;
            }
        }
    }
    out
}

fn page_slug_of(path: &str) -> String {
    path.strip_prefix("_graph/")
        .unwrap_or(path)
        .trim_end_matches(".md")
        .to_string()
}

fn page_slug_opt(path: &Option<String>) -> String {
    path.as_deref().map(page_slug_of).unwrap_or_default()
}

/// Superseded or contradicted nodes on a path are surfaced, not silently used.
fn node_caveats(db: &Database, id: i64, name: &str) -> Result<Vec<String>> {
    db.with_conn(|conn| {
        let mut out = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT n.display_name, e.relationship, e.evidence_path
             FROM entity_edges e JOIN entity_nodes n ON n.id = CASE WHEN e.source_id = ?1 THEN e.target_id ELSE e.source_id END
             WHERE (e.target_id = ?1 AND e.relationship = ?2)
                OR ((e.source_id = ?1 OR e.target_id = ?1) AND e.relationship = ?3)",
        )?;
        let rows = stmt.query_map(params![id, REL_SUPERSEDES, REL_CONTRADICTS], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?))
        })?;
        for row in rows {
            let (other, rel, src) = row?;
            let src = src.unwrap_or_else(|| "unknown source".into());
            out.push(if rel == REL_SUPERSEDES {
                format!("{name} is superseded by {other} (see {src})")
            } else {
                format!("{name} conflicts with {other} (see {src})")
            });
        }
        Ok(out)
    })
}

fn build_path(
    db: &Database,
    vault: &VaultClient,
    cand: &Candidate,
    notes: &mut Vec<String>,
) -> Result<Option<GraphPath>> {
    let infos: Vec<NodeInfo> = cand
        .nodes
        .iter()
        .map(|&id| node_info(db, id))
        .collect::<Result<_>>()?;
    let mut edges = Vec::new();
    for (i, (raw, reversed)) in cand.edges.iter().enumerate() {
        let (from, to) = if *reversed {
            (&infos[i + 1], &infos[i])
        } else {
            (&infos[i], &infos[i + 1])
        };
        match verify_edge(vault, raw, from, to) {
            Ok((status, modified, sources)) => edges.push(PathEdge {
                from: from.name.clone(),
                to: to.name.clone(),
                relation: raw.relation.clone(),
                evidence_path: raw.evidence_path.clone().unwrap_or_default(),
                evidence_updated_at: modified,
                sources,
                status,
                reversed: *reversed,
            }),
            Err(reason) => {
                notes.push(format!(
                    "dropped a path through {} to {}: {reason}",
                    from.name, to.name
                ));
                return Ok(None);
            }
        }
    }
    let mut caveats = Vec::new();
    for (id, info) in cand.nodes.iter().zip(&infos) {
        caveats.extend(node_caveats(db, *id, &info.name)?);
    }
    caveats.dedup();
    if edges
        .iter()
        .any(|e| is_generated(&e.evidence_path) && e.sources.is_empty())
    {
        caveats.push(UNSUPPORTED_CAVEAT.to_string());
    }
    Ok(Some(GraphPath {
        nodes: infos
            .into_iter()
            .map(|i| PathNode {
                name: i.name,
                page_path: i.page_path,
            })
            .collect(),
        edges,
        score: cand.score,
        caveats,
    }))
}

/// Run the query against the current index. Never trusts an unchecked index.
pub fn discover(db: &Database, vault: &VaultClient, q: &GraphQuery) -> GraphOutcome {
    match check_freshness(db, vault) {
        IndexFreshness::Fresh => {}
        IndexFreshness::Degraded(reason) if reason == NEVER_BUILT => {
            if let Some(unavailable) = recover_or_unavailable(
                db,
                vault,
                |db, vault| crate::graph_index::rebuild(db, vault).map(|_| ()),
                reason,
            ) {
                return unavailable;
            }
        }
        IndexFreshness::Degraded(reason) => return unavailable(reason),
        IndexFreshness::Stale {
            changed,
            missing_from_index,
            gone_from_vault,
        } => {
            let reason = format!(
                "index is stale ({changed} changed, {missing_from_index} unindexed, {gone_from_vault} removed pages)"
            );
            if let Some(unavailable) = recover_or_unavailable(
                db,
                vault,
                |db, vault| crate::graph_index::reconcile(db, vault).map(|_| ()),
                reason,
            ) {
                return unavailable;
            }
        }
    }
    match find_paths(db, vault, q) {
        Ok(result) => GraphOutcome::Paths(result),
        Err(e) => unavailable(format!("graph query failed: {e}")),
    }
}

/// One recovery attempt so a first query builds a missing index instead of reporting it
/// unavailable. `None` means the index is fresh again; otherwise the caller falls back to search.
fn recover_or_unavailable(
    db: &Database,
    vault: &VaultClient,
    repair: impl FnOnce(&Database, &VaultClient) -> Result<()>,
    reason: String,
) -> Option<GraphOutcome> {
    if let Err(e) = repair(db, vault) {
        return Some(unavailable(format!("{reason}; rebuild failed: {e}")));
    }
    match check_freshness(db, vault) {
        IndexFreshness::Fresh => None,
        _ => Some(unavailable(reason)),
    }
}

fn unavailable(reason: String) -> GraphOutcome {
    GraphOutcome::Unavailable {
        reason,
        fallback: "vault_search and direct references".to_string(),
    }
}

fn find_paths(db: &Database, vault: &VaultClient, q: &GraphQuery) -> Result<GraphResult> {
    let mut result = GraphResult::default();
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut seen_seeds = HashSet::new();
    for seed in &q.seeds {
        match resolve_seed(db, seed) {
            Some(id) if seen_seeds.insert(id) => {
                candidates.extend(enumerate(db, id, q, &mut result.truncated)?);
            }
            Some(_) => {}
            None => result
                .notes
                .push(format!("seed {seed:?} is not in the graph")),
        }
    }
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let budget = q.max_paths * VERIFY_ATTEMPTS_PER_PATH;
    let mut used_chars = 0usize;
    for cand in candidates.iter().take(budget) {
        if result.paths.len() >= q.max_paths {
            result.truncated = true;
            break;
        }
        let Some(path) = build_path(db, vault, cand, &mut result.notes)? else {
            continue;
        };
        used_chars += path.describe().chars().count();
        if used_chars > q.max_chars && !result.paths.is_empty() {
            result.truncated = true;
            break;
        }
        result.paths.push(path);
    }
    if candidates.len() > budget {
        result.truncated = true;
    }
    Ok(result)
}

#[cfg(test)]
#[path = "graph_paths_tests.rs"]
mod tests;
