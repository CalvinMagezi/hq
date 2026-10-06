use crate::registry::{HqTool, ToolPolicy};
use hq_db::Database;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

pub struct VaultFindShortcut {
    db: Arc<Database>,
}
pub struct VaultNoteShortcut {
    vault_path: PathBuf,
}
pub struct VaultLogShortcut {
    vault_path: PathBuf,
}

impl VaultFindShortcut {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}
impl VaultNoteShortcut {
    pub fn new(vault_path: PathBuf) -> Self {
        Self { vault_path }
    }
}
impl VaultLogShortcut {
    pub fn new(vault_path: PathBuf) -> Self {
        Self { vault_path }
    }
}

#[async_trait::async_trait]
impl HqTool for VaultFindShortcut {
    fn name(&self) -> &str {
        "vault_find"
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "The cheap first search over the vault. Reach for vault_search when you need ranked full-text results with excerpts rather than a name match.",
        )
    }

    fn description(&self) -> &str {
        "Search the vault and return relevant note content by topic. Pass any natural language \
         topic — no need to choose between vault_search, vault_read, or vault_context."
    }

    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"topic":{"type":"string","description":"Natural language topic to look up"},"limit":{"type":"integer","description":"Max notes to return (default: 3)"}},"required":["topic"]})
    }

    fn tool_policy(&self) -> ToolPolicy {
        ToolPolicy::Weak
    }

    fn category(&self) -> &str {
        "vault"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> anyhow::Result<Value> {
        let topic = args
            .get("topic")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: topic"))?
            .to_string();
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(3) as usize;

        let db = Arc::clone(&self.db);
        let results = tokio::task::spawn_blocking(move || {
            db.with_conn(move |conn| hq_db::search::keyword_search(conn, &topic, limit))
        })
        .await??;

        if results.is_empty() {
            return Ok(json!({"notes": [], "message": "No notes found for topic"}));
        }

        let notes: Vec<Value> = results
            .iter()
            .map(|r| json!({"title": r.title, "content": r.snippet, "path": r.note_path}))
            .collect();

        Ok(json!({"notes": notes}))
    }
}

#[async_trait::async_trait]
impl HqTool for VaultNoteShortcut {
    fn name(&self) -> &str {
        "vault_note"
    }

    fn description(&self) -> &str {
        "Write a note to the vault. Auto-title and auto-path. Pass content (markdown)."
    }

    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"content":{"type":"string","description":"Note content in markdown"},"title":{"type":"string","description":"Title (auto-generated if omitted)"},"folder":{"type":"string","description":"Sub-folder under Notebooks/ (default: Inbox)"}},"required":["content"]})
    }

    fn tool_policy(&self) -> ToolPolicy {
        ToolPolicy::Weak
    }

    fn category(&self) -> &str {
        "vault"
    }

    async fn execute(&self, args: Value) -> anyhow::Result<Value> {
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: content"))?
            .to_string();
        let folder = args
            .get("folder")
            .and_then(|v| v.as_str())
            .unwrap_or("Inbox")
            .to_string();

        let title = if let Some(t) = args.get("title").and_then(|v| v.as_str()) {
            t.to_string()
        } else {
            let first_line = content.lines().next().unwrap_or("Untitled");
            let stripped = first_line.trim_start_matches('#').trim();
            let t: String = stripped.chars().take(60).collect();
            if t.is_empty() {
                "Untitled".to_string()
            } else {
                t
            }
        };

        // Strip path traversal sequences
        let folder: String = folder.replace("..", "").replace(['/', '\\'], "");
        let folder = if folder.is_empty() {
            "Inbox".to_string()
        } else {
            folder
        };

        let title: String = title.replace("..", "").replace(['/', '\\'], "");
        let title = if title.is_empty() {
            "Untitled".to_string()
        } else {
            title
        };

        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let relative_path = format!("Notebooks/{}/{}.md", folder, title);
        let note_path = self.vault_path.join(&relative_path);

        if let Some(parent) = note_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let body = format!(
            "---\ntitle: {}\ncreated: {}\n---\n\n{}",
            title, date, content
        );
        tokio::fs::write(&note_path, body).await?;

        Ok(json!({"written": true, "path": relative_path}))
    }
}

#[async_trait::async_trait]
impl HqTool for VaultLogShortcut {
    fn name(&self) -> &str {
        "vault_log"
    }

    fn description(&self) -> &str {
        "Append a one-line event to today's activity log in the vault."
    }

    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"event":{"type":"string","description":"What happened — a short sentence"}},"required":["event"]})
    }

    fn tool_policy(&self) -> ToolPolicy {
        ToolPolicy::Weak
    }

    fn category(&self) -> &str {
        "vault"
    }

    async fn execute(&self, args: Value) -> anyhow::Result<Value> {
        let event = args
            .get("event")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: event"))?
            .to_string();

        let now = chrono::Local::now();
        let date_str = now.format("%Y-%m-%d").to_string();
        let time_str = now.format("%H:%M").to_string();

        let log_path = self
            .vault_path
            .join("Notebooks")
            .join("Daily")
            .join(format!("{}.md", date_str));

        if let Some(parent) = log_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let line = format!("- {} {}\n", time_str, event);

        let mut file = match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&log_path)
            .await
        {
            Ok(mut f) => {
                let header = format!("# {}\n\n", date_str);
                f.write_all(header.as_bytes()).await?;
                f
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                tokio::fs::OpenOptions::new()
                    .append(true)
                    .open(&log_path)
                    .await?
            }
            Err(e) => return Err(e.into()),
        };
        file.write_all(line.as_bytes()).await?;

        Ok(json!({"logged": true}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn note_infers_title_from_first_line_and_strips_leading_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = VaultNoteShortcut::new(tmp.path().to_path_buf());
        let result = tool
            .execute(json!({"content": "# My Great Idea\n\nDetails here."}))
            .await
            .unwrap();
        assert_eq!(result["path"], json!("Notebooks/Inbox/My Great Idea.md"));
        let body = tokio::fs::read_to_string(tmp.path().join("Notebooks/Inbox/My Great Idea.md"))
            .await
            .unwrap();
        assert!(body.contains("title: My Great Idea"));
        assert!(body.contains("Details here."));
    }

    #[tokio::test]
    async fn note_uses_explicit_title_over_inferred_one() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = VaultNoteShortcut::new(tmp.path().to_path_buf());
        let result = tool
            .execute(json!({"content": "# Ignored heading", "title": "Chosen Title"}))
            .await
            .unwrap();
        assert_eq!(result["path"], json!("Notebooks/Inbox/Chosen Title.md"));
    }

    #[tokio::test]
    async fn note_falls_back_to_untitled_when_content_has_no_usable_first_line() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = VaultNoteShortcut::new(tmp.path().to_path_buf());
        let result = tool.execute(json!({"content": "# \n\nbody"})).await.unwrap();
        assert_eq!(result["path"], json!("Notebooks/Inbox/Untitled.md"));
    }

    #[tokio::test]
    async fn note_strips_path_traversal_from_folder_and_title() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = VaultNoteShortcut::new(tmp.path().to_path_buf());
        let result = tool
            .execute(json!({
                "content": "body",
                "title": "../../etc/passwd",
                "folder": "../../../tmp"
            }))
            .await
            .unwrap();
        // ".." and path separators are stripped, not rejected outright — the
        // write must land inside Notebooks/, never escape it.
        let path = result["path"].as_str().unwrap();
        assert!(path.starts_with("Notebooks/"));
        assert!(!path.contains(".."));
        assert!(!path.contains('/') || path.matches('/').count() == 2);
    }

    #[tokio::test]
    async fn note_falls_back_to_inbox_when_folder_is_only_traversal_sequences() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = VaultNoteShortcut::new(tmp.path().to_path_buf());
        let result = tool
            .execute(json!({"content": "body", "title": "t", "folder": "../.."}))
            .await
            .unwrap();
        assert_eq!(result["path"], json!("Notebooks/Inbox/t.md"));
    }

    #[tokio::test]
    async fn log_creates_file_with_header_on_first_write() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = VaultLogShortcut::new(tmp.path().to_path_buf());
        tool.execute(json!({"event": "started working"}))
            .await
            .unwrap();

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let log_path = tmp
            .path()
            .join("Notebooks/Daily")
            .join(format!("{today}.md"));
        let body = tokio::fs::read_to_string(&log_path).await.unwrap();
        assert!(body.starts_with(&format!("# {today}\n\n")));
        assert!(body.contains("started working"));
    }

    #[tokio::test]
    async fn log_appends_without_repeating_the_header_on_subsequent_events() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = VaultLogShortcut::new(tmp.path().to_path_buf());
        tool.execute(json!({"event": "first event"})).await.unwrap();
        tool.execute(json!({"event": "second event"}))
            .await
            .unwrap();

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let log_path = tmp
            .path()
            .join("Notebooks/Daily")
            .join(format!("{today}.md"));
        let body = tokio::fs::read_to_string(&log_path).await.unwrap();
        assert_eq!(body.matches(&format!("# {today}")).count(), 1);
        assert!(body.contains("first event"));
        assert!(body.contains("second event"));
    }
}
