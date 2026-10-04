//! Inbound conversion — any supported format → Markdown.
//!
//! Uses the `transmutation` crate (pure Rust, zero Python deps).
//! Format detection uses the file extension as primary signal.

use std::path::Path;
use transmutation::{ConversionOptions, Converter, OutputFormat};

use crate::types::{ConvertError, DetectedFormat};

/// Converts arbitrary documents to Markdown text.
pub struct InboundConverter {
    converter: Converter,
}

impl InboundConverter {
    /// Create a new converter.
    pub fn new() -> Result<Self, ConvertError> {
        let converter = Converter::new().map_err(|e| ConvertError::InboundFailed(e.to_string()))?;
        Ok(Self { converter })
    }

    /// Detect the format of a file from its extension alone.
    pub fn detect_format(path: &Path) -> DetectedFormat {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        match ext.as_str() {
            "pdf" => DetectedFormat::Pdf,
            "docx" => DetectedFormat::Docx,
            "xlsx" | "xls" => DetectedFormat::Xlsx,
            "pptx" | "ppt" => DetectedFormat::Pptx,
            "html" | "htm" => DetectedFormat::Html,
            "xml" => DetectedFormat::Xml,
            "txt" => DetectedFormat::Txt,
            "csv" => DetectedFormat::Csv,
            "tsv" => DetectedFormat::Tsv,
            "rtf" => DetectedFormat::Rtf,
            "odt" => DetectedFormat::Odt,
            "jpg" | "jpeg" | "png" | "tiff" | "tif" | "bmp" | "gif" | "webp" => {
                DetectedFormat::Image(ext)
            }
            other => DetectedFormat::Unknown(other.to_string()),
        }
    }

    /// Convert a file at `path` to Markdown text.
    ///
    /// Returns `ConvertError::UnsupportedFormat` for `.md` inputs and
    /// unknown extensions.
    pub async fn convert(&self, path: &Path) -> Result<String, ConvertError> {
        let format = Self::detect_format(path);

        if !format.is_supported_inbound() {
            return Err(ConvertError::UnsupportedFormat(format.to_string()));
        }

        if let DetectedFormat::Image(_) = format {
            return crate::ocr::OcrEngine::extract_text(path).await;
        }

        let opts = ConversionOptions {
            optimize_for_llm: true,
            normalize_whitespace: true,
            include_metadata: true,
            ..Default::default()
        };

        let result =
            self.converter
                .convert(path.to_str().ok_or_else(|| {
                    ConvertError::Other("path contains invalid UTF-8".to_string())
                })?)
                .to(OutputFormat::Markdown {
                    split_pages: false,
                    optimize_for_llm: true,
                })
                .with_options(opts)
                .execute()
                .await
                .map_err(|e| ConvertError::InboundFailed(e.to_string()))?;

        // Collect all page outputs and join as text
        let markdown = result
            .content
            .iter()
            .map(|output| String::from_utf8_lossy(&output.data).into_owned())
            .collect::<Vec<_>>()
            .join("\n\n");

        if markdown.trim().is_empty() {
            return Err(ConvertError::InboundFailed(
                "conversion produced no text".to_string(),
            ));
        }

        Ok(markdown)
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
}
