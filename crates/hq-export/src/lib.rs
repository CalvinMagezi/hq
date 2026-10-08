//! hq-export: turn a vault note into shareable files without any external tool.
//!
//! A note is parsed once into a [`doc::Document`]. Writers then render that
//! model: [`render`] uses an embedded Typst engine for PDF, PNG and SVG.
//! Nothing here shells out, so an export works the same on a laptop, a server
//! and a container.

pub mod doc;
pub mod error;
pub mod render;
pub mod theme;
mod typst_markup;

pub use doc::Document;
pub use error::ExportError;
pub use render::RenderOptions;
pub use theme::Theme;
