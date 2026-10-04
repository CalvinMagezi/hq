//! Media download and upload helpers for the Telegram relay.
//!
//! Handles photos, documents, voice notes, videos, and outbound file delivery.

use anyhow::Result;
use hq_convert::attachments::{self, FileEntry};

use teloxide::prelude::*;
use teloxide::types::FileId;

/// Result of `handle_media`: the augmented prompt text plus any images
/// found, so a vision-capable model can see them directly (FR-017) rather
/// than relying solely on OCR text baked into the prompt.
pub struct MediaResult {
    pub text: String,
    pub images: Vec<hq_core::types::ImageAttachment>,
    pub files: Vec<FileEntry>,
    /// `text` without the inventory block, so an album can list all its files once.
    pub body: String,
}

/// Folds the messages of one Telegram album into a single turn with one inventory.
pub fn merge_album(parts: Vec<MediaResult>) -> MediaResult {
    let mut files = Vec::new();
    let mut images = Vec::new();
    let mut bodies = Vec::new();
    for p in parts {
        files.extend(p.files);
        images.extend(p.images);
        if !p.body.trim().is_empty() {
            bodies.push(p.body);
        }
    }
    let body = bodies.join("\n\n");
    MediaResult {
        text: with_inventory(&files, &body),
        images,
        files,
        body,
    }
}

fn with_inventory(files: &[FileEntry], body: &str) -> String {
    let inventory = attachments::build_inventory(files);
    if inventory.is_empty() {
        body.to_string()
    } else {
        format!("{inventory}\n\n{body}")
    }
}

/// Download a file from Telegram's CDN to a local path.
async fn download_file(
    bot: &Bot,
    token: &str,
    file_id: &FileId,
    dest: &std::path::Path,
) -> Result<std::path::PathBuf> {
    use teloxide::prelude::Requester;

    let tg_file = bot
        .get_file(file_id.clone())
        .await
        .map_err(|e| anyhow::anyhow!("telegram get_file: {e}"))?;
    let file_path = tg_file.path;
    let url = format!("https://api.telegram.org/file/bot{token}/{file_path}");
    let bytes = reqwest::get(&url)
        .await
        .map_err(|_| anyhow::anyhow!("download file failed (id: {file_id})"))?
        .bytes()
        .await
        .map_err(|e| anyhow::anyhow!("read file bytes: {e}"))?;

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(dest, &bytes)?;
    Ok(dest.to_path_buf())
}

/// What one message's attachments add to the turn.
struct MediaOut<'a> {
    bot: &'a Bot,
    token: &'a str,
    media_dir: std::path::PathBuf,
    augmented: String,
    images: Vec<hq_core::types::ImageAttachment>,
    files: Vec<FileEntry>,
}

/// Download any media attachments from a message and append descriptors/content to the text.
/// Returns the original text augmented with attachment descriptions (or inline file content
/// for text files). Voice notes are saved and referenced, not transcribed.
pub async fn handle_media(
    bot: &Bot,
    msg: &teloxide::types::Message,
    text: String,
    vault_path: &std::path::Path,
    token: &str,
) -> MediaResult {
    let date_str = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let mut out = MediaOut {
        bot,
        token,
        media_dir: vault_path.join("_media").join(&date_str),
        augmented: text,
        images: Vec::new(),
        files: Vec::new(),
    };
    if let Some(photo) = msg.photo().and_then(|p| p.last()) {
        out.photo(msg, photo).await;
    }
    if let Some(doc) = msg.document() {
        out.document(doc).await;
    }
    if let Some(voice) = msg.voice() {
        let filename = format!("voice_{}.ogg", msg.id.0);
        out.saved_only(&filename, &voice.file.id, "Voice note", " (not transcribed)").await;
    }
    if let Some(video) = msg.video() {
        let ext = video
            .mime_type
            .as_ref()
            .map(|m| m.subtype().as_str().to_string())
            .unwrap_or_else(|| "mp4".to_string());
        let filename = format!("video_{}_{}.{ext}", msg.id.0, video.file.unique_id);
        out.saved_only(&filename, &video.file.id, "Video", "").await;
    }
    MediaResult {
        text: with_inventory(&out.files, &out.augmented),
        images: out.images,
        files: out.files,
        body: out.augmented,
    }
}

impl MediaOut<'_> {
    async fn download(&self, file_id: &FileId, filename: &str) -> Result<std::path::PathBuf> {
        download_file(self.bot, self.token, file_id, &self.media_dir.join(filename)).await
    }

    /// A compressed photo, attached as an image part (FR-017).
    async fn photo(&mut self, msg: &teloxide::types::Message, photo: &teloxide::types::PhotoSize) {
        let filename = format!("photo_{}_{}.jpg", msg.id.0, photo.file.unique_id);
        self.download_and_attach(&photo.file.id, &filename, "image/jpeg").await;
    }

    async fn document(&mut self, doc: &teloxide::types::Document) {
        let filename = doc
            .file_name
            .clone()
            .unwrap_or_else(|| format!("doc_{}", doc.file.unique_id));
        self.download_and_attach(&doc.file.id, &filename, "document").await;
    }

    /// Text is inlined, documents converted, images sent as image parts with
    /// OCR text, the same way the web chat reads an upload.
    async fn download_and_attach(&mut self, file_id: &FileId, filename: &str, kind: &str) {
        let path = match self.download(file_id, filename).await {
            Ok(path) => path,
            Err(e) => {
                tracing::warn!("telegram: failed to download {filename}: {e}");
                let status = attachments::failed("download failed");
                self.files.push(FileEntry::new(filename, kind, None, status));
                return;
            }
        };
        let prepared = attachments::prepare_attachments(&[(filename.to_string(), path)]).await;
        self.augmented.push_str(&prepared.body);
        self.images.extend(prepared.images.into_iter().map(|(path, mime)| {
            hq_core::types::ImageAttachment { path, mime_type: mime.to_string() }
        }));
        self.files.extend(prepared.files);
    }

    /// Voice notes and videos are saved and referenced, not read.
    async fn saved_only(&mut self, filename: &str, file_id: &FileId, label: &str, note: &str) {
        match self.download(file_id, filename).await {
            Ok(path) => self
                .augmented
                .push_str(&format!("\n[{label} attached: {}{note}]", path.display())),
            Err(e) => tracing::warn!("telegram: failed to download {label}: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_convert::attachments::FileStatus;

    #[test]
    fn an_album_becomes_one_turn_with_one_inventory() {
        let part = |name: &str, body: &str| {
            let files = vec![FileEntry::new(name, "pdf", None, FileStatus::Extracted)];
            MediaResult {
                text: with_inventory(&files, body),
                images: Vec::new(),
                files,
                body: body.to_string(),
            }
        };
        let merged = merge_album(vec![
            part("A level.pdf", "which universities?\n[A level text]"),
            part("GCSE.pdf", "[GCSE text]"),
        ]);
        assert!(
            merged.text.starts_with("[Attachments this turn: 2]\n"),
            "{}",
            merged.text
        );
        assert_eq!(merged.text.matches("[Attachments this turn").count(), 1);
        assert!(merged.text.contains("[A level text]") && merged.text.contains("[GCSE text]"));
        assert_eq!(merged.files.len(), 2);
    }
}
