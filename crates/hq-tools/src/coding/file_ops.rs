use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::Path;

use crate::registry::HqTool;


pub struct ListDirTool;

#[async_trait]
impl HqTool for ListDirTool {
    fn name(&self) -> &str {
        "list_dir"
    }

    fn description(&self) -> &str {
        "List directory contents with file sizes. Use file_find with glob patterns for recursive searches."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory path to list"
                }
            }
        })
    }

    fn category(&self) -> &str {
        "coding"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let dir_path = args["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing path"))?;

        let path = Path::new(dir_path);
        if !path.is_dir() {
            return Ok(json!({"error": format!("Not a directory: {}", dir_path)}));
        }

        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(path).await?;
        while let Some(entry) = read_dir.next_entry().await? {
            let meta = entry.metadata().await?;
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = meta.is_dir();
            let size = if is_dir { 0 } else { meta.len() };
            entries.push(json!({
                "name": if is_dir { format!("{}/", name) } else { name },
                "size": size,
                "is_dir": is_dir,
            }));
        }
        entries.sort_by(|a, b| {
            let a_name = a["name"].as_str().unwrap_or("");
            let b_name = b["name"].as_str().unwrap_or("");
            a_name.cmp(b_name)
        });

        Ok(json!({
            "path": dir_path,
            "entries": entries,
            "count": entries.len()
        }))
    }
}
