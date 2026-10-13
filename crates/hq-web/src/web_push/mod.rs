//! Web Push: the server end of push notifications to the PWA (`docs/WEB_PUSH.md`).
//!
//! Browsers subscribe through `/api/notifications/push/*` (see [`api`]). New inbox items
//! (`value_items`) are taken by the dispatcher, encrypted per subscription ([`encrypt`]),
//! signed for with the server's VAPID key ([`vapid`]) and posted to each browser's push
//! service. Anything that writes the inbox, `notify_owner` included, gets pushed without
//! holding a key, since every process shares the vault database.

pub(crate) mod api;
pub(crate) mod encrypt;
pub(crate) mod vapid;

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use chrono::Utc;
use hq_core::config::WebPushConfig;
use hq_core::types::{ValueItem, ValueKind, ValueState};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::WsState;

/// How often the dispatcher looks for new inbox items.
const DISPATCH_INTERVAL: Duration = Duration::from_secs(3);
/// Items older than this are never pushed, so a server that was down does not replay them.
const PUSH_WINDOW_MINUTES: i64 = 60;
/// How long a push service keeps a message for a device that is offline.
const PUSH_TTL_SECS: u32 = 24 * 60 * 60;
/// Failed pushes in a row after which a subscription is dropped.
const MAX_FAILURES: i64 = 5;
const TITLE_CHARS: usize = 120;
const BODY_CHARS: usize = 300;
/// Where a tap goes when the item names no page.
const DEFAULT_LINK: &str = "/notifications";

/// Inbox sources that are always pushed: what agents send through `notify_owner`, and a new
/// device signing in, which the owner must hear about even with no chat app connected.
const ALWAYS_PUSHED_SOURCES: &[&str] = &[
    hq_tools::notify::NOTIFY_SOURCE,
    hq_tools::notify::NOTIFY_RELAY_SOURCE,
    crate::device_auth::DEVICE_SIGN_IN_SOURCE,
];

pub(crate) struct WebPush {
    key: vapid::VapidKey,
    subject: String,
    pub(crate) config: WebPushConfig,
    http: reqwest::Client,
}

/// What one push to one subscription came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PushOutcome {
    Sent,
    /// The push service says the subscription is gone; it was removed.
    Removed,
    Failed(String),
}

impl WebPush {
    pub(crate) fn new(
        vault_path: &Path,
        config: &WebPushConfig,
        web_allowed_origins: &[String],
    ) -> Result<Self> {
        let key = vapid::VapidKey::load_or_create(&vapid::key_path(vault_path))?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .context("building the push HTTP client")?;
        Ok(Self {
            key,
            subject: config.effective_subject(web_allowed_origins),
            config: config.clone(),
            http,
        })
    }

    pub(crate) fn public_key(&self) -> String {
        self.key.public_key_b64()
    }

    /// Whether the dispatcher pushes `item`.
    fn wants(&self, item: &ValueItem) -> bool {
        let open = matches!(
            item.state,
            ValueState::Pending | ValueState::Routed | ValueState::Delivered
        );
        open && (self.config.push_all
            || ALWAYS_PUSHED_SOURCES.contains(&item.source_task.as_str())
            || matches!(item.kind, ValueKind::ActionNeeded | ValueKind::Proposal))
    }

    /// Pushes `payload` to one subscription, recording the result on it.
    pub(crate) async fn send(
        &self,
        db: &hq_db::Database,
        sub: &hq_db::push_subscriptions::PushSubscription,
        payload: &[u8],
        urgent: bool,
    ) -> PushOutcome {
        let outcome = match self.post(sub, payload, urgent).await {
            Ok(status) if status.is_success() => PushOutcome::Sent,
            Ok(status) if status.as_u16() == 404 || status.as_u16() == 410 => PushOutcome::Removed,
            Ok(status) => PushOutcome::Failed(format!("push service answered {status}")),
            Err(e) => PushOutcome::Failed(format!("{e:#}")),
        };
        let recorded = match &outcome {
            PushOutcome::Sent => hq_db::push_subscriptions::record_success(db, &sub.endpoint),
            PushOutcome::Removed => {
                hq_db::push_subscriptions::delete(db, &sub.endpoint).map(|_| ())
            }
            PushOutcome::Failed(err) => {
                hq_db::push_subscriptions::record_failure(db, &sub.endpoint, err).and_then(|n| {
                    if n >= MAX_FAILURES {
                        tracing::warn!(endpoint = %host_of(&sub.endpoint), failures = n, "web push: dropping a subscription that keeps failing");
                        hq_db::push_subscriptions::delete(db, &sub.endpoint)?;
                    }
                    Ok(())
                })
            }
        };
        if let Err(e) = recorded {
            tracing::warn!("web push: could not record a push result: {e:#}");
        }
        outcome
    }

    async fn post(
        &self,
        sub: &hq_db::push_subscriptions::PushSubscription,
        payload: &[u8],
        urgent: bool,
    ) -> Result<reqwest::StatusCode> {
        let endpoint = reqwest::Url::parse(&sub.endpoint).context("bad endpoint")?;
        let keys = encrypt::ReceiverKeys::new(
            &B64.decode(sub.p256dh.trim_end_matches('='))?,
            &B64.decode(sub.auth.trim_end_matches('='))?,
        )?;
        let body = encrypt::encrypt(&keys, payload)?;
        let res = self
            .http
            .post(endpoint.clone())
            .header("TTL", PUSH_TTL_SECS.to_string())
            .header("Urgency", if urgent { "high" } else { "normal" })
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream")
            .header(
                "Authorization",
                self.key.authorization(&endpoint, &self.subject)?,
            )
            .body(body)
            .send()
            .await?;
        Ok(res.status())
    }

    /// Pushes `payload` to every subscription. Returns how many took it.
    pub(crate) async fn broadcast(
        &self,
        db: &hq_db::Database,
        payload: &[u8],
        urgent: bool,
    ) -> Result<usize> {
        let subs = hq_db::push_subscriptions::list(db)?;
        let outcomes =
            futures::future::join_all(subs.iter().map(|sub| self.send(db, sub, payload, urgent)))
                .await;
        Ok(outcomes.iter().filter(|o| **o == PushOutcome::Sent).count())
    }

    /// One pass of the dispatcher: claims each new item it wants and pushes it.
    pub(crate) async fn dispatch_once(&self, db: &hq_db::Database) -> Result<usize> {
        let since = Utc::now() - chrono::Duration::minutes(PUSH_WINDOW_MINUTES);
        let mut pushed = 0;
        for item in hq_db::value_items::list_unpushed(db, &since)? {
            // Claimed even when unwanted, so it is not looked at again.
            if !hq_db::value_items::claim_push(db, &item.id)? || !self.wants(&item) {
                continue;
            }
            let urgent = matches!(item.kind, ValueKind::ActionNeeded | ValueKind::Proposal);
            let delivered = self.broadcast(db, &payload(&item), urgent).await?;
            tracing::debug!(id = %item.id, delivered, "web push: pushed an inbox item");
            pushed += 1;
        }
        Ok(pushed)
    }
}

/// The JSON the service worker turns into a notification.
pub(crate) fn payload(item: &ValueItem) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "title": clip(&item.title, TITLE_CHARS),
        "body": clip(&item.body, BODY_CHARS),
        "url": item.link.as_deref().filter(|l| is_app_path(l)).unwrap_or(DEFAULT_LINK),
        "tag": format!("hq-inbox-{}", item.id),
    }))
    .unwrap_or_default()
}

/// A path on this web app, never another origin.
pub(crate) fn is_app_path(link: &str) -> bool {
    link.starts_with('/') && !link.starts_with("//") && !link.contains('\\')
}

fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// The endpoint's host, for logs: the path is a per-device secret.
fn host_of(endpoint: &str) -> String {
    reqwest::Url::parse(endpoint)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

/// Starts the dispatcher loop when push is on.
pub(crate) fn spawn_dispatcher(state: Arc<WsState>) {
    let Some(push) = state.web_push.clone() else {
        return;
    };
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(DISPATCH_INTERVAL);
        loop {
            tick.tick().await;
            if let Err(e) = push.dispatch_once(&state.db).await {
                tracing::warn!("web push: dispatch failed: {e:#}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Bytes, http::HeaderMap, routing::post};
    use ring::agreement;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Received {
        requests: Mutex<Vec<(String, HeaderMap, Bytes)>>,
    }

    /// A stand-in push service on loopback: `/ok/*` takes a push, `/gone/*` answers 410,
    /// `/down/*` answers 500.
    async fn mock_push_service() -> (String, Arc<Received>) {
        let received = Arc::new(Received::default());
        let r = received.clone();
        let app = Router::new().route(
            "/{kind}/{id}",
            post(
                move |axum::extract::Path((kind, id)): axum::extract::Path<(String, String)>,
                      headers: HeaderMap,
                      body: Bytes| {
                    let r = r.clone();
                    async move {
                        r.requests
                            .lock()
                            .unwrap()
                            .push((format!("{kind}/{id}"), headers, body));
                        match kind.as_str() {
                            "ok" => axum::http::StatusCode::CREATED,
                            "gone" => axum::http::StatusCode::GONE,
                            _ => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                        }
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), received)
    }

    fn browser_keys() -> (String, String) {
        let rng = ring::rand::SystemRandom::new();
        let key = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).unwrap();
        let public = key.compute_public_key().unwrap();
        (B64.encode(public.as_ref()), B64.encode([9u8; 16]))
    }

    fn push(dir: &Path, push_all: bool) -> WebPush {
        let cfg = WebPushConfig {
            enabled: true,
            push_all,
            ..Default::default()
        };
        WebPush::new(dir, &cfg, &[]).unwrap()
    }

    fn subscribe(db: &hq_db::Database, endpoint: &str) {
        let (p256dh, auth) = browser_keys();
        hq_db::push_subscriptions::upsert(db, endpoint, &p256dh, &auth, None).unwrap();
    }

    #[tokio::test]
    async fn pushes_agent_items_once_and_skips_plain_notices() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = hq_db::Database::open_memory().unwrap();
        let (base, received) = mock_push_service().await;
        subscribe(&db, &format!("{base}/ok/a"));
        let push = push(dir.path(), false);

        let agent = ValueItem::new(
            hq_tools::notify::NOTIFY_SOURCE,
            ValueKind::Fyi,
            "Deploy finished",
            "v1.2 is live",
        )
        .with_link("/tasks");
        let notice = ValueItem::new("task_stale_digest", ValueKind::Fyi, "Stale tasks", "3");
        let approval = ValueItem::new("self-update", ValueKind::ActionNeeded, "Approve", "b");
        for item in [&agent, &notice, &approval] {
            hq_db::value_items::emit(&db, item).unwrap();
        }

        assert_eq!(push.dispatch_once(&db).await.unwrap(), 2);
        assert_eq!(push.dispatch_once(&db).await.unwrap(), 0, "nothing twice");

        let reqs = received.requests.lock().unwrap();
        assert_eq!(reqs.len(), 2);
        let (_, headers, body) = &reqs[0];
        assert_eq!(headers["content-encoding"], "aes128gcm");
        assert_eq!(headers["ttl"], PUSH_TTL_SECS.to_string());
        assert_eq!(headers["urgency"], "normal");
        assert!(
            headers["authorization"]
                .to_str()
                .unwrap()
                .starts_with("vapid t=")
        );
        assert_eq!(&body[16..20], &4096u32.to_be_bytes(), "an aes128gcm body");
        assert_eq!(reqs[1].1["urgency"], "high", "an approval is urgent");
        drop(reqs);
        assert!(
            hq_db::push_subscriptions::list(&db).unwrap()[0]
                .last_ok_at
                .is_some()
        );
    }

    #[tokio::test]
    async fn push_all_pushes_every_new_item() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = hq_db::Database::open_memory().unwrap();
        let (base, received) = mock_push_service().await;
        subscribe(&db, &format!("{base}/ok/a"));
        let item = ValueItem::new("task_stale_digest", ValueKind::Fyi, "Stale tasks", "3");
        hq_db::value_items::emit(&db, &item).unwrap();
        assert_eq!(push(dir.path(), true).dispatch_once(&db).await.unwrap(), 1);
        assert_eq!(received.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_gone_subscription_is_removed_and_a_failing_one_dropped_after_five() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = hq_db::Database::open_memory().unwrap();
        let (base, _) = mock_push_service().await;
        let gone = format!("{base}/gone/a");
        let down = format!("{base}/down/b");
        subscribe(&db, &gone);
        subscribe(&db, &down);
        let push = push(dir.path(), false);

        assert_eq!(push.broadcast(&db, b"{}", false).await.unwrap(), 0);
        let left = hq_db::push_subscriptions::list(&db).unwrap();
        assert_eq!(left.len(), 1, "the 410 subscription is gone");
        assert_eq!(left[0].endpoint, down);
        assert_eq!(left[0].failures, 1);

        for _ in 1..MAX_FAILURES {
            push.broadcast(&db, b"{}", false).await.unwrap();
        }
        assert!(hq_db::push_subscriptions::list(&db).unwrap().is_empty());
    }

    #[test]
    fn payload_clips_text_and_keeps_links_on_the_app() {
        let mut item = ValueItem::new(
            "agent_notify",
            ValueKind::Fyi,
            "x".repeat(200),
            "y".repeat(900),
        );
        item.link = Some("https://example.com".into());
        let v: serde_json::Value = serde_json::from_slice(&payload(&item)).unwrap();
        assert_eq!(v["title"].as_str().unwrap().chars().count(), TITLE_CHARS);
        assert_eq!(v["body"].as_str().unwrap().chars().count(), BODY_CHARS);
        assert_eq!(v["url"], DEFAULT_LINK);
        assert!(payload(&item).len() < encrypt::MAX_PLAINTEXT);
        item.link = Some("/chat?thread=t1".into());
        let v: serde_json::Value = serde_json::from_slice(&payload(&item)).unwrap();
        assert_eq!(v["url"], "/chat?thread=t1");
    }
}
