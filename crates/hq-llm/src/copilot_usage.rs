//! Copilot credit balance for the account HQ actually runs on.
//!
//! `GET https://api.github.com/copilot_internal/user` reports the seat's plan and a
//! `premium_interactions` snapshot: with token-based billing its `entitlement`, `credits_used` and
//! `remaining` are AI credits. The request uses the same raw token as inference
//! (`CopilotProvider::resolve_raw_token`), so it describes the subscription paying for HQ's turns
//! and never some other account that happens to be logged in to `gh`.

use crate::copilot::{CopilotProvider, EDITOR_VERSION};
use crate::provider::LlmError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const USER_URL: &str = "https://api.github.com/copilot_internal/user";
const USER_AGENT: &str = "GitHubCopilotChat/0.26.7";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CopilotQuota {
    pub login: Option<String>,
    pub plan: Option<String>,
    pub sku: Option<String>,
    pub token_based_billing: bool,
    /// Total credits in the cycle.
    pub entitlement: f64,
    pub credits_used: f64,
    pub remaining: f64,
    pub percent_remaining: f64,
    pub overage_permitted: bool,
    /// The plan has no cap on this meter.
    pub unlimited: bool,
    /// False for plans with no premium allowance (a personal free account reports 0 of 0).
    pub has_quota: bool,
    /// When the cycle resets, if the API says.
    pub reset_at: Option<DateTime<Utc>>,
    pub fetched_at: DateTime<Utc>,
}

impl CopilotQuota {
    /// True when the numbers describe a real credit allowance worth tracking.
    pub fn is_metered(&self) -> bool {
        self.has_quota && !self.unlimited && self.entitlement > 0.0
    }
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Parse a `copilot_internal/user` body. `None` when it has no `premium_interactions` snapshot.
pub fn parse_quota(body: &Value, now: DateTime<Utc>) -> Option<CopilotQuota> {
    let snap = body.pointer("/quota_snapshots/premium_interactions")?;
    let entitlement = num(snap, "entitlement").unwrap_or(0.0);
    let credits_used = num(snap, "credits_used").unwrap_or(0.0);
    let remaining = num(snap, "quota_remaining")
        .or_else(|| num(snap, "remaining"))
        .unwrap_or(0.0);
    let reset_at = text(body, "quota_reset_date_utc")
        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
        .map(|d| d.with_timezone(&Utc));
    Some(CopilotQuota {
        login: text(body, "login"),
        plan: text(body, "copilot_plan"),
        sku: text(body, "access_type_sku"),
        token_based_billing: snap
            .get("token_based_billing")
            .or_else(|| body.get("token_based_billing"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        entitlement,
        credits_used,
        remaining,
        percent_remaining: num(snap, "percent_remaining").unwrap_or(0.0),
        overage_permitted: snap
            .get("overage_permitted")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        unlimited: snap
            .get("unlimited")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        has_quota: snap
            .get("has_quota")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        reset_at,
        fetched_at: now,
    })
}

/// Read the balance now, with the credential the Copilot backend uses.
pub async fn fetch_quota() -> Result<CopilotQuota, LlmError> {
    let raw = CopilotProvider::resolve_raw_token().await?;
    let resp = crate::http::SHARED_HTTP_CLIENT
        .get(USER_URL)
        .header("Authorization", format!("token {raw}"))
        .header("Editor-Version", EDITOR_VERSION)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| LlmError::from_request_error(&e))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(LlmError::Auth {
            status,
            message: "Copilot usage endpoint refused the token".into(),
        });
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| LlmError::Other(anyhow::anyhow!("Copilot usage body was not JSON: {e}")))?;
    parse_quota(&body, Utc::now()).ok_or_else(|| {
        LlmError::Other(anyhow::anyhow!(
            "Copilot usage response had no premium_interactions snapshot"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-30T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    /// Shape captured from a Business seat with token-based billing.
    fn business() -> Value {
        json!({
            "login": "seat-user", "access_type_sku": "copilot_for_business_seat_quota", "copilot_plan": "business",
            "quota_reset_date_utc": "2026-10-01T00:00:00.000Z",
            "quota_snapshots": {"premium_interactions": {
                "entitlement": 50000, "credits_used": 37009, "remaining": 12940, "quota_remaining": 12940.4,
                "percent_remaining": 25.8, "overage_permitted": true, "unlimited": false,
                "has_quota": true, "token_based_billing": true
            }}
        })
    }

    #[test]
    fn a_business_seat_reports_credits_left_of_the_total() {
        let q = parse_quota(&business(), now()).unwrap();
        assert_eq!((q.entitlement, q.credits_used), (50000.0, 37009.0));
        assert!((q.remaining - 12940.4).abs() < 1e-9);
        assert_eq!(q.plan.as_deref(), Some("business"));
        assert!(q.token_based_billing && q.overage_permitted && q.is_metered());
        assert_eq!(
            q.reset_at.unwrap().to_rfc3339(),
            "2026-10-01T00:00:00+00:00"
        );
    }

    #[test]
    fn a_free_personal_account_is_not_metered() {
        let body = json!({"login": "me", "copilot_plan": "individual", "quota_snapshots": {"premium_interactions": {
            "entitlement": 0, "credits_used": 0, "remaining": 0, "has_quota": false, "unlimited": false, "percent_remaining": 0.0
        }}});
        let q = parse_quota(&body, now()).unwrap();
        assert!(!q.is_metered(), "0 of 0 must not be shown as a balance");
    }

    #[test]
    fn a_body_without_the_premium_snapshot_is_not_a_quota() {
        assert!(parse_quota(&json!({"login": "x"}), now()).is_none());
    }
}
