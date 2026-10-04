//! Shared types for hq-convert.

use std::fmt;
use thiserror::Error;

/// Errors that can occur during conversion.
#[derive(Error, Debug)]
pub enum ConvertError {
    #[error("unsupported input format: {0}")]
    UnsupportedFormat(String),

    #[error("inbound conversion failed: {0}")]
    InboundFailed(String),

    #[error("outbound conversion failed: {0}")]
    OutboundFailed(String),

    #[error(
        "pandoc not found in PATH.\n\
         Install it with:\n  \
         macOS: brew install pandoc\n  \
         Linux: sudo apt-get install pandoc\n  \
         Windows: winget install JohnMacFarlane.Pandoc"
    )]
    PandocNotFound,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

/// Format detected from a file's extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectedFormat {
    Pdf,
    Docx,
    Xlsx,
    Pptx,
    Html,
    Xml,
    Txt,
    Csv,
    Tsv,
    Rtf,
    Odt,
    /// Routed to the `ocr` module rather than transmutation.
    Image(String),
    /// Rejected, carrying the extension. Audio, video and zip land here because
    /// transmutation's matching features are off, and markdown needs no conversion.
    Unknown(String),
}

impl DetectedFormat {
    /// Return a short human-readable label.
    pub fn label(&self) -> &str {
        match self {
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
            Self::Pptx => "pptx",
            Self::Html => "html",
            Self::Xml => "xml",
            Self::Txt => "txt",
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::Rtf => "rtf",
            Self::Odt => "odt",
            Self::Image(_) => "image",
            Self::Unknown(_) => "unknown",
        }
    }

    /// True if the format can be converted to markdown by this crate.
    pub fn is_supported_inbound(&self) -> bool {
        !matches!(self, Self::Unknown(_))
    }
}

impl fmt::Display for DetectedFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(ext) if !ext.is_empty() => f.write_str(ext),
            _ => f.write_str(self.label()),
        }
    }
}

/// Target format for outbound (Markdown → X) conversion via pandoc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportFormat {
    Pdf,
    Docx,
    Pptx,
    Html,
    Epub,
    Rtf,
    /// Pandoc format string (passthrough for advanced usage)
    Custom(String),
}

impl ExportFormat {
    /// Pandoc format string.
    pub fn pandoc_format(&self) -> &str {
        match self {
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Pptx => "pptx",
            Self::Html => "html",
            Self::Epub => "epub",
            Self::Rtf => "rtf",
            Self::Custom(s) => s.as_str(),
        }
    }

}

impl fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.pandoc_format())
    }
}

impl std::str::FromStr for ExportFormat {
    type Err = ConvertError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "pdf" => Self::Pdf,
            "docx" | "word" => Self::Docx,
            "pptx" | "powerpoint" => Self::Pptx,
            "html" => Self::Html,
            "epub" => Self::Epub,
            "rtf" => Self::Rtf,
            other => Self::Custom(other.to_string()),
        })
    }
}
