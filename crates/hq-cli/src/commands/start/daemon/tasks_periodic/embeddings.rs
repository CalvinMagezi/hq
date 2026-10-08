use anyhow::Result;
use hq_core::config::HqConfig;
use hq_core::frontmatter_utils::{split_frontmatter, strip_frontmatter};
use hq_core::types::{ChatMessage, MessageRole};
use hq_db::Database;
use hq_llm::{ChatRequest, LlmProvider, OpenRouterProvider};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

use super::super::helpers::*;

const TRIAGE_SAMPLE_CHARS: usize = 500;

/// Index notes into FTS5 for full-text search, and queue for embedding.
/// Uses incremental hash-based diffing: skips unchanged files, handles deletions.
pub async fn run_embeddings(vault_path: &Path, db: &Database, config: &HqConfig) -> Result<()> {
    let notebooks_dir = vault_path.join("Notebooks");
    if !notebooks_dir.exists() {
        return Ok(());
    }

    let notebooks_dir_clone = notebooks_dir.clone();
    match db
        .with_conn(move |conn| hq_db::search::sync_index_incremental(conn, &notebooks_dir_clone))
    {
        Ok((indexed, removed, unchanged)) => {
            if indexed > 0 || removed > 0 {
                info!(
                    indexed,
                    removed, unchanged, "embeddings: FTS5 sync complete"
                );
            } else {
                debug!(unchanged, "embeddings: FTS5 up to date");
            }
        }
        Err(e) => warn!(error = %e, "embeddings: FTS5 sync failed"),
    }

    // Generate vector embeddings for up to 5 notes per cycle (rate-limited).
    // OpenRouter when configured (e.g. the VPS, no local Ollama); Ollama otherwise.
    let vault = hq_vault::VaultClient::new(vault_path.to_path_buf())?;
    match hq_daemon::process_embeddings(&vault, db, 5, config.openrouter_api_key.as_deref()).await {
        Ok(n) if n > 0 => info!(embedded = n, "embeddings: vector embeddings generated"),
        Ok(_) => {}
        Err(e) => warn!(error = %e, "embeddings: vector embedding batch failed"),
    }

    Ok(())
}

/// Scan Notebooks/ for recently created untagged notes and classify them via LLM.
pub async fn run_inbox_triage(vault_path: &Path, config: &HqConfig) -> Result<()> {
    let api_key = match config.openrouter_api_key {
        Some(ref key) => key,
        None => return Ok(()),
    };

    let notebooks_dir = vault_path.join("Notebooks");
    if !notebooks_dir.exists() {
        return Ok(());
    }

    let mut untagged = Vec::new();
    find_untagged_recent_notes(&notebooks_dir, &mut untagged, 5);

    if untagged.is_empty() {
        return Ok(());
    }

    let provider = OpenRouterProvider::new(api_key);
    let mut triaged = 0u32;

    for path in &untagged {
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

            match hq_llm::with_origin(hq_llm::origin::EMBEDDINGS, provider.chat(&request)).await {
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

                        if !tags.is_empty() && update_note_metadata(path, category, &tags).is_ok() {
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

    if triaged > 0 {
        let sys_dir = vault_path.join("_system");
        ensure_dir(&sys_dir);
        let now = chrono::Utc::now().to_rfc3339();
        let report = format!(
            "---\ntriaged_at: {now}\ncount: {triaged}\n---\n\n# Inbox Triage Report\n\nClassified {triaged} notes.\n"
        );
        let _ = std::fs::write(sys_dir.join("INBOX-TRIAGE.md"), report);
        info!(triaged, "inbox-triage: classified notes");
    }

    Ok(())
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

fn update_note_metadata(path: &Path, category: &str, tags: &[String]) -> Result<()> {
    let raw = std::fs::read_to_string(path)?;
    // Hand-written notes sometimes start with a blank line before the fence.
    let content = raw.trim_start();

    let tag_yaml = format!(
        "category: {category}\ntags:\n{}",
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
    use super::update_note_metadata;

    fn tagged(before: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, before).unwrap();
        update_note_metadata(&path, "area", &["a".into(), "b".into()]).unwrap();
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
