//! The proxy in front of a fake HQ web server, over real sockets.

use axum::{
    Json, Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{any, get},
};
use hq_desktop::proxy::{self, ProxyConfig, ProxyHandle};
use std::net::SocketAddr;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Default)]
struct Seen(Arc<AtomicUsize>);

async fn echo_request(State(seen): State<Seen>, headers: HeaderMap, body: String) -> Response {
    seen.0.fetch_add(1, Ordering::SeqCst);
    let pick = |n: &str| {
        headers
            .get(n)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let mut res = Json(serde_json::json!({
        "host": pick("host"),
        "authorization": pick("authorization"),
        "cookie": pick("cookie"),
        "client": pick("x-hq-client"),
        "body": body,
    }))
    .into_response();
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        "default-src 'self'".parse().unwrap(),
    );
    h.insert("access-control-allow-origin", "*".parse().unwrap());
    h.append(header::SET_COOKIE, "a=1; Path=/".parse().unwrap());
    h.append(
        header::SET_COOKIE,
        "b=2; Path=/api/oauth/requests".parse().unwrap(),
    );
    res
}

async fn socket(ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(|mut s: WebSocket| async move {
        while let Some(Ok(m)) = s.recv().await {
            if let Message::Text(t) = m {
                let _ = s.send(Message::Text(format!("echo:{t}").into())).await;
            }
        }
    })
}

async fn endless_stream() -> Response {
    let ticks = futures::stream::unfold((), |()| async {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        Some((
            Ok::<_, std::convert::Infallible>(axum::body::Bytes::from("tick\n")),
            (),
        ))
    });
    Response::new(axum::body::Body::from_stream(ticks))
}

struct Rig {
    proxy: ProxyHandle,
    upstream_hits: Seen,
    _upstream: tokio::task::JoinHandle<()>,
}

async fn rig() -> Rig {
    let seen = Seen::default();
    let app = Router::new()
        .route("/ws", get(socket))
        .route("/api/socket", get(socket))
        .route("/api/stream", get(endless_stream))
        .route("/health", get(|| async { "ok" }))
        .fallback(any(echo_request))
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let proxy = proxy::start(ProxyConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream,
    })
    .await
    .unwrap();
    Rig {
        proxy,
        upstream_hits: seen,
        _upstream: task,
    }
}

impl Rig {
    fn port(&self) -> u16 {
        self.proxy.addr.port()
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port())
    }

    fn hits(&self) -> usize {
        self.upstream_hits.0.load(Ordering::SeqCst)
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

/// One raw HTTP/1.1 exchange, for requests a client library would not send.
async fn raw(addr: SocketAddr, request: &str) -> String {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), s.read_to_end(&mut out)).await;
    String::from_utf8_lossy(&out).into_owned()
}

#[tokio::test]
async fn a_request_is_forwarded_with_its_own_host_and_credentials_and_none_added() {
    let rig = rig().await;
    let c = client();

    let bare: serde_json::Value = c
        .get(rig.url("/api/oauth/requests/abc"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(bare["host"], format!("127.0.0.1:{}", rig.port()));
    assert!(
        bare["authorization"].is_null(),
        "the proxy added a credential: {bare}"
    );

    let own: serde_json::Value = c
        .post(rig.url("/api/oauth/requests/abc/approve"))
        .header("authorization", "Bearer client-token")
        .header("cookie", "hq_oauth_request=binder")
        .header("x-hq-client", "web")
        .body(r#"{"scope":"read_only"}"#)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(own["authorization"], "Bearer client-token");
    assert_eq!(own["cookie"], "hq_oauth_request=binder");
    assert_eq!(own["client"], "web");
    assert_eq!(own["body"], r#"{"scope":"read_only"}"#);
}

#[tokio::test]
async fn the_consent_page_and_its_api_are_served() {
    let rig = rig().await;
    let c = client();
    for path in [
        "/connect?request=1",
        "/oauth/authorize?client_id=x",
        "/api/oauth/requests/1",
    ] {
        let res = c.get(rig.url(path)).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{path}");
    }
    assert_eq!(rig.hits(), 3);
}

#[tokio::test]
async fn responses_gain_a_second_policy_and_lose_cross_origin_grants() {
    let rig = rig().await;
    let res = client().get(rig.url("/api/x")).send().await.unwrap();
    let policies: Vec<_> = res
        .headers()
        .get_all("content-security-policy")
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .collect();
    assert_eq!(policies.len(), 2, "{policies:?}");
    assert_eq!(policies[0], "default-src 'self'");
    assert!(policies[1].contains(&format!("ws://127.0.0.1:{}", rig.port())));
    assert!(policies[1].contains("frame-ancestors 'none'"));
    assert!(res.headers().get("access-control-allow-origin").is_none());
    assert_eq!(res.headers()["x-content-type-options"], "nosniff");
    assert_eq!(res.headers()["referrer-policy"], "no-referrer");
    // Both cookies survive, and the proxy does not rewrite them.
    let cookies: Vec<_> = res
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .collect();
    assert_eq!(cookies, ["a=1; Path=/", "b=2; Path=/api/oauth/requests"]);
}

#[tokio::test]
async fn a_request_for_another_host_never_reaches_hq() {
    let rig = rig().await;
    let c = client();
    for host in ["evil.example", "127.0.0.1:5678", "localhost.evil.com:80"] {
        let res = c
            .get(rig.url("/api/x"))
            .header("host", host)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::MISDIRECTED_REQUEST, "{host}");
    }
    let res = c
        .get(rig.url("/api/x"))
        .header("host", format!("localhost:{}", rig.port()))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::MISDIRECTED_REQUEST);
    let res = c.get(rig.url("/api/x")).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(rig.hits(), 1);
}

#[tokio::test]
async fn another_sites_origin_is_refused_before_hq_sees_it() {
    let rig = rig().await;
    let res = client()
        .post(rig.url("/api/oauth/requests/1/approve"))
        .header("origin", "http://evil.example")
        .header("x-hq-client", "web")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(rig.hits(), 0);
}

#[tokio::test]
async fn it_does_not_fetch_other_urls_for_a_client() {
    let rig = rig().await;
    let own = format!("127.0.0.1:{}", rig.port());
    let absolute = raw(
        rig.proxy.addr,
        &format!(
            "GET http://example.com/secret HTTP/1.1\r\nHost: {own}\r\nConnection: close\r\n\r\n"
        ),
    )
    .await;
    assert!(absolute.starts_with("HTTP/1.1 400"), "{absolute}");
    let connect = raw(
        rig.proxy.addr,
        &format!("CONNECT example.com:443 HTTP/1.1\r\nHost: {own}\r\nConnection: close\r\n\r\n"),
    )
    .await;
    assert!(connect.starts_with("HTTP/1.1 400"), "{connect}");
    assert_eq!(rig.hits(), 0);
}

#[tokio::test]
async fn the_machine_endpoints_are_not_served_here() {
    let rig = rig().await;
    let c = client();
    for path in [
        "/mcp",
        "/mcp/sse",
        "/oauth/token",
        "/oauth/revoke",
        "/.well-known/oauth-authorization-server",
    ] {
        let res = c.post(rig.url(path)).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{path}");
    }
    assert_eq!(rig.hits(), 0);
}

#[tokio::test]
async fn paths_that_a_url_parser_would_resolve_to_a_machine_endpoint_never_reach_hq() {
    let rig = rig().await;
    let own = format!("127.0.0.1:{}", rig.port());
    // A client library would tidy these paths before sending, so they go out raw.
    for path in [
        "/./mcp",
        "/x/../mcp",
        "/%2e/mcp",
        "/oauth/./token",
        "/a/../.well-known/oauth-authorization-server",
        "//mcp",
    ] {
        let reply = raw(
            rig.proxy.addr,
            &format!("GET {path} HTTP/1.1\r\nHost: {own}\r\nConnection: close\r\n\r\n"),
        )
        .await;
        assert!(reply.starts_with("HTTP/1.1 400"), "{path}: {reply}");
    }
    assert_eq!(rig.hits(), 0);
}

#[tokio::test]
async fn stopping_the_proxy_does_not_wait_for_a_stream_that_never_ends() {
    use futures::StreamExt;
    let rig = rig().await;
    let mut stream = client()
        .get(rig.url("/api/stream"))
        .send()
        .await
        .unwrap()
        .bytes_stream();
    // The first chunk proves the stream is open and flowing through the proxy.
    assert_eq!(&stream.next().await.unwrap().unwrap()[..], b"tick\n");

    let stopped = tokio::time::timeout(std::time::Duration::from_secs(8), rig.proxy.shutdown());
    assert!(
        stopped.await.is_ok(),
        "stopping the proxy hung on an open stream"
    );
}

#[tokio::test]
async fn an_unreachable_hq_is_a_bad_gateway() {
    let dead = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let proxy = proxy::start(ProxyConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream: dead,
    })
    .await
    .unwrap();
    let res = client()
        .get(format!("http://127.0.0.1:{}/", proxy.addr.port()))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn the_chat_socket_passes_through_and_other_upgrades_do_not() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{Message as M, client::IntoClientRequest};

    let rig = rig().await;
    let mut request = format!("ws://127.0.0.1:{}/ws?ticket=t", rig.port())
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "origin",
        format!("http://127.0.0.1:{}", rig.port()).parse().unwrap(),
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    ws.send(M::text("hi")).await.unwrap();
    let got = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(got, M::text("echo:hi"));
    ws.close(None).await.unwrap();

    // A socket HQ would accept on another path is not carried, and the machine paths are not either.
    for path in ["/api/socket", "/mcp"] {
        let elsewhere = format!("ws://127.0.0.1:{}{path}", rig.port())
            .into_client_request()
            .unwrap();
        assert!(
            tokio_tungstenite::connect_async(elsewhere).await.is_err(),
            "{path}"
        );
    }
    let mut foreign = format!("ws://127.0.0.1:{}/ws", rig.port())
        .into_client_request()
        .unwrap();
    foreign
        .headers_mut()
        .insert("origin", "http://evil.example".parse().unwrap());
    assert!(tokio_tungstenite::connect_async(foreign).await.is_err());
}

#[tokio::test]
async fn a_second_proxy_on_a_taken_port_says_so_and_a_non_loopback_one_never_binds() {
    let rig = rig().await;
    let upstream: SocketAddr = "127.0.0.1:5678".parse().unwrap();
    let taken = proxy::start(ProxyConfig {
        listen: rig.proxy.addr,
        upstream,
    })
    .await;
    assert!(taken.is_err());
    let open = proxy::start(ProxyConfig {
        listen: "0.0.0.0:0".parse().unwrap(),
        upstream,
    })
    .await;
    assert!(open.is_err());
}
