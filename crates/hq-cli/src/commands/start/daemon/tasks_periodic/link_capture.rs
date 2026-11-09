use anyhow::{Context, Result, anyhow};
use hq_core::config::HqConfig;
use hq_core::types::{ChatMessage, MessageRole};
use hq_db::Database;
use hq_llm::{LlmProvider, provider::ChatRequest, router::LlmRouter};
use hq_tools::link_capture::{MAX_PAGE_CHARS, SYSTEM_PROMPT, has_due, run_due};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const MODEL: &str = "fast";
const MAX_REPLY_TOKENS: u32 = 400;
const MODEL_TIMEOUT: Duration = Duration::from_secs(60);

fn message(role: MessageRole, content: String) -> ChatMessage {
    ChatMessage {
        image_parts: Vec::new(),
        role,
        content,
        tool_calls: vec![],
        tool_call_id: None,
        reasoning_content: None,
    }
}

/// One call with no tools: whatever the page says, the worst it can do is a bad summary.
async fn ask(router: Arc<LlmRouter>, user: String) -> Result<String> {
    let request = ChatRequest {
        model: MODEL.to_string(),
        messages: vec![
            message(MessageRole::System, SYSTEM_PROMPT.to_string()),
            message(MessageRole::User, user),
        ],
        tools: Vec::new(),
        max_tokens: Some(MAX_REPLY_TOKENS),
        ..Default::default()
    };
    let call = hq_llm::with_origin(hq_llm::origin::BACKGROUND, router.chat(&request));
    let reply = tokio::time::timeout(MODEL_TIMEOUT, call)
        .await
        .map_err(|_| anyhow!("the model took too long"))?
        .context("the model call failed")?;
    Ok(reply.message.content)
}

pub async fn run_link_capture(vault_path: &Path, db: &Database, config: &HqConfig) -> Result<()> {
    if !config.daemon.link_capture || !has_due(db)? {
        return Ok(());
    }
    let vault = Arc::new(hq_vault::VaultClient::new(vault_path.to_path_buf())?);
    let router = Arc::new(LlmRouter::from_env());
    run_due(
        Arc::new(db.clone()),
        vault,
        |url| async move {
            let page = hq_tools::web::web_fetch_direct(&url, MAX_PAGE_CHARS).await?;
            Ok(page.content)
        },
        move |user| ask(router.clone(), user),
    )
    .await?;
    Ok(())
}
