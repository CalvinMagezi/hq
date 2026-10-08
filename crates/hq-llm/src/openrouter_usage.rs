//! OpenRouter spend and balance for the key HQ runs on.
//!
//! `GET {base}/key` describes the key: lifetime, daily, weekly and monthly spend in USD, an
//! optional spend limit and what is left of it. `GET {base}/credits` reports the account's
//! purchased credits and lifetime usage, but OpenRouter limits it to some keys, so it is optional
//! here. OpenRouter publishes no time series, so nothing in this module estimates a burn rate:
//! the daily, weekly and monthly figures are the only rates it genuinely exposes.

use crate::provider::LlmError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenRouterUsage {
    pub label: Option<String>,
    /// Lifetime spend of this key, USD.
    pub usage: f64,
    pub usage_daily: Option<f64>,
    pub usage_weekly: Option<f64>,
    pub usage_monthly: Option<f64>,
    /// Spend cap on this key, USD. `None` means the key has no cap.
    pub limit: Option<f64>,
    pub limit_remaining: Option<f64>,
    pub limit_reset: Option<String>,
    pub is_free_tier: bool,
    /// Purchased credits on the account, USD. `None` when OpenRouter would not say for this key.
    pub total_credits: Option<f64>,
    /// Lifetime account spend, USD, from the credits endpoint.
    pub total_usage: Option<f64>,
}

impl OpenRouterUsage {
    /// Purchased credits minus lifetime spend, when both are known.
    pub fn credits_left(&self) -> Option<f64> {
        Some(self.total_credits? - self.total_usage?)
    }
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

/// Parse the `/key` body, then fold in the `/credits` body when there is one.
pub fn parse_usage(key_body: &Value, credits_body: Option<&Value>) -> Option<OpenRouterUsage> {
    let k = key_body.get("data")?;
    let credits = credits_body.and_then(|b| b.get("data"));
    Some(OpenRouterUsage {
        label: k.get("label").and_then(Value::as_str).map(str::to_string),
        usage: num(k, "usage")?,
        usage_daily: num(k, "usage_daily"),
        usage_weekly: num(k, "usage_weekly"),
        usage_monthly: num(k, "usage_monthly"),
        limit: num(k, "limit"),
        limit_remaining: num(k, "limit_remaining"),
        limit_reset: k
            .get("limit_reset")
            .and_then(Value::as_str)
            .map(str::to_string),
        is_free_tier: k
            .get("is_free_tier")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        total_credits: credits.and_then(|c| num(c, "total_credits")),
        total_usage: credits.and_then(|c| num(c, "total_usage")),
    })
}

async fn get_json(base: &str, path: &str, key: &str) -> Result<Value, LlmError> {
    let resp = crate::http::SHARED_HTTP_CLIENT
        .get(format!("{}/{path}", base.trim_end_matches('/')))
        .bearer_auth(key)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| LlmError::from_request_error(&e))?;
    let status = resp.status().as_u16();
    if matches!(status, 401 | 403) {
        return Err(LlmError::Auth {
            status,
            message: format!("OpenRouter refused the key on /{path}"),
        });
    }
    if !(200..300).contains(&status) {
        return Err(LlmError::ServerError {
            status,
            message: format!("OpenRouter /{path} failed"),
        });
    }
    resp.json()
        .await
        .map_err(|e| LlmError::Other(anyhow::anyhow!("OpenRouter /{path} body was not JSON: {e}")))
}

/// Read the key's spend now. `base` is the backend endpoint, normally `https://openrouter.ai/api/v1`.
pub async fn fetch_usage(base: &str, key: &str) -> Result<OpenRouterUsage, LlmError> {
    let key_body = get_json(base, "key", key).await?;
    // The credits endpoint is optional: a refusal there must not hide the key's own numbers.
    let credits_body = get_json(base, "credits", key).await.ok();
    parse_usage(&key_body, credits_body.as_ref()).ok_or_else(|| {
        LlmError::Other(anyhow::anyhow!(
            "OpenRouter /key response had no usage figure"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key_body() -> Value {
        json!({"data": {"label": "sk-or-v1-abc...xyz", "limit": 50.0, "usage": 12.5, "usage_daily": 0.75,
            "usage_weekly": 4.0, "usage_monthly": 12.5, "is_free_tier": false, "limit_remaining": 37.5,
            "limit_reset": null}})
    }

    #[test]
    fn a_key_with_a_limit_reports_spend_and_what_is_left() {
        let credits = json!({"data": {"total_credits": 100.0, "total_usage": 30.0}});
        let u = parse_usage(&key_body(), Some(&credits)).unwrap();
        assert_eq!(
            (u.usage, u.usage_daily, u.limit_remaining),
            (12.5, Some(0.75), Some(37.5))
        );
        assert_eq!(u.credits_left(), Some(70.0));
    }

    #[test]
    fn missing_credits_and_no_limit_stay_unknown_not_zero() {
        let body = json!({"data": {"usage": 3.0, "limit": null, "limit_remaining": null}});
        let u = parse_usage(&body, None).unwrap();
        assert!(u.limit.is_none() && u.credits_left().is_none() && u.usage_daily.is_none());
    }

    #[test]
    fn a_body_without_usage_is_not_a_reading() {
        assert!(parse_usage(&json!({"data": {"label": "x"}}), None).is_none());
        assert!(parse_usage(&json!({}), None).is_none());
    }
}
