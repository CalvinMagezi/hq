use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::registry::HqTool;

pub(super) const SESSION_DIR: &str = "_sessions";

pub struct SessionListTool;

#[async_trait]
impl HqTool for SessionListTool {
    fn name(&self) -> &str {
        "session_list"
    }

    fn description(&self) -> &str {
        "List all saved sessions available for resume."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "description": "Maximum sessions to return (default: 20)"
                }
            }
        })
    }

    fn category(&self) -> &str {
        "session"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

        let vault_path = std::env::var("HQ_VAULT_PATH").unwrap_or_else(|_| ".vault".to_string());
        let session_dir = format!("{}/{}", vault_path, SESSION_DIR);

        if !std::path::Path::new(&session_dir).exists() {
            return Ok(json!({"sessions": [], "count": 0}));
        }

        let mut sessions = Vec::new();
        let mut entries = tokio::fs::read_dir(&session_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(content) = tokio::fs::read_to_string(&path).await
                    && let Ok(session) = serde_json::from_str::<Value>(&content)
                {
                    sessions.push(json!({
                        "id": session.get("id").and_then(|v| v.as_str()).unwrap_or("unknown"),
                        "title": session.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled"),
                        "saved_at": session.get("saved_at").and_then(|v| v.as_str()).unwrap_or(""),
                        "working_dir": session.get("working_dir").and_then(|v| v.as_str()).unwrap_or(""),
                    }));
                }
                if sessions.len() >= limit {
                    break;
                }
            }
        }

        sessions.sort_by(|a, b| {
            let a_date = a["saved_at"].as_str().unwrap_or("");
            let b_date = b["saved_at"].as_str().unwrap_or("");
            b_date.cmp(a_date)
        });

        Ok(json!({
            "sessions": sessions,
            "count": sessions.len()
        }))
    }
}
