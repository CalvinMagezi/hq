//! hq-export: turn a vault note into shareable files without any external tool.
//!
//! A note is parsed once into a [`doc::Document`]. Writers then render that
//! model: [`render`] uses an embedded Typst engine for PDF, PNG and SVG, and the
//! other writers produce HTML, Markdown, spreadsheets, JSON, XML, LaTeX,
//! notebooks and Jira markup. [`export`] is the single entry point.
//! Nothing here shells out, so an export works the same on a laptop, a server
//! and a container.

mod assets;
mod code;
pub mod doc;
pub mod error;
mod formats;
mod html;
mod jira;
pub mod markdown;
mod note;
mod notebook;
pub mod render;
pub mod theme;
mod tables;
mod typst_markup;
mod util;

pub use doc::Document;
pub use error::ExportError;
pub use formats::{ExportOptions, Format, Output, export};
pub use note::{NoteExport, export_note, export_note_blocking};
pub use render::RenderOptions;
pub use theme::Theme;
