//! `office_create` and `office_export`: the agent makes a spreadsheet, document or deck as a native
//! note, then renders it as the file the owner opens, saved under `Exports/` in the vault.
//!
//! The note is the truth; the file is a render that can always be made again from it. The work is
//! `hq-suite-adapter::author`, and the file entry the chat draws as a card is
//! `hq_tool_api::files::file_entry`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{Value, json};

use hq_tool_api::registry::HqTool;

fn text<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .with_context(|| format!("`{key}` is needed for this action"))
}

pub fn create_author_tools(vault: PathBuf) -> Vec<Box<dyn HqTool>> {
    vec![
        Box::new(OfficeCreateTool {
            vault: vault.clone(),
        }),
        Box::new(OfficeExportTool { vault }),
    ]
}

/// The note as a PDF, by the same exporter `vault_export` uses.
async fn export_pdf(
    vault: &Path,
    note_rel: &str,
    dest: &Path,
) -> Result<hq_suite_adapter::author::Exported> {
    let note = hq_convert::note_pdf::resolve_note(vault, note_rel)
        .with_context(|| format!("note not found in the vault: {note_rel}"))?;
    let done = hq_export::export_note(vault, &note, hq_export::Format::Pdf, None, &[])
        .await
        .map_err(|e| anyhow::anyhow!("pdf export failed: {e}"))?;
    std::fs::write(dest, &done.output.bytes)?;
    Ok(hq_suite_adapter::author::Exported {
        fidelity:
            "A fixed-layout copy of the note's text, tables and pictures. Sheets appear as tables."
                .into(),
    })
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
        let stem = std::path::Path::new(&path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("export");
        let dir = self.vault.join(hq_tool_api::files::exports_month_dir());
        std::fs::create_dir_all(&dir)?;
        let out =
            hq_tool_api::files::unique_path(&dir, &hq_tool_api::files::clean_stem(stem), &format);
        let vault = self.vault.clone();
        let note_rel = path.clone();
        let (note, fmt, dest) = (path, format, out.clone());
        let done = if fmt == "pdf" {
            export_pdf(&vault, &note, &dest).await?
        } else {
            tokio::task::spawn_blocking(move || {
                hq_suite_adapter::author::export_note(&vault, &note, &fmt, &dest)
            })
            .await??
        };
        let rel = out
            .strip_prefix(&self.vault)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut entry = hq_tool_api::files::file_entry(&self.vault, &rel);
        entry["note"] = json!(note_rel);
        entry["fidelity"] = json!(done.fidelity);
        Ok(json!({
            "files": [entry],
            "web_path": rel,
            "fidelity": done.fidelity,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn author_tools(vault: &std::path::Path) -> (Box<dyn HqTool>, Box<dyn HqTool>) {
        let mut t = create_author_tools(vault.to_path_buf());
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
        let (create, export) = author_tools(vault.path());
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
        assert_eq!(file["note"], note);
        assert!(file["fidelity"].as_str().unwrap().contains("Formulas"));
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
        let (create, export) = author_tools(vault.path());
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
        let (create, export) = author_tools(vault.path());
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
