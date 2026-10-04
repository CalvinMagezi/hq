//! MemoryConsolidator — the "brain during sleep" cycle.
//!
//! Runs periodically (default: every 30 minutes via daemon).
//! Takes unconsolidated memories, finds connections via topic clustering,
//! generates insights, then writes them back to:
//!   1. The consolidations table (SQLite)
//!   2. Notebooks/Memories/ as a markdown note (visible in the vault)
//!
//! Ported from vault-memory/src/consolidator.ts.

use anyhow::Result;
use hq_db::Database;
use hq_llm::provider::LlmProvider;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

use crate::db::{
    get_consolidation_history, get_unconsolidated_memories,
    store_consolidation,
};
use crate::llm_bridge::MemoryLlm;
use crate::types::{Connection, ConsolidationResult, Memory};

/// Max lines in the auto-generated insights section of MEMORY.md.
/// Matches Claude Code's MAX_ENTRYPOINT_LINES.
const MAX_INDEX_LINES: usize = 200;

/// Max total file size for MEMORY.md (bytes).
/// Matches Claude Code's 25KB cap.
const MAX_INDEX_BYTES: usize = 25 * 1024;

const CONSOLIDATE_SYSTEM: &str = r#"You are a memory consolidation agent for Agent-HQ, a personal AI hub.

You receive a list of recent memories from various agent harnesses (Claude Code, Gemini CLI, Discord relay, etc),
plus a snapshot of active project READMEs for grounding.

Your job is to:
1. Find meaningful connections between memories
2. Compare memories against project state — flag gaps, stale areas, or things that haven't been touched recently
3. Generate one specific, grounded insight (e.g. "Mobile app missing push notifications, last touched 14 days ago")

Return a JSON object with exactly:
{
  "connections": [
    { "from_id": 1, "to_id": 3, "relationship": "brief description of how they relate" }
  ],
  "insight": "One specific, grounded insight comparing memory to current project state"
}

Be specific. Prefer concrete project-level observations over generic patterns."#;

const META_SYSTEM: &str = r#"You are a meta-synthesis agent. Given a list of insights from different topic clusters,
identify the single most important cross-cutting pattern or connection that spans them all.
Return JSON: { "insight": "one concise cross-cluster insight", "connections": [] }"#;

/// Memory consolidator with topic clustering and cross-cluster synthesis.
pub struct MemoryConsolidator {
    db: Database,
    vault_path: PathBuf,
    /// `None` without a provider: consolidation is skipped, the index and
    /// MEMORY.md refresh still run.
    llm: Option<MemoryLlm>,
}

impl MemoryConsolidator {
    pub fn new(db: Database, vault_path: PathBuf) -> Self {
        Self {
            db,
            vault_path,
            llm: None,
        }
    }

    pub fn new_with_provider(
        db: Database,
        vault_path: PathBuf,
        provider: Arc<dyn LlmProvider>,
        model: String,
    ) -> Self {
        Self {
            db,
            vault_path,
            llm: Some(MemoryLlm::with_provider(provider, model)),
        }
    }

    /// Scan vault for project READMEs and return a compact grounding context string.
    ///
    /// Reads up to 8 READMEs from `Notebooks/Projects/*/README.md`, truncates each
    /// to 600 chars, and formats them as a block the LLM can compare against memories.
    fn load_project_context(&self) -> String {
        let projects_dir = self.vault_path.join("Notebooks").join("Projects");
        let mut sections: Vec<String> = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&projects_dir) {
            for entry in entries.flatten().take(8) {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let readme = path.join("README.md");
                if !readme.exists() {
                    continue;
                }
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                if let Ok(content) = std::fs::read_to_string(&readme) {
                    // Take first 600 chars to keep prompt size reasonable
                    let preview: String = content.chars().take(600).collect();
                    sections.push(format!("### {name}\n{preview}"));
                }
            }
        }

        if sections.is_empty() {
            return String::new();
        }

        format!(
            "## Active Project Snapshots\n\n{}",
            sections.join("\n\n---\n\n")
        )
    }

    /// Run one consolidation cycle using topic clustering.
    ///
    /// Inspired by hippocampal sharp-wave ripple replay: the brain doesn't replay
    /// all memories at once — it replays related memories in clusters, strengthening
    /// connections within each cluster before doing cross-cluster integration.
    ///
    /// Returns the final insight generated, or None if nothing to consolidate.
    pub async fn run_cycle(&self) -> Result<Option<String>> {
        let memories = get_unconsolidated_memories(&self.db, 30)?;

        if memories.is_empty() {
            info!("Consolidation skipped — no unconsolidated memories");
            return Ok(None);
        }

        if self.llm.is_none() {
            warn!("no memory LLM provider configured, skipping consolidation");
            return Ok(None);
        }

        // ── Load project READMEs for grounding context ────────────────────
        let project_context = self.load_project_context();
        if !project_context.is_empty() {
            info!(
                projects = project_context.len(),
                "consolidation: loaded project grounding context"
            );
        }

        // ── Cluster memories by topic (hippocampal replay grouping) ──────
        let clusters = cluster_by_topic(&memories);
        let mut sorted_clusters: Vec<_> = clusters.into_iter().collect();
        sorted_clusters.sort_by_key(|a| std::cmp::Reverse(a.1.len())); // largest first

        let mut cluster_insights: Vec<String> = Vec::new();
        let mut last_insight: Option<String> = None;
        let mut insight_collector: Vec<(String, String)> = Vec::new();

        for (topic, cluster) in &sorted_clusters {
            if cluster.len() < 2 {
                continue; // skip singletons
            }
            if let Some(insight) = self
                .consolidate_cluster(cluster, topic, &project_context, &mut insight_collector)
                .await?
            {
                cluster_insights.push(insight.clone());
                last_insight = Some(insight);
            }
        }

        // ── Cross-cluster synthesis (schema integration) ─────────────────
        if cluster_insights.len() >= 2
            && let Some(meta) = self.synthesize_clusters(&cluster_insights).await
        {
            last_insight = Some(meta);
        }

        // Fallback: if no clusters with 2+ memories, consolidate all together
        if cluster_insights.is_empty() && memories.len() >= 3 {
            let capped: Vec<_> = memories.into_iter().take(15).collect();
            last_insight = self
                .consolidate_cluster(&capped, "general", &project_context, &mut insight_collector)
                .await?;
        }

        // ── Batch-flush all insight notes collected during this run ──────
        self.flush_insight_notes(insight_collector).await;

        // ── Insight chaining: check if new insight evolves a previous one ──
        if let Some(ref insight) = last_insight
            && let Some(chained) = self.chain_insights(insight).await?
        {
            info!(chained = %chained.chars().take(80).collect::<String>(), "evolved understanding");
            // Use the chained insight as the final output
            let _ = self.refresh_entity_index();
            return Ok(Some(chained));
        }

        // Re-derive entity_nodes/entity_edges from concept pages so the
        // SQLite cache stays in sync after every consolidation cycle.
        let _ = self.refresh_entity_index();
        Ok(last_insight)
    }

    /// Update _system/MEMORY.md with top insights from consolidation history.
    ///
    /// Inspired by Claude Code's dream Phase 4 (Prune & Index):
    /// - Caps the auto-generated section at MAX_INDEX_LINES lines
    /// - Caps total file size at MAX_INDEX_BYTES
    /// - Removes stale/superseded insights
    /// - Keeps most recent + highest-quality insights
    pub fn refresh_memory_file(&self) -> Result<()> {
        let history = get_consolidation_history(&self.db, 50)?;
        if history.is_empty() {
            return Ok(());
        }

        let memory_path = self.vault_path.join("_system").join("MEMORY.md");
        if !memory_path.exists() {
            // Seed the file so subsequent cycles can maintain it.
            if let Some(parent) = memory_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(
                &memory_path,
                "---\nnoteType: system-file\nfileName: memory\nversion: 1\npinned: true\n---\n\n# HQ Memory\n\n",
            )?;
            info!("created _system/MEMORY.md (was missing)");
        }

        let existing = std::fs::read_to_string(&memory_path)?;

        // Build insight lines, capped at MAX_INDEX_LINES
        let section_header = "## Agent Insights (Auto-Generated)";
        let insight_lines: Vec<String> = history
            .iter()
            .take(MAX_INDEX_LINES)
            .map(|c| {
                // Truncate each line to ~150 chars (matching Claude Code's index style)
                let preview: String = c.insight.chars().take(140).collect();
                format!(
                    "- [{}] {}",
                    &c.created_at[..10.min(c.created_at.len())],
                    preview
                )
            })
            .collect();

        let pruned_count = history.len().saturating_sub(MAX_INDEX_LINES);
        let mut section = format!("{section_header}\n\n{}\n", insight_lines.join("\n"));

        if pruned_count > 0 {
            info!(
                pruned = pruned_count,
                kept = insight_lines.len(),
                "pruned old insights from MEMORY.md index"
            );
        }

        // Replace or append the section
        let updated = if existing.contains(section_header) {
            // Find the section and replace it up to the next ## header or end of file.
            // Rust regex doesn't support look-ahead, so we do it manually.
            if let Some(start) = existing.find(section_header) {
                let rest = &existing[start + section_header.len()..];
                let end_offset = rest.find("\n## ").unwrap_or(rest.len());
                format!("{}{section}{}", &existing[..start], &rest[end_offset..])
            } else {
                format!("{}\n\n{section}", existing.trim_end())
            }
        } else {
            format!("{}\n\n{section}", existing.trim_end())
        };

        // Enforce MAX_INDEX_BYTES cap on the entire file
        let final_content = if updated.len() > MAX_INDEX_BYTES {
            warn!(
                size = updated.len(),
                max = MAX_INDEX_BYTES,
                "MEMORY.md exceeds size cap, truncating auto-generated section"
            );
            // Truncate the auto-generated section by reducing insight count
            let base = existing.split(section_header).next().unwrap_or(&existing);
            let remaining_budget = MAX_INDEX_BYTES.saturating_sub(base.len() + 100);
            let reduced_lines: Vec<&String> = insight_lines
                .iter()
                .scan(0usize, |acc, line| {
                    *acc += line.len() + 1;
                    if *acc <= remaining_budget {
                        Some(line)
                    } else {
                        None
                    }
                })
                .collect();
            section = format!(
                "{section_header}\n\n{}\n",
                reduced_lines
                    .iter()
                    .map(|l| l.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            if let Some(start) = existing.find(section_header) {
                let rest = &existing[start + section_header.len()..];
                let end_offset = rest.find("\n## ").unwrap_or(rest.len());
                format!("{}{section}{}", &existing[..start], &rest[end_offset..])
            } else {
                format!("{}\n\n{section}", existing.trim_end())
            }
        } else {
            updated
        };

        std::fs::write(&memory_path, final_content)?;
        info!("updated _system/MEMORY.md with agent insights");
        Ok(())
    }

    /// Rebuilds entity_nodes/entity_edges from the current set of `_graph/`
    /// concept pages. Concept pages are the source of truth; this SQLite
    /// cache is what spreading_activation/graph_boost/the CLI/the MCP tool
    /// actually read, and it is fully rebuilt (not incrementally patched)
    /// every time this runs.
    pub fn refresh_entity_index(&self) -> Result<crate::concept_pages::DeriveStats> {
        let vault = hq_vault::VaultClient::new(self.vault_path.clone())?;
        crate::concept_pages::derive_entity_index(&self.db, &vault)
    }

    /// Insight chaining: after a new insight is generated, check whether it
    /// extends, contradicts, or deepens a previous insight. If so, create a
    /// chained consolidation that references the parent.
    ///
    /// This gives the agent evolving understanding: "Previously I noted X,
    /// but based on recent activity, it's more accurate to say Y."
    pub async fn chain_insights(&self, new_insight: &str) -> Result<Option<String>> {
        let recent = get_consolidation_history(&self.db, 10)?;
        if recent.len() < 2 {
            return Ok(None);
        }

        let Some(llm) = &self.llm else {
            return Ok(None);
        };

        // Format previous insights for comparison
        let previous: String = recent
            .iter()
            .skip(1) // skip the most recent (which is the new one we just created)
            .take(8)
            .map(|c| {
                let preview: String = c.insight.chars().take(200).collect();
                format!("[Insight #{}] {preview}", c.id)
            })
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            "Given a NEW insight and PREVIOUS insights, determine if the new one \
             extends, contradicts, or significantly deepens any previous one.\n\n\
             NEW INSIGHT: \"{new_insight}\"\n\n\
             PREVIOUS INSIGHTS:\n{previous}\n\n\
             If a meaningful connection exists, produce a synthesis that evolves \
             the understanding. Return JSON:\n\
             {{\"related_id\": <id of the related previous insight>, \"synthesis\": \"evolved understanding\"}}\n\n\
             If no meaningful connection exists, return:\n\
             {{\"related_id\": null, \"synthesis\": \"\"}}"
        );

        #[derive(serde::Deserialize)]
        struct ChainResult {
            related_id: Option<i64>,
            synthesis: String,
        }

        match llm
            .json::<ChainResult>(
                "You detect when new insights extend or contradict previous ones. Be concise.",
                &prompt,
            )
            .await
        {
            Ok(result) => {
                let parent_id = match result.related_id {
                    Some(pid) if !result.synthesis.is_empty() => pid,
                    _ => return Ok(None),
                };

                // Verify the parent exists
                let valid_parent = recent.iter().any(|c| c.id == parent_id);
                if !valid_parent {
                    return Ok(None);
                }

                // Store the chained insight
                crate::db::store_consolidation_chained(
                    &self.db,
                    &[], // no direct source memories
                    &result.synthesis,
                    &[],
                    parent_id,
                )?;

                let preview: String = result.synthesis.chars().take(100).collect();
                info!(parent_id, insight = %preview, "chained insight");
                Ok(Some(result.synthesis))
            }
            _ => Ok(None),
        }
    }

    // ── Private: Cluster consolidation ──────────────────────────────────

    async fn consolidate_cluster(
        &self,
        cluster: &[Memory],
        topic: &str,
        project_context: &str,
        collector: &mut Vec<(String, String)>,
    ) -> Result<Option<String>> {
        let Some(llm) = &self.llm else {
            return Ok(None);
        };
        // Cap cluster size to prevent oversized payloads
        let capped: Vec<&Memory> = cluster.iter().take(12).collect();
        info!(
            topic,
            count = capped.len(),
            total = cluster.len(),
            "Consolidating cluster"
        );

        let memory_summary: String = capped
            .iter()
            .map(|m| {
                let summary_preview: String = m.summary.chars().take(200).collect();
                format!(
                    "[Memory #{}] ({}/{}) {}",
                    m.id, m.source, m.harness, summary_preview
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Build system prompt: identity preamble + static instructions + optional project grounding
        let preamble = hq_vault::system::identity_preamble(&self.vault_path);
        let system_prompt = if project_context.is_empty() {
            format!("{preamble}\n\n{CONSOLIDATE_SYSTEM}")
        } else {
            format!("{preamble}\n\n{CONSOLIDATE_SYSTEM}\n\n{project_context}")
        };

        let result = match llm
            .json::<ConsolidationResult>(
                &system_prompt,
                &format!(
                    "Consolidate these memories (topic cluster: \"{topic}\"):\n\n{memory_summary}"
                ),
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!(topic, error = %e, "Cluster consolidation failed");
                return Ok(None);
            }
        };

        if result.insight.is_empty() {
            return Ok(None);
        }

        let valid_ids: std::collections::HashSet<i64> = capped.iter().map(|m| m.id).collect();
        let valid_connections: Vec<Connection> = result
            .connections
            .into_iter()
            .filter(|c| valid_ids.contains(&c.from_id) && valid_ids.contains(&c.to_id))
            .collect();

        let source_ids: Vec<i64> = capped.iter().map(|m| m.id).collect();
        store_consolidation(&self.db, &source_ids, &result.insight, &valid_connections)?;

        // Collect insight note for batch write after the cluster loop
        let memories_owned: Vec<Memory> = capped.into_iter().cloned().collect();
        self.prepare_insight_note(
            &memories_owned,
            &result.insight,
            &valid_connections,
            collector,
        )?;

        // Wikilink entity concept pages based on LLM-discovered connections
        // between memories. entity_nodes/entity_edges are no longer written
        // here directly — refresh_entity_index (called at the end of
        // run_cycle) rebuilds them from these pages.
        if let Ok(vault) = hq_vault::VaultClient::new(self.vault_path.clone()) {
            for conn in &valid_connections {
                let from_mem = memories_owned.iter().find(|m| m.id == conn.from_id);
                let to_mem = memories_owned.iter().find(|m| m.id == conn.to_id);

                if let (Some(fm), Some(tm)) = (from_mem, to_mem) {
                    for fe in &fm.entities {
                        let related: Vec<String> = tm.entities.clone();
                        let entity_type = crate::entity_graph::classify_entity_type(fe);
                        let source_ref = format!("memory:{}", fm.id);
                        let _ = crate::concept_pages::upsert_concept_page(
                            &vault,
                            fe,
                            entity_type,
                            &related,
                            &source_ref,
                        );
                    }
                    for te in &tm.entities {
                        let entity_type = crate::entity_graph::classify_entity_type(te);
                        let source_ref = format!("memory:{}", tm.id);
                        let _ = crate::concept_pages::upsert_concept_page(
                            &vault,
                            te,
                            entity_type,
                            &[],
                            &source_ref,
                        );
                    }
                }
            }
        }

        let preview: String = result.insight.chars().take(100).collect();
        info!(topic, insight = %preview, "Cluster consolidated");
        Ok(Some(result.insight))
    }

    async fn synthesize_clusters(&self, insights: &[String]) -> Option<String> {
        let llm = self.llm.as_ref()?;
        let numbered: String = insights
            .iter()
            .enumerate()
            .map(|(i, insight)| format!("{}. {insight}", i + 1))
            .collect::<Vec<_>>()
            .join("\n");

        match llm
            .json::<ConsolidationResult>(
                META_SYSTEM,
                &format!("Find the cross-cluster pattern in these insights:\n\n{numbered}"),
            )
            .await
        {
            Ok(r) if !r.insight.is_empty() => {
                let preview: String = r.insight.chars().take(100).collect();
                info!(insight = %preview, "Cross-cluster insight");
                Some(r.insight)
            }
            _ => None,
        }
    }

    /// Build an insight note's (file_path_string, content) and push it onto
    /// `collector`. The actual disk write is deferred so callers can batch all
    /// notes from a consolidation run into a single parallel flush.
    fn prepare_insight_note(
        &self,
        memories: &[Memory],
        insight: &str,
        connections: &[Connection],
        collector: &mut Vec<(String, String)>,
    ) -> Result<()> {
        let notes_dir = self.vault_path.join("Notebooks").join("Memories");
        std::fs::create_dir_all(&notes_dir)?;

        let now = chrono::Utc::now();
        let date = now.format("%Y-%m-%d").to_string();
        let time = now.format("%H-%M-%S").to_string();
        let filename = format!("{date}-{time}-insight.md");
        let file_path = notes_dir.join(&filename);

        let sources: Vec<String> = memories.iter().map(|m| m.source.clone()).collect();
        let sources_dedup: Vec<&String> = {
            let mut seen = std::collections::HashSet::new();
            sources.iter().filter(|s| seen.insert(s.as_str())).collect()
        };

        let harnesses: Vec<String> = memories.iter().map(|m| m.harness.clone()).collect();
        let harnesses_dedup: Vec<&String> = {
            let mut seen = std::collections::HashSet::new();
            harnesses
                .iter()
                .filter(|h| seen.insert(h.as_str()))
                .collect()
        };

        let all_topics: Vec<String> = {
            let mut seen = std::collections::HashSet::new();
            memories
                .iter()
                .flat_map(|m| m.topics.iter().cloned())
                .filter(|t| seen.insert(t.clone()))
                .take(8)
                .collect()
        };

        let tags_str = all_topics
            .iter()
            .map(|t| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let sources_str = sources_dedup
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let harnesses_str = harnesses_dedup
            .iter()
            .map(|h| h.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        let connection_lines = if connections.is_empty() {
            "_No connections identified_".to_string()
        } else {
            connections
                .iter()
                .map(|c| {
                    format!(
                        "- Memory #{} <-> #{}: {}",
                        c.from_id, c.to_id, c.relationship
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let memory_summaries: String = memories
            .iter()
            .map(|m| format!("- **#{}** ({}): {}", m.id, m.source, m.summary))
            .collect::<Vec<_>>()
            .join("\n");

        // Heuristic: does the insight reference concrete code artifacts?
        let actionable = insight.contains(".rs")
            || insight.contains("crates/")
            || insight.contains("hq-")
            || insight.contains("fn ")
            || insight.contains("struct ")
            || insight.contains("impl ");

        let content = format!(
            r#"---
noteType: consolidation-insight
tags: [{tags_str}]
sources: [{sources_str}]
harnesses: [{harnesses_str}]
memoriesConsolidated: {count}
createdAt: "{created_at}"
actionable: {actionable}
---

# Agent Memory Insight -- {date}

## Key Insight

{insight}

## Connections Found

{connection_lines}

## Source Memories

{memory_summaries}
"#,
            count = memories.len(),
            created_at = now.to_rfc3339(),
        );

        collector.push((file_path.to_string_lossy().into_owned(), content));
        Ok(())
    }

    /// Flush collected insight notes to disk in parallel using spawn_blocking.
    async fn flush_insight_notes(&self, collector: Vec<(String, String)>) {
        if collector.is_empty() {
            return;
        }
        use tokio::task::JoinSet;
        let mut set: JoinSet<(String, std::io::Result<()>)> = JoinSet::new();
        for (path, content) in collector {
            set.spawn_blocking(move || (path.clone(), std::fs::write(&path, content)));
        }
        let mut written = 0u32;
        while let Some(res) = set.join_next().await {
            match res {
                Ok((path, Ok(()))) => {
                    written += 1;
                    let filename = std::path::Path::new(&path)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or(path);
                    info!(filename, "Wrote insight note");
                }
                Ok((path, Err(e))) => warn!(path, error = %e, "Failed to write insight note"),
                Err(e) => warn!(error = %e, "Insight note write task panicked"),
            }
        }
        if written > 0 {
            info!(written, "Batch-wrote insight notes");
        }
    }
}

/// Group memories by their primary (first) topic.
fn cluster_by_topic(memories: &[Memory]) -> HashMap<String, Vec<Memory>> {
    let mut clusters: HashMap<String, Vec<Memory>> = HashMap::new();
    for memory in memories {
        let primary_topic = memory
            .topics
            .first()
            .cloned()
            .unwrap_or_else(|| "general".into());
        clusters
            .entry(primary_topic)
            .or_default()
            .push(memory.clone());
    }
    clusters
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::Database;

    #[test]
    fn run_cycle_re_derives_entity_index_from_concept_pages_after_consolidating() {
        // This is a structural test: confirm MemoryConsolidator has a method
        // that, given a vault with concept pages already on disk, refreshes
        // entity_nodes/entity_edges via concept_pages::derive_entity_index
        // rather than writing them directly.
        let db = Database::open_memory().unwrap();

        let dir = std::env::temp_dir().join(format!(
            "hq-consolidator-derive-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let vault = hq_vault::VaultClient::new(dir.clone()).unwrap();

        crate::concept_pages::upsert_concept_page(
            &vault,
            "Rust",
            "tool",
            &["Agent HQ".to_string()],
            "memory:1",
        )
        .unwrap();
        crate::concept_pages::upsert_concept_page(&vault, "Agent HQ", "tool", &[], "memory:1")
            .unwrap();

        let consolidator = MemoryConsolidator::new(db.clone(), dir);
        let stats = consolidator.refresh_entity_index().unwrap();

        assert_eq!(stats.nodes, 2);
        assert_eq!(stats.edges, 1);
    }
}
