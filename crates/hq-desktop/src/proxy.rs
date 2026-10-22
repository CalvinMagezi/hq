//! The app's loopback proxy: the one origin the app's web window talks to, and the address the
//! consent page is served on, so no tailnet name is needed.
//!
//! It is a reverse proxy to one upstream, the local HQ web server, and nothing else. It binds
//! loopback only, answers only to its own `Host`, refuses cross-origin browser requests, never
//! acts as a forward proxy, keeps the machine endpoints (`/mcp`, the OAuth token routes) off this
//! door, and never adds a credential: whatever authenticates a request is what the client sent.

use anyhow::{Context, Result, bail};
use axum::{
    Router,
    body::Body,
    extract::{FromRequestParts, Request, State, WebSocketUpgrade, ws},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Request headers that describe one hop, not the message.
const HOP_BY_HOP: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
];
/// Paths that belong to the public machine door, never to this one.
const MACHINE_PATHS: [&str; 4] = ["/mcp", "/oauth/token", "/oauth/revoke", "/.well-known"];
const WS_PATH: &str = "/ws";

#[derive(Debug, Clone)]
pub struct ProxyConfig {
    pub listen: SocketAddr,
    pub upstream: SocketAddr,
}

impl ProxyConfig {
    /// Both ends must be loopback: a proxy that listens elsewhere is a door onto the machine, and
    /// one that forwards elsewhere is an open proxy.
    pub fn validate(&self) -> Result<()> {
        if !self.listen.ip().is_loopback() {
            bail!(
                "the proxy listens on loopback only, not {}",
                self.listen.ip()
            );
        }
        if !self.upstream.ip().is_loopback() {
            bail!(
                "the proxy forwards to loopback only, not {}",
                self.upstream.ip()
            );
        }
        Ok(())
    }
}

/// Why a request was turned away before it reached HQ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// An absolute-form target, `CONNECT`, or no `Host` at all.
    BadTarget,
    /// `Host` is not this proxy's own address: a rebound DNS name or a misdirected request.
    WrongHost,
    /// A browser request that carries another site's `Origin`.
    CrossOrigin,
    /// A browser fetch made from another site that is not a plain page navigation.
    CrossSiteFetch,
    /// A path that only the public machine door serves.
    MachineEndpoint,
}

impl Refusal {
    fn status(self) -> StatusCode {
        match self {
            Self::BadTarget => StatusCode::BAD_REQUEST,
            Self::WrongHost => StatusCode::MISDIRECTED_REQUEST,
            Self::CrossOrigin | Self::CrossSiteFetch => StatusCode::FORBIDDEN,
            Self::MachineEndpoint => StatusCode::NOT_FOUND,
        }
    }
}

/// The names this proxy answers to.
fn own_authorities(port: u16) -> [String; 2] {
    [format!("127.0.0.1:{port}"), format!("localhost:{port}")]
}

/// Decides, from the request line and headers alone, whether the request may go on.
pub fn judge(method: &Method, uri: &Uri, headers: &HeaderMap, port: u16) -> Result<(), Refusal> {
    let own = own_authorities(port);
    let is_own = |a: &str| own.iter().any(|o| o.eq_ignore_ascii_case(a.trim()));
    if method == Method::CONNECT {
        return Err(Refusal::BadTarget);
    }
    // Absolute-form targets are how a client asks a proxy to fetch someone else's URL.
    if uri.scheme().is_some() || uri.authority().is_some_and(|a| !is_own(a.as_str())) {
        return Err(Refusal::BadTarget);
    }
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(str::to_owned)
        .or_else(|| uri.authority().map(|a| a.as_str().to_owned()));
    match host.as_deref() {
        None => return Err(Refusal::BadTarget),
        Some(h) if !is_own(h) => return Err(Refusal::WrongHost),
        Some(_) => {}
    }
    if let Some(origin) = headers.get(header::ORIGIN) {
        let ok = origin
            .to_str()
            .ok()
            .and_then(|o| o.strip_prefix("http://"))
            .is_some_and(is_own);
        if !ok {
            return Err(Refusal::CrossOrigin);
        }
    }
    let fetch_site = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok());
    let navigation = headers
        .get("sec-fetch-mode")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|m| m == "navigate");
    if matches!(fetch_site, Some("cross-site" | "same-site")) && !navigation {
        return Err(Refusal::CrossSiteFetch);
    }
    if is_machine_path(uri.path()) {
        return Err(Refusal::MachineEndpoint);
    }
    Ok(())
}

fn is_machine_path(path: &str) -> bool {
    MACHINE_PATHS.iter().any(|p| {
        path == *p
            || path
                .strip_prefix(p)
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

fn is_ws_path(path: &str) -> bool {
    path == WS_PATH || path.starts_with("/ws/")
}

/// The second policy added to every response. A browser enforces both this and the one HQ sends,
/// so it can only narrow what the page may do: no connections but this origin's, no framing.
fn extra_csp(port: u16) -> String {
    format!(
        "connect-src 'self' blob: ws://127.0.0.1:{port} ws://localhost:{port}; \
         frame-ancestors 'none'; object-src 'none'; base-uri 'self'; form-action 'self'"
    )
}

struct ProxyState {
    client: reqwest::Client,
    upstream: SocketAddr,
    port: u16,
    csp: HeaderValue,
}

pub struct ProxyHandle {
    pub addr: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl ProxyHandle {
    /// `http://127.0.0.1:<port>`, the origin the windows and the consent page use.
    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.addr.port())
    }

    pub async fn shutdown(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let _ = self.task.await;
    }
}

pub async fn start(config: ProxyConfig) -> Result<ProxyHandle> {
    config.validate()?;
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .with_context(|| {
            format!(
                "binding the proxy to {} (is another app using it?)",
                config.listen
            )
        })?;
    let addr = listener.local_addr()?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()?;
    let state = Arc::new(ProxyState {
        client,
        upstream: config.upstream,
        port: addr.port(),
        csp: HeaderValue::from_str(&extra_csp(addr.port()))?,
    });
    let app = Router::new().fallback(handle).with_state(state);
    let (stop, stopped) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await;
    });
    Ok(ProxyHandle {
        addr,
        stop: Some(stop),
        task,
    })
}

async fn handle(State(state): State<Arc<ProxyState>>, req: Request) -> Response {
    if let Err(refusal) = judge(req.method(), req.uri(), req.headers(), state.port) {
        return refusal.status().into_response();
    }
    let is_upgrade = req
        .headers()
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
    if is_upgrade {
        if !is_ws_path(req.uri().path()) {
            return StatusCode::NOT_FOUND.into_response();
        }
        return websocket(&state, req).await;
    }
    forward(&state, req).await
}

fn is_hop_by_hop(name: &HeaderName) -> bool {
    HOP_BY_HOP.contains(&name.as_str())
}

async fn forward(state: &ProxyState, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let path = parts
        .uri
        .path_and_query()
        .map_or("/", |p| p.as_str())
        .to_owned();
    let url = format!("http://{}{path}", state.upstream);
    let mut headers = HeaderMap::new();
    for (name, value) in &parts.headers {
        if !is_hop_by_hop(name) {
            headers.append(name.clone(), value.clone());
        }
    }
    // HQ builds its CSP and its origin checks from `Host`, so the client's own is kept.
    let upstream = state
        .client
        .request(parts.method, url)
        .headers(headers)
        .body(reqwest::Body::wrap_stream(body.into_data_stream()))
        .send()
        .await;
    let upstream = match upstream {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!("proxy: upstream unreachable: {e}");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };
    let mut response = Response::builder().status(upstream.status());
    {
        let out = response.headers_mut().expect("builder is fresh");
        for (name, value) in upstream.headers() {
            let n = name.as_str();
            let hop = is_hop_by_hop(name);
            // The proxy is same-origin for its clients; HQ's CORS answers would only widen that.
            if !hop && !n.starts_with("access-control-") {
                out.append(name.clone(), value.clone());
            }
        }
        out.append(header::CONTENT_SECURITY_POLICY, state.csp.clone());
        out.insert(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
        out.insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        );
        out.insert(
            HeaderName::from_static("cross-origin-resource-policy"),
            HeaderValue::from_static("same-origin"),
        );
    }
    response
        .body(Body::from_stream(upstream.bytes_stream()))
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}

/// Headers the chat socket's own handshake needs upstream.
const WS_FORWARDED: [&str; 5] = [
    "authorization",
    "cookie",
    "origin",
    "sec-websocket-protocol",
    "user-agent",
];

async fn websocket(state: &ProxyState, req: Request) -> Response {
    let (mut parts, _body) = req.into_parts();
    let upgrade = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
        Ok(u) => u,
        Err(e) => return e.into_response(),
    };
    let path = parts
        .uri
        .path_and_query()
        .map_or("/", |p| p.as_str())
        .to_owned();
    let Ok(mut request) = format!("ws://{}{path}", state.upstream).into_client_request() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    for name in WS_FORWARDED {
        if let Some(value) = parts.headers.get(name) {
            request
                .headers_mut()
                .insert(HeaderName::from_static(name), value.clone());
        }
    }
    let connected =
        tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(request)).await;
    let Ok(Ok((upstream, _))) = connected else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    upgrade.on_upgrade(move |client| pump(client, upstream))
}

async fn pump(
    client: ws::WebSocket,
    upstream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    let (mut to_client, mut from_client) = client.split();
    let (mut to_upstream, mut from_upstream) = upstream.split();
    let up = async {
        while let Some(Ok(message)) = from_client.next().await {
            if to_upstream.send(upstream_message(message)).await.is_err() {
                break;
            }
        }
        let _ = to_upstream.close().await;
    };
    let down = async {
        while let Some(Ok(message)) = from_upstream.next().await {
            let Some(message) = client_message(message) else {
                continue;
            };
            if to_client.send(message).await.is_err() {
                break;
            }
        }
        let _ = to_client.close().await;
    };
    tokio::select! {
        _ = up => {}
        _ = down => {}
    }
}

fn upstream_message(m: ws::Message) -> tungstenite::Message {
    match m {
        ws::Message::Text(t) => tungstenite::Message::Text(t.as_str().into()),
        ws::Message::Binary(b) => tungstenite::Message::Binary(b),
        ws::Message::Ping(b) => tungstenite::Message::Ping(b),
        ws::Message::Pong(b) => tungstenite::Message::Pong(b),
        ws::Message::Close(frame) => {
            tungstenite::Message::Close(frame.map(|f| tungstenite::protocol::CloseFrame {
                code: f.code.into(),
                reason: f.reason.as_str().into(),
            }))
        }
    }
}

fn client_message(m: tungstenite::Message) -> Option<ws::Message> {
    Some(match m {
        tungstenite::Message::Text(t) => ws::Message::Text(t.as_str().into()),
        tungstenite::Message::Binary(b) => ws::Message::Binary(b),
        tungstenite::Message::Ping(b) => ws::Message::Ping(b),
        tungstenite::Message::Pong(b) => ws::Message::Pong(b),
        tungstenite::Message::Close(frame) => ws::Message::Close(frame.map(|f| ws::CloseFrame {
            code: f.code.into(),
            reason: f.reason.as_str().into(),
        })),
        tungstenite::Message::Frame(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(method: &str, uri: &str, headers: &[(&str, &str)]) -> Result<(), Refusal> {
        let mut map = HeaderMap::new();
        for (k, v) in headers {
            map.insert(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        judge(
            &method.parse().unwrap(),
            &uri.parse().unwrap(),
            &map,
            47_474,
        )
    }

    const HOST: (&str, &str) = ("host", "127.0.0.1:47474");

    #[test]
    fn only_loopback_ends_are_accepted() {
        let ok = ProxyConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            upstream: "127.0.0.1:5678".parse().unwrap(),
        };
        assert!(ok.validate().is_ok());
        for (listen, upstream) in [
            ("0.0.0.0:0", "127.0.0.1:5678"),
            ("192.168.1.5:0", "127.0.0.1:5678"),
            ("127.0.0.1:0", "10.0.0.2:5678"),
            ("127.0.0.1:0", "93.184.216.34:80"),
        ] {
            let c = ProxyConfig {
                listen: listen.parse().unwrap(),
                upstream: upstream.parse().unwrap(),
            };
            assert!(c.validate().is_err(), "{listen} -> {upstream}");
        }
    }

    #[test]
    fn the_proxys_own_names_pass_and_every_other_host_does_not() {
        assert_eq!(req("GET", "/", &[HOST]), Ok(()));
        assert_eq!(req("GET", "/", &[("host", "LOCALHOST:47474")]), Ok(()));
        for host in [
            "evil.example:47474",
            "127.0.0.1:5678",
            "127.0.0.1",
            "localhost.evil.com:47474",
            "",
        ] {
            assert_eq!(
                req("GET", "/", &[("host", host)]),
                Err(Refusal::WrongHost),
                "{host}"
            );
        }
        assert_eq!(req("GET", "/", &[]), Err(Refusal::BadTarget));
    }

    #[test]
    fn it_never_acts_as_a_forward_proxy() {
        assert_eq!(
            req("CONNECT", "127.0.0.1:47474", &[HOST]),
            Err(Refusal::BadTarget)
        );
        assert_eq!(
            req("GET", "http://example.com/", &[HOST]),
            Err(Refusal::BadTarget)
        );
        assert_eq!(
            req("GET", "http://127.0.0.1:47474/", &[HOST]),
            Err(Refusal::BadTarget)
        );
    }

    #[test]
    fn a_foreign_or_null_origin_is_refused_and_the_proxys_own_passes() {
        for origin in [
            "http://evil.example",
            "null",
            "https://127.0.0.1:47474",
            "http://127.0.0.1:1",
        ] {
            assert_eq!(
                req("POST", "/api/x", &[HOST, ("origin", origin)]),
                Err(Refusal::CrossOrigin),
                "{origin}"
            );
        }
        assert_eq!(
            req(
                "POST",
                "/api/x",
                &[HOST, ("origin", "http://127.0.0.1:47474")]
            ),
            Ok(())
        );
        assert_eq!(
            req(
                "POST",
                "/api/x",
                &[HOST, ("origin", "http://localhost:47474")]
            ),
            Ok(())
        );
    }

    #[test]
    fn a_cross_site_fetch_is_refused_but_a_cross_site_navigation_is_not() {
        let fetch = [
            HOST,
            ("sec-fetch-site", "cross-site"),
            ("sec-fetch-mode", "cors"),
        ];
        assert_eq!(
            req("GET", "/api/oauth/requests/1", &fetch),
            Err(Refusal::CrossSiteFetch)
        );
        let nav = [
            HOST,
            ("sec-fetch-site", "cross-site"),
            ("sec-fetch-mode", "navigate"),
        ];
        assert_eq!(req("GET", "/oauth/authorize", &nav), Ok(()));
        let same = [
            HOST,
            ("sec-fetch-site", "same-origin"),
            ("sec-fetch-mode", "cors"),
        ];
        assert_eq!(req("GET", "/api/x", &same), Ok(()));
    }

    #[test]
    fn the_machine_endpoints_stay_off_this_door() {
        for path in [
            "/mcp",
            "/mcp/sse",
            "/oauth/token",
            "/oauth/revoke",
            "/.well-known/oauth-protected-resource",
        ] {
            assert_eq!(
                req("GET", path, &[HOST]),
                Err(Refusal::MachineEndpoint),
                "{path}"
            );
        }
        for path in [
            "/mcpx",
            "/oauth/authorize",
            "/connect",
            "/api/oauth/requests/1",
            "/oauth/tokens",
        ] {
            assert_eq!(req("GET", path, &[HOST]), Ok(()), "{path}");
        }
    }
}
