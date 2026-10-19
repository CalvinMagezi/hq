//! Inbound conversion: a file in any supported format to Markdown.
//!
//! This is the original entry point and keeps its shape. The work is done by
//! the import pipeline in [`crate::ingest`], which looks at the file's content
//! (not only its name), reads Office, OpenDocument, RTF, HTML and XML natively,
//! rebuilds tables in PDFs, and reports what it kept and dropped. Use
//! [`InboundConverter::convert_with_report`] when the report matters.

use std::path::Path;

use crate::ingest::{self, ConvertOptions, Converted, OcrMode};
use crate::types::{ConvertError, DetectedFormat};

/// Converts arbitrary documents to Markdown text.
pub struct InboundConverter;

impl InboundConverter {
    /// Create a new converter.
    pub fn new() -> Result<Self, ConvertError> {
        Ok(Self)
    }

    /// The format a file's extension suggests. A first guess only: conversion
    /// itself looks at the content.
    pub fn detect_format(path: &Path) -> DetectedFormat {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        match ext.as_str() {
            "pdf" => DetectedFormat::Pdf,
            "docx" => DetectedFormat::Docx,
            "doc" => DetectedFormat::Doc,
            "xlsx" | "xls" => DetectedFormat::Xlsx,
            "pptx" => DetectedFormat::Pptx,
            "ppt" => DetectedFormat::Ppt,
            "html" | "htm" => DetectedFormat::Html,
            "xml" => DetectedFormat::Xml,
            "txt" => DetectedFormat::Txt,
            "csv" => DetectedFormat::Csv,
            "tsv" => DetectedFormat::Tsv,
            "rtf" => DetectedFormat::Rtf,
            "odt" => DetectedFormat::Odt,
            "ods" => DetectedFormat::Ods,
            "odp" => DetectedFormat::Odp,
            "jpg" | "jpeg" | "png" | "tiff" | "tif" | "bmp" | "gif" | "webp" => {
                DetectedFormat::Image(ext)
            }
            other => DetectedFormat::Unknown(other.to_string()),
        }
    }

    /// Convert a file at `path` to Markdown text.
    ///
    /// A PDF with no text layer is an error here; use [`Self::convert_with_report`]
    /// with OCR on to read it.
    pub async fn convert(&self, path: &Path) -> Result<String, ConvertError> {
        let opts = ConvertOptions {
            ocr: OcrMode::Off,
            ..ConvertOptions::default()
        };
        Ok(ingest::convert_file(path, &opts).await?.markdown)
    }

    /// Convert with explicit options and get the report back with the Markdown.
    pub async fn convert_with_report(
        &self,
        path: &Path,
        opts: &ConvertOptions,
    ) -> Result<Converted, ConvertError> {
        ingest::convert_file(path, opts).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_format_pdf() {
        let p = Path::new("document.pdf");
        assert_eq!(InboundConverter::detect_format(p), DetectedFormat::Pdf);
    }

    #[test]
    fn detect_format_docx() {
        let p = Path::new("report.docx");
        assert_eq!(InboundConverter::detect_format(p), DetectedFormat::Docx);
    }

    #[test]
    fn detect_format_md_unsupported_inbound() {
        let p = Path::new("note.md");
        let fmt = InboundConverter::detect_format(p);
        assert!(!fmt.is_supported_inbound());
    }

    #[test]
    fn rejected_formats_keep_their_extension() {
        let fmt = InboundConverter::detect_format(Path::new("song.MP3"));
        assert_eq!(fmt, DetectedFormat::Unknown("mp3".to_string()));
        assert_eq!(fmt.to_string(), "mp3");
    }

    #[test]
    fn detect_format_unknown() {
        let p = Path::new("archive.7z");
        let fmt = InboundConverter::detect_format(p);
        assert!(!fmt.is_supported_inbound());
    }

    #[test]
    fn opendocument_and_legacy_office_are_recognised() {
        for (name, label) in [
            ("a.ods", "ods"),
            ("a.odp", "odp"),
            ("a.doc", "doc"),
            ("a.ppt", "ppt"),
        ] {
            let f = InboundConverter::detect_format(Path::new(name));
            assert_eq!(f.label(), label);
            assert!(f.is_supported_inbound());
        }
    }
}
