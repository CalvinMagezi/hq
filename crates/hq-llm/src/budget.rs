//! The point where a spending limit can stop a call. A [`BudgetGate`] is asked before every attempt
//! against a provider, with the model and the provider known, so a limit on one backend lets the
//! backend chain move on to the next one instead of failing the whole turn.

use std::future::Future;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::cost::ProviderClass;
use crate::models::get_model_info;
use crate::provider::ChatRequest;

/// Completion length assumed when the request sets no `max_tokens`.
const DEFAULT_OUTPUT_ALLOWANCE_TOKENS: u32 = 4096;
const TOKENS_PER_MILLION: f64 = 1_000_000.0;
/// Tokens a tool definition adds to the prompt beyond its JSON length is small next to the
/// messages, so they are counted from the serialized schema.
const TOOLS_JSON_FALLBACK: &str = "";

/// What the gate is asked about: one attempt against one provider.
pub struct GateRequest<'a> {
    pub provider: &'a str,
    pub class: ProviderClass,
    pub model: &'a str,
    pub origin: &'static str,
    /// Worst-case dollars for the attempt: the prompt plus the output allowance. `None` when HQ has
    /// no price for the model, which a blocking budget treats as unknown rather than free.
    pub estimate_usd: Option<f64>,
}

pub enum Admission {
    Allow,
    /// Run the attempt on this model instead.
    Downgrade { model: String },
    Deny(BudgetBlocked),
}

/// A call refused because a budget would be exceeded.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct BudgetBlocked {
    pub budget: String,
    pub spent_usd: f64,
    pub limit_usd: f64,
    pub message: String,
}

#[async_trait]
pub trait BudgetGate: Send + Sync {
    async fn admit(&self, request: &GateRequest<'_>) -> Admission;
}

pub type SharedGate = Arc<dyn BudgetGate>;

type BlockedSlot = Arc<Mutex<Option<BudgetBlocked>>>;

tokio::task_local! {
    static BLOCKED: BlockedSlot;
}

/// Run `fut` remembering the last budget refusal inside it, so the caller can report that instead
/// of a generic "all providers failed".
pub(crate) async fn with_blocked_slot<F: Future>(fut: F) -> F::Output {
    BLOCKED.scope(BlockedSlot::default(), fut).await
}

pub(crate) fn note_blocked(blocked: &BudgetBlocked) {
    let _ = BLOCKED.try_with(|slot| *slot.lock().unwrap() = Some(blocked.clone()));
}

/// The most recent refusal in the current call, if any attempt was refused.
pub(crate) fn last_blocked() -> Option<BudgetBlocked> {
    BLOCKED
        .try_with(|slot| slot.lock().unwrap().clone())
        .ok()
        .flatten()
}

/// Whether an attempt error is a budget refusal, which is not the provider's fault.
pub(crate) fn is_budget_block(error: &anyhow::Error) -> bool {
    error.downcast_ref::<BudgetBlocked>().is_some()
}

/// Worst-case cost of an attempt, or `None` when the model has no known price.
pub fn estimate_cost(class: ProviderClass, request: &ChatRequest) -> Option<f64> {
    if class != ProviderClass::Metered {
        return Some(0.0);
    }
    let info = get_model_info(&request.model)?;
    let input_tokens: usize = request
        .messages
        .iter()
        .map(|m| hq_core::tokens::count_tokens_fast(&m.content))
        .sum::<usize>()
        + hq_core::tokens::count_tokens_fast(
            &serde_json::to_string(&request.tools).unwrap_or_else(|_| TOOLS_JSON_FALLBACK.into()),
        );
    let output_tokens = request.max_tokens.unwrap_or(DEFAULT_OUTPUT_ALLOWANCE_TOKENS);
    Some(
        (input_tokens as f64 * info.input_cost_per_million
            + f64::from(output_tokens) * info.output_cost_per_million)
            / TOKENS_PER_MILLION,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::types::{ChatMessage, MessageRole};

    const PRICED: &str = "anthropic/claude-haiku-5.5";

    fn request(model: &str, text: &str, max_tokens: Option<u32>) -> ChatRequest {
        ChatRequest {
            model: model.into(),
            messages: vec![ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::User,
                content: text.into(),
                tool_calls: vec![],
                tool_call_id: None,
                reasoning_content: None,
            }],
            max_tokens,
            ..Default::default()
        }
    }

    #[test]
    fn the_estimate_grows_with_the_prompt_and_the_output_allowance() {
        let small = estimate_cost(ProviderClass::Metered, &request(PRICED, "hi", Some(10)));
        let big_prompt = estimate_cost(
            ProviderClass::Metered,
            &request(PRICED, &"word ".repeat(50_000), Some(10)),
        );
        let big_output = estimate_cost(ProviderClass::Metered, &request(PRICED, "hi", Some(50_000)));
        assert!(small.unwrap() < big_prompt.unwrap());
        assert!(small.unwrap() < big_output.unwrap());
    }

    #[test]
    fn an_unknown_model_has_no_estimate_but_local_and_flat_cost_nothing() {
        let r = request("nobody/unknown", "hi", None);
        assert_eq!(estimate_cost(ProviderClass::Metered, &r), None);
        assert_eq!(estimate_cost(ProviderClass::Local, &r), Some(0.0));
        assert_eq!(estimate_cost(ProviderClass::Flat, &r), Some(0.0));
    }
}
