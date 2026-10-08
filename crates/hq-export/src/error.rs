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

    /// The request cannot be met for this note or format, for example a table
    /// export of a note with no tables. The message is meant for the user.
    #[error("{0}")]
    Unsupported(String),

    /// An external tool the chosen path depends on is missing.
    #[error("{0}")]
    Unavailable(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
