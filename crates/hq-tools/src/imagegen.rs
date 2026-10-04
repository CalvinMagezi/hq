//! Image generation tool through OpenRouter, trying the cheapest capable
//! image model first and moving down the chain on transient failures.

use anyhow::{Result, bail};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tokio::fs;

use crate::registry::HqTool;

const MODELS_CHEAPEST_FIRST: &[&str] = &[
    "google/gemini-2.5-flash-image", // $0.0003/1k img — cheapest confirmed
    "google/gemini-3.1-flash-image-preview", // next tier
    "google/gemini-3-pro-image-preview", // $0.002/1k img — fallback quality
    "openai/gpt-5-image-mini",
];

const OR_BASE: &str = "https://openrouter.ai/api/v1";

/// Image generation through OpenRouter.
pub struct ImageGenTool {
    vault_path: PathBuf,
    http: Client,
    openrouter_key: Option<String>,
}

impl ImageGenTool {
    pub fn new(vault_path: PathBuf, openrouter_key: Option<String>) -> Self {
        Self {
            vault_path,
            http: Client::new(),
            openrouter_key: openrouter_key.filter(|k| !k.is_empty()),
        }
    }

    /// Config takes priority over env, matching the resolution order every
    /// other OpenRouter consumer uses (`hq-agent/src/builder.rs`). imagegen
    /// used to read the env var only, which silently no-oped whenever the
    /// key lived in `~/.hq/config.yaml` instead.
    fn resolve_openrouter_key(&self) -> Option<String> {
        self.openrouter_key
            .clone()
            .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
            .filter(|k| !k.is_empty())
    }

    async fn call_openrouter(&self, api_key: &str, model_id: &str, prompt: &str) -> Result<String> {
        let mut req = self
            .http
            .post(format!("{OR_BASE}/chat/completions"))
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .header("X-Title", "Agent-HQ");
        if let Some(referer) = hq_core::config::http_referer() {
            req = req.header("HTTP-Referer", referer);
        }
        let resp = req
            .timeout(std::time::Duration::from_secs(120))
            .json(&json!({
                "model": model_id,
                "messages": [{"role": "user", "content": prompt}],
                "modalities": ["image", "text"],
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("OpenRouter {status}: {body}");
        }

        let data: Value = resp.json().await?;
        let images = data
            .pointer("/choices/0/message/images")
            .and_then(|v| v.as_array());

        let images = match images {
            Some(imgs) if !imgs.is_empty() => imgs,
            _ => bail!("No images returned from {model_id}"),
        };

        let img = &images[0];
        // Try various response shapes
        let url = img
            .pointer("/image_url/url")
            .or_else(|| img.get("url"))
            .and_then(|v| v.as_str())
            .or_else(|| img.as_str());

        match url {
            Some(u) => Ok(u.to_string()),
            None => bail!("Could not extract image URL from OpenRouter response"),
        }
    }

    async fn save_image(&self, image_url: &str, model_id: &str) -> Result<(PathBuf, String)> {
        let (bytes, mime_type) = if image_url.starts_with("data:") {
            let comma_idx = image_url.find(',').unwrap_or(0);
            let header = &image_url[..comma_idx];
            let b64 = &image_url[comma_idx + 1..];
            use base64::Engine;
            let decoded = base64::engine::general_purpose::STANDARD.decode(b64)?;
            let mime = header
                .split(':')
                .nth(1)
                .and_then(|s| s.split(';').next())
                .unwrap_or("image/png")
                .to_string();
            (decoded, mime)
        } else {
            // The URL comes from a model's response, so it gets the SSRF-guarded client.
            crate::web::check_public_url(image_url)?;
            let resp = crate::web::guarded_client()
                .get(image_url)
                .timeout(std::time::Duration::from_secs(60))
                .send()
                .await?;
            let mime = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("image/png")
                .to_string();
            let bytes = resp.bytes().await?.to_vec();
            (bytes, mime)
        };

        let ext = safe_image_extension(&mime_type);
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let hash = hex::encode(&hasher.finalize()[..4]);
        let now = chrono::Utc::now().timestamp_millis();
        let filename = format!("img-{now}-{hash}.{ext}");

        let output_dir = self.vault_path.join("_system").join("outputs");
        fs::create_dir_all(&output_dir).await?;
        let file_path = output_dir.join(&filename);
        fs::write(&file_path, &bytes).await?;

        Ok((file_path, model_id.to_string()))
    }

    /// Try every model in the cheapest-first chain (or just `model_override`
    /// if given).
    async fn generate_via_openrouter(
        &self,
        api_key: &str,
        prompt_text: &str,
        model_override: Option<&str>,
    ) -> Result<Value> {
        let models: Vec<&str> = if let Some(m) = model_override {
            vec![m]
        } else {
            MODELS_CHEAPEST_FIRST.to_vec()
        };

        let mut last_error = String::new();
        for model_id in &models {
            match self.call_openrouter(api_key, model_id, prompt_text).await {
                Ok(image_url) => {
                    let (file_path, model) = self.save_image(&image_url, model_id).await?;
                    let display_name = file_path.file_name().unwrap_or_default().to_string_lossy();
                    return Ok(json!({
                        "message": format!(
                            "Image generated ({model}):\n[FILE: {} | {display_name}]",
                            file_path.display()
                        ),
                        "filePath": file_path.to_string_lossy(),
                        "model": model,
                        "engine": "openrouter",
                    }));
                }
                Err(e) => {
                    last_error = e.to_string();
                    let is_transient = last_error.contains("404")
                        || last_error.contains("503")
                        || last_error.contains("429")
                        || last_error.contains("rate")
                        || last_error.contains("timeout")
                        || last_error.contains("No endpoints");
                    if !is_transient || model_override.is_some() {
                        bail!("{last_error}");
                    }
                    tracing::warn!("[imageGen] {model_id} failed ({last_error}), trying next...");
                }
            }
        }

        bail!("All OpenRouter models failed. Last error: {last_error}")
    }
}

#[async_trait]
impl HqTool for ImageGenTool {
    fn name(&self) -> &str {
        "generate_image"
    }

    fn description(&self) -> &str {
        "Generate an image from a text prompt through OpenRouter (cheapest capable model \
         first). The image is saved to the vault and a markdown embed is returned."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": { "type": "string", "description": "Text description of the image to generate" },
                "width": { "type": "integer", "description": "Image width in pixels (optional hint)" },
                "height": { "type": "integer", "description": "Image height in pixels (optional hint)" },
                "model": { "type": "string", "description": "OpenRouter model override. Defaults to cheapest available." }
            },
            "required": ["prompt"]
        })
    }

    fn category(&self) -> &str {
        "creative"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if prompt.is_empty() {
            bail!("prompt is required");
        }
        let width = args.get("width").and_then(|v| v.as_u64());
        let height = args.get("height").and_then(|v| v.as_u64());
        let model_override = args.get("model").and_then(|v| v.as_str());
        let mut prompt_text = prompt.to_string();
        if let (Some(w), Some(h)) = (width, height) {
            prompt_text.push_str(&format!("\n\nDesired resolution: {w}x{h}"));
        }

        let Some(key) = self.resolve_openrouter_key() else {
            bail!(
                "OPENROUTER_API_KEY not configured (set openrouter_api_key in \
                 ~/.hq/config.yaml or the OPENROUTER_API_KEY env var)"
            );
        };
        self.generate_via_openrouter(&key, &prompt_text, model_override)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // `cargo test` runs `#[test]` functions in parallel threads by default,
    // not single-threaded — two tests each mutating the process-wide
    // OPENROUTER_API_KEY env var raced here (confirmed live: intermittent
    // failure on `falls_back_to_env_when_config_key_absent` reading a value
    // `config_key_takes_priority_over_env` had already removed). Held for
    // the whole set→call→restore span, and the prior value is restored
    // rather than unset, matching this repo's own established pattern for
    // this exact bug class (see agent_comm.rs's tests).
    static OPENROUTER_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[tokio::test]
    async fn empty_prompt_is_rejected() {
        let tool = ImageGenTool::new(PathBuf::from("/tmp/hq-test-vault"), None);
        let err = tool.execute(json!({"prompt": ""})).await.unwrap_err();
        assert!(err.to_string().contains("prompt is required"));
    }

    #[test]
    fn config_key_takes_priority_over_env() {
        let _guard = OPENROUTER_ENV_LOCK.lock().unwrap();
        let prior = std::env::var("OPENROUTER_API_KEY").ok();
        unsafe { std::env::set_var("OPENROUTER_API_KEY", "env-key") };
        let tool = ImageGenTool::new(
            PathBuf::from("/tmp/hq-test-vault"),
            Some("config-key".to_string()),
        );
        assert_eq!(tool.resolve_openrouter_key().as_deref(), Some("config-key"));
        match prior {
            Some(v) => unsafe { std::env::set_var("OPENROUTER_API_KEY", v) },
            None => unsafe { std::env::remove_var("OPENROUTER_API_KEY") },
        }
    }

    #[test]
    fn falls_back_to_env_when_config_key_absent() {
        let _guard = OPENROUTER_ENV_LOCK.lock().unwrap();
        let prior = std::env::var("OPENROUTER_API_KEY").ok();
        unsafe { std::env::set_var("OPENROUTER_API_KEY", "env-key") };
        let tool = ImageGenTool::new(PathBuf::from("/tmp/hq-test-vault"), None);
        assert_eq!(tool.resolve_openrouter_key().as_deref(), Some("env-key"));
        match prior {
            Some(v) => unsafe { std::env::set_var("OPENROUTER_API_KEY", v) },
            None => unsafe { std::env::remove_var("OPENROUTER_API_KEY") },
        }
    }
}

/// Extension for the saved file. Only raster types the web UI previews as images are
/// kept; svg, html, xml and anything unknown become `png`, so a saved output can never
/// be an active document.
fn safe_image_extension(mime: &str) -> &'static str {
    match mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/avif" => "avif",
        "image/bmp" => "bmp",
        _ => "png",
    }
}

#[cfg(test)]
mod extension_tests {
    use super::safe_image_extension;

    #[test]
    fn active_and_unknown_types_never_pick_the_extension() {
        for mime in [
            "image/svg+xml",
            "text/html",
            "application/xml",
            "image/svg+xml; charset=utf-8",
            "../../x",
            "",
            "image/x-icon",
        ] {
            assert_eq!(safe_image_extension(mime), "png", "{mime}");
        }
        assert_eq!(safe_image_extension("image/jpeg"), "jpg");
        assert_eq!(safe_image_extension("IMAGE/WEBP; q=1"), "webp");
    }
}
