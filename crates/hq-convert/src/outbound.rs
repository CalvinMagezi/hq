//! Outbound conversion — Markdown → any format via pandoc.
//!
//! `pandoc` must be available in PATH. If it is not found, the error
//! message includes platform-specific install instructions.

use std::path::Path;
use tokio::process::Command;
use tracing::{debug, warn};
use which::which;

use crate::brand::BrandKit;
use crate::types::{ConvertError, ExportFormat};

/// Converts Markdown content to other formats by calling `pandoc`.
pub struct OutboundConverter;

/// Reference-doc arg for docx/pptx: brand's own template if set, else (pptx
/// only) the legacy global `~/.hq/templates/reference.pptx` if present.
/// docx never had a legacy fallback — there was nothing to preserve there.
fn reference_doc_arg(format: &ExportFormat, brand: Option<&BrandKit>) -> Option<String> {
    let brand_template = brand.and_then(|kit| match format {
        ExportFormat::Docx => kit.reference_docx.as_ref(),
        ExportFormat::Pptx => kit.reference_pptx.as_ref(),
        _ => None,
    });
    if let Some(path) = brand_template {
        return Some(format!("--reference-doc={}", path.display()));
    }
    if *format == ExportFormat::Pptx
        && let Some(home) = dirs::home_dir()
    {
        let ref_doc = home.join(".hq").join("templates").join("reference.pptx");
        if ref_doc.exists() {
            return Some(format!("--reference-doc={}", ref_doc.to_string_lossy()));
        }
    }
    None
}

impl OutboundConverter {
    /// Check whether pandoc is available in PATH.
    /// Returns `Ok(path)` or `Err(ConvertError::PandocNotFound)`.
    pub fn check_pandoc() -> Result<std::path::PathBuf, ConvertError> {
        which("pandoc").map_err(|_| ConvertError::PandocNotFound)
    }

    /// Convert `markdown` to `format` at `dest`. `brand` only styles docx and
    /// pptx, the formats with a pandoc `--reference-doc`; PDF needs `xelatex`.
    pub async fn convert(
        markdown: &str,
        format: &ExportFormat,
        dest: &Path,
        brand: Option<&BrandKit>,
    ) -> Result<(), ConvertError> {
        let pandoc = Self::check_pandoc()?;

        // A file input keeps large documents off the child's stdin pipe.
        let tmp_dir = tempfile::tempdir().map_err(ConvertError::Io)?;
        let input_path = tmp_dir.path().join("input.md");
        std::fs::write(&input_path, markdown.as_bytes()).map_err(ConvertError::Io)?;

        let mut args: Vec<String> = vec![
            input_path.to_string_lossy().into_owned(),
            "-f".to_string(),
            "markdown".to_string(),
            "-t".to_string(),
            format.pandoc_format().to_string(),
            "-o".to_string(),
            dest.to_string_lossy().into_owned(),
        ];

        // For PDF, use xelatex to handle Unicode properly
        if *format == ExportFormat::Pdf {
            args.push("--pdf-engine=xelatex".to_string());
        }

        if let Some(reference_arg) = reference_doc_arg(format, brand) {
            args.push(reference_arg);
        }

        debug!(
            pandoc = %pandoc.display(),
            format = %format,
            dest = %dest.display(),
            "running pandoc"
        );

        let mut cmd = Command::new(&pandoc);
        cmd.kill_on_drop(true).args(&args);
        crate::run_tool(cmd, "pandoc").await.map_err(|msg| {
            warn!(error = %msg, "pandoc conversion failed");
            ConvertError::OutboundFailed(msg)
        })?;
        Ok(())
    }

}
