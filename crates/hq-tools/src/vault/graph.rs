//! Vault search, similarity, links, backlinks, tags, the entity graph, and reindexing.

use anyhow::Result;
use async_trait::async_trait;
use hq_db::Database;
use hq_vault::VaultClient;
use serde_json::{Value, json};
use std::sync::Arc;

use super::{validate_path, validate_path_in_vault};
use crate::registry::HqTool;

/// Multi-modal search tool: supports hybrid (FTS5 + embeddings), keyword, and semantic search modes.
pub struct VaultSearchTool {
    db: Arc<Database>,
}

impl VaultSearchTool {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl HqTool for VaultSearchTool {
    fn is_read_only(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "vault_search"
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Search the vault before answering anything about projects, decisions, or past events — the vault is ground truth and your training data is not. Try a narrower query before concluding something is not there. For \"what have we been working on\" style questions with no useful keyword, use mode: \"recent\" instead of guessing a query — it lists the most recently-touched notes directly, which a keyword search of an empty/weak query cannot do.",
        )
    }

    fn description(&self) -> &str {
        "Search vault notes by keyword, semantic, hybrid, or recency. Supports filtering by notebook and tags, and sorting by relevance or recency. Each result includes when it was last touched and any related notes."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query (omit or ignore when mode is \"recent\")" },
                "mode": { "type": "string", "enum": ["hybrid", "keyword", "semantic", "recent"], "description": "Search mode (default: hybrid). \"recent\" lists the most recently-touched notes, no query needed.", "default": "hybrid" },
                "sort_by": { "type": "string", "enum": ["relevance", "recency"], "description": "Result ordering (default: relevance)", "default": "relevance" },
                "limit": { "type": "integer", "description": "Max results (default 20)", "default": 20 },
                "notebook": { "type": "string", "description": "Optional notebook filter (e.g. Projects)" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Optional tag filters" }
            },
            "required": []
        })
    }

    fn category(&self) -> &str {
        "vault"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let mode = args
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("hybrid");
        let sort_by = args
            .get("sort_by")
            .and_then(|v| v.as_str())
            .unwrap_or("relevance")
            .to_string();
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
        let notebook_filter = args
            .get("notebook")
            .and_then(|v| v.as_str())
            .map(|s| s.to_lowercase());
        let tag_filters: Vec<String> = args
            .get("tags")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let db = self.db.clone();
        let mode_owned = mode.to_string();
        let query_owned = query.clone();
        let sort_by_owned = sort_by.clone();

        let (results, mtimes, related): (
            Vec<hq_core::types::SearchResult>,
            std::collections::HashMap<String, i64>,
            std::collections::HashMap<String, Vec<String>>,
        ) = tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| {
                let mut results = match mode_owned.as_str() {
                    "recent" => hq_db::search::recent_notes(conn, limit * 2)?,
                    "keyword" => hq_db::search::keyword_search(conn, &query_owned, limit * 2)?,
                    "semantic" => {
                        hq_db::search::hybrid_search(conn, &query_owned, None, limit * 2)?
                    }
                    _ => hq_db::search::hybrid_search(conn, &query_owned, None, limit * 2)?,
                };

                results.retain(|r| {
                    if let Some(ref nb) = notebook_filter
                        && r.notebook.to_lowercase() != *nb
                    {
                        return false;
                    }
                    if !tag_filters.is_empty() {
                        let has_tag = tag_filters
                            .iter()
                            .any(|t| r.tags.iter().any(|rt| rt.eq_ignore_ascii_case(t)));
                        if !has_tag {
                            return false;
                        }
                    }
                    true
                });

                let mut mtimes = std::collections::HashMap::new();
                for r in &results {
                    if let Ok(Some(m)) = hq_db::vault_cache::get_mtime(conn, &r.note_path) {
                        mtimes.insert(r.note_path.clone(), m);
                    }
                }

                if sort_by_owned == "recency" {
                    results.sort_by_key(|r| {
                        std::cmp::Reverse(mtimes.get(&r.note_path).copied().unwrap_or(0))
                    });
                }
                results.truncate(limit);

                let mut related = std::collections::HashMap::new();
                for r in &results {
                    if let Ok(paths) = hq_db::search::get_related_paths(conn, &r.note_path, 3)
                        && !paths.is_empty()
                    {
                        related.insert(r.note_path.clone(), paths);
                    }
                }

                Ok((results, mtimes, related))
            })
        })
        .await??;

        let now = chrono::Utc::now().timestamp();
        let result_values: Vec<Value> = results
            .into_iter()
            .map(|r| {
                let last_touched = mtimes
                    .get(&r.note_path)
                    .map(|mtime| humanize_age(now, *mtime));
                let related_paths = related.get(&r.note_path).cloned().unwrap_or_default();
                json!({
                    "path": r.note_path,
                    "title": r.title,
                    "notebook": r.notebook,
                    "snippet": r.snippet,
                    "relevance": r.relevance,
                    "match_type": format!("{:?}", r.match_type),
                    "tags": r.tags,
                    "last_touched": last_touched,
                    "related": related_paths,
                })
            })
            .collect();

        Ok(json!({ "results": result_values, "count": result_values.len() }))
    }
}

/// Format a note's last-cache-write time (epoch seconds) as a short relative
/// string (e.g. "3h ago") for display in search results.
fn humanize_age(now: i64, mtime: i64) -> String {
    let secs = (now - mtime).max(0);
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3_600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3_600)
    } else if secs < 2_592_000 {
        format!("{}d ago", secs / 86_400)
    } else {
        format!("{}mo ago", secs / 2_592_000)
    }
}

// ─── VaultFindSimilarTool ───────────────────────────────────────

/// Find semantically similar notes using stored vector embeddings.
pub struct VaultFindSimilarTool {
    db: Arc<Database>,
}

impl VaultFindSimilarTool {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl HqTool for VaultFindSimilarTool {
    fn is_read_only(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "vault_find_similar"
    }

    fn description(&self) -> &str {
        "Find notes semantically similar to a given note using embedding cosine similarity."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative note path" },
                "limit": { "type": "integer", "description": "Max results (default 10)", "default": 10 },
                "threshold": { "type": "number", "description": "Minimum similarity score 0.0-1.0 (default 0.5)", "default": 0.5 }
            },
            "required": ["path"]
        })
    }

    fn category(&self) -> &str {
        "vault"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
        let threshold = args
            .get("threshold")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5) as f32;

        validate_path(&path)?;

        let db = self.db.clone();
        let results: Vec<hq_core::types::SearchResult> = tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| hq_db::search::find_similar_notes(conn, &path, limit, threshold))
        })
        .await??;

        let items: Vec<Value> = results
            .into_iter()
            .map(|r| {
                json!({
                    "path": r.note_path,
                    "title": r.title,
                    "notebook": r.notebook,
                    "similarity": r.relevance,
                    "tags": r.tags,
                })
            })
            .collect();

        Ok(json!({ "similar": items, "count": items.len() }))
    }
}

// ─── VaultBacklinksTool ─────────────────────────────────────────

/// Find all notes linking to a targeted note.
pub struct VaultBacklinksTool {
    vault: Arc<VaultClient>,
    db: Option<Arc<Database>>,
}

impl VaultBacklinksTool {
    pub fn new(vault: Arc<VaultClient>, db: Option<Arc<Database>>) -> Self {
        Self { vault, db }
    }
}

#[async_trait]
impl HqTool for VaultBacklinksTool {
    fn is_read_only(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "vault_backlinks"
    }

    fn description(&self) -> &str {
        "Find all notes that link to a target note via [[WikiLinks]] or markdown links."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative path or note title" }
            },
            "required": ["path"]
        })
    }

    fn category(&self) -> &str {
        "vault"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        validate_path_in_vault(self.vault.vault_path(), path)?;

        let title_query = path.strip_suffix(".md").unwrap_or(path);
        let title_lower = title_query.to_lowercase();

        let vault = self.vault.clone();
        let db = self.db.clone();
        let path_owned = path.to_string();

        let backlinks = tokio::task::spawn_blocking(move || -> Result<Vec<Value>> {
            let mut matches = Vec::new();
            if let Some(ref db) = db {
                let fts_results =
                    db.with_conn(|conn| hq_db::search::keyword_search(conn, &title_lower, 50))?;
                for r in fts_results {
                    if r.note_path != path_owned
                        && let Ok(links) = vault.extract_links(&r.note_path)
                        && links.iter().any(|l| {
                            l.to_lowercase() == title_lower || l.eq_ignore_ascii_case(&path_owned)
                        })
                    {
                        matches.push(json!({
                            "path": r.note_path,
                            "title": r.title,
                            "snippet": r.snippet
                        }));
                    }
                }
            } else {
                let all_notes = vault.list_notes_recursive("")?;
                for n_path in all_notes {
                    if n_path != path_owned
                        && let Ok(links) = vault.extract_links(&n_path)
                        && links.iter().any(|l| {
                            l.to_lowercase() == title_lower || l.eq_ignore_ascii_case(&path_owned)
                        })
                    {
                        matches.push(json!({ "path": n_path }));
                    }
                }
            }
            Ok(matches)
        })
        .await??;

        Ok(json!({ "target": path, "backlinks": backlinks, "count": backlinks.len() }))
    }
}

// ─── VaultLinksTool ─────────────────────────────────────────────

/// Extract outgoing links from a note.
pub struct VaultLinksTool {
    vault: Arc<VaultClient>,
}

impl VaultLinksTool {
    pub fn new(vault: Arc<VaultClient>) -> Self {
        Self { vault }
    }
}

#[async_trait]
impl HqTool for VaultLinksTool {
    fn is_read_only(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "vault_links"
    }

    fn description(&self) -> &str {
        "Extract all outgoing [[WikiLinks]] and markdown links from a specified note."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative note path" }
            },
            "required": ["path"]
        })
    }

    fn category(&self) -> &str {
        "vault"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        validate_path_in_vault(self.vault.vault_path(), path)?;

        let vault = self.vault.clone();
        let path_owned = path.to_string();

        let links = tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
            vault.extract_links(&path_owned)
        })
        .await??;

        Ok(json!({ "path": path, "links": links, "count": links.len() }))
    }
}

// ─── VaultTagsTool ──────────────────────────────────────────────

/// Inspect tag usage cloud or find notes matching a tag.
pub struct VaultTagsTool {
    db: Arc<Database>,
}

impl VaultTagsTool {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl HqTool for VaultTagsTool {
    fn is_read_only(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "vault_tags"
    }

    fn description(&self) -> &str {
        "Inspect all tags across the vault with usage frequencies, or find notes matching a tag."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tag": { "type": "string", "description": "Optional tag name to filter notes" }
            }
        })
    }

    fn category(&self) -> &str {
        "vault"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let tag = args.get("tag").and_then(|v| v.as_str());

        let db = self.db.clone();
        if let Some(tag_str) = tag {
            let tag_owned = tag_str.to_string();
            let paths: Vec<String> = tokio::task::spawn_blocking(move || {
                db.with_conn(|conn| hq_db::search::get_tagged_note_paths(conn, &tag_owned))
            })
            .await??;
            Ok(json!({ "tag": tag_str, "notes": paths, "count": paths.len() }))
        } else {
            let tags_map =
                tokio::task::spawn_blocking(move || db.with_conn(hq_db::search::get_all_tags))
                    .await??;
            Ok(json!({ "tags": tags_map, "count": tags_map.len() }))
        }
    }
}

// ─── MemoryEntityGraphTool ──────────────────────────────────────

/// Visualize entity connections and spreading activation.
pub struct MemoryEntityGraphTool {
    db: Arc<Database>,
    vault: Option<Arc<VaultClient>>,
}

impl MemoryEntityGraphTool {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db, vault: None }
    }

    /// Enables `mode: "paths"`, which re-reads each edge's source note.
    pub fn with_vault(mut self, vault: Arc<VaultClient>) -> Self {
        self.vault = Some(vault);
        self
    }

    async fn paths(&self, args: &Value) -> Result<Value> {
        let Some(vault) = self.vault.clone() else {
            anyhow::bail!("paths mode needs a vault; this registry has none");
        };
        let mut query: hq_memory::graph_paths::GraphQuery =
            serde_json::from_value(args.clone()).unwrap_or_default();
        if let Some(hops) = args.get("hops").and_then(|v| v.as_u64()) {
            query.max_hops = hops as usize;
        }
        let db = self.db.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            hq_memory::graph_paths::discover(&db, &vault, &query)
        })
        .await?;
        Ok(serde_json::to_value(outcome)?)
    }
}

#[async_trait]
impl HqTool for MemoryEntityGraphTool {
    fn is_read_only(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "memory_entity_graph"
    }

    fn description(&self) -> &str {
        "Explore connections between entities in memory. Default mode uses spreading activation from seed entities to list related concepts. mode \"paths\" returns bounded relationship chains (for example project <- criticism <- decision <- task), each edge re-checked against its source note and labelled verified or tentative; when the graph index is stale or degraded it says so and points to vault_search."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "seeds": { "type": "array", "items": { "type": "string" }, "description": "Seed entity names to start traversal from" },
                "hops": { "type": "integer", "description": "Max traversal depth (default 2; paths mode default 3, capped at 4)" },
                "limit": { "type": "integer", "description": "Max results (default 10)", "default": 10 },
                "mode": { "type": "string", "enum": ["neighborhood", "paths"], "description": "neighborhood (default) or source-backed relationship paths" },
                "max_paths": { "type": "integer", "description": "Paths mode: maximum chains returned (default 5)" }
            },
            "required": ["seeds"]
        })
    }

    fn category(&self) -> &str {
        "memory"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        if args.get("mode").and_then(|v| v.as_str()) == Some("paths") {
            return self.paths(&args).await;
        }
        let seeds: Vec<String> = args
            .get("seeds")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let hops = args.get("hops").and_then(|v| v.as_u64()).unwrap_or(2) as usize;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

        let db = self.db.clone();
        let results: Vec<hq_memory::types::ActivatedEntity> =
            tokio::task::spawn_blocking(move || {
                hq_memory::entity_graph::spreading_activation(&db, &seeds, hops, 0.1, limit)
            })
            .await??;

        Ok(json!({ "entities": results, "count": results.len() }))
    }
}

// ─── VaultReindexTool ───────────────────────────────────────────

/// Force a full FTS rebuild, bypassing the daemon's 30-minute incremental
/// tick — for right after a bulk vault write (migration, rsync) that would
/// otherwise sit unsearchable for up to 30 minutes.
pub struct VaultReindexTool {
    db: Arc<Database>,
    notebooks_dir: std::path::PathBuf,
}

impl VaultReindexTool {
    pub fn new(db: Arc<Database>, vault_path: std::path::PathBuf) -> Self {
        Self {
            db,
            notebooks_dir: vault_path.join("Notebooks"),
        }
    }
}

#[async_trait]
impl HqTool for VaultReindexTool {
    fn name(&self) -> &str {
        "vault_reindex"
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Call this right after writing many notes at once (a migration, a bulk import) if vault_search comes back empty for content you just wrote — the background indexer only runs every 30 minutes.",
        )
    }

    fn description(&self) -> &str {
        "Force an immediate full rebuild of the vault's keyword search index, instead of waiting on the background indexer's 30-minute cycle."
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    fn category(&self) -> &str {
        "vault"
    }

    async fn execute(&self, _args: Value) -> Result<Value> {
        let db = self.db.clone();
        let notebooks_dir = self.notebooks_dir.clone();
        let (indexed, errors) = tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| hq_db::search::rebuild_index(conn, &notebooks_dir))
        })
        .await??;

        Ok(json!({ "indexed": indexed, "errors": errors }))
    }
}
