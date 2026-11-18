//! `office_file`: make an office file smaller, from a chat. One tool with an `action`, like
//! `hq_coder`, so a call that moves an original asks the owner and the rest do not.
//!
//! The work is `hq-suite-adapter::api`, the same code the phone's bar and `hq suite` use.

/// The tool's name, for the places that classify it (owner approval, the connector gate).
pub const OFFICE_TOOL: &str = "office_file";

/// Actions that never move an original: they read, write the new note or put things back.
pub const SAFE_ACTIONS: [&str; 4] = ["status", "shrink", "abandon", "undo"];

/// Whether a call with this `action` moves an original out of its place (and so asks the owner).
pub fn moves_original(action: &str) -> bool {
    !SAFE_ACTIONS.contains(&action)
}

#[cfg(feature = "suite")]
pub use tool::create_office_tools;

#[cfg(feature = "suite")]
mod tool {
    use std::path::PathBuf;

    use anyhow::{Context, Result, bail};
    use async_trait::async_trait;
    use serde_json::{Value, json};

    use super::OFFICE_TOOL;
    use crate::registry::HqTool;

    pub struct OfficeFileTool {
        vault: PathBuf,
    }

    pub fn create_office_tools(vault: PathBuf) -> Vec<Box<dyn HqTool>> {
        vec![
            Box::new(OfficeFileTool {
                vault: vault.clone(),
            }),
            Box::new(OfficeCreateTool {
                vault: vault.clone(),
            }),
            Box::new(OfficeExportTool { vault }),
        ]
    }

    /// `office_create`: a native sheet, document or deck note, written from structured content.
    pub struct OfficeCreateTool {
        vault: PathBuf,
    }

    #[async_trait]
    impl HqTool for OfficeCreateTool {
        fn name(&self) -> &str {
            "office_create"
        }

        fn description(&self) -> &str {
            "Make a spreadsheet, document or slide deck as a note in the vault. Give `kind` \
             (sheet, doc or slides), a `title`, and either `sheets` (for a sheet: columns with a \
             `type` of text, int, num, bool, date, pct or a currency code like ugx, and rows; a \
             cell starting with `=` is a formula, a blank cell stays blank) or `content` (Markdown, \
             for a document; for slides one `## heading` per slide). The note is the truth; call \
             `office_export` next to make the .xlsx, .docx or .pptx file the owner opens. Never \
             write a deliverable anywhere else."
        }

        fn parameters(&self) -> Value {
            json!({
                "type": "object",
                "required": ["kind", "title"],
                "properties": {
                    "kind": { "type": "string", "enum": ["sheet", "doc", "slides"] },
                    "title": { "type": "string" },
                    "folder": { "type": "string", "description": "Vault-relative folder, default Notebooks/Office." },
                    "content": { "type": "string", "description": "Markdown body for doc or slides." },
                    "sheets": {
                        "type": "array",
                        "description": "For kind=sheet: one entry per sheet.",
                        "items": {
                            "type": "object",
                            "required": ["columns", "rows"],
                            "properties": {
                                "name": { "type": "string" },
                                "columns": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "required": ["name"],
                                        "properties": {
                                            "name": { "type": "string" },
                                            "type": { "type": "string" }
                                        }
                                    }
                                },
                                "rows": { "type": "array", "items": { "type": "array" } }
                            }
                        }
                    }
                }
            })
        }

        fn category(&self) -> &str {
            "office"
        }

        async fn execute(&self, args: Value) -> Result<Value> {
            use hq_suite_adapter::author::{Kind, SheetInput, create_note};
            let kind = Kind::parse(text(&args, "kind")?)?;
            let title = text(&args, "title")?.to_string();
            let folder = args
                .get("folder")
                .and_then(Value::as_str)
                .map(str::to_string);
            let content = args
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_string);
            let sheets: Vec<SheetInput> = match args.get("sheets") {
                Some(v) => serde_json::from_value(v.clone()).context("`sheets` is not valid")?,
                None => Vec::new(),
            };
            let vault = self.vault.clone();
            let rel = tokio::task::spawn_blocking(move || {
                create_note(
                    &vault,
                    kind,
                    &title,
                    folder.as_deref(),
                    content.as_deref(),
                    &sheets,
                )
            })
            .await??;
            Ok(json!({
                "note": rel,
                "next": "Call office_export with this note to make the file the owner opens.",
            }))
        }
    }

    /// `office_export`: render a note as xlsx, csv, docx, pptx or pdf into `Exports/`.
    pub struct OfficeExportTool {
        vault: PathBuf,
    }

    #[async_trait]
    impl HqTool for OfficeExportTool {
        fn name(&self) -> &str {
            "office_export"
        }

        fn description(&self) -> &str {
            "Turn an office note into a file the owner can open or share: `path` is the note, \
             `format` is xlsx or csv (a note with sheets), docx (a document), pptx (a deck) or pdf. \
             The file is saved under Exports/ in the vault and returned as a `files` entry the chat \
             shows as a card. The result's `fidelity` says in one line what was kept; repeat it to \
             the owner."
        }

        fn parameters(&self) -> Value {
            json!({
                "type": "object",
                "required": ["path", "format"],
                "properties": {
                    "path": { "type": "string", "description": "The note, vault-relative." },
                    "format": { "type": "string", "enum": ["xlsx", "csv", "docx", "pptx", "pdf"] }
                }
            })
        }

        fn category(&self) -> &str {
            "office"
        }

        async fn execute(&self, args: Value) -> Result<Value> {
            let path = text(&args, "path")?.to_string();
            let format = text(&args, "format")?.to_string();
            if format == "pdf" {
                let out = crate::convert::run_note_export(
                    &self.vault,
                    &json!({"path": path, "format": "pdf"}),
                    None,
                )
                .await?;
                let mut out = out;
                out["fidelity"] = json!(
                    "A fixed-layout copy of the note's text, tables and pictures. Sheets appear as tables."
                );
                return Ok(out);
            }
            let stem = std::path::Path::new(&path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("export");
            let dir = self.vault.join(crate::deliver::exports_month_dir());
            std::fs::create_dir_all(&dir)?;
            let out = crate::deliver::unique_path(&dir, &crate::deliver::clean_stem(stem), &format);
            let vault = self.vault.clone();
            let (note, fmt, dest) = (path, format, out.clone());
            let done = tokio::task::spawn_blocking(move || {
                hq_suite_adapter::author::export_note(&vault, &note, &fmt, &dest)
            })
            .await??;
            let rel = out
                .strip_prefix(&self.vault)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            Ok(json!({
                "files": [crate::deliver::file_entry(&self.vault, &rel)],
                "web_path": rel,
                "fidelity": done.fidelity,
            }))
        }
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
                other => bail!(
                    "`{other}` is not an action; use status, shrink, confirm, abandon or undo"
                ),
            };
            Ok(serde_json::to_value(status)?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_confirm_moves_an_original() {
        for a in SAFE_ACTIONS {
            assert!(!moves_original(a), "{a}");
        }
        assert!(moves_original("confirm"));
        assert!(moves_original("anything-new"), "an unknown action asks");
    }
}

#[cfg(all(test, feature = "suite"))]
mod suite_tests {
    use super::*;
    use crate::registry::HqTool;
    use serde_json::{Value, json};

    #[tokio::test]
    async fn the_tool_reports_a_files_state_and_refuses_what_is_missing_or_unknown() {
        let vault = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
        std::fs::write(vault.path().join("Notebooks/a.docx"), b"x").unwrap();
        let tool = create_office_tools(vault.path().to_path_buf()).remove(0);
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

    fn tools(vault: &std::path::Path) -> (Box<dyn HqTool>, Box<dyn HqTool>) {
        let mut t = create_office_tools(vault.to_path_buf());
        let export = t.pop().unwrap();
        let create = t.pop().unwrap();
        (create, export)
    }

    fn stock_args() -> Value {
        json!({
            "kind": "sheet",
            "title": "AI stocks",
            "sheets": [{
                "name": "Stocks",
                "columns": [
                    {"name": "ticker"}, {"name": "price", "type": "usd"},
                    {"name": "shares", "type": "int"}, {"name": "value", "type": "usd"},
                    {"name": "source"}, {"name": "as of", "type": "date"}
                ],
                "rows": [
                    ["NVDA", 120.5, 10, "=B2*C2", "example.com/quote", "2026-11-18"],
                    ["AMD", 150, 4, "=B3*C3", "example.com/quote", "2026-11-18"],
                    ["Total", null, "=SUM(C2:C3)", "=SUM(D2:D3)", "", ""]
                ]
            }]
        })
    }

    #[tokio::test]
    async fn a_sheet_goes_from_request_to_note_to_xlsx_in_the_vault() {
        let vault = tempfile::tempdir().unwrap();
        let (create, export) = tools(vault.path());
        let made = create.execute(stock_args()).await.unwrap();
        let note = made["note"].as_str().unwrap();
        assert_eq!(note, "Notebooks/Office/AI stocks.md");
        let text = std::fs::read_to_string(vault.path().join(note)).unwrap();
        assert!(text.contains("suite_kind: sheet") && text.contains("suite_fidelity: native"));
        assert!(
            text.contains("```sheet name=Stocks types=\"text,usd,int,usd,text,date\""),
            "{text}"
        );

        let out = export
            .execute(json!({"path": note, "format": "xlsx"}))
            .await
            .unwrap();
        let file = &out["files"][0];
        let rel = file["path"].as_str().unwrap();
        assert!(
            rel.starts_with("Exports/") && rel.ends_with("/AI stocks.xlsx"),
            "{rel}"
        );
        assert_eq!(file["kind"], "sheet");
        assert!(
            out["fidelity"].as_str().unwrap().contains("Formulas"),
            "{out}"
        );
        let bytes = std::fs::read(vault.path().join(rel)).unwrap();
        let sniffed = hq_convert::ingest::sniff::sniff(&bytes, "xlsx");
        assert_eq!(sniffed.kind, hq_convert::ingest::sniff::Kind::Xlsx);

        // A second export never overwrites the first.
        let again = export
            .execute(json!({"path": note, "format": "xlsx"}))
            .await
            .unwrap();
        assert_ne!(again["files"][0]["path"], file["path"]);
        let csv = export
            .execute(json!({"path": note, "format": "csv"}))
            .await
            .unwrap();
        let csv_text =
            std::fs::read_to_string(vault.path().join(csv["web_path"].as_str().unwrap())).unwrap();
        assert!(csv_text.contains("NVDA,120.5,10,1205,"), "{csv_text}");
    }

    #[tokio::test]
    async fn a_document_and_a_deck_export_and_bad_requests_say_why() {
        let vault = tempfile::tempdir().unwrap();
        let (create, export) = tools(vault.path());
        let doc = create
            .execute(json!({"kind": "doc", "title": "Brief", "content": "Intro text.\n\n## Scope\n\nOne page."}))
            .await
            .unwrap();
        let deck = create
            .execute(json!({"kind": "slides", "title": "Pitch", "content": "## Problem\n\n- slow\n\n## Plan\n\n- fast"}))
            .await
            .unwrap();
        let d = export
            .execute(json!({"path": doc["note"], "format": "docx"}))
            .await
            .unwrap();
        assert_eq!(d["files"][0]["kind"], "doc");
        let p = export
            .execute(json!({"path": deck["note"], "format": "pptx"}))
            .await
            .unwrap();
        assert_eq!(p["files"][0]["kind"], "slides");
        assert!(p["fidelity"].as_str().unwrap().contains("slide"));

        let no_sheet = export
            .execute(json!({"path": doc["note"], "format": "xlsx"}))
            .await
            .unwrap_err();
        assert!(no_sheet.to_string().contains("no sheet"), "{no_sheet}");
        assert!(
            create
                .execute(json!({"kind": "doc", "title": "x"}))
                .await
                .is_err()
        );
        let bad_type = create
            .execute(json!({"kind": "sheet", "title": "t", "sheets": [{"columns": [{"name": "a", "type": "money"}], "rows": []}]}))
            .await
            .unwrap_err();
        assert!(bad_type.to_string().contains("column type"), "{bad_type}");
        let ragged = create
            .execute(json!({"kind": "sheet", "title": "t", "sheets": [{"columns": [{"name": "a"}], "rows": [[1, 2]]}]}))
            .await
            .unwrap_err();
        assert!(ragged.to_string().contains("row 1"), "{ragged}");
    }

    /// Opens an exported workbook in LibreOffice headless and converts it to csv, which only
    /// works when its formulas are intact. Opt in:
    /// `cargo test -p hq-tools --features suite --lib libreoffice_opens -- --ignored`
    #[tokio::test]
    #[ignore = "needs LibreOffice (`soffice`) on the PATH"]
    async fn libreoffice_opens_the_workbook_and_recalculates_its_formulas() {
        let vault = tempfile::tempdir().unwrap();
        let (create, export) = tools(vault.path());
        let made = create.execute(stock_args()).await.unwrap();
        let out = export
            .execute(json!({"path": made["note"], "format": "xlsx"}))
            .await
            .unwrap();
        let xlsx = vault.path().join(out["web_path"].as_str().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let status = std::process::Command::new("soffice")
            .args(["--headless", "--convert-to", "csv", "--outdir"])
            .arg(dir.path())
            .arg(&xlsx)
            .status()
            .unwrap();
        assert!(status.success());
        let csv = std::fs::read_to_string(dir.path().join("AI stocks.csv")).unwrap();
        assert!(csv.contains("1205") && csv.contains("1805"), "{csv}");
    }
}
