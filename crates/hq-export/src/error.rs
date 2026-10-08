//! Errors raised while exporting.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ExportError {
    /// The generated layout was rejected by the engine. This is a bug in the
    /// markup generator, not in the note, so the message carries the engine's
    /// diagnostics for the report.
    #[error("layout failed: {0}")]
    Layout(String),

    #[error("render failed: {0}")]
    Render(String),

    #[error("{0}")]
    Unsupported(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
