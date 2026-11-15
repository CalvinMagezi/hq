//! Claude Code: the login Claude Code itself keeps, sent to Anthropic's usage endpoint.

use chrono::{DateTime, Utc};
use hq_core::quota::{Confidence, LaneQuota, QuotaSource, QuotaWindow, WindowKind};
use serde_json::Value;

use super::{HTTP_TIMEOUT, LOCAL_CMD_TIMEOUT, OUTPUT_CAP_BYTES, clamp_percent, run_capture};

const LANE: &str = "claude";
const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const BETA_HEADER: &str = "oauth-2025-04-20";
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

/// The login as the credentials file or keychain item stores it. `Debug` is deliberately absent.
struct Login {
    token: String,
    plan: Option<String>,
}

fn parse_login(raw: &str) -> Option<Login> {
    let v: Value = serde_json::from_str(raw.trim()).ok()?;
    let oauth = v.get("claudeAiOauth")?;
    let token = oauth.get("accessToken")?.as_str()?.to_string();
    let plan = oauth
        .get("subscriptionType")
        .and_then(Value::as_str)
        .map(str::to_string);
    (!token.is_empty()).then_some(Login { token, plan })
}

fn from_keychain() -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let argv = [
        "security",
        "find-generic-password",
        "-s",
        KEYCHAIN_SERVICE,
        "-w",
    ]
    .map(String::from)
    .to_vec();
    run_capture(&argv, None, LOCAL_CMD_TIMEOUT, OUTPUT_CAP_BYTES)
        .ok()
        .map(|c| c.stdout)
}

fn from_file() -> Option<String> {
    let home = std::env::var_os("HOME")?;
    let path = std::path::Path::new(&home).join(".claude/.credentials.json");
    std::fs::read_to_string(path).ok()
}

fn window(label: &str, kind: WindowKind, node: Option<&Value>) -> QuotaWindow {
    // A window that has not started comes back null, or with null fields: nothing used yet.
    let used = node
        .and_then(|n| n.get("utilization"))
        .and_then(Value::as_f64)
        .and_then(clamp_percent)
        .unwrap_or(0.0);
    let resets_at = node
        .and_then(|n| n.get("resets_at"))
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));
    QuotaWindow {
        label: label.to_string(),
        kind,
        used_percent: Some(used),
        remaining: None,
        resets_at,
    }
}

/// The usage body as windows. `None` when it has neither window key, so it is not a usage body.
pub(crate) fn parse_usage(body: &Value) -> Option<Vec<QuotaWindow>> {
    if body.get("five_hour").is_none() && body.get("seven_day").is_none() {
        return None;
    }
    Some(vec![
        window("5 hours", WindowKind::FiveHour, body.get("five_hour")),
        window("7 days", WindowKind::Weekly, body.get("seven_day")),
    ])
}

fn unknown(reason: &str) -> LaneQuota {
    LaneQuota::unknown(LANE, QuotaSource::Api, reason)
}

pub(crate) fn read() -> LaneQuota {
    let Some(login) = from_keychain()
        .and_then(|raw| parse_login(&raw))
        .or_else(|| from_file().and_then(|raw| parse_login(&raw)))
    else {
        return unknown("no Claude Code login found on this machine");
    };
    let headers = [
        ("Authorization", format!("Bearer {}", login.token)),
        ("anthropic-beta", BETA_HEADER.to_string()),
    ];
    let body = match super::http_get_json(USAGE_URL, &headers, HTTP_TIMEOUT) {
        Ok(b) => b,
        Err(e) => return unknown(&format!("usage endpoint: {e}")),
    };
    let Some(windows) = parse_usage(&body) else {
        return unknown("usage endpoint answered in an unknown shape");
    };
    LaneQuota {
        lane: LANE.into(),
        account: login.plan,
        windows,
        source: QuotaSource::Api,
        observed_at: Utc::now(),
        confidence: Confidence::Live,
        reason: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let path = format!("{}/tests/fixtures/quota/{name}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn both_windows_are_read() {
        let w = parse_usage(&fixture("claude_usage.json")).unwrap();
        assert_eq!(w[0].kind, WindowKind::FiveHour);
        assert_eq!(w[0].used_percent, Some(42.0));
        assert_eq!(w[1].kind, WindowKind::Weekly);
        assert_eq!(w[1].used_percent, Some(17.5));
        assert!(w[1].resets_at.is_some());
    }

    #[test]
    fn a_window_that_has_not_started_is_zero_not_unknown() {
        let w = parse_usage(&fixture("claude_usage_null.json")).unwrap();
        assert_eq!(w[0].used_percent, Some(0.0));
        assert!(w[0].resets_at.is_none());
        assert_eq!(w[1].used_percent, Some(0.0));
    }

    #[test]
    fn a_body_with_no_window_is_not_a_reading() {
        assert!(parse_usage(&serde_json::json!({"error": "x"})).is_none());
    }

    #[test]
    fn the_login_is_read_from_the_credentials_json() {
        let raw = r#"{"claudeAiOauth":{"accessToken":"t-fake","subscriptionType":"max"}}"#;
        let l = parse_login(raw).unwrap();
        assert_eq!(
            (l.token.as_str(), l.plan.as_deref()),
            ("t-fake", Some("max"))
        );
        assert!(parse_login("{}").is_none());
    }
}
