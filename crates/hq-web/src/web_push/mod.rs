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
/// Waiting items a returning subscription gets again at most; the worker folds the rest.
const REPLAY_MAX: usize = 50;
/// Where a tap goes when the item names no page.
const DEFAULT_LINK: &str = "/inbox";

/// Inbox sources that are always pushed: what agents send through `notify_owner`, and a new
/// device signing in, which the owner must hear about even with no chat app connected.
const ALWAYS_PUSHED_SOURCES: &[&str] = &[
    hq_tools::notify::NOTIFY_SOURCE,
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

    /// Whether the dispatcher pushes `item`. Never one that is device-only (how the person uses
    /// HQ): a push crosses a third party's push service, even if encrypted.
    fn wants(&self, item: &ValueItem) -> bool {
        if item.is_device_only() {
            return false;
        }
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

    /// One pass of the dispatcher: claims each new item it wants and pushes it.
    pub(crate) async fn dispatch_once(&self, db: &hq_db::Database) -> Result<usize> {
        let since = Utc::now() - chrono::Duration::minutes(PUSH_WINDOW_MINUTES);
        let mut pushed = 0;
        for item in hq_db::value_items::list_unpushed(db, &since)? {
            // Claimed even when unwanted, so it is not looked at again.
            if !hq_db::value_items::claim_push(db, &item.id)? || !self.wants(&item) {
                continue;
            }
            let subs = hq_db::push_subscriptions::list(db)?;
            let delivered = self.push_item(db, &item, &subs).await;
            tracing::debug!(id = %item.id, delivered, "web push: pushed an inbox item");
            pushed += 1;
        }
        Ok(pushed)
    }

    /// Pushes one item to each subscription that receives its kind, with or without its first
    /// line as each chose. Returns how many took it.
    async fn push_item(
        &self,
        db: &hq_db::Database,
        item: &ValueItem,
        subs: &[hq_db::push_subscriptions::PushSubscription],
    ) -> usize {
        let kind = PushKind::of(item);
        let urgent = kind == PushKind::Needs;
        let who = helper_name(db, item);
        let plain = payload(item, who.as_deref(), false);
        let detailed = payload(item, who.as_deref(), true);
        let sends = subs.iter().filter(|s| s.receives(kind.name())).map(|sub| {
            let body = if sub.details { &detailed } else { &plain };
            self.send(db, sub, body, urgent)
        });
        let outcomes = futures::future::join_all(sends).await;
        outcomes.iter().filter(|o| **o == PushOutcome::Sent).count()
    }

    /// Posts each item still waiting, of the kinds a subscription just gained, to that
    /// subscription once. The service worker tags them by item id and folds past the newest few.
    pub(crate) async fn replay(
        &self,
        db: &hq_db::Database,
        endpoint: &str,
        gained: &[PushKind],
    ) -> Result<usize> {
        let Some(sub) = hq_db::push_subscriptions::get(db, endpoint)? else {
            return Ok(0);
        };
        let mut waiting = Vec::new();
        for state in [
            ValueState::Pending,
            ValueState::Routed,
            ValueState::Delivered,
        ] {
            waiting.extend(hq_db::value_items::list_by_state(db, state)?);
        }
        let now = Utc::now();
        waiting.retain(|i| {
            let live = i.expires_at.is_none_or(|at| at > now);
            live && self.wants(i) && gained.contains(&PushKind::of(i))
        });
        waiting.sort_by_key(|i| i.created_at);
        let skip = waiting.len().saturating_sub(REPLAY_MAX);
        let mut sent = 0;
        for item in waiting.iter().skip(skip) {
            // A push service that refuses one stops the replay: five failures in a row would
            // drop the subscription, and a gone one has nothing left to post to.
            if self.push_item(db, item, std::slice::from_ref(&sub)).await == 0 {
                break;
            }
            sent += 1;
        }
        Ok(sent)
    }
}

/// Which notification switch an item belongs to (`push_subscriptions.kinds`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PushKind {
    Needs,
    Updates,
}

impl PushKind {
    pub(crate) fn of(item: &ValueItem) -> Self {
        let needs = matches!(item.kind, ValueKind::ActionNeeded | ValueKind::Proposal)
            || item.source_task == crate::device_auth::DEVICE_SIGN_IN_SOURCE;
        if needs { Self::Needs } else { Self::Updates }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Needs => hq_db::push_subscriptions::KIND_NEEDS,
            Self::Updates => hq_db::push_subscriptions::KIND_UPDATES,
        }
    }
}

/// Every task source shares this prefix (`hq_tools::tasks::json`).
const TASK_SOURCE_PREFIX: &str = "task_";
/// The supervisor's dedup keys name the session: a blocked one, and one stuck at a prompt.
const BLOCKED_KEY_PREFIX: &str = "session-blocked-";
const STUCK_KEY_PREFIX: &str = "session-dismiss-cap-";
const NEEDS_BODY: &str = "Open HQ to see what it wants to do.";
/// A push with details on carries at most this much of the item's first line.
const DETAIL_CHARS: usize = 120;

/// The helper an item is about, by its display name, when it names a harness session.
fn helper_name(db: &hq_db::Database, item: &ValueItem) -> Option<String> {
    let from_alert = hq_db::dialog_alerts::get(db, &item.id)
        .ok()
        .flatten()
        .map(|a| a.session_id);
    let from_key = || {
        let key = item.dedup_key.as_deref()?;
        let rest = key
            .strip_prefix(BLOCKED_KEY_PREFIX)
            .and_then(|r| r.rsplit_once('-').map(|(id, _seq)| id))
            .or_else(|| key.strip_prefix(STUCK_KEY_PREFIX))?;
        Some(rest.to_string())
    };
    let session = from_alert.or_else(from_key)?;
    let row = db
        .with_conn(|c| hq_db::harness_sessions_registry::get(c, &session))
        .ok()
        .flatten()?;
    Some(harness_display_name(&row.harness))
}

/// The name people know a harness by, matching the web app's `AGENT_NAMES`.
fn harness_display_name(harness: &str) -> String {
    let known = match harness {
        "claude-code" => "Claude Code",
        "codex" => "Codex",
        "cursor" => "Cursor",
        "pi" => "Pi",
        "opencode" => "OpenCode",
        "kimi" => "Kimi",
        "qwen" => "Qwen",
        "antigravity" => "Antigravity",
        "github-copilot" | "copilot" | "copilot-lean" => "GitHub Copilot",
        "hermes" => "Hermes",
        "openclaw" => "OpenClaw",
        _ => return "A helper".into(),
    };
    known.into()
}

/// The words a push shows, built here so no client ever receives an item's content
/// (`docs/plans/ux-run/web/wave-0/01-live-bugs.md`). `who` is the helper's display name, if any.
fn words(item: &ValueItem, who: Option<&str>, details: bool) -> (String, String) {
    let stuck = item
        .dedup_key
        .as_deref()
        .is_some_and(|k| k.starts_with(STUCK_KEY_PREFIX));
    let (title, body) = if item.source_task == crate::device_auth::DEVICE_SIGN_IN_SOURCE {
        ("A new sign-in to your HQ".to_string(), "Open HQ to see it.")
    } else if stuck {
        ("A helper is stuck".to_string(), "Open HQ to see which one.")
    } else if PushKind::of(item) == PushKind::Updates {
        // An update's title is the sender's own headline; its body stays in HQ.
        (clip(&item.title, TITLE_CHARS), "")
    } else if item.source_task.starts_with(TASK_SOURCE_PREFIX) {
        ("A task needs your OK".to_string(), NEEDS_BODY)
    } else {
        (format!("{} needs your OK", who.unwrap_or("HQ")), NEEDS_BODY)
    };
    let first_line = item.body.lines().map(str::trim).find(|l| !l.is_empty());
    let body = match first_line {
        Some(line) if details => clip(line, DETAIL_CHARS),
        _ => body.to_string(),
    };
    (title, body)
}

/// The JSON the service worker turns into a notification: words, a page to open and a tag.
/// Never actions, tokens or the item's id, so a push cannot answer anything.
pub(crate) fn payload(item: &ValueItem, who: Option<&str>, details: bool) -> Vec<u8> {
    let (title, body) = words(item, who, details);
    let body = serde_json::json!({
        "title": title,
        "body": body,
        "kind": PushKind::of(item).name(),
        "url": item.link.as_deref().filter(|l| is_app_path(l)).unwrap_or(DEFAULT_LINK),
        "tag": format!("hq-inbox-{}", item.id),
    });
    serde_json::to_vec(&body).unwrap_or_default()
}

/// A path on this web app, never another origin.
pub(crate) fn is_app_path(link: &str) -> bool {
    link.starts_with('/') && !link.starts_with("//") && !link.contains('\\')
}

fn clip(s: &str, max: usize) -> String {
    hq_core::text::truncate_chars_ellipsis(s.trim(), max)
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

    /// An empty push to every subscription; how many took it.
    async fn broadcast(push: &WebPush, db: &hq_db::Database) -> usize {
        let subs = hq_db::push_subscriptions::list(db).unwrap();
        let sends = subs.iter().map(|sub| push.send(db, sub, b"{}", false));
        let outcomes = futures::future::join_all(sends).await;
        outcomes.iter().filter(|o| **o == PushOutcome::Sent).count()
    }

    fn subscribe(db: &hq_db::Database, endpoint: &str) {
        let (p256dh, auth) = browser_keys();
        hq_db::push_subscriptions::upsert(db, endpoint, &p256dh, &auth, None, None).unwrap();
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
    async fn adoption_data_is_never_pushed_even_with_push_all() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = hq_db::Database::open_memory().unwrap();
        let (base, received) = mock_push_service().await;
        subscribe(&db, &format!("{base}/ok/a"));
        let item = ValueItem::new(
            hq_core::types::ADOPTION_DIGEST_SOURCE,
            ValueKind::ActionNeeded,
            "How HQ was used this week",
            "private",
        );
        hq_db::value_items::emit(&db, &item).unwrap();
        assert_eq!(push(dir.path(), true).dispatch_once(&db).await.unwrap(), 0);
        assert!(received.requests.lock().unwrap().is_empty());
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

        assert_eq!(broadcast(&push, &db).await, 0);
        let left = hq_db::push_subscriptions::list(&db).unwrap();
        assert_eq!(left.len(), 1, "the 410 subscription is gone");
        assert_eq!(left[0].endpoint, down);
        assert_eq!(left[0].failures, 1);

        for _ in 1..MAX_FAILURES {
            broadcast(&push, &db).await;
        }
        assert!(hq_db::push_subscriptions::list(&db).unwrap().is_empty());
    }

    fn parsed(item: &ValueItem, who: Option<&str>, details: bool) -> serde_json::Value {
        let raw = payload(item, who, details);
        assert!(raw.len() < encrypt::MAX_PLAINTEXT);
        serde_json::from_slice(&raw).unwrap()
    }

    #[test]
    fn payload_keeps_links_on_the_app() {
        let mut item = ValueItem::new("agent_notify", ValueKind::Fyi, "x".repeat(200), "y");
        item.link = Some("https://example.com".into());
        let v = parsed(&item, None, false);
        assert_eq!(v["title"].as_str().unwrap().chars().count(), TITLE_CHARS);
        assert_eq!(v["url"], DEFAULT_LINK);
        assert_eq!(
            DEFAULT_LINK, "/inbox",
            "HQ writes the new link forms (web wave 2, rule 5)"
        );
        item.link = Some("/chat?thread=t1".into());
        assert_eq!(parsed(&item, None, false)["url"], "/chat?thread=t1");
    }

    fn blocked_item() -> ValueItem {
        ValueItem::new(
            "session-supervisor",
            ValueKind::ActionNeeded,
            "Harness session 'copilot' is waiting for you",
            "Session hs-1 on laptop is blocked on a prompt or approval:\nrm -rf /tmp/x",
        )
        .with_dedup_key("session-blocked-hs-1-7")
    }

    #[test]
    fn a_blocked_sessions_push_names_the_helper_and_carries_no_command_or_answer() {
        let item = blocked_item();
        let raw = payload(&item, Some("Claude Code"), false);
        assert!(!String::from_utf8_lossy(&raw).contains("rm -rf"));
        let v: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v["title"], "Claude Code needs your OK");
        assert_eq!(v["body"], "Open HQ to see what it wants to do.");
        for retired in ["actions", "tokens", "item"] {
            assert!(v.get(retired).is_none(), "no {retired} in a push");
        }
        assert_eq!(parsed(&item, None, false)["title"], "HQ needs your OK");
    }

    #[test]
    fn details_show_the_first_line_never_the_command() {
        let v = parsed(&blocked_item(), Some("Codex"), true);
        assert_eq!(v["title"], "Codex needs your OK");
        assert_eq!(
            v["body"],
            "Session hs-1 on laptop is blocked on a prompt or approval:"
        );
    }

    #[test]
    fn a_notify_owner_push_is_its_title_only() {
        let item = ValueItem::new(
            hq_tools::notify::NOTIFY_SOURCE,
            ValueKind::Fyi,
            "Payment",
            "Your card ending 4242 was charged",
        );
        let raw = payload(&item, None, false);
        assert!(!String::from_utf8_lossy(&raw).contains("4242"));
        let v: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v["title"], "Payment");
        assert_eq!(v["body"], "");
        assert_eq!(v["kind"], "updates");
    }

    #[test]
    fn task_stuck_and_sign_in_pushes_use_fixed_words() {
        let task = ValueItem::new(
            "task_ready_for_review",
            ValueKind::ActionNeeded,
            "T-1: Pay rent",
            "Pay the rent by Friday",
        );
        let raw = payload(&task, None, false);
        assert!(!String::from_utf8_lossy(&raw).contains("Pay rent"));
        assert_eq!(parsed(&task, None, false)["title"], "A task needs your OK");

        let stuck = ValueItem::new(
            "session-supervisor",
            ValueKind::ActionNeeded,
            "Harness session 'x' keeps showing a prompt",
            "Session hs-2 on laptop",
        )
        .with_dedup_key("session-dismiss-cap-hs-2");
        let v = parsed(&stuck, Some("Codex"), false);
        assert_eq!(v["title"], "A helper is stuck");
        assert_eq!(v["body"], "Open HQ to see which one.");

        let sign_in = ValueItem::new(
            crate::device_auth::DEVICE_SIGN_IN_SOURCE,
            ValueKind::Fyi,
            "A new device signed in to HQ",
            "at 10:00 UTC",
        );
        let v = parsed(&sign_in, None, false);
        assert_eq!(v["title"], "A new sign-in to your HQ");
        assert_eq!(v["body"], "Open HQ to see it.");
        assert_eq!(v["kind"], "needs");
    }

    #[test]
    fn a_helpers_name_comes_from_its_session() {
        let db = hq_db::Database::open_memory().unwrap();
        db.with_conn(|c| {
            hq_db::harness_sessions_registry::insert(
                c,
                &hq_db::harness_sessions_registry::NewSession {
                    id: "hs-1",
                    harness: "codex",
                    label: "x",
                    cwd: "/tmp",
                    mission_id: None,
                    placement: hq_db::harness_sessions_registry::Placement {
                        host: "laptop",
                        agent_name: "a",
                        workspace_id: "w",
                        pane_id: "p",
                    },
                    purpose: None,
                },
            )
        })
        .unwrap();
        assert_eq!(helper_name(&db, &blocked_item()).as_deref(), Some("Codex"));
    }

    fn choose(db: &hq_db::Database, endpoint: &str, kinds: &str, details: bool) {
        let (p256dh, auth) = browser_keys();
        let choice = hq_db::push_subscriptions::Choice {
            kinds: Some(kinds),
            details: Some(details),
        };
        hq_db::push_subscriptions::upsert_with(db, endpoint, &p256dh, &auth, None, None, &choice)
            .unwrap();
    }

    #[tokio::test]
    async fn each_subscription_gets_only_its_kinds() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = hq_db::Database::open_memory().unwrap();
        let (base, received) = mock_push_service().await;
        choose(&db, &format!("{base}/ok/updates"), "updates", false);
        choose(&db, &format!("{base}/ok/both"), "needs,updates", false);
        hq_db::value_items::emit(&db, &blocked_item()).unwrap();
        push(dir.path(), false).dispatch_once(&db).await.unwrap();
        let reqs = received.requests.lock().unwrap();
        let to: Vec<&str> = reqs.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(
            to,
            ["ok/both"],
            "a Needs you item skips an updates-only browser"
        );
    }

    #[tokio::test]
    async fn a_kind_turned_back_on_replays_what_still_waits_once() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = hq_db::Database::open_memory().unwrap();
        let (base, received) = mock_push_service().await;
        let endpoint = format!("{base}/ok/a");
        choose(&db, &endpoint, "updates", false);
        hq_db::value_items::emit(&db, &blocked_item()).unwrap();
        let fyi = ValueItem::new(hq_tools::notify::NOTIFY_SOURCE, ValueKind::Fyi, "n", "b");
        hq_db::value_items::emit(&db, &fyi).unwrap();
        let push = push(dir.path(), false);
        push.dispatch_once(&db).await.unwrap();
        assert_eq!(
            received.requests.lock().unwrap().len(),
            1,
            "the update only"
        );

        choose(&db, &endpoint, "needs,updates", false);
        let sent = push
            .replay(&db, &endpoint, &[PushKind::Needs])
            .await
            .unwrap();
        assert_eq!(sent, 1, "the waiting Needs you item, once");
        let mut lapsed = blocked_item().with_dedup_key("session-blocked-hs-9-1");
        lapsed.expires_at = Some(Utc::now() - chrono::Duration::minutes(1));
        hq_db::value_items::emit(&db, &lapsed).unwrap();
        let sent = push
            .replay(&db, &endpoint, &[PushKind::Needs])
            .await
            .unwrap();
        assert_eq!(
            sent, 1,
            "the same waiting item again, not the one that ran out"
        );
        assert_eq!(received.requests.lock().unwrap().len(), 3);
    }
}
