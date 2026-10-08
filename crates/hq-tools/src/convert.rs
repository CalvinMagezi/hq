//! Conversion tools — inbound (any → Markdown) and outbound (Markdown → any).
//!
//! Registered under category `"convert"` in the tool registry.
//!
//! Tools:
//!   - `convert_to_markdown` — convert a file at a given path to Markdown
//!   - `convert_from_markdown` — export Markdown to DOCX, PDF, HTML, etc. via pandoc
//!   - `ocr_extract_text` — extract text from an image via macOS Vision (on-device, model-agnostic)

use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use tracing::info;

use hq_convert::brand::load_brand_kit;
use hq_convert::inbound::InboundConverter;
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

/// Vault folders that hold identity, threads and databases; documents never belong there.
const PROTECTED_VAULT_DIRS: [&str; 4] = ["_system", "_threads", "_data", "_trash"];

/// An export may not land in HQ's own config directory or the vault's private folders.
fn refuse_protected_output(dest: &Path, vault: &Path) -> Result<()> {
    let absolute = if dest.is_absolute() {
        dest.to_path_buf()
    } else {
        std::env::current_dir()?.join(dest)
    };
    let dest = crate::util::lexically_normalize(&absolute);
    let hq_dir = crate::util::lexically_normalize(&hq_core::config::HqConfig::hq_dir());
    let in_vault_private = PROTECTED_VAULT_DIRS
        .iter()
        .any(|d| dest.starts_with(crate::util::lexically_normalize(&vault.join(d))));
    if dest.starts_with(&hq_dir) || in_vault_private {
        anyhow::bail!(
            "output '{}' is inside HQ's config directory or the vault's private folders; pick another location",
            dest.display()
        );
    }
    Ok(())
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
        refuse_protected_output(&dest, &self.vault_path)?;

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
        Box::new(ConvertFromMarkdownTool { vault_path }),
        Box::new(OcrExtractTextTool),
    ]
}

#[cfg(test)]
mod protected_output_tests {
    use super::*;

    #[test]
    fn exports_cannot_land_in_config_or_vault_private_folders() {
        let vault = Path::new("/srv/hq/.vault");
        let hq_dir = hq_core::config::HqConfig::hq_dir();
        for bad in [
            hq_dir.join("config.yaml"),
            vault.join("_system/SOUL.md"),
            vault.join("_threads/x.jsonl"),
            vault.join("Notebooks/../_data/vault.db"),
        ] {
            assert!(refuse_protected_output(&bad, vault).is_err(), "{}", bad.display());
        }
        for ok in [vault.join("Notebooks/report.docx"), PathBuf::from("/tmp/out.pdf")] {
            assert!(refuse_protected_output(&ok, vault).is_ok(), "{}", ok.display());
        }
    }
}
