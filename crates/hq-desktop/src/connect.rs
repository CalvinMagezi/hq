//! The Connect Claude screen's backend: whether the claude.ai connector is on, the address to
//! paste into claude.ai, and the window that lets one new connection through.
//!
//! It talks to the local HQ web server's own `/api/oauth/connector/*` routes and does not
//! reimplement them. The address shown is whatever the connector reports; until `hq edge` and the
//! device-side TLS (workstreams W1 and W2) land there is no address of this machine's own, and the
//! screen says so instead of inventing one.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const CLIENT_NAME: &str = "hq-desktop";
const STATUS_PATH: &str = "/api/oauth/connector";
const ARM_PATH: &str = "/api/oauth/connector/arm";
const DISARM_PATH: &str = "/api/oauth/connector/disarm";
const DEVICE_CODE_PATH: &str = "/api/auth/device-code";
pub const DEFAULT_ARM_MINUTES: i64 = 10;

/// What the screen says when the connector has no address yet.
pub const NO_ADDRESS_YET: &str = "This machine has no address Claude can reach yet. A device address comes \
with hq edge and the device-side TLS (workstreams W1 and W2 of the Native Bridge plan), which this \
build does not include. To use an address you already run, set mcp_connector.public_url in the HQ \
config and restart the host.";

/// What the screen says about the consent page being served on this app's own address.
pub const CONSENT_NOT_LOCAL: &str = "The consent page is served here on the app's own address, but HQ \
tells Claude to send your browser to the authorization address in its config, which must be an \
https origin. Consent on this app's loopback address needs that to be allowed (workstream W6).";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reach {
    /// HQ's web server did not answer.
    Unreachable,
    /// It answered 401: there is no token, or HQ no longer accepts it.
    NeedsSignIn,
    Ok,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectView {
    pub reach: Reach,
    pub enabled: bool,
    /// The address to paste into claude.ai, when HQ has one.
    pub connector_url: Option<String>,
    /// Unix seconds until which one new connection is accepted.
    pub armed_until: Option<i64>,
    pub grants: usize,
    /// The authorization address HQ advertises to Claude.
    pub advertised_consent: Option<String>,
    /// This app's own consent address (the loopback proxy).
    pub local_consent: String,
    /// Whether Claude would be sent to this app's own address.
    pub consent_is_local: bool,
    /// Why there is no address, or why consent is not local, in plain words.
    pub notes: Vec<String>,
}

#[derive(Deserialize)]
struct Status {
    enabled: bool,
    url: Option<String>,
    consent_origin: Option<String>,
    armed_until: Option<i64>,
    #[serde(default)]
    grants: Vec<serde_json::Value>,
}

pub struct ConnectClient {
    base: String,
    token: Option<String>,
    http: reqwest::Client,
    local_consent: String,
}

impl ConnectClient {
    /// `base` is HQ's web server, for example `http://127.0.0.1:5678`; `local_consent` is the
    /// app's own proxy origin.
    pub fn new(base: String, token: Option<String>, local_consent: String) -> Result<Self> {
        Ok(Self {
            base,
            token,
            http: reqwest::Client::builder()
                .no_proxy()
                .timeout(REQUEST_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            local_consent,
        })
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut req = self
            .http
            .request(method, format!("{}{path}", self.base))
            .header("x-hq-client", CLIENT_NAME);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }
        req
    }

    pub async fn status(&self) -> ConnectView {
        let mut view = ConnectView {
            reach: Reach::Unreachable,
            enabled: false,
            connector_url: None,
            armed_until: None,
            grants: 0,
            advertised_consent: None,
            local_consent: self.local_consent.clone(),
            consent_is_local: false,
            notes: Vec::new(),
        };
        let res = match self.request(reqwest::Method::GET, STATUS_PATH).send().await {
            Ok(res) => res,
            Err(_) => return view,
        };
        if res.status() == reqwest::StatusCode::UNAUTHORIZED {
            view.reach = Reach::NeedsSignIn;
            return view;
        }
        let Ok(status) = res.json::<Status>().await else {
            return view;
        };
        view.reach = Reach::Ok;
        view.enabled = status.enabled;
        view.connector_url = status.url;
        view.armed_until = status.armed_until;
        view.grants = status.grants.len();
        view.consent_is_local = status
            .consent_origin
            .as_deref()
            .is_some_and(|o| o.trim_end_matches('/') == self.local_consent);
        view.advertised_consent = status.consent_origin;
        if view.connector_url.is_none() {
            view.notes.push(NO_ADDRESS_YET.into());
        }
        if view.enabled && !view.consent_is_local {
            view.notes.push(CONSENT_NOT_LOCAL.into());
        }
        view
    }

    /// Opens the window that admits one new connection, for `minutes`.
    pub async fn arm(&self, minutes: i64) -> Result<i64> {
        let res = self
            .request(reqwest::Method::POST, ARM_PATH)
            .json(&serde_json::json!({ "minutes": minutes }))
            .send()
            .await
            .context("HQ's web server did not answer")?;
        match res.status() {
            s if s.is_success() => {
                let body: serde_json::Value = res.json().await?;
                body["armed_until"]
                    .as_i64()
                    .context("HQ answered without a closing time")
            }
            reqwest::StatusCode::NOT_FOUND => {
                bail!("the connector is off: set mcp_connector.enabled and restart the host")
            }
            reqwest::StatusCode::UNAUTHORIZED => bail!("HQ did not accept this app's sign-in"),
            s => bail!("HQ refused to open the window ({s})"),
        }
    }

    pub async fn disarm(&self) -> Result<()> {
        let res = self
            .request(reqwest::Method::POST, DISARM_PATH)
            .send()
            .await
            .context("HQ's web server did not answer")?;
        if !res.status().is_success() {
            bail!("HQ refused to close the window ({})", res.status());
        }
        Ok(())
    }

    /// A one-time sign-in code, so the browser that opens the consent page can sign in to HQ.
    pub async fn sign_in_code(&self) -> Result<String> {
        let res = self
            .request(reqwest::Method::POST, DEVICE_CODE_PATH)
            .send()
            .await
            .context("HQ's web server did not answer")?;
        if !res.status().is_success() {
            bail!(
                "HQ would not issue a sign-in code ({}); it needs a web_auth_token",
                res.status()
            );
        }
        let body: serde_json::Value = res.json().await?;
        body["code"]
            .as_str()
            .map(str::to_owned)
            .context("HQ answered without a code")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, StatusCode},
        routing::{get, post},
    };
    use std::sync::{Arc, Mutex};

    type Log = Arc<Mutex<Vec<(String, Option<String>, Option<String>)>>>;

    async fn serve(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn note(log: &Log, what: &str, h: &HeaderMap) {
        let get = |n: &str| h.get(n).and_then(|v| v.to_str().ok()).map(str::to_owned);
        log.lock()
            .unwrap()
            .push((what.into(), get("authorization"), get("x-hq-client")));
    }

    fn daemon(status: serde_json::Value, log: Log) -> Router {
        Router::new()
            .route(
                STATUS_PATH,
                get({
                    let log = log.clone();
                    move |h: HeaderMap| async move {
                        note(&log, "status", &h);
                        Json(status)
                    }
                }),
            )
            .route(
                ARM_PATH,
                post({
                    let log = log.clone();
                    move |h: HeaderMap, Json(b): Json<serde_json::Value>| async move {
                        note(&log, &format!("arm {}", b["minutes"]), &h);
                        Json(serde_json::json!({ "armed_until": 1_700_000_600 }))
                    }
                }),
            )
            .route(
                DISARM_PATH,
                post({
                    let log = log.clone();
                    move |h: HeaderMap| async move {
                        note(&log, "disarm", &h);
                        Json(serde_json::json!({ "armed_until": null }))
                    }
                }),
            )
            .route(
                DEVICE_CODE_PATH,
                post(|| async { Json(serde_json::json!({ "code": "ABCD-EFGH" })) }),
            )
    }

    const LOCAL: &str = "http://127.0.0.1:47474";

    #[tokio::test]
    async fn an_off_connector_shows_the_no_address_note_and_nothing_else() {
        let log = Log::default();
        let base = serve(daemon(serde_json::json!({ "enabled": false }), log.clone())).await;
        let view = ConnectClient::new(base, Some("tok".into()), LOCAL.into())
            .unwrap()
            .status()
            .await;
        assert_eq!(view.reach, Reach::Ok);
        assert!(!view.enabled && view.connector_url.is_none());
        assert_eq!(view.notes, [NO_ADDRESS_YET]);
        let sent = log.lock().unwrap().clone();
        assert_eq!(
            sent[0],
            (
                "status".into(),
                Some("Bearer tok".into()),
                Some("hq-desktop".into())
            )
        );
    }

    #[tokio::test]
    async fn an_on_connector_shows_its_address_and_flags_a_consent_page_that_is_not_local() {
        let status = serde_json::json!({
            "enabled": true, "url": "https://hq.example.com/mcp",
            "consent_origin": "https://hq.example.ts.net", "armed_until": null, "grants": [{}, {}],
        });
        let base = serve(daemon(status, Log::default())).await;
        let view = ConnectClient::new(base, None, LOCAL.into())
            .unwrap()
            .status()
            .await;
        assert_eq!(
            view.connector_url.as_deref(),
            Some("https://hq.example.com/mcp")
        );
        assert_eq!(view.grants, 2);
        assert!(!view.consent_is_local);
        assert_eq!(view.notes, [CONSENT_NOT_LOCAL]);
    }

    #[tokio::test]
    async fn a_consent_origin_that_is_this_apps_own_adds_no_warning() {
        let status = serde_json::json!({
            "enabled": true, "url": "https://hq.example.com/mcp",
            "consent_origin": format!("{LOCAL}/"), "armed_until": 5, "grants": [],
        });
        let base = serve(daemon(status, Log::default())).await;
        let view = ConnectClient::new(base, None, LOCAL.into())
            .unwrap()
            .status()
            .await;
        assert!(view.consent_is_local && view.notes.is_empty());
        assert_eq!(view.armed_until, Some(5));
    }

    #[tokio::test]
    async fn arming_and_disarming_go_through_hqs_own_routes_with_the_apps_credentials() {
        let log = Log::default();
        let base = serve(daemon(serde_json::json!({ "enabled": true }), log.clone())).await;
        let client = ConnectClient::new(base, Some("tok".into()), LOCAL.into()).unwrap();
        assert_eq!(
            client.arm(DEFAULT_ARM_MINUTES).await.unwrap(),
            1_700_000_600
        );
        client.disarm().await.unwrap();
        assert_eq!(client.sign_in_code().await.unwrap(), "ABCD-EFGH");
        let sent = log.lock().unwrap().clone();
        assert_eq!(sent[0].0, "arm 10");
        assert_eq!(sent[1].0, "disarm");
        assert!(sent.iter().all(|(_, auth, who)| {
            auth.as_deref() == Some("Bearer tok") && who.as_deref() == Some("hq-desktop")
        }));
    }

    #[tokio::test]
    async fn a_refused_token_and_a_dead_server_are_told_apart() {
        let app = Router::new()
            .route(
                STATUS_PATH,
                get(|State(()): State<()>| async { StatusCode::UNAUTHORIZED }),
            )
            .with_state(());
        let base = serve(app).await;
        let view = ConnectClient::new(base, None, LOCAL.into())
            .unwrap()
            .status()
            .await;
        assert_eq!(view.reach, Reach::NeedsSignIn);

        let dead = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let view = ConnectClient::new(format!("http://{dead}"), None, LOCAL.into())
            .unwrap()
            .status()
            .await;
        assert_eq!(view.reach, Reach::Unreachable);
        let err = ConnectClient::new(format!("http://{dead}"), None, LOCAL.into())
            .unwrap()
            .arm(10)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("did not answer"));
    }

    #[tokio::test]
    async fn arming_an_off_connector_says_how_to_turn_it_on() {
        let app = Router::new().route(ARM_PATH, post(|| async { StatusCode::NOT_FOUND }));
        let base = serve(app).await;
        let err = ConnectClient::new(base, None, LOCAL.into())
            .unwrap()
            .arm(10)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("mcp_connector.enabled"));
    }
}
