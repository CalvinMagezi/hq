//! hq-convert — bidirectional document conversion for the HQ vault.
//!
//! **Inbound** (any format → Markdown): handled by the `transmutation` crate
//! (pure Rust, ~20MB footprint, zero Python deps). Supports PDF, DOCX, XLSX,
//! PPTX, HTML, XML, TXT, CSV, RTF and ODT. Audio, video and zip are rejected
//! because the matching transmutation features are off. Images are routed to
//! `ocr` instead (see below).
//!
//! **OCR**: images go through the `ocr` module (macOS Vision, or `tesseract`
//! on Linux), not transmutation, so extraction stays on-device with no LLM.
//!
//! **Outbound** (Markdown → any format): shells to `pandoc`. Returns a clear
//! error with install instructions if pandoc is not found.

pub mod attachments;
pub mod brand;
pub mod inbound;
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
