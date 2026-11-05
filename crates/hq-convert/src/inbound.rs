//! Inbound conversion: a file in any supported format to Markdown.
//!
//! This is the original entry point and keeps its shape. The work is done by
//! the import pipeline in [`crate::ingest`], which looks at the file's content
//! (not only its name), reads Office, OpenDocument, RTF, HTML and XML natively,
//! rebuilds tables in PDFs, and reports what it kept and dropped. Use
//! [`InboundConverter::convert_with_report`] when the report matters.

use std::path::Path;

use crate::ingest::{self, ConvertOptions, Converted, OcrMode};
use crate::types::ConvertError;

/// Converts arbitrary documents to Markdown text.
pub struct InboundConverter;

impl InboundConverter {
    /// Create a new converter.
    pub fn new() -> Result<Self, ConvertError> {
        Ok(Self)
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
