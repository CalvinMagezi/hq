//! OpenRouter embeddings client — used in place of a local Ollama embedding
//! model on hosts (like the VPS) where running Ollama isn't an option.
//!
//! OpenRouter's embeddings endpoint is OpenAI-compatible:
//! `POST https://openrouter.ai/api/v1/embeddings`.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

fn openrouter_base() -> String {
    std::env::var("OPENROUTER_BASE_URL").unwrap_or_else(|_| "https://openrouter.ai/api/v1".into())
}

/// Model used for OpenRouter embeddings. `text-embedding-3-small` and
/// `qwen/qwen3-embedding-0.6b` are both called out by OpenRouter as cheap
/// options; default to the former, override via `OPENROUTER_EMBEDDING_MODEL`.
pub fn openrouter_embedding_model() -> String {
    std::env::var("OPENROUTER_EMBEDDING_MODEL")
        .unwrap_or_else(|_| "openai/text-embedding-3-small".into())
}

#[derive(Serialize)]
struct OpenRouterEmbedRequest {
    model: String,
    input: String,
    encoding_format: &'static str,
}

#[derive(Deserialize)]
struct OpenRouterEmbedResponse {
    data: Vec<OpenRouterEmbedData>,
    #[serde(default)]
    usage: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct OpenRouterEmbedData {
    embedding: Vec<f32>,
}

/// Generate a vector embedding for `text` via OpenRouter, using `api_key`
/// (e.g. `HqConfig::openrouter_api_key`, the same field
/// `run_inbox_triage`/`model_intelligence` already read).
pub async fn generate_embedding(text: &str, api_key: &str) -> Result<Vec<f32>> {
    let model = openrouter_embedding_model();
    let instruments = hq_llm::Instruments::global();
    let call = hq_llm::ExternalCall {
        provider: "openrouter",
        class: hq_llm::cost::ProviderClass::Metered,
        model: &model,
        origin: hq_llm::origin::EMBEDDINGS,
        task_hint: "embed",
    };
    instruments.admit_external(&call).await?;
    let started = std::time::Instant::now();
    let result = embed_once(text, api_key, &model).await;
    match &result {
        Ok((_, usage)) => instruments.record_external(&call, Some(*usage), started.elapsed(), None),
        Err(_) => instruments.record_external(&call, None, started.elapsed(), Some("error")),
    }
    result.map(|(embedding, _)| embedding)
}

async fn embed_once(text: &str, api_key: &str, model: &str) -> Result<(Vec<f32>, hq_llm::cost::Usage)> {
    let model = model.to_string();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("failed to build reqwest client")?;

    let request = OpenRouterEmbedRequest {
        model: model.clone(),
        input: text.to_string(),
        encoding_format: "float",
    };

    let res = client
        .post(format!("{}/embeddings", openrouter_base()))
        .bearer_auth(api_key)
        .json(&request)
        .send()
        .await
        .context("OpenRouter embedding request failed")?;

    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        bail!("OpenRouter embedding error {status}: {body}");
    }

    let mut data: OpenRouterEmbedResponse = res
        .json()
        .await
        .context("Failed to parse OpenRouter embedding response")?;

    if data.data.is_empty() {
        bail!("OpenRouter embedding response had no data entries");
    }

    let usage = data
        .usage
        .as_ref()
        .map(hq_llm::usage_from_openrouter)
        .unwrap_or_default();
    Ok((data.data.remove(0).embedding, usage))
}
