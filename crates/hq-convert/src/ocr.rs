//! On-device OCR: the Vision framework on macOS, a system `tesseract` on Linux.
//!
//! On macOS a bundled Swift script is compiled once with `swiftc` (ships with
//! the Xcode Command Line Tools) and the binary is cached in the temp dir.
//! Deterministic and offline: extraction never touches an LLM, so it works
//! the same regardless of which model is currently answering.

use std::path::Path;

use crate::types::ConvertError;

#[cfg(target_os = "macos")]
const VISION_OCR_SCRIPT: &str = include_str!("../assets/vision_ocr.swift");

/// On-device text extraction from images.
pub struct OcrEngine;

const PDF_RASTER_DPI: u32 = 200;
/// Bounds OCR time on long scans; the full file stays on disk for tools.
const PDF_OCR_MAX_PAGES: u32 = 20;

/// Compiled once per script content (content-hashed cache path, so an
/// updated `vision_ocr.swift` in a future build invalidates itself rather
/// than silently reusing a stale binary) and reused for the rest of this
/// process — and across daemon restarts, since the binary is cached on disk,
/// not just in memory. `swift <script>` re-interprets from source every
/// call, ~1.2s dominated by interpreter startup; `swiftc` compiles once.
#[cfg(target_os = "macos")]
static COMPILED_BINARY: tokio::sync::OnceCell<std::path::PathBuf> =
    tokio::sync::OnceCell::const_new();

#[cfg(target_os = "macos")]
async fn compiled_binary_path() -> Result<&'static Path, ConvertError> {
    COMPILED_BINARY
        .get_or_try_init(|| async {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            VISION_OCR_SCRIPT.hash(&mut hasher);
            let hash = hasher.finish();

            let cache_dir = std::env::temp_dir().join("hq-vision-ocr");
            tokio::fs::create_dir_all(&cache_dir)
                .await
                .map_err(ConvertError::Io)?;
            let bin_path = cache_dir.join(format!("vision_ocr_{hash:x}"));

            if !bin_path.exists() {
                let script_path = cache_dir.join(format!("vision_ocr_{hash:x}.swift"));
                tokio::fs::write(&script_path, VISION_OCR_SCRIPT)
                    .await
                    .map_err(ConvertError::Io)?;
                let status = tokio::process::Command::new("swiftc")
                    .kill_on_drop(true)
                    .arg("-O")
                    .arg("-o")
                    .arg(&bin_path)
                    .arg(&script_path)
                    .status()
                    .await
                    .map_err(|e| {
                        ConvertError::Other(format!("failed to compile OCR script: {e}"))
                    })?;
                let _ = tokio::fs::remove_file(&script_path).await;
                if !status.success() {
                    return Err(ConvertError::Other(
                        "swiftc failed to compile vision_ocr.swift".to_string(),
                    ));
                }
            }
            Ok(bin_path)
        })
        .await
        .map(|p| p.as_path())
}

impl OcrEngine {
    /// Extract text from an image file using Apple's Vision framework.
    #[cfg(target_os = "macos")]
    pub async fn extract_text(image_path: &Path) -> Result<String, ConvertError> {
        let mut cmd = tokio::process::Command::new(compiled_binary_path().await?);
        cmd.kill_on_drop(true).arg(image_path);
        let stdout = crate::run_tool(cmd, "OCR binary").await.map_err(ConvertError::Other)?;
        Ok(String::from_utf8_lossy(&stdout).trim().to_string())
    }

    /// Text extraction via a system `tesseract` binary (FR-017). Not a Rust
    /// dependency — mirrors the macOS branch's pattern of shelling out to an
    /// external OCR tool rather than vendoring one. `find_tesseract_bin`
    /// tries `which` first, then a fixed list of common install locations.
    #[cfg(target_os = "linux")]
    pub async fn extract_text(image_path: &Path) -> Result<String, ConvertError> {
        let mut cmd = tokio::process::Command::new(find_tesseract_bin().await?);
        // `stdout` as the output base tells tesseract to write to stdout
        // instead of `<base>.txt`.
        cmd.kill_on_drop(true).arg(image_path).arg("stdout");
        let stdout = crate::run_tool(cmd, "tesseract").await.map_err(ConvertError::Other)?;
        Ok(String::from_utf8_lossy(&stdout).trim().to_string())
    }

    /// Rasterize a PDF with `pdftoppm` and OCR each page, for scanned PDFs
    /// that carry no text layer. Pages beyond `PDF_OCR_MAX_PAGES` are skipped.
    pub async fn extract_pdf(pdf_path: &Path) -> Result<String, ConvertError> {
        let pdftoppm = which::which("pdftoppm").map_err(|_| {
            ConvertError::Other(
                "pdftoppm not found. Install: apt install poppler-utils (or brew install poppler)"
                    .to_string(),
            )
        })?;
        let tmp = tempfile::tempdir().map_err(ConvertError::Io)?;
        let mut cmd = tokio::process::Command::new(pdftoppm);
        cmd.kill_on_drop(true)
            .args(["-r", &PDF_RASTER_DPI.to_string()])
            .args(["-l", &PDF_OCR_MAX_PAGES.to_string()])
            .arg("-png")
            .arg(pdf_path)
            .arg(tmp.path().join("p"));
        crate::run_tool(cmd, "pdftoppm").await.map_err(ConvertError::Other)?;

        // pdftoppm zero-pads page numbers to a common width, so a name sort is page order.
        let mut pages: Vec<_> = std::fs::read_dir(tmp.path())
            .map_err(ConvertError::Io)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "png"))
            .collect();
        pages.sort();

        let mut text = String::new();
        for (i, page) in pages.iter().enumerate() {
            let page_text = Self::extract_text(page).await?;
            if !page_text.trim().is_empty() {
                text.push_str(&format!("[page {}]\n{}\n\n", i + 1, page_text.trim()));
            }
        }
        Ok(text.trim_end().to_string())
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub async fn extract_text(_image_path: &Path) -> Result<String, ConvertError> {
        Err(ConvertError::Other(
            "OCR is not available on this platform".to_string(),
        ))
    }
}

#[cfg(target_os = "linux")]
const TESSERACT_BIN_FALLBACKS: &[&str] = &["/usr/bin/tesseract", "/usr/local/bin/tesseract"];

#[cfg(target_os = "linux")]
async fn find_tesseract_bin() -> Result<String, ConvertError> {
    if let Ok(path) = which::which("tesseract") {
        return Ok(path.to_string_lossy().into_owned());
    }
    for fallback in TESSERACT_BIN_FALLBACKS {
        if tokio::fs::try_exists(fallback).await.unwrap_or(false) {
            return Ok(fallback.to_string());
        }
    }
    Err(ConvertError::Other(
        "tesseract not found. Install: apt install tesseract-ocr".to_string(),
    ))
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    /// Needs a real image and the `swift` toolchain, so it's opt-in like the
    /// rest of this repo's external-state tests.
    /// Run: OCR_TEST_IMAGE=/path/to/image.png cargo test -p hq-convert -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires OCR_TEST_IMAGE env var pointing at a real image"]
    async fn extract_text_from_env_image() {
        let path = std::env::var("OCR_TEST_IMAGE").expect("set OCR_TEST_IMAGE");
        let text = OcrEngine::extract_text(Path::new(&path)).await.unwrap();
        println!("{text}");
        assert!(!text.trim().is_empty());
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;

    /// FR-017: the Linux fallback must never surface the old macOS-only
    /// error string. A nonexistent path still exercises the "tesseract not
    /// found" branch when tesseract isn't installed on the test host, or the
    /// "tesseract failed" branch when it is — either way, the message must
    /// not mention macOS/Vision framework.
    #[tokio::test]
    async fn error_path_never_mentions_macos() {
        let result = OcrEngine::extract_text(Path::new("/nonexistent/not-a-real-image.png")).await;
        if let Err(ConvertError::Other(msg)) = result {
            assert!(!msg.contains("macOS"), "{msg}");
            assert!(!msg.contains("Vision framework"), "{msg}");
        }
    }
}

#[cfg(test)]
mod pdf_tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    #[tokio::test]
    #[ignore = "needs pdftoppm and tesseract"]
    async fn extract_pdf_reads_text_pdf() {
        let text = OcrEngine::extract_pdf(&fixture("text.pdf")).await.unwrap();
        assert!(text.starts_with("[page 1]"), "{text}");
        assert!(text.contains("4821"), "{text}");
    }

    #[tokio::test]
    #[ignore = "needs pdftoppm and tesseract"]
    async fn extract_pdf_reads_image_only_pdf() {
        let text = OcrEngine::extract_pdf(&fixture("scanned.pdf"))
            .await
            .unwrap();
        assert!(text.contains("7093"), "{text}");
    }
}
