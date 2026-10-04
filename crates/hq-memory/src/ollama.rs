//! Local embeddings through Ollama's `/api/embeddings`, used when no
//! OpenRouter key is configured. Memory LLM calls go through `backends:`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

fn ollama_base() -> String {
    std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".into())
}

/// Default model for embedding operations.
pub fn embedding_model() -> String {
    std::env::var("EMBEDDING_MODEL").unwrap_or_else(|_| "nomic-embed-text".into())
}

#[derive(Serialize)]
struct OllamaEmbedRequest {
    model: String,
    prompt: String,
}

#[derive(Deserialize)]
struct OllamaEmbedResponse {
    embedding: Vec<f32>,
}

/// Generate a vector embedding for `text` using Ollama's local embedding API.
///
/// Uses `nomic-embed-text` by default (overridable via `EMBEDDING_MODEL` env).
/// Returns `None` if Ollama is unavailable or the model is missing.
pub async fn generate_embedding(text: &str) -> Result<Vec<f32>> {
    let model = embedding_model();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("failed to build reqwest client")?;

    let request = OllamaEmbedRequest {
        model,
        prompt: text.to_string(),
    };

    let res = client
        .post(format!("{}/api/embeddings", ollama_base()))
        .json(&request)
        .send()
        .await
        .context("Ollama embedding request failed")?;

    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        anyhow::bail!("Ollama embedding error {status}: {body}");
    }

    let data: OllamaEmbedResponse = res
        .json()
        .await
        .context("Failed to parse Ollama embedding response")?;
    Ok(data.embedding)
}
