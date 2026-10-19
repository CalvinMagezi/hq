//! The import pipeline: any supported file in, Markdown and a report out.
//!
//! `convert_file` looks at what the file is (signature first, name second),
//! picks a reader, and returns the Markdown with a [`ConversionReport`] that
//! says which engine ran and what was kept, flattened or dropped. Readers are
//! native Rust over the file's own XML where the generic extractor loses
//! structure; PDFs and images keep their text-layer and OCR paths.

mod docx;
mod html;
pub mod ir;
mod legacy;
mod odf;
pub mod package;
mod pdf;
mod pdf_layout;
mod pptx;
pub mod report;
mod rtf;
mod sheet;
pub mod sniff;
#[cfg(test)]
pub(crate) mod testutil;
mod text;
mod xmlconv;

use std::path::Path;
use std::sync::Arc;

use sha2::{Digest, Sha256};

pub use package::Limits;
pub use report::{ConversionReport, LossKind, Losses, Outcome, Stats, Warning};
pub use sniff::{By, Kind, Sniffed, sniff};

use crate::types::ConvertError;

/// What this host can do beyond the built-in readers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Capabilities {
    /// OCR for scanned PDFs and pictures of text.
    pub ocr: bool,
    /// Tables in PDFs rebuilt from the page layout (poppler's `pdftotext`).
    pub pdf_tables: bool,
    /// Old `.doc` and `.ppt` files, through LibreOffice.
    pub legacy_office: bool,
}

/// Probes the host for the optional tools the pipeline can use.
pub fn capabilities() -> Capabilities {
    Capabilities {
        ocr: crate::ocr::OcrEngine::available(),
        pdf_tables: which::which("pdftotext").is_ok(),
        legacy_office: legacy::available(),
    }
}

/// A reader for one package format (DOCX, PPTX, ODF).
type Reader = fn(&mut package::Package<'_>) -> Result<ir::Parsed, ConvertError>;

/// Whether a PDF or image with no text layer is read by OCR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrMode {
    Off,
    Auto,
}

#[derive(Debug, Clone)]
pub struct ConvertOptions {
    pub limits: Limits,
    /// Rows kept per sheet; the rest are counted in a warning.
    pub max_sheet_rows: usize,
    /// Output cut-off in characters.
    pub max_output_chars: usize,
    pub ocr: OcrMode,
    /// May an installed LibreOffice open legacy `.doc` and `.ppt` files.
    pub external_office: bool,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            max_sheet_rows: 1000,
            max_output_chars: 2_000_000,
            ocr: OcrMode::Auto,
            external_office: true,
        }
    }
}

#[derive(Debug)]
pub struct Converted {
    pub markdown: String,
    /// A title the file itself names (document properties or a title block).
    pub title: Option<String>,
    pub report: ConversionReport,
}

/// Converts the file at `path`.
pub async fn convert_file(path: &Path, opts: &ConvertOptions) -> Result<Converted, ConvertError> {
    let meta = tokio::fs::metadata(path).await?;
    if !meta.is_file() {
        return Err(ConvertError::InboundFailed(format!(
            "{} is not a file",
            path.display()
        )));
    }
    if meta.len() > opts.limits.max_input_bytes {
        return Err(ConvertError::TooLarge(format!(
            "{} bytes (limit {})",
            meta.len(),
            opts.limits.max_input_bytes
        )));
    }
    let bytes = tokio::fs::read(path).await?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string();
    convert_bytes(bytes, &ext, Some(path), opts).await
}

/// Converts file content already in memory. `ext` is the file's extension, if
/// it has one; `path` is the original location, used only by readers that need
/// a real file.
pub async fn convert_bytes(
    bytes: Vec<u8>,
    ext: &str,
    path: Option<&Path>,
    opts: &ConvertOptions,
) -> Result<Converted, ConvertError> {
    let _ = path;
    if bytes.len() as u64 > opts.limits.max_input_bytes {
        return Err(ConvertError::TooLarge(format!(
            "{} bytes (limit {})",
            bytes.len(),
            opts.limits.max_input_bytes
        )));
    }
    let bytes = Arc::new(bytes);
    let sniffed = sniff(&bytes, ext);
    if sniffed.encrypted {
        return Err(ConvertError::Encrypted(format!(
            "this {} file needs a password; save an unprotected copy and try again",
            sniffed.kind.label()
        )));
    }

    let mut losses = Losses::default();
    let ext = ext.to_ascii_lowercase();
    let ext = ext.as_str();
    let (markdown, title, engine) = match sniffed.kind {
        Kind::Docx => {
            let parsed = native(bytes.clone(), opts, docx::read).await?;
            finish(parsed, &mut losses, "native:docx".into())
        }
        Kind::Pptx => {
            let parsed = native(bytes.clone(), opts, pptx::read).await?;
            finish(parsed, &mut losses, "native:pptx".into())
        }
        Kind::Odt => {
            let parsed = native(bytes.clone(), opts, odf::read_text).await?;
            finish(parsed, &mut losses, "native:odt".into())
        }
        Kind::Odp => {
            let parsed = native(bytes.clone(), opts, odf::read_presentation).await?;
            finish(parsed, &mut losses, "native:odp".into())
        }
        Kind::Html => {
            let b = bytes.clone();
            let parsed = blocking(move || html::read(&b)).await?;
            finish(parsed, &mut losses, "native:html".into())
        }
        Kind::Xml => {
            let b = bytes.clone();
            let (nodes, rows) = (opts.limits.max_xml_nodes, opts.max_sheet_rows);
            let parsed = blocking(move || xmlconv::read(&b, nodes, rows)).await?;
            finish(parsed, &mut losses, "native:xml".into())
        }
        Kind::Csv | Kind::Tsv => {
            let b = bytes.clone();
            let (tab, rows) = (sniffed.kind == Kind::Tsv, opts.max_sheet_rows);
            let parsed = blocking(move || text::read_delimited(&b, tab, rows)).await?;
            finish(parsed, &mut losses, "native:csv".into())
        }
        Kind::Txt | Kind::Markdown => {
            let b = bytes.clone();
            let parsed = blocking(move || text::read_plain(&b)).await?;
            finish(parsed, &mut losses, "native:text".into())
        }
        Kind::Rtf => {
            let b = bytes.clone();
            let parsed = blocking(move || rtf::read(&b)).await?;
            finish(parsed, &mut losses, "native:rtf".into())
        }
        Kind::Xlsx | Kind::Xls | Kind::Ods => {
            let b = bytes.clone();
            let rows = opts.max_sheet_rows;
            let parsed = blocking(move || sheet::read(&b, rows)).await?;
            finish(
                parsed,
                &mut losses,
                format!("calamine:{}", sniffed.kind.label()),
            )
        }
        Kind::Pdf => {
            let dir = tempfile::tempdir().map_err(ConvertError::Io)?;
            let file = dir.path().join("input.pdf");
            tokio::fs::write(&file, bytes.as_slice()).await?;
            if let Some(layout) = pdf::layout(&file).await {
                let parsed = ir::Parsed {
                    blocks: layout.blocks,
                    title: None,
                    losses: Losses::default(),
                };
                finish(parsed, &mut losses, "poppler:pdftotext-layout".to_string())
            } else {
                let layer = pdf::text_layer(&file).await;
                match layer {
                    Ok(md) if md.trim().chars().count() >= pdf::MIN_TEXT_CHARS => {
                        if looks_tabular(&md) {
                            losses.note(
                            LossKind::LayoutNotPreserved,
                            "PDF text carries no table layout; rows and columns may be merged, so check amounts against the original",
                        );
                        }
                        (md, None, "transmutation:pdf".to_string())
                    }
                    other if opts.ocr == OcrMode::Auto => {
                        let _ = other;
                        let parsed = pdf::ocr_pdf(&file).await?;
                        finish(parsed, &mut losses, "ocr:tesseract".to_string())
                    }
                    Ok(_) => {
                        return Err(ConvertError::InboundFailed(
                            "this PDF has almost no text and OCR is off".into(),
                        ));
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        Kind::Image => {
            if opts.ocr == OcrMode::Off {
                return Err(ConvertError::UnsupportedFormat("image (OCR is off)".into()));
            }
            let dir = tempfile::tempdir().map_err(ConvertError::Io)?;
            let ext = if ext.is_empty() { "png" } else { ext };
            let file = dir.path().join(format!("input.{ext}"));
            tokio::fs::write(&file, bytes.as_slice()).await?;
            let parsed = pdf::ocr_image(&file).await?;
            finish(parsed, &mut losses, "ocr:tesseract".to_string())
        }
        Kind::Doc | Kind::Ppt => {
            if !opts.external_office || !legacy::available() {
                return Err(legacy::unavailable(sniffed.kind));
            }
            let (modern, kind) = legacy::upgrade(&bytes, sniffed.kind).await?;
            losses.note(
                LossKind::LegacyConverted,
                format!(
                    "old {} format opened through LibreOffice",
                    sniffed.kind.label()
                ),
            );
            let modern = Arc::new(modern);
            let (read, name): (Reader, &str) = match kind {
                Kind::Pptx => (pptx::read, "libreoffice+native:pptx"),
                _ => (docx::read, "libreoffice+native:docx"),
            };
            let parsed = native(modern, opts, read).await?;
            finish(parsed, &mut losses, name.to_string())
        }
        other => {
            return Err(ConvertError::UnsupportedFormat(other.label().to_string()));
        }
    };

    let mut warnings = losses.into_warnings();
    let mut markdown = markdown;
    if markdown.chars().count() > opts.max_output_chars {
        let cut: String = markdown.chars().take(opts.max_output_chars).collect();
        let at = cut.rfind("\n\n").unwrap_or(cut.len());
        markdown = format!("{}\n", cut[..at].trim_end());
        warnings.push(Warning {
            kind: LossKind::Truncated,
            detail: format!("output cut at {} characters", opts.max_output_chars),
        });
    }
    if markdown.trim().is_empty() {
        return Err(ConvertError::InboundFailed(
            "no text found in this file".to_string(),
        ));
    }
    if let Some(says) = sniffed.name_says {
        warnings.push(Warning {
            kind: LossKind::ExtensionMismatch,
            detail: format!(
                "the file name says {} but the content is {}",
                says.label(),
                sniffed.kind.label()
            ),
        });
    }
    let outcome = if warnings.iter().any(|w| w.kind.is_loss()) {
        Outcome::Degraded
    } else {
        Outcome::Ok
    };
    let report = ConversionReport {
        source_format: sniffed.kind.label().to_string(),
        detected_by: sniffed.by.label().to_string(),
        engine,
        outcome,
        warnings,
        stats: Stats::of(&markdown),
        source_bytes: bytes.len() as u64,
        source_sha256: format!("{:x}", Sha256::digest(bytes.as_slice())),
    };
    Ok(Converted {
        markdown,
        title,
        report,
    })
}

/// Runs a synchronous reader on the blocking pool.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, ConvertError> + Send + 'static,
) -> Result<T, ConvertError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ConvertError::Other(format!("reader stopped: {e}")))?
}

/// Runs a native reader on the blocking pool; they are CPU-bound and synchronous.
async fn native(
    bytes: Arc<Vec<u8>>,
    opts: &ConvertOptions,
    read: fn(&mut package::Package<'_>) -> Result<ir::Parsed, ConvertError>,
) -> Result<ir::Parsed, ConvertError> {
    let limits = opts.limits.clone();
    tokio::task::spawn_blocking(move || {
        let mut pkg = package::Package::open(&bytes, &limits)?;
        read(&mut pkg)
    })
    .await
    .map_err(|e| ConvertError::Other(format!("reader stopped: {e}")))?
}

/// A PDF whose text is mostly short lines carrying several numbers is probably a table.
fn looks_tabular(md: &str) -> bool {
    let numeric_lines = md
        .lines()
        .filter(|l| {
            l.split_whitespace()
                .filter(|w| {
                    w.chars().any(|c| c.is_ascii_digit())
                        && w.chars()
                            .all(|c| c.is_ascii_digit() || ",.-/:%()".contains(c))
                })
                .count()
                >= 3
        })
        .count();
    numeric_lines >= 6
}

fn finish(
    parsed: ir::Parsed,
    losses: &mut Losses,
    engine: String,
) -> (String, Option<String>, String) {
    losses.merge(parsed.losses);
    (ir::render(parsed.blocks), parsed.title, engine)
}
