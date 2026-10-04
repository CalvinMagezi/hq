//! Gmail ingest: fetch via gws and enqueue one deduped agent-worker triage event
//! per new message, for the draft -> PendingStore -> Telegram nudge pipeline.
//! The /hooks/gmail Pub/Sub push webhook is the primary trigger; the email-poll
//! daemon task is a time-gated fallback.

use anyhow::Result;
use axum::{
    Json,
    extract::{RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use hq_core::config::HqConfig;
use hq_core::mailbox;
use hq_core::types::MailboxMessageType;
use std::path::Path;
use std::sync::Arc;
use tokio::process::Command;
use tracing::{info, warn};

use crate::WsState;
use crate::error::ApiError;

const SEEN_FILE: &str = "_system/email-poll-seen.json";
const SEEN_CAP: usize = 1000;

fn load_seen(vault_path: &Path) -> Vec<String> {
    std::fs::read_to_string(vault_path.join(SEEN_FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_seen(vault_path: &Path, mut seen: Vec<String>) {
    if seen.len() > SEEN_CAP {
        let drop = seen.len() - SEEN_CAP;
        seen.drain(0..drop);
    }
    let path = vault_path.join(SEEN_FILE);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(s) = serde_json::to_string(&seen) {
        let _ = std::fs::write(path, s);
    }
}

/// Enqueue one triage event if the message is new for this company. Dedup key is
/// `company_id:message_id`, persisted immediately so a crash cannot re-enqueue.
/// Returns true when a new event was enqueued, false when deduped or invalid.
fn enqueue_triage(
    vault_path: &Path,
    company_id: &str,
    message_id: &str,
    from: &str,
    subject: &str,
) -> Result<bool> {
    if message_id.is_empty() {
        return Ok(false);
    }
    let key = format!("{}:{}", company_id, message_id);
    let mut seen = load_seen(vault_path);
    if seen.iter().any(|s| s == &key) {
        return Ok(false);
    }
    // `kind=email.totriage` is what is_email_triage matches on; `source` is only informational.
    let content = format!(
        "[mail-ingest flow=email-triage source=gmail kind=email.totriage]\n\nFrom: {from}\nSubject: {subject}"
    );
    let mut msg = mailbox::new_message(
        "mail-ingest",
        "agent-worker",
        MailboxMessageType::Direct,
        Some(subject),
        &content,
        None,
    );
    msg.meta.insert("messageId".into(), message_id.to_string());
    msg.meta.insert("from".into(), from.to_string());
    msg.meta.insert("subject".into(), subject.to_string());
    msg.meta.insert("company_id".into(), company_id.to_string());
    mailbox::send_message(vault_path, &msg)?;
    seen.push(key);
    save_seen(vault_path, seen);
    Ok(true)
}

#[derive(serde::Deserialize)]
struct TriageOut {
    #[serde(default)]
    messages: Vec<TriageMsg>,
}
#[derive(serde::Deserialize)]
struct TriageMsg {
    id: String,
    #[serde(default)]
    from: String,
    #[serde(default)]
    subject: String,
}

/// Outcome of a single `poll_and_enqueue` run. Distinguishes "checked every
/// configured mailbox, found nothing new" from "there was nothing configured
/// to check" — the two used to both come back as `Ok(0)`, which let the
/// daemon's periodic-task success count read as mailbox coverage that didn't
/// actually exist (see FEATURE-REQUESTS.md FR-009).
pub enum PollOutcome {
    /// Nothing in config declares a usable email backend for this run.
    Unconfigured(String),
    /// At least one company had a usable gws connector; `usize` enqueued.
    Polled(usize),
}

/// Fetch Gmail via gws for every company with an `email` listener backed by gws,
/// enqueue triage events.
pub async fn poll_and_enqueue(vault_path: &Path, config: &HqConfig) -> Result<PollOutcome> {
    let email_companies: Vec<&hq_core::config::CompanyConfig> = config
        .companies
        .iter()
        .filter(|c| c.listeners.iter().any(|l| l.kind == "email"))
        .collect();
    if email_companies.is_empty() {
        // This task did no work at all because nothing in `config.companies`
        // declares an `email` listener.
        warn!(
            "email-poll: no company declares an email listener — this task is a no-op, \
             not actively watching any mailbox"
        );
        return Ok(PollOutcome::Unconfigured(
            "no company declares an email listener".to_string(),
        ));
    }

    let mut total = 0usize;
    let mut usable = 0usize;
    let email_company_count = email_companies.len();
    for company in email_companies {
        if !company.connectors.iter().any(|b| b.kind == "gws") {
            // A company that DOES declare an email listener but has no
            // usable connector is not being watched either, and that's
            // worth surfacing, not just noting at info.
            warn!(company = %company.id, "gmail-ingest: declares an email listener but no gws connector, so it is not watched");
            continue;
        }
        usable += 1;
        let output = match Command::new(hq_core::paths::resolve_gws_binary())
            .args(["gmail", "+triage", "--format", "json"])
            .output()
            .await
        {
            Ok(o) if o.status.success() => o,
            Ok(o) => {
                warn!(company = %company.id, stderr = %String::from_utf8_lossy(&o.stderr), "gmail-ingest: gws failed");
                continue;
            }
            Err(e) => {
                warn!(company = %company.id, error = %e, "gmail-ingest: gws spawn failed");
                continue;
            }
        };
        let parsed: TriageOut = match serde_json::from_slice(&output.stdout) {
            Ok(p) => p,
            Err(e) => {
                warn!(company = %company.id, error = %e, "gmail-ingest: bad gws JSON");
                continue;
            }
        };
        for m in parsed.messages {
            match enqueue_triage(vault_path, &company.id, &m.id, &m.from, &m.subject) {
                Ok(true) => total += 1,
                Ok(false) => {}
                Err(e) => warn!(company = %company.id, error = %e, "gmail-ingest: enqueue failed"),
            }
        }
    }
    if total > 0 {
        info!(enqueued = total, "gmail-ingest: queued triage events");
    }
    if usable == 0 {
        return Ok(PollOutcome::Unconfigured(format!(
            "{email_company_count} company(ies) declare an email listener but none have a gws connector"
        )));
    }
    Ok(PollOutcome::Polled(total))
}

/// The hook secret, from a Bearer header or `?token=`. Unlike the web token,
/// a query secret is accepted here because Pub/Sub push subscriptions cannot set
/// headers; it is a hook-only secret that grants nothing but "go poll the inbox".
fn hook_request_authorized(headers: &HeaderMap, query: Option<&str>, expected: &str) -> bool {
    let from_query = query.and_then(|q| q.split('&').find_map(|pair| pair.strip_prefix("token=")));
    crate::auth::web_request_authorized(headers, expected)
        || from_query.is_some_and(|t| crate::auth::tokens_match(t, expected))
}

/// Gmail Pub/Sub push webhook. Any push means "inbox changed, go fetch"; the push
/// body is not parsed. Auth via `GMAIL_WEBHOOK_SECRET`: `Authorization: Bearer`
/// (preferred, headers stay out of URL logs) or `?token=` for Pub/Sub push
/// subscriptions, which cannot set headers. Without the secret the hook refuses every request,
/// since `/hooks` sits outside the web token guard; the 15m email-poll task still
/// fetches mail.
pub(crate) async fn gmail_hook_handler(
    State(state): State<Arc<WsState>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    _body: axum::body::Bytes,
) -> axum::response::Response {
    let expected = std::env::var("GMAIL_WEBHOOK_SECRET").unwrap_or_default();
    run_gmail_hook(&state, query.as_deref(), &headers, &expected).await
}

/// Never log `query` or the headers: the secret may be in either.
async fn run_gmail_hook(
    state: &WsState,
    query: Option<&str>,
    headers: &HeaderMap,
    expected: &str,
) -> axum::response::Response {
    if expected.is_empty() {
        tracing::warn!("/hooks/gmail refused: GMAIL_WEBHOOK_SECRET is not set");
        return ApiError::Unavailable("GMAIL_WEBHOOK_SECRET is not set".to_string()).into_response();
    }
    if !hook_request_authorized(headers, query, expected) {
        tracing::warn!("/hooks/gmail refused: missing or wrong secret");
        return ApiError::Forbidden("unauthorized".to_string()).into_response();
    }
    let Some(config) = state.hq_config.as_deref() else {
        return ApiError::Unavailable("config not loaded".to_string()).into_response();
    };
    match poll_and_enqueue(&state.vault_path, config).await {
        Ok(PollOutcome::Polled(n)) => {
            (StatusCode::OK, Json(serde_json::json!({"enqueued": n}))).into_response()
        }
        Ok(PollOutcome::Unconfigured(reason)) => (
            StatusCode::OK,
            Json(serde_json::json!({"enqueued": 0, "unconfigured": reason})),
        )
            .into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_gws_triage_shape() {
        let raw = r#"{"messages":[{"date":"x","from":"A <a@b.com>","id":"abc","subject":"Hi"}]}"#;
        let p: TriageOut = serde_json::from_str(raw).unwrap();
        assert_eq!(p.messages.len(), 1);
        assert_eq!(p.messages[0].id, "abc");
    }

    /// With no GMAIL_WEBHOOK_SECRET in the environment the hook refuses to poll.
    #[tokio::test]
    async fn hook_fails_closed_without_a_secret() {
        use tower::ServiceExt;
        if std::env::var("GMAIL_WEBHOOK_SECRET").is_ok_and(|v| !v.is_empty()) {
            return;
        }
        let vault = tempfile::TempDir::new().unwrap();
        let app = crate::create_router(Arc::new(WsState::new(vault.path().to_path_buf(), None)));
        let req = axum::http::Request::post("/hooks/gmail").body(axum::body::Body::empty()).unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    fn state() -> (WsState, tempfile::TempDir) {
        let vault = tempfile::TempDir::new().unwrap();
        let mut state = WsState::new(vault.path().to_path_buf(), None);
        // WsState::new loads the machine's real config; a poll would shell out to gws.
        state.hq_config = None;
        (state, vault)
    }

    /// The query form must keep working, the header form must too, and neither
    /// the right nor a wrong secret may reach the logs.
    #[tokio::test]
    async fn the_secret_is_accepted_both_ways_and_never_logged() {
        const SECRET: &str = "hook-secret-ZZZZ9999";
        const WRONG: &str = "wrong-secret-YYYY8888";
        let (logs, _guard) = hq_core::test_util::capture_logs();
        let (state, _vault) = state();
        let none = HeaderMap::new();
        let mut bearer = HeaderMap::new();
        bearer.insert("authorization", format!("Bearer {SECRET}").parse().unwrap());
        let mut wrong_bearer = HeaderMap::new();
        wrong_bearer.insert("authorization", format!("Bearer {WRONG}").parse().unwrap());

        let status = |r: axum::response::Response| r.status();
        let ok_query = run_gmail_hook(&state, Some(&format!("a=1&token={SECRET}")), &none, SECRET).await;
        let ok_header = run_gmail_hook(&state, None, &bearer, SECRET).await;
        let bad_query = run_gmail_hook(&state, Some(&format!("token={WRONG}")), &none, SECRET).await;
        let bad_header = run_gmail_hook(&state, None, &wrong_bearer, SECRET).await;
        let body = axum::body::to_bytes(bad_query.into_body(), usize::MAX).await.unwrap();

        // Past the auth gate the handler stops at "config not loaded", before any poll.
        assert_eq!(status(ok_query), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(status(ok_header), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(status(bad_header), StatusCode::FORBIDDEN);
        assert!(!String::from_utf8_lossy(&body).contains(WRONG));
        let out = logs.contents();
        assert!(out.contains("missing or wrong secret"), "{out}");
        assert!(!out.contains(SECRET) && !out.contains(WRONG), "secret in logs: {out}");
    }

    #[test]
    fn enqueue_dedups_per_company() {
        let tmp = tempfile::tempdir().unwrap();
        let v = tmp.path();
        assert!(enqueue_triage(v, "acme", "m1", "a@b.com", "Hi").unwrap());
        // same company + id -> deduped
        assert!(!enqueue_triage(v, "acme", "m1", "a@b.com", "Hi").unwrap());
        // different company, same id -> enqueued (namespaced key)
        assert!(enqueue_triage(v, "globex", "m1", "a@b.com", "Hi").unwrap());
    }

    #[test]
    fn empty_id_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!enqueue_triage(tmp.path(), "acme", "", "a@b.com", "Hi").unwrap());
    }

    #[test]
    fn seen_cap_trims_oldest() {
        let tmp = tempfile::tempdir().unwrap();
        let many: Vec<String> = (0..(SEEN_CAP + 50)).map(|i| i.to_string()).collect();
        save_seen(tmp.path(), many);
        let back = load_seen(tmp.path());
        assert_eq!(back.len(), SEEN_CAP);
        assert_eq!(back[0], "50");
    }
}
