//! `office_file`: make an office file smaller, from a chat. One tool with an `action`, like
//! `hq_coder`, so a call that moves an original asks the owner and the rest do not.
//!
//! The work is `hq-suite-adapter::api`, the same code the phone's bar and `hq suite` use. The tool
//! lives in its own crate so that a change anywhere in the HQ Suite rebuilds this one small crate
//! and the crates that really use the suite, not the whole of `hq-tools` and everything above it.
//! The tool's name and the actions that move an original are in `hq_tool_api::office`.

pub mod author;
pub mod edit;

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde_json::{Value, json};

use hq_tool_api::office::OFFICE_TOOL;
use hq_tool_api::registry::HqTool;

pub struct OfficeFileTool {
    vault: PathBuf,
}

pub fn create_office_tools(
    vault: PathBuf,
    seam: Option<hq_tool_api::office::SharedSeam>,
) -> Vec<Box<dyn HqTool>> {
    let mut tools: Vec<Box<dyn HqTool>> = vec![Box::new(OfficeFileTool {
        vault: vault.clone(),
    })];
    tools.push(Box::new(edit::OfficeEditTool {
        vault: vault.clone(),
        seam: seam.clone(),
    }));
    tools.extend(author::create_author_tools(vault, seam));
    tools
}

fn text<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .with_context(|| format!("`{key}` is needed for this action"))
}

#[async_trait]
impl HqTool for OfficeFileTool {
    fn name(&self) -> &str {
        OFFICE_TOOL
    }

    fn description(&self) -> &str {
        "Make a document in the vault (docx, xlsx, pptx, pdf) smaller. It keeps the text and \\
         tables as a note and the pictures once, recompressed, and sets the original aside in \\
         the trash for 30 days. Actions: `status` (where a file stands), `shrink` (write and \\
         check the record, show what it would save; nothing moves), `confirm` (move the \\
         original to the trash; the owner is asked), `abandon` (drop a prepared record), \\
         `undo` (bring the original back). Always `shrink` first and tell the owner what it \\
         would save and what is left out before calling `confirm`."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["status", "shrink", "confirm", "abandon", "undo"]
                },
                "path": {
                    "type": "string",
                    "description": "The file, vault-relative. For status and shrink."
                },
                "job": {
                    "type": "string",
                    "description": "The job id a shrink returned. For confirm, abandon and undo."
                }
            }
        })
    }

    fn category(&self) -> &str {
        "office"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        use hq_suite_adapter::api;
        let action = text(&args, "action")?.to_string();
        let vault = self.vault.clone();
        let status = match action.as_str() {
            "shrink" => api::prepare(&vault, text(&args, "path")?).await?,
            "status" | "confirm" | "abandon" | "undo" => {
                let (path, job) = (
                    text(&args, "path").map(str::to_string),
                    text(&args, "job").map(str::to_string),
                );
                tokio::task::spawn_blocking(move || match action.as_str() {
                    "status" => api::status(&vault, &path?),
                    "confirm" => api::confirm(&vault, &job?),
                    "abandon" => api::abandon(&vault, &job?),
                    _ => api::undo(&vault, &job?),
                })
                .await??
            }
            other => {
                bail!("`{other}` is not an action; use status, shrink, confirm, abandon or undo")
            }
        };
        Ok(serde_json::to_value(status)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn the_tool_reports_a_files_state_and_refuses_what_is_missing_or_unknown() {
        let vault = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
        std::fs::write(vault.path().join("Notebooks/a.docx"), b"x").unwrap();
        let tool = create_office_tools(vault.path().to_path_buf(), None).remove(0);
        assert_eq!(tool.name(), OFFICE_TOOL);

        let ok = tool
            .execute(json!({ "action": "status", "path": "Notebooks/a.docx" }))
            .await
            .unwrap();
        assert_eq!(ok["state"], "ready");
        assert!(
            tool.execute(json!({ "action": "status", "path": "Notebooks/none.docx" }))
                .await
                .is_err()
        );
        assert!(
            tool.execute(json!({ "action": "confirm" })).await.is_err(),
            "a job is needed"
        );
        assert!(tool.execute(json!({ "action": "explode" })).await.is_err());
    }
}
