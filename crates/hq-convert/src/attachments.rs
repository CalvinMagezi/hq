//! Files a user attached to a chat message, turned into prompt text plus
//! image parts: plain text inlined, documents converted, scanned PDFs and
//! images OCR'd, and an inventory so the model names any file it could not read.
//!
//! The web chat and the Telegram relay both build their turns with this.

use std::path::{Path, PathBuf};

use crate::{DetectedFormat, InboundConverter, OcrEngine};

/// Inline document text cap, shared by plain-text files and converted documents.
pub const INLINE_DOC_CHAR_LIMIT: usize = 10_000;
/// Below this many characters a PDF is treated as scanned and sent to OCR.
const MIN_PDF_TEXT_CHARS: usize = 50;
/// Keeps failure reasons in the inventory to one short line.
const MAX_FAILURE_REASON_CHARS: usize = 120;

const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "csv", "md", "json", "yaml", "yml", "py", "ts", "tsx", "js", "jsx", "rs", "go", "java", "c",
    "cpp", "h", "toml", "ini", "cfg", "sh", "bash", "zsh", "html", "css", "xml", "sql", "rb", "php",
    "swift", "kt", "scala", "r", "log",
];
/// Images a vision model accepts as an image part. Others (HEIC, TIFF, BMP) are OCR'd only.
const VISION_IMAGES: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
];
const OCR_ONLY_IMAGES: &[&str] = &["heic", "heif", "tiff", "tif", "bmp"];

/// How an attachment's content reached the prompt, if it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    Extracted,
    Ocr,
    Image,
    Failed(String),
    Unsupported,
}

impl std::fmt::Display for FileStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Extracted => f.write_str("extracted"),
            Self::Ocr => f.write_str("ocr"),
            Self::Image => f.write_str("image"),
            Self::Failed(reason) => write!(f, "failed: {reason}"),
            Self::Unsupported => f.write_str("unsupported"),
        }
    }
}

/// One attachment, as listed in the prompt's inventory block.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub kind: String,
    /// `None` when the file never arrived, such as a failed download.
    pub path: Option<PathBuf>,
    pub status: FileStatus,
}

impl FileEntry {
    pub fn new(name: &str, kind: &str, path: Option<&Path>, status: FileStatus) -> Self {
        Self {
            name: name.to_string(),
            kind: kind.to_string(),
            path: path.map(Path::to_path_buf),
            status,
        }
    }
}

/// What a set of attachments adds to a turn.
#[derive(Debug, Default)]
pub struct PreparedAttachments {
    /// The inventory block that goes before the user's text; empty with no files.
    pub inventory: String,
    /// Inline document text, OCR text and file references that go after it.
    pub body: String,
    /// Images to send as image parts, with their mime type.
    pub images: Vec<(PathBuf, &'static str)>,
    pub files: Vec<FileEntry>,
}

impl PreparedAttachments {
    /// The full prompt: inventory, then the user's text, then the file contents.
    pub fn prompt(&self, user_text: &str) -> String {
        if self.inventory.is_empty() {
            return format!("{user_text}{}", self.body);
        }
        format!("{}\n\n{user_text}{}", self.inventory, self.body)
    }
}

/// A failure status with the first line of `err`, capped for the inventory.
pub fn failed(err: impl std::fmt::Display) -> FileStatus {
    let first_line = err.to_string().lines().next().unwrap_or("").to_string();
    FileStatus::Failed(first_line.chars().take(MAX_FAILURE_REASON_CHARS).collect())
}

fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// The image-part mime type for a file a vision model can take, by extension.
pub fn vision_mime(path: &Path) -> Option<&'static str> {
    let ext = extension(path);
    VISION_IMAGES.iter().find(|(e, _)| *e == ext).map(|(_, mime)| *mime)
}

/// Lists every attachment with its status so the model cannot silently skip one.
pub fn build_inventory(files: &[FileEntry]) -> String {
    if files.is_empty() {
        return String::new();
    }
    let mut out = format!("[Attachments this turn: {}]\n", files.len());
    for (i, f) in files.iter().enumerate() {
        out.push_str(&format!("{}. {} ({}): {}", i + 1, f.name, f.kind, f.status));
        if let Some(path) = &f.path {
            out.push_str(&format!(" [{}]", path.display()));
        }
        out.push('\n');
    }
    out.push_str("Address every listed attachment; name any you could not read.");
    out
}

/// Pulls text out of a non-plain-text document, falling back to page OCR for
/// PDFs whose text layer is missing or near-empty.
pub async fn extract_document(path: &Path) -> (Option<String>, FileStatus) {
    let format = InboundConverter::detect_format(path);
    if !format.is_supported_inbound() {
        return (None, FileStatus::Unsupported);
    }
    let converted = match InboundConverter::new() {
        Ok(converter) => converter.convert(path).await,
        Err(e) => Err(e),
    };
    let is_pdf = format == DetectedFormat::Pdf;
    match converted {
        Ok(text) if !is_pdf || text.trim().chars().count() >= MIN_PDF_TEXT_CHARS => (Some(text), FileStatus::Extracted),
        Err(e) if !is_pdf => (None, failed(e)),
        _ => match OcrEngine::extract_pdf(path).await {
            Ok(text) if !text.trim().is_empty() => (Some(text), FileStatus::Ocr),
            Ok(_) => (None, failed("no text found, even after OCR")),
            Err(e) => (None, failed(e)),
        },
    }
}

fn inline_document(name: &str, path: &Path, contents: &str) -> String {
    let truncated: String = contents.chars().take(INLINE_DOC_CHAR_LIMIT).collect();
    format!("\n\n[Document: {name} (saved to {})]\n```\n{truncated}\n```", path.display())
}

/// Reads, converts or OCRs each `(display name, saved path)` into one turn's additions.
pub async fn prepare_attachments(files: &[(String, PathBuf)]) -> PreparedAttachments {
    let mut out = PreparedAttachments::default();
    for (name, path) in files {
        let (kind, status) = prepare_one(&mut out, name, path).await;
        if let FileStatus::Failed(reason) = &status {
            tracing::warn!(file = %name, %reason, "attachment could not be read");
        }
        out.files.push(FileEntry::new(name, &kind, Some(path), status));
    }
    out.inventory = build_inventory(&out.files);
    out
}

async fn prepare_one(out: &mut PreparedAttachments, name: &str, path: &Path) -> (String, FileStatus) {
    let ext = extension(path);
    if let Some(mime) = vision_mime(path) {
        out.body.push_str(&format!("\n\n[Image attached: {}]", path.display()));
        push_ocr_text(out, path).await;
        out.images.push((path.to_path_buf(), mime));
        return (mime.to_string(), FileStatus::Image);
    }
    if OCR_ONLY_IMAGES.contains(&ext.as_str()) {
        out.body.push_str(&format!("\n\n[Image attached, not viewable by the model: {}]", path.display()));
        let status = if push_ocr_text(out, path).await { FileStatus::Ocr } else { failed("no text found by OCR") };
        return (format!("image/{ext}"), status);
    }
    let (text, status) = if TEXT_EXTENSIONS.contains(&ext.as_str()) {
        match tokio::fs::read_to_string(path).await {
            Ok(contents) => (Some(contents), FileStatus::Extracted),
            Err(e) => (None, failed(e)),
        }
    } else {
        extract_document(path).await
    };
    match &text {
        Some(text) => out.body.push_str(&inline_document(name, path, text)),
        None => out.body.push_str(&format!("\n\n[Document attached: {}]", path.display())),
    }
    (if ext.is_empty() { "file".to_string() } else { ext }, status)
}

/// Appends OCR text for an image when there is any; returns whether there was.
async fn push_ocr_text(out: &mut PreparedAttachments, path: &Path) -> bool {
    match OcrEngine::extract_text(path).await {
        Ok(text) if !text.trim().is_empty() => {
            out.body.push_str(&format!("\n[OCR text from image]:\n{text}"));
            true
        }
        Ok(_) => false,
        Err(e) => {
            tracing::debug!(path = %path.display(), "OCR unavailable: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_is_empty_without_files() {
        assert_eq!(build_inventory(&[]), "");
    }

    #[test]
    fn only_formats_a_vision_model_takes_become_image_parts() {
        assert_eq!(vision_mime(Path::new("a.PNG")), Some("image/png"));
        assert_eq!(vision_mime(Path::new("a.jpeg")), Some("image/jpeg"));
        assert_eq!(vision_mime(Path::new("a.heic")), None);
        assert_eq!(vision_mime(Path::new("a.pdf")), None);
    }

    #[tokio::test]
    async fn text_files_are_inlined_and_listed() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes.md");
        std::fs::write(&notes, "# Plan\nship it").unwrap();

        let prepared = prepare_attachments(&[("notes.md".into(), notes.clone())]).await;

        assert!(prepared.images.is_empty());
        assert_eq!(prepared.files[0].status, FileStatus::Extracted);
        assert!(prepared.body.contains("ship it"), "{}", prepared.body);
        assert!(prepared.inventory.contains("1. notes.md (md): extracted"), "{}", prepared.inventory);
        let prompt = prepared.prompt("read this");
        assert!(prompt.starts_with("[Attachments this turn: 1]"), "{prompt}");
        assert!(prompt.contains("\n\nread this\n\n[Document: notes.md"), "{prompt}");
    }

    #[tokio::test]
    async fn long_text_is_cut_to_the_inline_limit() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big.txt");
        std::fs::write(&big, "x".repeat(INLINE_DOC_CHAR_LIMIT + 500)).unwrap();

        let prepared = prepare_attachments(&[("big.txt".into(), big)]).await;

        let fenced = prepared.body.split("```\n").nth(1).unwrap().trim_end_matches("\n```");
        assert_eq!(fenced.chars().count(), INLINE_DOC_CHAR_LIMIT);
    }

    #[tokio::test]
    async fn images_become_image_parts_and_unknown_files_are_referenced() {
        let dir = tempfile::tempdir().unwrap();
        let shot = dir.path().join("shot.png");
        let blob = dir.path().join("archive.bin");
        std::fs::write(&shot, b"not really a png").unwrap();
        std::fs::write(&blob, b"\0\x01").unwrap();

        let prepared = prepare_attachments(&[("shot.png".into(), shot.clone()), ("archive.bin".into(), blob.clone())]).await;

        assert_eq!(prepared.images, vec![(shot, "image/png")]);
        assert_eq!(prepared.files[0].status, FileStatus::Image);
        assert_eq!(prepared.files[1].status, FileStatus::Unsupported);
        assert!(prepared.body.contains(&format!("[Document attached: {}]", blob.display())));
    }

    #[test]
    fn inventory_lists_every_file_and_names_failures() {
        let files = vec![
            FileEntry::new("report.pdf", "pdf", Some(Path::new("/m/report.pdf")), FileStatus::Extracted),
            FileEntry::new("scan.pdf", "pdf", None, FileStatus::Ocr),
            FileEntry::new("broken.docx", "docx", None, failed("zip header invalid\nmore")),
            FileEntry::new("data.bin", "bin", None, FileStatus::Unsupported),
        ];
        let inv = build_inventory(&files);
        assert!(inv.starts_with("[Attachments this turn: 4]\n"), "{inv}");
        assert!(inv.contains("1. report.pdf (pdf): extracted [/m/report.pdf]\n"), "{inv}");
        assert!(inv.contains("2. scan.pdf (pdf): ocr\n"), "{inv}");
        assert!(inv.contains("3. broken.docx (docx): failed: zip header invalid\n"), "{inv}");
        assert!(inv.contains("4. data.bin (bin): unsupported"), "{inv}");
        assert!(build_inventory(&[]).is_empty());
    }

    #[test]
    fn failure_reason_is_capped() {
        let FileStatus::Failed(reason) = failed("x".repeat(500)) else {
            panic!("expected failed");
        };
        assert_eq!(reason.chars().count(), MAX_FAILURE_REASON_CHARS);
    }
}
