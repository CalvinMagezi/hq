//! "Share this computer's model": what the Devices screen shows about the model server (Ollama)
//! before and while the bridge lends it to this computer's hubs. Read only; the switch itself is
//! a [`hq_bridge::ServiceSwitch`] on the bridge's `llm` service.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// How long one question to the model server may take: it is on loopback.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// HQ's chat prompt runs to about 26,000 tokens; a shorter context cuts it without an error.
pub const MIN_CONTEXT: u64 = 32_768;
/// Models asked about their capabilities, at most.
const MAX_MODELS: usize = 24;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModelView {
    pub name: String,
    /// The server says the model takes native tool calls. Without them HQ's tools do not work.
    pub tools: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ShareView {
    pub on: bool,
    /// The model hubs are told to use, as chosen.
    pub model: Option<String>,
    pub server: String,
    pub reachable: bool,
    pub models: Vec<ModelView>,
    /// The context the server gives the chosen model, when the model is loaded.
    pub context_length: Option<u64>,
    /// Something the person should fix before sharing helps.
    pub warning: Option<String>,
}

#[derive(Deserialize)]
struct Tags {
    #[serde(default)]
    models: Vec<Named>,
}

#[derive(Deserialize)]
struct Named {
    name: String,
    #[serde(default)]
    context_length: Option<u64>,
}

#[derive(Deserialize)]
struct Show {
    #[serde(default)]
    capabilities: Vec<String>,
}

/// Asks the model server at `server` (a loopback `ip:port`) what it has.
pub async fn probe(server: &str, on: bool, model: Option<String>) -> ShareView {
    let client = reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .no_proxy()
        .build()
        .unwrap_or_default();
    let base = format!("http://{server}");
    let tags: Option<Tags> = get_json(&client, &format!("{base}/api/tags")).await;
    let Some(tags) = tags else {
        return ShareView {
            on,
            model,
            server: server.to_string(),
            reachable: false,
            models: Vec::new(),
            context_length: None,
            warning: Some(format!(
                "No model server answers at {server}. Start Ollama first."
            )),
        };
    };
    let mut models = Vec::new();
    for named in tags.models.into_iter().take(MAX_MODELS) {
        let show: Option<Show> = post_json(
            &client,
            &format!("{base}/api/show"),
            serde_json::json!({ "model": named.name }),
        )
        .await;
        let tools = show.is_some_and(|s| s.capabilities.iter().any(|c| c == "tools"));
        models.push(ModelView {
            name: named.name,
            tools,
        });
    }
    let chosen = model
        .clone()
        .or_else(|| models.first().map(|m| m.name.clone()));
    let loaded: Option<Tags> = get_json(&client, &format!("{base}/api/ps")).await;
    let context_length = loaded.and_then(|ps| {
        ps.models
            .into_iter()
            .find(|m| Some(&m.name) == chosen.as_ref())
            .and_then(|m| m.context_length)
    });
    let warning = advice(&models, chosen.as_deref(), context_length);
    ShareView {
        on,
        model,
        server: server.to_string(),
        reachable: true,
        models,
        context_length,
        warning,
    }
}

/// What is wrong with sharing `chosen`, if anything.
pub fn advice(models: &[ModelView], chosen: Option<&str>, context: Option<u64>) -> Option<String> {
    let Some(chosen) = chosen else {
        return Some(
            "Ollama has no models yet. Pull one with tool calling, such as qwen3:8b.".into(),
        );
    };
    let Some(model) = models.iter().find(|m| m.name == chosen) else {
        return Some(format!("Ollama does not have {chosen}."));
    };
    if !model.tools {
        return Some(format!(
            "{chosen} does not take tool calls, so HQ's tools will not work through it. Pick another, or turn on prompted_tools for it on the hub."
        ));
    }
    match context {
        Some(n) if n < MIN_CONTEXT => Some(format!(
            "Ollama gives {chosen} {n} tokens of context; HQ needs {MIN_CONTEXT}. Raise Ollama's context length (OLLAMA_CONTEXT_LENGTH, or the Ollama app's setting) and restart Ollama."
        )),
        Some(_) => None,
        None => Some(format!(
            "Make sure Ollama's context length is at least {MIN_CONTEXT} (OLLAMA_CONTEXT_LENGTH, or the Ollama app's setting): a shorter one cuts HQ's prompt without an error."
        )),
    }
}

async fn get_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
) -> Option<T> {
    client
        .get(url)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()
}

async fn post_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    body: serde_json::Value,
) -> Option<T> {
    client
        .post(url)
        .json(&body)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(name: &str, tools: bool) -> ModelView {
        ModelView {
            name: name.into(),
            tools,
        }
    }

    #[test]
    fn advice_names_what_would_break_hq() {
        let models = [model("qwen3:8b", true), model("tiny", false)];
        assert!(advice(&[], None, None).unwrap().contains("no models"));
        assert!(
            advice(&models, Some("gone"), None)
                .unwrap()
                .contains("does not have")
        );
        assert!(
            advice(&models, Some("tiny"), Some(MIN_CONTEXT))
                .unwrap()
                .contains("tool calls")
        );
        assert!(
            advice(&models, Some("qwen3:8b"), Some(4_096))
                .unwrap()
                .contains("4096 tokens")
        );
        assert!(
            advice(&models, Some("qwen3:8b"), None)
                .unwrap()
                .contains("at least")
        );
        assert_eq!(advice(&models, Some("qwen3:8b"), Some(MIN_CONTEXT)), None);
    }

    #[tokio::test]
    async fn a_server_that_does_not_answer_is_reported_not_an_error() {
        let view = probe("127.0.0.1:1", true, None).await;
        assert!(!view.reachable && view.on);
        assert!(view.warning.unwrap().contains("Start Ollama"));
    }
}
