//! Compile a [`Document`] with Typst and write PDF, SVG or PNG.

use std::path::PathBuf;

use typst_as_lib::{TypstEngine, typst_kit_options::TypstKitFontOptions};
use typst_layout::PagedDocument;

use crate::doc::Document;
use crate::error::ExportError;
use crate::theme::Theme;
use crate::typst_markup::{self, PageMode};

/// Longest PNG side we will produce, in pixels. Larger pages are scaled down
/// instead of exhausting memory on a very long note.
const MAX_PNG_SIDE: f64 = 16_000.0;

#[derive(Debug, Clone)]
pub struct RenderOptions {
    pub theme: Theme,
    /// Images are read only from below this directory. With `None`, no image
    /// is embedded.
    pub asset_root: Option<PathBuf>,
    /// Pixels per typographic point for PNG output (2.0 is about 144 dpi).
    pub png_pixels_per_pt: f64,
    /// Extra directories searched for fonts, after `HQ_EXPORT_FONT_DIR` and
    /// `~/.hq/fonts`.
    pub font_dirs: Vec<PathBuf>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            theme: Theme::default(),
            asset_root: None,
            png_pixels_per_pt: 2.0,
            font_dirs: Vec::new(),
        }
    }
}

/// Directories searched for fonts besides the system ones. This is how a
/// machine gets CJK or brand fonts without them being bundled in the binary.
fn font_dirs(extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("HQ_EXPORT_FONT_DIR") {
        dirs.extend(std::env::split_paths(&dir));
    }
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".hq").join("fonts"));
    }
    dirs.extend(extra.iter().cloned());
    dirs.retain(|d| d.is_dir());
    dirs
}

fn compile(
    doc: &Document,
    opts: &RenderOptions,
    mode: PageMode,
) -> Result<PagedDocument, ExportError> {
    match compile_once(doc, opts, mode, opts.asset_root.as_deref()) {
        Err(ExportError::Layout(msg)) if opts.asset_root.is_some() && msg.contains("image") => {
            // An attachment the engine cannot decode (corrupt, truncated, or a
            // format it does not support) must not cost the reader the whole
            // note. Retry with images off: they print as "[alt unavailable]".
            tracing::warn!(error = %msg, "export retried without images");
            compile_once(doc, opts, mode, None)
        }
        other => other,
    }
}

fn compile_once(
    doc: &Document,
    opts: &RenderOptions,
    mode: PageMode,
    asset_root: Option<&std::path::Path>,
) -> Result<PagedDocument, ExportError> {
    let markup = typst_markup::build(doc, &opts.theme, mode, asset_root);
    let assets: Vec<(&str, &[u8])> = markup
        .assets
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    let engine = TypstEngine::builder()
        .main_file(markup.source.as_str())
        .search_fonts_with(
            TypstKitFontOptions::default()
                .include_system_fonts(true)
                .include_embedded_fonts(true)
                .include_dirs(font_dirs(&opts.font_dirs)),
        )
        .with_static_file_resolver(assets)
        .build();
    let warned = engine.compile::<PagedDocument>();
    for w in &warned.warnings {
        tracing::debug!(warning = %w.message, "typst warning");
    }
    warned
        .output
        .map_err(|e| ExportError::Layout(e.to_string()))
}

/// A paginated A4 PDF.
pub fn pdf(doc: &Document, opts: &RenderOptions) -> Result<Vec<u8>, ExportError> {
    let compiled = compile(doc, opts, PageMode::Paged)?;
    typst_pdf::pdf(&compiled, &Default::default()).map_err(|e| {
        let msgs: Vec<String> = e.iter().map(|d| d.message.to_string()).collect();
        ExportError::Render(msgs.join("; "))
    })
}

/// SVG of the whole note as one tall page.
pub fn svg(doc: &Document, opts: &RenderOptions) -> Result<String, ExportError> {
    let compiled = compile(doc, opts, PageMode::Single)?;
    let page = compiled
        .pages()
        .first()
        .ok_or_else(|| ExportError::Render("document has no pages".into()))?;
    Ok(typst_svg::svg(page, &Default::default()))
}

/// SVG with one entry per A4 page.
pub fn svg_pages(doc: &Document, opts: &RenderOptions) -> Result<Vec<String>, ExportError> {
    let compiled = compile(doc, opts, PageMode::Paged)?;
    Ok(compiled
        .pages()
        .iter()
        .map(|p| typst_svg::svg(p, &Default::default()))
        .collect())
}

fn png_of(page: &typst_layout::Page, pixels_per_pt: f64) -> Result<Vec<u8>, ExportError> {
    let size = page.frame.size();
    let longest = size.x.to_pt().max(size.y.to_pt()).max(1.0);
    let scale = pixels_per_pt.min(MAX_PNG_SIDE / longest);
    let options = typst_render::RenderOptions {
        pixel_per_pt: typst_utils::Scalar::new(scale),
        ..Default::default()
    };
    typst_render::render(page, &options)
        .encode_png()
        .map_err(|e| ExportError::Render(e.to_string()))
}

/// PNG of the whole note as one tall image.
pub fn png(doc: &Document, opts: &RenderOptions) -> Result<Vec<u8>, ExportError> {
    let compiled = compile(doc, opts, PageMode::Single)?;
    let page = compiled
        .pages()
        .first()
        .ok_or_else(|| ExportError::Render("document has no pages".into()))?;
    png_of(page, opts.png_pixels_per_pt)
}

/// PNG with one entry per A4 page.
pub fn png_pages(doc: &Document, opts: &RenderOptions) -> Result<Vec<Vec<u8>>, ExportError> {
    let compiled = compile(doc, opts, PageMode::Paged)?;
    compiled
        .pages()
        .iter()
        .map(|p| png_of(p, opts.png_pixels_per_pt))
        .collect()
}
