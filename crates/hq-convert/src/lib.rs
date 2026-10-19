//! hq-convert — bidirectional document conversion for the HQ vault.
//!
//! **Inbound** (any format → Markdown): the import pipeline in [`ingest`]. It decides
//! what a file is from its content before its name, reads DOCX, PPTX, ODT, ODP, RTF, HTML
//! and XML natively, spreadsheets (XLSX, XLS, ODS) through calamine, rebuilds tables in PDFs
//! from the page layout, and returns a report of what it kept, flattened or left out. Old
//! `.doc` and `.ppt` files open through LibreOffice when it is installed. The older
//! [`InboundConverter`] entry point is kept and runs on the same pipeline.
//!
//! **OCR**: scanned PDFs and pictures of text are read by the `ocr` module (macOS Vision, or
//! `tesseract` on Linux), so extraction stays on-device with no LLM.
//!
//! **Shareable PDFs** (vault note → PDF): see `note_pdf`. Renders through a headless
//! Chromium-family browser or WeasyPrint when present, falling back to `xelatex`.
//!
//! **Outbound** (Markdown → any format): shells to `pandoc`. Returns a clear
//! error with install instructions if pandoc is not found.

pub mod attachments;
pub mod brand;
pub mod inbound;
pub mod ingest;
pub mod note_pdf;
pub mod ocr;
pub mod outbound;
pub mod types;

pub use inbound::InboundConverter;
pub use ocr::OcrEngine;
pub use types::DetectedFormat;

/// Runs an external tool to completion and returns its stdout, or a message
/// naming `name` with the exit status and stderr.
pub(crate) async fn run_tool(
    mut cmd: tokio::process::Command,
    name: &str,
) -> Result<Vec<u8>, String> {
    let output = cmd
        .output()
        .await
        .map_err(|e| format!("failed to run {name}: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{name} exited with {}: {stderr}", output.status));
    }
    Ok(output.stdout)
}
