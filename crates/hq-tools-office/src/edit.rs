//! `office_edit`: change part of a note that already exists, instead of making a copy.
//!
//! Every action goes through `hq-suite-adapter` (`blocks`, `sheets`), which applies the edit as an
//! hqmd patch: a block or cell that changed since the agent last read it is refused and nothing is
//! written, and the rest of the note comes back byte for byte.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde_json::{Value, json};

use hq_suite_adapter::blocks::{self, BlocksView};
use hq_tool_api::registry::HqTool;

/// Longest source excerpt a listing returns per block, so a read stays small.
const EXCERPT_CHARS: usize = 400;

pub struct OfficeEditTool {
    pub(crate) vault: PathBuf,
}

fn text<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("`{key}` is needed for this action"))
}

fn number(args: &Value, key: &str) -> Result<u64> {
    args.get(key)
        .and_then(Value::as_u64)
        .with_context(|| format!("`{key}` (a whole number) is needed for this action"))
}

fn listing(view: &BlocksView) -> Value {
    let blocks: Vec<Value> = view
        .blocks
        .iter()
        .map(|b| {
            let src: String = b.src.chars().take(EXCERPT_CHARS).collect();
            json!({"index": b.index, "hash": b.hash, "kind": b.kind, "src": src})
        })
        .collect();
    json!({"revision": view.revision, "blocks": blocks})
}

fn run(vault: &std::path::Path, args: &Value) -> Result<Value> {
    let action = text(args, "action")?;
    let note = text(args, "note")?;
    match action {
        "blocks" => Ok(listing(&blocks::list(vault, note)?)),
        "replace_block" => {
            let view = blocks::edit(
                vault,
                note,
                number(args, "index")? as usize,
                text(args, "hash")?,
                text(args, "markdown")?,
            )?;
            Ok(listing(&view))
        }
        "insert_block" => {
            let after = match args.get("after_index").and_then(Value::as_u64) {
                Some(i) => Some((i as usize, text(args, "hash")?)),
                None => None,
            };
            Ok(listing(&blocks::insert(
                vault,
                note,
                after,
                text(args, "markdown")?,
            )?))
        }
        "delete_block" => Ok(listing(&blocks::delete(
            vault,
            note,
            number(args, "index")? as usize,
            text(args, "hash")?,
        )?)),
        "set_cell" => {
            let sheet = args.get("sheet").and_then(Value::as_u64).unwrap_or(0) as usize;
            hq_suite_adapter::sheets::edit_cell(
                vault,
                note,
                sheet,
                number(args, "row")? as u32,
                number(args, "col")? as u32,
                text(args, "expected")?,
                text(args, "text")?,
            )?;
            Ok(json!({"changed": true}))
        }
        other => bail!(
            "unknown action `{other}`; use blocks, replace_block, insert_block, delete_block or set_cell"
        ),
    }
}

#[async_trait]
impl HqTool for OfficeEditTool {
    fn name(&self) -> &str {
        "office_edit"
    }

    fn description(&self) -> &str {
        "Change part of a sheet, document or deck note that already exists, instead of making a \
         new one. Start with action `blocks` to list the note's blocks (each has an `index` and a \
         `hash`). Then `replace_block`, `insert_block` (after `after_index`, or at the top) or \
         `delete_block`, quoting the block's `hash`; or `set_cell` for one cell of a sheet \
         (`sheet` number from 0, `row` where 0 is the header row, `col` from 0, `expected` the \
         text the cell holds now, `text` the new text; a formula starts with `=`). If the note \
         changed since you read it the edit is refused and nothing is written: read again and \
         retry. Call `office_export` afterwards to refresh the file the owner opens."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["action", "note"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["blocks", "replace_block", "insert_block", "delete_block", "set_cell"]
                },
                "note": { "type": "string", "description": "Vault-relative path of the note." },
                "index": { "type": "integer", "description": "Block index, from `blocks`." },
                "after_index": { "type": "integer", "description": "insert_block: insert after this block (omit for the top)." },
                "hash": { "type": "string", "description": "The block's hash, from `blocks`." },
                "markdown": { "type": "string", "description": "The new block's Markdown." },
                "sheet": { "type": "integer", "description": "set_cell: which sheet block, from 0." },
                "row": { "type": "integer" },
                "col": { "type": "integer" },
                "expected": { "type": "string", "description": "set_cell: what the cell holds now." },
                "text": { "type": "string", "description": "set_cell: the new cell text." }
            }
        })
    }

    fn category(&self) -> &str {
        "office"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let vault = self.vault.clone();
        tokio::task::spawn_blocking(move || run(&vault, &args)).await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(vault: &std::path::Path) {
        let dir = vault.join("Notebooks");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("plan.md"),
            "---\nsuite_kind: doc\n---\n# Plan\n\nFirst.\n\nSecond.\n",
        )
        .unwrap();
    }

    #[tokio::test]
    async fn a_block_is_read_then_replaced_and_a_stale_hash_writes_nothing() {
        let d = tempfile::tempdir().unwrap();
        note(d.path());
        let tool = OfficeEditTool {
            vault: d.path().to_path_buf(),
        };
        let read = tool
            .execute(json!({"action": "blocks", "note": "Notebooks/plan.md"}))
            .await
            .unwrap();
        let first = read["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["src"].as_str().unwrap().contains("First"))
            .unwrap()
            .clone();
        let edit = |hash: String, md: &str| {
            json!({"action": "replace_block", "note": "Notebooks/plan.md",
                   "index": first["index"], "hash": hash, "markdown": md})
        };
        tool.execute(edit(first["hash"].as_str().unwrap().into(), "Changed."))
            .await
            .unwrap();
        let after = std::fs::read_to_string(d.path().join("Notebooks/plan.md")).unwrap();
        assert!(after.contains("Changed.") && after.contains("Second."));
        // The old hash no longer matches: refused, file unchanged.
        let stale = tool
            .execute(edit(first["hash"].as_str().unwrap().into(), "Again."))
            .await;
        assert!(stale.is_err());
        assert_eq!(
            std::fs::read_to_string(d.path().join("Notebooks/plan.md")).unwrap(),
            after
        );
    }

    #[tokio::test]
    async fn an_unknown_action_or_a_missing_argument_says_what_is_wrong() {
        let d = tempfile::tempdir().unwrap();
        note(d.path());
        let tool = OfficeEditTool {
            vault: d.path().to_path_buf(),
        };
        let e = tool
            .execute(json!({"action": "explode", "note": "Notebooks/plan.md"}))
            .await
            .unwrap_err();
        assert!(e.to_string().contains("unknown action"));
        let e = tool
            .execute(json!({"action": "delete_block", "note": "Notebooks/plan.md"}))
            .await
            .unwrap_err();
        assert!(e.to_string().contains("index"));
    }
}
