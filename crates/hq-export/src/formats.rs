//! One entry point for every output format.

use crate::code;
use crate::doc::Document;
use crate::docx;
use crate::error::ExportError;
use crate::html;
use crate::jira;
use crate::markdown;
use crate::notebook;
use crate::render::{self, RenderOptions};
use crate::tables;

/// An output format. [`Format::parse`] accepts the common spellings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Pdf,
    /// The whole note as one tall image.
    Png,
    Svg,
    Docx,
    Html,
    Md,
    Csv,
    Json,
    Jsonl,
    Xml,
    Latex,
    Xlsx,
    Ipynb,
    Jira,
    /// The note's code blocks, as one file or a zip.
    Code,
}

impl Format {
    pub const ALL: &'static [Format] = &[
        Format::Pdf,
        Format::Png,
        Format::Svg,
        Format::Docx,
        Format::Html,
        Format::Md,
        Format::Csv,
        Format::Json,
        Format::Jsonl,
        Format::Xml,
        Format::Latex,
        Format::Xlsx,
        Format::Ipynb,
        Format::Jira,
        Format::Code,
    ];

    /// Canonical name, as printed in help and accepted back by [`Format::parse`].
    pub fn name(self) -> &'static str {
        match self {
            Format::Pdf => "pdf",
            Format::Png => "png",
            Format::Svg => "svg",
            Format::Docx => "docx",
            Format::Html => "html",
            Format::Md => "md",
            Format::Csv => "csv",
            Format::Json => "json",
            Format::Jsonl => "jsonl",
            Format::Xml => "xml",
            Format::Latex => "latex",
            Format::Xlsx => "xlsx",
            Format::Ipynb => "ipynb",
            Format::Jira => "jira",
            Format::Code => "code",
        }
    }

    pub fn parse(s: &str) -> Option<Format> {
        Some(
            match s
                .trim()
                .trim_start_matches('.')
                .to_ascii_lowercase()
                .as_str()
            {
                "pdf" => Format::Pdf,
                "png" => Format::Png,
                "svg" => Format::Svg,
                "docx" | "word" => Format::Docx,
                "html" | "htm" => Format::Html,
                "md" | "markdown" => Format::Md,
                "csv" => Format::Csv,
                "json" => Format::Json,
                "jsonl" | "ndjson" => Format::Jsonl,
                "xml" => Format::Xml,
                "latex" | "tex" => Format::Latex,
                "xlsx" | "excel" => Format::Xlsx,
                "ipynb" | "notebook" | "jupyter" => Format::Ipynb,
                "jira" => Format::Jira,
                "code" | "codeblocks" => Format::Code,
                _ => return None,
            },
        )
    }

    /// Whether the format needs the note to contain tables.
    pub fn needs_tables(self) -> bool {
        matches!(
            self,
            Format::Csv | Format::Json | Format::Jsonl | Format::Xml | Format::Latex | Format::Xlsx
        )
    }
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl std::str::FromStr for Format {
    type Err = ExportError;

    fn from_str(s: &str) -> Result<Self, ExportError> {
        Format::parse(s).ok_or_else(|| {
            let names: Vec<&str> = Format::ALL.iter().map(|f| f.name()).collect();
            ExportError::Unsupported(format!(
                "unknown export format '{s}'; choose one of: {}",
                names.join(", ")
            ))
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    pub render: RenderOptions,
    /// For [`Format::Code`]: keep only blocks in these languages.
    pub languages: Vec<String>,
}

/// A finished export. `extension` can differ from the format's usual one:
/// formats that normally hold a single file come back as `zip` when the note
/// has several tables or code blocks.
#[derive(Debug, Clone)]
pub struct Output {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub extension: String,
}

impl Output {
    fn new(bytes: Vec<u8>, mime: &'static str, extension: &str) -> Self {
        Output {
            bytes,
            mime,
            extension: extension.to_owned(),
        }
    }
}

pub fn export(doc: &Document, format: Format, opts: &ExportOptions) -> Result<Output, ExportError> {
    let text = |s: String, mime, ext| Ok(Output::new(s.into_bytes(), mime, ext));
    let table = |p: tables::Packed| Ok(Output::new(p.bytes, p.mime, p.extension));
    let asset_root = opts.render.asset_root.as_deref();
    match format {
        Format::Pdf => Ok(Output::new(
            render::pdf(doc, &opts.render)?,
            "application/pdf",
            "pdf",
        )),
        Format::Png => Ok(Output::new(
            render::png(doc, &opts.render)?,
            "image/png",
            "png",
        )),
        Format::Svg => text(render::svg(doc, &opts.render)?, "image/svg+xml", "svg"),
        Format::Docx => Ok(Output::new(
            docx::to_docx(doc, &opts.render.theme, asset_root)?,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "docx",
        )),
        Format::Html => text(
            html::to_html(doc, &opts.render.theme, asset_root),
            "text/html; charset=utf-8",
            "html",
        ),
        Format::Md => text(
            markdown::to_markdown(doc),
            "text/markdown; charset=utf-8",
            "md",
        ),
        Format::Jira => text(jira::to_jira(doc), "text/plain; charset=utf-8", "txt"),
        Format::Ipynb => text(
            notebook::to_notebook(doc),
            "application/x-ipynb+json",
            "ipynb",
        ),
        Format::Csv => table(tables::csv(doc)?),
        Format::Json => table(tables::json(doc)?),
        Format::Jsonl => table(tables::jsonl(doc)?),
        Format::Xml => table(tables::xml(doc)?),
        Format::Latex => table(tables::latex(doc)?),
        Format::Xlsx => table(tables::xlsx(doc)?),
        Format::Code => {
            let p = code::extract(doc, &opts.languages)?;
            Ok(Output {
                bytes: p.bytes,
                mime: p.mime,
                extension: p.extension,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_parses_back_to_itself() {
        for f in Format::ALL {
            assert_eq!(Format::parse(f.name()), Some(*f));
        }
    }

    #[test]
    fn aliases_and_dots_are_accepted() {
        assert_eq!(Format::parse(".XLSX"), Some(Format::Xlsx));
        assert_eq!(Format::parse("tex"), Some(Format::Latex));
        assert_eq!(Format::parse("Markdown"), Some(Format::Md));
        assert_eq!(Format::parse("ndjson"), Some(Format::Jsonl));
        assert_eq!(Format::parse("Word"), Some(Format::Docx));
        assert_eq!(Format::parse("pptx"), None, "not implemented yet");
    }

    #[test]
    fn unknown_format_error_lists_the_choices() {
        let err = "wat".parse::<Format>().unwrap_err().to_string();
        assert!(
            err.contains("pdf") && err.contains("xlsx") && err.contains("wat"),
            "{err}"
        );
    }

    #[test]
    fn every_non_render_format_exports_a_sample_note() {
        let doc = Document::from_markdown(
            "Sample",
            "## Sites\n\n| a | b |\n|---|---|\n| 1 | x |\n\n```python\nprint(1)\n```\n",
        );
        let opts = ExportOptions::default();
        for f in Format::ALL {
            if matches!(f, Format::Pdf | Format::Png | Format::Svg) {
                continue;
            }
            let out = export(&doc, *f, &opts).unwrap_or_else(|e| panic!("{f}: {e}"));
            assert!(!out.bytes.is_empty(), "{f} produced no bytes");
            assert!(!out.mime.is_empty() && !out.extension.is_empty());
        }
    }

    #[test]
    fn table_formats_reject_a_note_without_tables() {
        let doc = Document::from_markdown("t", "just prose\n");
        for f in Format::ALL.iter().filter(|f| f.needs_tables()) {
            let err = export(&doc, *f, &ExportOptions::default()).unwrap_err();
            assert!(err.to_string().contains("no tables"), "{f}: {err}");
        }
    }
}
