//! Export a note that lives in a vault: resolve, clean, render.
//!
//! This is the seam the CLI, the agent tool and the web route share, so a note
//! is prepared and styled the same way whichever door it came through.

use std::path::Path;

use hq_convert::brand::BrandKit;
use hq_convert::note_pdf::prepare_note_with;

use crate::doc::Document;
use crate::error::ExportError;
use crate::formats::{ExportOptions, Format, Output, export};
use crate::render::RenderOptions;
use crate::theme::Theme;

/// A finished export of a vault note.
#[derive(Debug, Clone)]
pub struct NoteExport {
    pub title: String,
    pub output: Output,
}

/// Whether `HQ_PDF_ENGINE` asks for one of the older external PDF engines
/// (`chromium`, `weasyprint`, `xelatex`) instead of the built-in renderer.
fn legacy_pdf_engine_requested() -> bool {
    std::env::var("HQ_PDF_ENGINE").is_ok_and(|v| !v.trim().is_empty())
}

/// Export `note_abs` (a file inside `vault`) in `format`.
///
/// Rendering is CPU work and runs on a blocking thread. `languages` only
/// matters for [`Format::Code`].
pub async fn export_note(
    vault: &Path,
    note_abs: &Path,
    format: Format,
    brand: Option<&BrandKit>,
    languages: &[String],
) -> Result<NoteExport, ExportError> {
    if format == Format::Pdf && legacy_pdf_engine_requested() {
        return legacy_pdf(vault, note_abs, brand).await;
    }
    let (vault, note_abs) = (vault.to_path_buf(), note_abs.to_path_buf());
    let (brand, languages) = (brand.cloned(), languages.to_vec());
    tokio::task::spawn_blocking(move || {
        export_note_blocking(&vault, &note_abs, format, brand.as_ref(), &languages)
    })
    .await
    .map_err(|e| ExportError::Render(format!("export task failed: {e}")))?
}

/// The synchronous form of [`export_note`], for callers already off the
/// async runtime.
pub fn export_note_blocking(
    vault: &Path,
    note_abs: &Path,
    format: Format,
    brand: Option<&BrandKit>,
    languages: &[String],
) -> Result<NoteExport, ExportError> {
    let raw = std::fs::read_to_string(note_abs)?;
    let stem = note_abs
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Note".to_owned());
    let note_dir = note_abs.parent().unwrap_or(vault);
    // Callouts are kept as written: this renderer draws them as real boxes.
    let prepared = prepare_note_with(&raw, &stem, note_dir, vault, true);
    let doc = Document::from_markdown(prepared.title.clone(), &prepared.markdown);
    let opts = ExportOptions {
        render: RenderOptions {
            theme: brand.map(Theme::from_brand).unwrap_or_default(),
            asset_root: Some(vault.to_path_buf()),
            ..Default::default()
        },
        languages: languages.to_vec(),
    };
    let output = export(&doc, format, &opts)?;
    Ok(NoteExport {
        title: prepared.title,
        output,
    })
}

async fn legacy_pdf(
    vault: &Path,
    note_abs: &Path,
    brand: Option<&BrandKit>,
) -> Result<NoteExport, ExportError> {
    use hq_convert::types::ConvertError;
    let (info, bytes) = hq_convert::note_pdf::export_note_pdf_bytes(vault, note_abs, brand)
        .await
        .map_err(|e| match e {
            ConvertError::NoPdfEngine | ConvertError::PandocNotFound => {
                ExportError::Unavailable(e.to_string())
            }
            other => ExportError::Render(other.to_string()),
        })?;
    Ok(NoteExport {
        title: info.title,
        output: Output {
            bytes,
            mime: "application/pdf",
            extension: "pdf".to_owned(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault_with_note(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let v = tempfile::tempdir().unwrap();
        let dir = v.path().join("Notebooks/Inbox");
        std::fs::create_dir_all(&dir).unwrap();
        let note = dir.join("Plan.md");
        std::fs::write(&note, body).unwrap();
        (v, note)
    }

    const NOTE: &str = "---\ntitle: Quarterly plan\ntags: [a]\n---\n# Quarterly plan\n\nSee [[Other note|the other one]].\n\n> [!tip] Remember\n> Bring forms.\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";

    #[test]
    fn html_export_cleans_the_note_and_keeps_callouts_as_boxes() {
        let (v, note) = vault_with_note(NOTE);
        let out = export_note_blocking(v.path(), &note, Format::Html, None, &[]).unwrap();
        assert_eq!(out.title, "Quarterly plan");
        let html = String::from_utf8(out.output.bytes).unwrap();
        assert!(!html.contains("tags:"), "frontmatter must be dropped");
        assert!(!html.contains("[["), "wikilinks become plain text");
        assert!(html.contains("the other one"));
        assert!(
            html.contains("class=\"callout\""),
            "callout must survive: {html}"
        );
        assert_eq!(
            html.matches("Quarterly plan").count(),
            2,
            "title once in <title>, once as h1"
        );
    }

    #[test]
    fn table_export_reads_the_table() {
        let (v, note) = vault_with_note(NOTE);
        let out = export_note_blocking(v.path(), &note, Format::Csv, None, &[]).unwrap();
        assert_eq!(out.output.extension, "csv");
        assert_eq!(String::from_utf8(out.output.bytes).unwrap(), "a,b\n1,2\n");
    }

    #[test]
    fn images_outside_the_vault_are_never_embedded() {
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.png");
        std::fs::write(&secret, b"\x89PNGsecret").unwrap();
        let (v, note) = vault_with_note(&format!("![x]({})\n", secret.display()));
        let out = export_note_blocking(v.path(), &note, Format::Html, None, &[]).unwrap();
        let html = String::from_utf8(out.output.bytes).unwrap();
        assert!(!html.contains("data:image"), "{html}");
    }

    #[tokio::test]
    async fn async_export_matches_the_blocking_one() {
        let (v, note) = vault_with_note(NOTE);
        let out = export_note(v.path(), &note, Format::Md, None, &[])
            .await
            .unwrap();
        assert!(
            String::from_utf8(out.output.bytes)
                .unwrap()
                .starts_with("# Quarterly plan")
        );
    }

    #[tokio::test]
    async fn native_pdf_is_a_real_pdf() {
        let (v, note) = vault_with_note(NOTE);
        // Guard against a developer shell that forces a legacy engine.
        if legacy_pdf_engine_requested() {
            return;
        }
        let out = export_note(v.path(), &note, Format::Pdf, None, &[])
            .await
            .unwrap();
        assert!(out.output.bytes.starts_with(b"%PDF-"));
        assert_eq!(out.output.mime, "application/pdf");
    }
}
