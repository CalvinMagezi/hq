//! Discord attachments: download into the vault and describe them for the prompt.

use super::*;

fn media_dir(vault_path: &Path) -> std::path::PathBuf {
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    vault_path.join("_media").join(date)
}

fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1_048_576 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    }
}

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];

pub(super) fn is_image(filename: &str) -> bool {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    IMAGE_EXTENSIONS.contains(&ext.as_str())
}

/// Mime type for a filename already confirmed image-shaped by `is_image`.
fn image_mime(filename: &str) -> &'static str {
    crate::relay_common::image_mime(filename.rsplit('.').next().unwrap_or(""))
}

/// Build the inline descriptor injected into the user message sent to the agent.
pub fn format_attachment_descriptor(filename: &str, size: u64, image: bool) -> String {
    let kind = if image { "[Image]" } else { "[File]" };
    format!("{kind} {filename} ({})", human_size(size))
}

/// Download a Discord attachment and save to _media/{date}/dc-{channel_id}-{filename}.
/// Prevents path traversal by using only the base filename component.
pub(super) async fn download_discord_attachment(
    url: &str,
    filename: &str,
    channel_id: u64,
    vault_path: &Path,
) -> anyhow::Result<std::path::PathBuf> {
    let dir = media_dir(vault_path);
    tokio::fs::create_dir_all(&dir).await?;

    // Prevent path traversal: extract only the base filename component
    let base = std::path::Path::new(filename)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("attachment");
    let safe_name = format!("dc-{channel_id}-{base}");
    let dest = dir.join(&safe_name);

    let bytes = reqwest::get(url).await?.bytes().await?;
    tokio::fs::write(&dest, &bytes).await?;
    Ok(dest)
}

/// Download every attachment. Returns the prompt descriptors, plus the images
/// so a vision-capable model sees them directly rather than only their names.
pub(super) async fn collect_attachments(
    msg: &Message,
    vault_path: &Path,
) -> (Vec<String>, Vec<hq_core::types::ImageAttachment>) {
    let mut descriptors = Vec::with_capacity(msg.attachments.len());
    let mut images = Vec::new();
    for att in &msg.attachments {
        let img = is_image(&att.filename);
        descriptors.push(format_attachment_descriptor(&att.filename, att.size as u64, img));
        let saved =
            download_discord_attachment(&att.url, &att.filename, msg.channel_id.get(), vault_path)
                .await;
        match saved {
            Ok(path) => {
                tracing::info!(path = %path.display(), "discord: saved attachment");
                if img {
                    let mime_type = image_mime(&att.filename).to_string();
                    images.push(hq_core::types::ImageAttachment { path, mime_type });
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, filename = %att.filename, "discord: attachment download failed");
            }
        }
    }
    (descriptors, images)
}
