//! What every app draws for a file: one record view, made on the server and drawn by the clients
//! with the renderers they already have. A note is shown as written; an office file is read by
//! the importer in memory (nothing is written) and shown as Markdown with its loss lines.
//! It is the content, not the layout, and says so.

use std::path::Path;

use anyhow::{Result, bail};
use hq_convert::ingest::{ConvertOptions, OcrMode, convert_bytes, sniff::sniff};
use hq_suite_md::{Document, frontmatter};
use hq_suite_store::SuitePaths;
use serde::Serialize;

/// Largest file the preview reads.
const MAX_PREVIEW_BYTES: u64 = 64 * 1024 * 1024;
/// Longest Markdown the preview returns, in characters.
const MAX_PREVIEW_CHARS: usize = 400_000;
/// Rows of a sheet the preview keeps; the rest are counted in a note.
const MAX_PREVIEW_ROWS: usize = 500;
/// Vault folders a preview never reads.
const PRIVATE_DIRS: [&str; 4] = ["_system", "_threads", "_data", "_trash"];
/// Said on every preview of an office file.
pub const CONTENT_ONLY: &str = "Preview of the content, not the layout.";

#[derive(Debug, Serialize)]
pub struct Preview {
    pub path: String,
    pub name: String,
    /// `sheet`, `doc`, `slides` or `file`.
    pub kind: String,
    pub title: String,
    pub markdown: String,
    /// What the preview leaves out, one plain line each.
    pub notes: Vec<String>,
    /// Set when the preview is not the file's own look.
    pub banner: Option<&'static str>,
    /// The note declares a suite kind, so it is an office note and may offer office exports.
    pub suite: bool,
}

fn kind_of_extension(ext: &str) -> &'static str {
    match ext {
        "xlsx" | "xls" | "ods" | "csv" | "tsv" => "sheet",
        "docx" | "doc" | "odt" | "rtf" => "doc",
        "pptx" | "ppt" | "odp" => "slides",
        _ => "file",
    }
}

pub async fn preview(vault: &Path, rel: &str) -> Result<Preview> {
    let first = rel.split('/').next().unwrap_or("");
    if PRIVATE_DIRS.contains(&first) {
        bail!("that folder is kept for HQ itself");
    }
    let path = SuitePaths::new(vault).resolve(rel)?;
    let meta = std::fs::metadata(&path).map_err(|_| anyhow::anyhow!("{rel} cannot be read"))?;
    if !meta.is_file() || meta.len() > MAX_PREVIEW_BYTES {
        bail!("{rel} is not a file the preview can read");
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let bytes = std::fs::read(&path)?;
    if ext == "md" {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let doc = Document::parse(&text);
        let declared = frontmatter::get(doc.frontmatter(), "suite_kind");
        let suite = declared.is_some();
        let kind = declared.unwrap_or_else(|| "doc".into());
        let title = frontmatter::get(doc.frontmatter(), "title")
            .unwrap_or_else(|| name.trim_end_matches(".md").to_string());
        let body = crate::docs::body_of(&text).to_string();
        return Ok(Preview {
            path: rel.into(),
            name,
            kind,
            title,
            markdown: cap(body),
            notes: Vec::new(),
            banner: None,
            suite,
        });
    }
    let sniffed = sniff(&bytes, &ext);
    if sniffed.encrypted {
        bail!("{name} is password protected");
    }
    let opts = ConvertOptions {
        ocr: OcrMode::Off,
        external_office: false,
        max_sheet_rows: MAX_PREVIEW_ROWS,
        max_output_chars: MAX_PREVIEW_CHARS,
        ..ConvertOptions::default()
    };
    let done = convert_bytes(bytes, &ext, None, &opts)
        .await
        .map_err(|e| anyhow::anyhow!("{name} cannot be previewed: {e}"))?;
    Ok(Preview {
        path: rel.into(),
        title: done.title.clone().unwrap_or_else(|| name.clone()),
        kind: kind_of_extension(&ext).into(),
        name,
        notes: done.report.warning_lines(),
        markdown: cap(done.markdown),
        banner: Some(CONTENT_ONLY),
        suite: false,
    })
}

fn cap(s: String) -> String {
    if s.chars().count() <= MAX_PREVIEW_CHARS {
        return s;
    }
    s.chars().take(MAX_PREVIEW_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_note_is_shown_as_written_and_private_folders_are_refused() {
        let v = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(v.path().join("Notebooks")).unwrap();
        std::fs::create_dir_all(v.path().join("_system")).unwrap();
        std::fs::write(
            v.path().join("Notebooks/n.md"),
            "---\ntitle: \"Plan\"\nsuite_kind: sheet\n---\n\n# Plan\n\ntext\n",
        )
        .unwrap();
        std::fs::write(v.path().join("_system/x.md"), "secret").unwrap();
        let p = preview(v.path(), "Notebooks/n.md").await.unwrap();
        assert_eq!((p.kind.as_str(), p.title.as_str()), ("sheet", "Plan"));
        assert!(p.markdown.contains("# Plan") && p.banner.is_none());
        assert!(preview(v.path(), "_system/x.md").await.is_err());
        assert!(preview(v.path(), "../x.md").await.is_err());
        assert!(preview(v.path(), "Notebooks/none.xlsx").await.is_err());
    }

    #[tokio::test]
    async fn an_exported_workbook_previews_as_a_table_with_its_losses_and_the_banner() {
        use hq_suite_md::ColType;
        let v = tempfile::tempdir().unwrap();
        let sheet = hq_suite_xlsx::SheetSpec {
            name: "Stock".into(),
            header: vec!["item".into(), "qty".into(), "total".into()],
            types: vec![ColType::Text, ColType::Int, ColType::Int],
            rows: vec![
                vec!["Sugar".into(), "40".into(), "=B2*2".into()],
                vec!["Salt".into(), "5".into(), "=B3*2".into()],
            ],
        };
        let (bytes, _) = hq_suite_xlsx::write_xlsx(&[sheet]).unwrap();
        std::fs::create_dir_all(v.path().join("Exports")).unwrap();
        std::fs::write(v.path().join("Exports/stock.xlsx"), bytes).unwrap();
        let p = preview(v.path(), "Exports/stock.xlsx").await.unwrap();
        assert_eq!((p.kind.as_str(), p.banner), ("sheet", Some(CONTENT_ONLY)));
        assert!(p.markdown.contains("| Sugar | 40 | 80 |"), "{}", p.markdown);
        assert!(
            p.notes.iter().any(|n| n.contains("2 formulas")),
            "{:?}",
            p.notes
        );
    }
}
