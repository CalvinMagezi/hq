use anyhow::Result;
use hq_core::config::{EmbeddingProvider, HqConfig};
use hq_core::frontmatter_utils::{split_frontmatter, strip_frontmatter};
use hq_core::types::{ChatMessage, MessageRole};
use hq_db::Database;
use hq_llm::{ChatRequest, OpenRouterProvider};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

use super::super::helpers::*;

const TRIAGE_SAMPLE_CHARS: usize = 500;
/// Untagged notes looked at per run.
const TRIAGE_BATCH: usize = 5;
/// Neighbours consulted when tags are borrowed from similar notes.
const TRIAGE_NEIGHBOURS: usize = 8;
/// A neighbour below this cosine similarity is too far away to lend its tags. Tuned on
/// potion-base-8M, where related notes score about 0.4 to 0.6 and unrelated ones under 0.25.
const TRIAGE_MIN_SIMILARITY: f32 = 0.4;
/// A tag is borrowed only when at least this many neighbours carry it.
const TRIAGE_MIN_VOTES: usize = 2;
const TRIAGE_MAX_TAGS: usize = 3;

/// Bring the keyword index in step with the notes on disk.
/// Uses incremental hash-based diffing: skips unchanged files, handles deletions.
///
/// Runs on every scheduled embeddings tick and once when the daemon starts, so edits made
/// while it was down are searchable without waiting out the first tick's delay.
pub async fn run_fts_sync(vault_path: &Path, db: &Database) -> Result<()> {
    let notebooks_dir = vault_path.join("Notebooks");
    if !notebooks_dir.exists() {
        return Ok(());
    }

    match db.with_conn(move |conn| hq_db::search::sync_index(conn, &notebooks_dir)) {
        Ok(report) => {
            if report.indexed > 0 || report.removed > 0 {
                info!(
                    indexed = report.indexed,
                    removed = report.removed,
                    unchanged = report.unchanged,
                    "embeddings: FTS5 sync complete"
                );
            } else {
                debug!(unchanged = report.unchanged, "embeddings: FTS5 up to date");
            }
            if report.failed > 0 || report.unlisted_dirs > 0 {
                warn!(
                    failed = report.failed,
                    unlisted_dirs = report.unlisted_dirs,
                    "embeddings: FTS5 sync left some notes and folders as they were"
                );
            }
        }
        Err(e) => warn!(error = %e, "embeddings: FTS5 sync failed"),
    }
    Ok(())
}

/// Index notes into FTS5 for full-text search, and queue for embedding.
pub async fn run_embeddings(vault_path: &Path, db: &Database, config: &HqConfig) -> Result<()> {
    if !vault_path.join("Notebooks").exists() {
        return Ok(());
    }

    run_fts_sync(vault_path, db).await?;

    // Lite, and a mobile instance with its data saver on and remote embeddings, keep only the
    // keyword index current (`HqConfig::embeds_notes`).
    if !config.embeds_notes() {
        return Ok(());
    }

    // Vector embeddings for up to 5 notes per cycle (rate-limited), from the configured provider.
    let vault = hq_vault::VaultClient::new(vault_path.to_path_buf())?;
    let provider = config.daemon.embedding_provider;
    let key = config.openrouter_api_key.as_deref();
    match hq_daemon::process_embeddings(&vault, db, 5, provider, key).await {
        Ok(n) if n > 0 => info!(embedded = n, "embeddings: vector embeddings generated"),
        Ok(_) => {}
        Err(e) => warn!(error = %e, "embeddings: vector embedding batch failed"),
    }

    Ok(())
}

/// Tag recently created untagged notes in Notebooks/. With the local embedder the tags are
/// borrowed from the most similar tagged notes and no model is asked; otherwise an LLM
/// classifies each note.
pub async fn run_inbox_triage(vault_path: &Path, db: &Database, config: &HqConfig) -> Result<()> {
    let notebooks_dir = vault_path.join("Notebooks");
    if !notebooks_dir.exists() {
        return Ok(());
    }
    let mut untagged = Vec::new();
    find_untagged_recent_notes(&notebooks_dir, &mut untagged, TRIAGE_BATCH);
    if untagged.is_empty() {
        return Ok(());
    }
    let triaged = if config.daemon.embedding_provider == EmbeddingProvider::Local {
        triage_by_neighbours(vault_path, db, &untagged).await
    } else {
        let Some(api_key) = config.openrouter_api_key.as_deref() else {
            return Ok(());
        };
        triage_by_llm(api_key, config, &untagged).await
    };
    write_triage_report(vault_path, triaged);
    Ok(())
}

/// Give each note the tags its nearest tagged neighbours agree on. A note with no close,
/// agreeing neighbours stays untagged rather than getting a guess.
async fn triage_by_neighbours(vault_path: &Path, db: &Database, notes: &[PathBuf]) -> u32 {
    let mut triaged = 0;
    for path in notes {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let embedding = match hq_memory::generate_embedding_local(strip_frontmatter(&content)).await
        {
            Ok(v) => v,
            Err(e) => {
                debug!(error = %e, "inbox-triage: local embedding failed");
                return triaged;
            }
        };
        let own = path
            .strip_prefix(vault_path)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let neighbours = db.with_conn(|conn| {
            hq_db::search::semantic_search(
                conn,
                &embedding,
                hq_memory::LOCAL_EMBEDDING_MODEL,
                TRIAGE_NEIGHBOURS + 1,
            )
        });
        let Ok(neighbours) = neighbours else {
            continue;
        };
        let votes: Vec<(f32, Vec<String>)> = neighbours
            .into_iter()
            .filter(|n| n.note_path != own)
            .take(TRIAGE_NEIGHBOURS)
            .map(|n| (n.relevance as f32, n.tags))
            .collect();
        let tags = vote_tags(&votes);
        if !tags.is_empty() && update_note_metadata(path, None, &tags).is_ok() {
            triaged += 1;
        }
    }
    triaged
}

/// The tags at least `TRIAGE_MIN_VOTES` close neighbours share, strongest support first.
// ponytail: similarity-weighted vote with fixed thresholds; calibrate against reviewed tags once
// the owner's corrections are recorded.
fn vote_tags(neighbours: &[(f32, Vec<String>)]) -> Vec<String> {
    let mut support: Vec<(String, usize, f32)> = Vec::new();
    for (similarity, tags) in neighbours {
        if *similarity < TRIAGE_MIN_SIMILARITY {
            continue;
        }
        for tag in tags {
            match support.iter_mut().find(|(t, _, _)| t == tag) {
                Some(entry) => {
                    entry.1 += 1;
                    entry.2 += similarity;
                }
                None => support.push((tag.clone(), 1, *similarity)),
            }
        }
    }
    support.retain(|(_, votes, _)| *votes >= TRIAGE_MIN_VOTES);
    support.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    support
        .into_iter()
        .take(TRIAGE_MAX_TAGS)
        .map(|(tag, _, _)| tag)
        .collect()
}

fn write_triage_report(vault_path: &Path, triaged: u32) {
    if triaged == 0 {
        return;
    }
    let sys_dir = vault_path.join("_system");
    ensure_dir(&sys_dir);
    let now = chrono::Utc::now().to_rfc3339();
    let report = format!(
        "---\ntriaged_at: {now}\ncount: {triaged}\n---\n\n# Inbox Triage Report\n\nClassified {triaged} notes.\n"
    );
    let _ = std::fs::write(sys_dir.join("INBOX-TRIAGE.md"), report);
    info!(triaged, "inbox-triage: classified notes");
}

async fn triage_by_llm(api_key: &str, config: &HqConfig, untagged: &[PathBuf]) -> u32 {
    let provider = hq_llm::InstrumentedProvider::wrap(
        std::sync::Arc::new(OpenRouterProvider::new(api_key)),
        "openrouter",
        hq_llm::cost::ProviderClass::Metered,
        hq_llm::Instruments::global(),
    );
    let mut triaged = 0u32;

    for path in untagged {
        if let Ok(content) = std::fs::read_to_string(path) {
            let title = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled");

            let body = strip_frontmatter(&content);
            let sample = body
                .char_indices()
                .nth(TRIAGE_SAMPLE_CHARS)
                .map_or(body, |(i, _)| &body[..i]);

            let prompt = format!(
                "Classify this note and suggest 2-3 tags.\nTitle: {title}\nContent: {sample}\n\n\
                 Respond ONLY with JSON: {{ \"category\": \"project|area|resource|reference\", \"tags\": [\"tag1\", \"tag2\"], \"summary\": \"one-liner\" }}"
            );

            let request = ChatRequest {
                model: config.default_model.clone(),
                messages: vec![ChatMessage {
                    image_parts: Vec::new(),
                    role: MessageRole::User,
                    content: prompt,
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    reasoning_content: None,
                }],
                temperature: Some(0.2),
                max_tokens: Some(200),
                ..Default::default()
            };

            match hq_llm::with_origin(hq_llm::origin::TRIAGE, provider.chat(&request)).await {
                Ok(resp) => {
                    if let Ok(data) =
                        serde_json::from_str::<serde_json::Value>(&resp.message.content)
                    {
                        let category = data["category"].as_str().unwrap_or("reference");
                        let tags: Vec<String> = data["tags"]
                            .as_array()
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                    .collect()
                            })
                            .unwrap_or_default();

                        if !tags.is_empty()
                            && update_note_metadata(path, Some(category), &tags).is_ok()
                        {
                            triaged += 1;
                        }
                    }
                }
                Err(e) => {
                    debug!(error = %e, "inbox-triage: LLM call failed");
                }
            }
        }
    }

    triaged
}

fn find_untagged_recent_notes(dir: &Path, results: &mut Vec<PathBuf>, limit: usize) {
    if results.len() >= limit {
        return;
    }
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if results.len() >= limit {
                return;
            }
            let path = entry.path();
            if path.is_dir() {
                find_untagged_recent_notes(&path, results, limit);
            } else if path.extension().is_some_and(|e| e == "md")
                && let Ok(meta) = entry.metadata()
                && let Ok(modified) = meta.modified()
                && modified > cutoff
                && let Ok(content) = std::fs::read_to_string(&path)
            {
                let untagged = split_frontmatter(content.trim_start())
                    .0
                    .is_none_or(|fm| fm.contains("tags: []") || !fm.contains("tags:"));
                if untagged {
                    results.push(path);
                }
            }
        }
    }
}

fn update_note_metadata(path: &Path, category: Option<&str>, tags: &[String]) -> Result<()> {
    let raw = std::fs::read_to_string(path)?;
    // Hand-written notes sometimes start with a blank line before the fence.
    let content = raw.trim_start();

    let category_line = category
        .map(|c| format!("category: {c}\n"))
        .unwrap_or_default();
    let tag_yaml = format!(
        "{category_line}tags:\n{}",
        tags.iter()
            .map(|t| format!("  - {t}"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    let new_content = match split_frontmatter(content) {
        (Some(fm), body) => {
            let new_fm = if fm.contains("tags: []") {
                fm.replace("tags: []", &tag_yaml)
            } else if !fm.contains("tags:") {
                // A non-empty frontmatter slice always ends with its own newline.
                format!("{fm}{tag_yaml}\n")
            } else {
                return Ok(());
            };
            format!("---\n{new_fm}---\n{body}")
        }
        (None, _) => format!("---\n{tag_yaml}\n---\n\n{content}"),
    };

    std::fs::write(path, new_content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{update_note_metadata, vote_tags};

    fn n(similarity: f32, tags: &[&str]) -> (f32, Vec<String>) {
        (similarity, tags.iter().map(|t| t.to_string()).collect())
    }

    #[test]
    fn tags_come_from_close_neighbours_that_agree() {
        let tags = vote_tags(&[
            n(0.7, &["android", "edge-ai"]),
            n(0.6, &["edge-ai", "research"]),
            n(0.5, &["android", "edge-ai"]),
            n(0.3, &["research", "research-noise"]),
            n(0.2, &["research"]),
        ]);
        // research has one close vote; the far neighbours do not count.
        assert_eq!(tags, ["edge-ai", "android"]);
    }

    #[test]
    fn no_agreement_means_no_tags() {
        assert!(vote_tags(&[n(0.9, &["a"]), n(0.8, &["b"])]).is_empty());
        assert!(vote_tags(&[]).is_empty());
    }

    #[test]
    fn a_local_triage_writes_tags_without_a_category() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, "---\ntitle: x\n---\nbody\n").unwrap();
        update_note_metadata(&path, None, &["edge-ai".into()]).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "---\ntitle: x\ntags:\n  - edge-ai\n---\nbody\n"
        );
    }

    fn tagged(before: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, before).unwrap();
        update_note_metadata(&path, Some("area"), &["a".into(), "b".into()]).unwrap();
        std::fs::read_to_string(&path).unwrap()
    }

    #[test]
    fn tags_are_added_inside_the_existing_frontmatter() {
        let tags = "category: area\ntags:\n  - a\n  - b\n";
        assert_eq!(
            tagged("---\ntitle: x\n---\nbody\n---\nmore\n"),
            format!("---\ntitle: x\n{tags}---\nbody\n---\nmore\n")
        );
        assert_eq!(
            tagged("---\ntags: []\n---\nbody\n"),
            "---\ncategory: area\ntags:\n  - a\n  - b\n---\nbody\n"
        );
        assert_eq!(tagged("body\n"), format!("---\n{tags}---\n\nbody\n"));
        let kept = "---\ntags: [x]\n---\nbody\n";
        assert_eq!(tagged(kept), kept);
        assert_eq!(
            tagged("\n---\ntitle: x\n---\nbody\n"),
            format!("---\ntitle: x\n{tags}---\nbody\n")
        );
    }
}
