//! `/api/notifications/push/*`: a browser turns push on or off, and can ask for a test push.
//! Behind the web token like every `/api` route.

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, header},
};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use super::encrypt::ReceiverKeys;
use crate::WsState;
use crate::error::ApiError;

const MAX_ENDPOINT_CHARS: usize = 2048;

/// `PushSubscription.toJSON()`, as the browser hands it over.
#[derive(Debug, Deserialize)]
pub(crate) struct SubscribeRequest {
    endpoint: String,
    keys: SubscriptionKeys,
}

#[derive(Debug, Deserialize)]
struct SubscriptionKeys {
    p256dh: String,
    auth: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EndpointRequest {
    #[serde(default)]
    endpoint: Option<String>,
}

fn push_on(state: &WsState) -> Result<&super::WebPush, ApiError> {
    state.web_push.as_deref().ok_or_else(|| {
        ApiError::Conflict("push notifications are off on this server (web_push.enabled)".into())
    })
}

/// `GET /api/notifications/push`: whether push is on, and the key a browser subscribes with.
pub(crate) async fn status_handler(
    State(state): State<Arc<WsState>>,
) -> Result<Json<Value>, ApiError> {
    let Some(push) = state.web_push.as_deref() else {
        return Ok(Json(json!({ "enabled": false })));
    };
    let subscriptions = hq_db::push_subscriptions::list(&state.db)
        .map_err(ApiError::internal)?
        .len();
    Ok(Json(json!({
        "enabled": true,
        "public_key": push.public_key(),
        "subscriptions": subscriptions,
    })))
}

/// `POST /api/notifications/push/subscribe`: saves this browser's subscription.
pub(crate) async fn subscribe_handler(
    State(state): State<Arc<WsState>>,
    headers: HeaderMap,
    Json(req): Json<SubscribeRequest>,
) -> Result<Json<Value>, ApiError> {
    let push = push_on(&state)?;
    let endpoint = check_endpoint(&req.endpoint, push)?;
    let p256dh = decode_key(&req.keys.p256dh, "p256dh")?;
    let auth = decode_key(&req.keys.auth, "auth")?;
    ReceiverKeys::new(&p256dh, &auth).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|ua| ua.chars().take(200).collect::<String>());
    hq_db::push_subscriptions::upsert(
        &state.db,
        &endpoint,
        &B64.encode(&p256dh),
        &B64.encode(&auth),
        user_agent.as_deref(),
    )
    .map_err(ApiError::internal)?;
    Ok(Json(json!({ "subscribed": true })))
}

/// `POST /api/notifications/push/unsubscribe`: forgets one browser's subscription.
pub(crate) async fn unsubscribe_handler(
    State(state): State<Arc<WsState>>,
    Json(req): Json<EndpointRequest>,
) -> Result<Json<Value>, ApiError> {
    let Some(endpoint) = req.endpoint else {
        return Err(ApiError::bad_request("endpoint is required"));
    };
    let removed =
        hq_db::push_subscriptions::delete(&state.db, &endpoint).map_err(ApiError::internal)?;
    Ok(Json(json!({ "removed": removed })))
}

/// `POST /api/notifications/push/test`: a test push to one subscription, or to all of them.
pub(crate) async fn test_handler(
    State(state): State<Arc<WsState>>,
    Json(req): Json<EndpointRequest>,
) -> Result<Json<Value>, ApiError> {
    let push = push_on(&state)?;
    let payload = serde_json::to_vec(&json!({
        "title": "HQ push works",
        "body": "Notifications from HQ and its agents will show up here.",
        "url": "/notifications",
        "tag": "hq-push-test",
    }))
    .map_err(ApiError::internal)?;
    let subs = hq_db::push_subscriptions::list(&state.db).map_err(ApiError::internal)?;
    let targets: Vec<_> = match &req.endpoint {
        Some(ep) => subs.into_iter().filter(|s| &s.endpoint == ep).collect(),
        None => subs,
    };
    if targets.is_empty() {
        return Err(ApiError::NotFound("no matching push subscription".into()));
    }
    let mut results = Vec::new();
    for sub in &targets {
        let outcome = push.send(&state.db, sub, &payload, false).await;
        results.push(match outcome {
            super::PushOutcome::Sent => json!({ "sent": true }),
            super::PushOutcome::Removed => {
                json!({ "sent": false, "error": "the push service no longer knows this subscription; it was removed" })
            }
            super::PushOutcome::Failed(e) => json!({ "sent": false, "error": e }),
        });
    }
    let sent = results.iter().filter(|r| r["sent"] == true).count();
    Ok(Json(json!({ "sent": sent, "results": results })))
}

/// The endpoint must be an https URL on a known push service: the server posts to it, so a
/// made-up address would turn this route into a way to reach other machines.
fn check_endpoint(raw: &str, push: &super::WebPush) -> Result<String, ApiError> {
    let raw = raw.trim();
    if raw.chars().count() > MAX_ENDPOINT_CHARS {
        return Err(ApiError::bad_request("endpoint is too long"));
    }
    let url =
        reqwest::Url::parse(raw).map_err(|_| ApiError::bad_request("endpoint is not a URL"))?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err(ApiError::bad_request("endpoint must be a plain https URL"));
    }
    let host = url.host_str().unwrap_or_default();
    if !push.config.push_host_allowed(host) {
        return Err(ApiError::bad_request(format!(
            "{host} is not a known push service; add it to web_push.extra_push_hosts if your browser uses it"
        )));
    }
    Ok(url.to_string())
}

fn decode_key(value: &str, name: &str) -> Result<Vec<u8>, ApiError> {
    // Browsers send base64url without padding; accept padded and standard base64 too.
    let cleaned: String = value
        .trim()
        .trim_end_matches('=')
        .chars()
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            c => c,
        })
        .collect();
    B64.decode(cleaned)
        .map_err(|_| ApiError::bad_request(format!("keys.{name} is not base64url")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn app(enabled: bool) -> (axum::Router, Arc<WsState>, tempfile::TempDir) {
        let vault = tempfile::TempDir::new().unwrap();
        let mut state = WsState::new(vault.path().to_path_buf(), None);
        let cfg = hq_core::config::WebPushConfig {
            enabled,
            ..Default::default()
        };
        state.configure_web_push(&cfg).unwrap();
        let state = Arc::new(state);
        (crate::create_router(state.clone()), state, vault)
    }

    fn browser_keys() -> Value {
        let rng = ring::rand::SystemRandom::new();
        let key = ring::agreement::EphemeralPrivateKey::generate(&ring::agreement::ECDH_P256, &rng)
            .unwrap();
        json!({
            "p256dh": B64.encode(key.compute_public_key().unwrap().as_ref()),
            "auth": B64.encode([1u8; 16]),
        })
    }

    async fn call(
        app: &axum::Router,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn status_reports_off_and_subscribing_is_refused() {
        let (app, _, _v) = app(false);
        let (status, body) = call(&app, "GET", "/api/notifications/push", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["enabled"], false);
        let sub =
            json!({"endpoint": "https://fcm.googleapis.com/fcm/send/x", "keys": browser_keys()});
        let (status, _) = call(&app, "POST", "/api/notifications/push/subscribe", Some(sub)).await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn a_browser_subscribes_and_unsubscribes() {
        let (app, state, _v) = app(true);
        let (_, body) = call(&app, "GET", "/api/notifications/push", None).await;
        assert_eq!(body["enabled"], true);
        assert_eq!(
            B64.decode(body["public_key"].as_str().unwrap())
                .unwrap()
                .len(),
            65
        );

        let endpoint = "https://fcm.googleapis.com/fcm/send/device-1";
        let sub = json!({"endpoint": endpoint, "keys": browser_keys()});
        let (status, _) = call(&app, "POST", "/api/notifications/push/subscribe", Some(sub)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(hq_db::push_subscriptions::list(&state.db).unwrap().len(), 1);
        let (_, body) = call(&app, "GET", "/api/notifications/push", None).await;
        assert_eq!(body["subscriptions"], 1);

        let (status, body) = call(
            &app,
            "POST",
            "/api/notifications/push/unsubscribe",
            Some(json!({"endpoint": endpoint})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["removed"], true);
        assert!(
            hq_db::push_subscriptions::list(&state.db)
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn refuses_endpoints_off_known_push_services_and_bad_keys() {
        let (app, state, _v) = app(true);
        for endpoint in [
            "http://fcm.googleapis.com/fcm/send/x",
            "https://127.0.0.1/x",
            "https://metadata.google.internal/computeMetadata",
            "https://user@fcm.googleapis.com/x",
            "not a url",
        ] {
            let sub = json!({"endpoint": endpoint, "keys": browser_keys()});
            let (status, _) =
                call(&app, "POST", "/api/notifications/push/subscribe", Some(sub)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{endpoint}");
        }
        let sub = json!({
            "endpoint": "https://fcm.googleapis.com/fcm/send/x",
            "keys": {"p256dh": B64.encode([4u8; 10]), "auth": B64.encode([1u8; 16])},
        });
        let (status, _) = call(&app, "POST", "/api/notifications/push/subscribe", Some(sub)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            hq_db::push_subscriptions::list(&state.db)
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_test_push_needs_a_subscription() {
        let (app, _, _v) = app(true);
        let (status, _) = call(
            &app,
            "POST",
            "/api/notifications/push/test",
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
