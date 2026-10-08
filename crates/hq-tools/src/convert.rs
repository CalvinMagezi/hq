//! Conversion tools — inbound (any → Markdown) and outbound (Markdown → any).
//!
//! Registered under category `"convert"` in the tool registry.
//!
//! Tools:
//!   - `convert_to_markdown` — convert a file at a given path to Markdown
//!   - `convert_from_markdown` — export Markdown to DOCX, PDF, HTML, etc. via pandoc
//!   - `vault_export` — export a vault note as pdf, png, svg, html, md, xlsx, csv, json, jsonl, xml, latex, ipynb, jira or code (frontmatter and wikilinks cleaned up)
//!   - `vault_export_pdf` — the PDF-only form of `vault_export`
//!   - `ocr_extract_text` — extract text from an image via macOS Vision (on-device, model-agnostic)

use std::path::PathBuf;
use std::str::FromStr;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use tracing::info;

use hq_convert::brand::load_brand_kit;
use hq_convert::inbound::InboundConverter;
use hq_convert::note_pdf::resolve_note;
use hq_export::{Format, export_note};
use hq_convert::ocr::OcrEngine;

use crate::brand::brand_param_schema;
use hq_convert::outbound::OutboundConverter;
use hq_convert::types::{ConvertError, ExportFormat};

use crate::registry::HqTool;

// ─── ConvertToMarkdownTool ─────────────────────────────────────────────────

pub struct ConvertToMarkdownTool;

#[async_trait]
impl HqTool for ConvertToMarkdownTool {
    fn name(&self) -> &str {
        "convert_to_markdown"
    }

    fn description(&self) -> &str {
        "Convert a file (PDF, DOCX, XLSX, PPTX, HTML, CSV, image, etc.) to Markdown text. \
         Uses transmutation (pure Rust, no Python). The result can be written to the vault."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the source file."
                }
            },
            "required": ["path"]
        })
    }

    fn category(&self) -> &str {
        "convert"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required arg: path"))?;

        let path = PathBuf::from(path_str);

        if !path.exists() {
            anyhow::bail!("file not found: {}", path.display());
        }

        let format = InboundConverter::detect_format(&path);

        if !format.is_supported_inbound() {
            anyhow::bail!(
                "unsupported format '{}' — cannot convert to markdown",
                format
            );
        }

        let converter = InboundConverter::new()
            .map_err(|e| anyhow::anyhow!("failed to initialise converter: {e}"))?;

        info!(path = %path.display(), format = %format, "convert_to_markdown: converting");

        let markdown = converter
            .convert(&path)
            .await
            .map_err(|e| anyhow::anyhow!("conversion failed: {e}"))?;

        Ok(json!({
            "markdown": markdown,
            "source_path": path.to_string_lossy(),
            "format_detected": format.label(),
        }))
    }
}

// ─── ConvertFromMarkdownTool ────────────────────────────────────────────────

pub struct ConvertFromMarkdownTool {
    vault_path: PathBuf,
}

#[async_trait]
impl HqTool for ConvertFromMarkdownTool {
    fn name(&self) -> &str {
        "convert_from_markdown"
    }

    fn description(&self) -> &str {
        "Export Markdown content to another format (DOCX, PDF, PPTX, HTML, EPUB, RTF) using pandoc. \
         Pandoc must be installed: `brew install pandoc` on macOS. Pass `brand` for docx/pptx to \
         apply that client's reference-doc styling — see brand_assets_list for what's registered."
    }

    fn parameters(&self) -> Value {
        let mut brand_schema = brand_param_schema(&self.vault_path);
        if let Some(obj) = brand_schema.as_object_mut() {
            obj.insert(
                "description".into(),
                json!(
                    "Brand slug. Resolves a per-brand --reference-doc for docx/pptx from \
                     Notebooks/Projects/<Brand>/Branding/brand.yaml. Ignored for other formats \
                     (no pandoc reference-doc mechanism exists for xlsx/pdf)."
                ),
            );
        }
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Markdown text to convert."
                },
                "format": {
                    "type": "string",
                    "enum": ["pdf", "docx", "pptx", "html", "epub", "rtf"],
                    "description": "Target output format."
                },
                "output": {
                    "type": "string",
                    "description": "Absolute destination path for the output file."
                },
                "brand": brand_schema
            },
            "required": ["content", "format", "output"]
        })
    }

    fn category(&self) -> &str {
        "convert"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required arg: content"))?;

        let format_str = args
            .get("format")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required arg: format"))?;

        let output_str = args
            .get("output")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required arg: output"))?;

        let format = ExportFormat::from_str(format_str)
            .map_err(|e| anyhow::anyhow!("invalid format '{format_str}': {e}"))?;

        let dest = PathBuf::from(output_str);

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        OutboundConverter::check_pandoc().map_err(|e| match e {
            ConvertError::PandocNotFound => anyhow::anyhow!("{e}"),
            other => anyhow::anyhow!("{other}"),
        })?;

        let brand_kit = match args.get("brand").and_then(|v| v.as_str()) {
            Some(slug) => Some(
                load_brand_kit(&self.vault_path, slug)
                    .map_err(|e| anyhow::anyhow!("brand resolution failed: {e}"))?,
            ),
            None => None,
        };

        info!(format = %format, dest = %dest.display(), "convert_from_markdown: exporting");

        OutboundConverter::convert(content, &format, &dest, brand_kit.as_ref())
            .await
            .map_err(|e| anyhow::anyhow!("export failed: {e}"))?;

        let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);

        Ok(json!({
            "output_path": dest.to_string_lossy(),
            "format": format.pandoc_format(),
            "size_bytes": size,
        }))
    }
}

// ─── VaultExportTool / VaultExportPdfTool ──────────────────────────────────

/// Shared by `vault_export` and its `vault_export_pdf` alias.
async fn run_note_export(
    vault_path: &std::path::Path,
    args: &Value,
    fixed_format: Option<Format>,
) -> Result<Value> {
    let reference = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing required arg: path"))?;
    let format = match fixed_format {
        Some(f) => f,
        None => args
            .get("format")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required arg: format"))?
            .parse::<Format>()
            .map_err(|e| anyhow::anyhow!("{e}"))?,
    };
    let languages: Vec<String> = args
        .get("languages")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|l| l.as_str().map(str::to_owned)).collect())
        .unwrap_or_default();

    let note = resolve_note(vault_path, reference)
        .ok_or_else(|| anyhow::anyhow!("note not found in the vault: {reference}"))?;
    let brand_kit = match args.get("brand").and_then(|v| v.as_str()) {
        Some(slug) => Some(
            load_brand_kit(vault_path, slug)
                .map_err(|e| anyhow::anyhow!("brand resolution failed: {e}"))?,
        ),
        None => None,
    };

    info!(note = %note.display(), %format, "vault_export: exporting");
    let done = export_note(vault_path, &note, format, brand_kit.as_ref(), &languages)
        .await
        .map_err(|e| anyhow::anyhow!("{format} export failed: {e}"))?;

    // The extension comes from the result: a note with several tables exports
    // as a zip even when the format is csv.
    let (dest, in_vault) = match args.get("output").and_then(|v| v.as_str()) {
        Some(out) => (PathBuf::from(out), false),
        None => {
            let stem = note
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "note".into());
            let name = format!("{stem}.{}", done.output.extension);
            (vault_path.join("Exports").join(name), true)
        }
    };
    if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&dest, &done.output.bytes)?;

    let web_path = in_vault
        .then(|| dest.strip_prefix(vault_path).ok())
        .flatten()
        .map(|p| p.to_string_lossy().into_owned());

    Ok(json!({
        "output_path": dest.to_string_lossy(),
        "web_path": web_path,
        "title": done.title,
        "format": format.name(),
        "mime": done.output.mime,
        "size_bytes": done.output.bytes.len(),
    }))
}

fn export_params(vault_path: &std::path::Path, with_format: bool) -> Value {
    let mut brand_schema = brand_param_schema(vault_path);
    if let Some(obj) = brand_schema.as_object_mut() {
        obj.insert(
            "description".into(),
            json!("Brand slug: applies that brand's accent colour, font and footer."),
        );
    }
    let mut props = json!({
        "path": {
            "type": "string",
            "description": "The note: vault-relative path (Notebooks/Inbox/Plan.md), the same without .md, or a bare note name."
        },
        "output": {
            "type": "string",
            "description": "Optional absolute destination. Defaults to Exports/<note name>.<ext> inside the vault."
        },
        "brand": brand_schema
    });
    if with_format {
        let names: Vec<&str> = Format::ALL.iter().map(|f| f.name()).collect();
        props["format"] = json!({
            "type": "string",
            "enum": names,
            "description": "Output format."
        });
        props["languages"] = json!({
            "type": "array",
            "items": {"type": "string"},
            "description": "For format=code only: keep just the code blocks in these languages."
        });
    }
    let required: Vec<&str> = if with_format { vec!["path", "format"] } else { vec!["path"] };
    json!({ "type": "object", "properties": props, "required": required })
}

pub struct VaultExportTool {
    vault_path: PathBuf,
}

#[async_trait]
impl HqTool for VaultExportTool {
    fn name(&self) -> &str {
        "vault_export"
    }

    fn description(&self) -> &str {
        "Export a vault note as a file that can be shared outside the vault, with no external tools. \
         Formats: pdf, docx, png and svg (a styled page or one tall image), html (one self-contained file), \
         md (cleaned Markdown); xlsx, csv, json, jsonl, xml and latex export \
         the note's tables (several tables become sheets, a zip, or keyed groups); ipynb (a notebook \
         with Python blocks as code cells); jira (wiki markup); code (the note's fenced code blocks as \
         a file or zip). Frontmatter is dropped, [[wikilinks]] become plain text, callouts become boxes, \
         and images are embedded only when they live inside the vault. Saved to \
         Exports/<note>.<ext> in the vault unless `output` is given; the web UI serves it from the \
         returned `web_path`. Pass `brand` for a client's accent colour and font. For pptx, or a docx built on a\
         brand's Word template, use convert_from_markdown."
    }

    fn parameters(&self) -> Value {
        export_params(&self.vault_path, true)
    }

    fn category(&self) -> &str {
        "convert"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        run_note_export(&self.vault_path, &args, None).await
    }
}

/// The original PDF-only tool, kept so existing prompts and skills keep working.
pub struct VaultExportPdfTool {
    vault_path: PathBuf,
}

#[async_trait]
impl HqTool for VaultExportPdfTool {
    fn name(&self) -> &str {
        "vault_export_pdf"
    }

    fn description(&self) -> &str {
        "Export a vault note as a PDF that can be shared outside the vault (same as vault_export with \
         format=pdf). Frontmatter is dropped, [[wikilinks]] become plain text, and images embedded \
         from the vault are included (anything outside the vault is left out). Saved to \
         Exports/<note>.pdf in the vault unless `output` is given; the web UI serves it from the \
         returned `web_path`. Pass `brand` to use that client's colour and font."
    }

    fn parameters(&self) -> Value {
        export_params(&self.vault_path, false)
    }

    fn category(&self) -> &str {
        "convert"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        run_note_export(&self.vault_path, &args, Some(Format::Pdf)).await
    }
}

// ─── OcrExtractTextTool ─────────────────────────────────────────────────────

pub struct OcrExtractTextTool;

#[async_trait]
impl HqTool for OcrExtractTextTool {
    fn name(&self) -> &str {
        "ocr_extract_text"
    }

    fn description(&self) -> &str {
        "Extract text from an image (screenshot, photo, scan) using the macOS Vision framework. \
         Runs entirely on-device — no LLM call, so it works regardless of which model is active. \
         Use this whenever you're handed an image and need its text (e.g. a screenshot of a card, \
         chat, or document)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the image file (png, jpg, jpeg, tiff, bmp, gif, webp)."
                }
            },
            "required": ["path"]
        })
    }

    fn category(&self) -> &str {
        "convert"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required arg: path"))?;

        let path = PathBuf::from(path_str);

        if !path.exists() {
            anyhow::bail!("file not found: {}", path.display());
        }

        info!(path = %path.display(), "ocr_extract_text: running Vision OCR");

        let text = OcrEngine::extract_text(&path)
            .await
            .map_err(|e| anyhow::anyhow!("OCR failed: {e}"))?;

        Ok(json!({
            "text": text,
            "source_path": path.to_string_lossy(),
            "engine": "vision",
        }))
    }
}

/// Create all conversion tools.
pub fn create_convert_tools(vault_path: PathBuf) -> Vec<Box<dyn HqTool>> {
    vec![
        Box::new(ConvertToMarkdownTool),
        Box::new(ConvertFromMarkdownTool { vault_path: vault_path.clone() }),
        Box::new(VaultExportTool {
            vault_path: vault_path.clone(),
        }),
        Box::new(VaultExportPdfTool { vault_path }),
        Box::new(OcrExtractTextTool),
    ]
}

#[cfg(test)]
mod export_tests {
    use super::*;

    fn vault() -> tempfile::TempDir {
        let v = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(v.path().join("Notebooks/Inbox")).unwrap();
        std::fs::write(
            v.path().join("Notebooks/Inbox/Plan.md"),
            "---\ntitle: Plan\n---\nHello\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```python\nprint(1)\n```\n",
        )
        .unwrap();
        std::fs::write(v.path().join("Notebooks/Inbox/Prose.md"), "Only words.\n").unwrap();
        v
    }

    fn tool(v: &tempfile::TempDir) -> VaultExportTool {
        VaultExportTool {
            vault_path: v.path().to_path_buf(),
        }
    }

    #[tokio::test]
    async fn exports_land_in_the_vault_exports_folder_with_the_real_extension() {
        let v = vault();
        let out = tool(&v)
            .execute(json!({"path": "Plan", "format": "csv"}))
            .await
            .unwrap();
        assert_eq!(out["web_path"], "Exports/Plan.csv");
        assert_eq!(out["format"], "csv");
        let written = std::fs::read_to_string(v.path().join("Exports/Plan.csv")).unwrap();
        assert_eq!(written, "a,b\n1,2\n");
    }

    #[tokio::test]
    async fn an_explicit_output_path_is_honoured_and_has_no_web_path() {
        let v = vault();
        let dest = v.path().join("elsewhere/out.md");
        let out = tool(&v)
            .execute(json!({"path": "Plan", "format": "md", "output": dest.to_string_lossy()}))
            .await
            .unwrap();
        assert!(out["web_path"].is_null());
        assert!(std::fs::read_to_string(dest).unwrap().starts_with("# Plan"));
    }

    #[tokio::test]
    async fn code_export_filters_by_language() {
        let v = vault();
        let out = tool(&v)
            .execute(json!({"path": "Plan", "format": "code", "languages": ["python"]}))
            .await
            .unwrap();
        assert_eq!(out["web_path"], "Exports/Plan.py");
    }

    #[tokio::test]
    async fn bad_requests_fail_with_a_message_the_agent_can_act_on() {
        let v = vault();
        let t = tool(&v);
        let err = |r: Result<Value>| r.unwrap_err().to_string();
        assert!(err(t.execute(json!({"format": "csv"})).await).contains("path"));
        assert!(err(t.execute(json!({"path": "Plan"})).await).contains("format"));
        let unknown = err(t.execute(json!({"path": "Plan", "format": "wat"})).await);
        assert!(unknown.contains("unknown export format") && unknown.contains("xlsx"), "{unknown}");
        assert!(err(t.execute(json!({"path": "Missing", "format": "csv"})).await).contains("not found"));
        let no_tables = err(t.execute(json!({"path": "Prose", "format": "xlsx"})).await);
        assert!(no_tables.contains("no tables"), "{no_tables}");
        assert!(
            !v.path().join("Exports/Prose.xlsx").exists(),
            "a failed export must leave no file behind"
        );
    }

    #[tokio::test]
    async fn the_pdf_alias_always_writes_a_pdf() {
        if std::env::var("HQ_PDF_ENGINE").is_ok_and(|v| !v.trim().is_empty()) {
            return;
        }
        let v = vault();
        let alias = VaultExportPdfTool {
            vault_path: v.path().to_path_buf(),
        };
        // `format` is not a parameter of the alias, and passing one must not change the result.
        let out = alias
            .execute(json!({"path": "Plan", "format": "csv"}))
            .await
            .unwrap();
        assert_eq!(out["web_path"], "Exports/Plan.pdf");
        let bytes = std::fs::read(v.path().join("Exports/Plan.pdf")).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
    }

    #[test]
    fn schemas_advertise_formats_only_on_the_generic_tool() {
        let v = vault();
        let generic = tool(&v).parameters();
        let formats = generic["properties"]["format"]["enum"].as_array().unwrap();
        assert!(formats.iter().any(|f| f == "xlsx") && formats.iter().any(|f| f == "pdf"));
        assert_eq!(generic["required"], json!(["path", "format"]));
        let alias = VaultExportPdfTool {
            vault_path: v.path().to_path_buf(),
        }
        .parameters();
        assert!(alias["properties"].get("format").is_none());
        assert_eq!(alias["required"], json!(["path"]));
    }
}
