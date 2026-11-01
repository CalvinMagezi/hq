//! The two doors from `docs/plans/zero-setup-pairing.md`, kept apart:
//!
//! - door 1, the own-devices listener (an iroh stream in production, a TCP port here), serves
//!   the web app and its API to devices that hold the owner's credentials;
//! - door 2, the connector's TLS door, serves the machine paths to a client holding a connector
//!   grant.
//!
//! Neither depends on the other. These tests hold the four promises the plan lists. Each one was
//! also broken on purpose to see it fail; the mutation is named beside the test.

use super::support::*;
use crate::tls::{CONNECTOR_PATHS, refuse_connector_paths};
use axum::Router;
use std::net::SocketAddr;
use tokio::task::JoinHandle;

const TOOLS_LIST: &str = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;

/// Door 1: the real router on a plain port, behind the guard the own-devices listener must carry.
async fn own_devices_door(router: Router) -> (SocketAddr, JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = refuse_connector_paths(router);
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (addr, task)
}

async fn own(addr: SocketAddr, method: &str, path: &str, bearer: Option<&str>) -> (u16, String) {
    let client = reqwest::Client::new();
    let mut req = client
        .request(method.parse().unwrap(), format!("http://{addr}{path}"))
        .header("x-hq-client", "test")
        .header("content-type", "application/json")
        .body(if method == "POST" { "{}" } else { "" });
    if let Some(token) = bearer {
        req = req.bearer_auth(token);
    }
    let res = req.send().await.unwrap();
    (res.status().as_u16(), res.text().await.unwrap())
}

/// Door 2: a POST to `/mcp` through the TLS door with whatever credentials the test chooses.
async fn connector_mcp(door: &Door, headers: &[(&str, &str)]) -> (u16, String) {
    let mut all = vec![("content-type", "application/json")];
    all.extend_from_slice(headers);
    let reply = exchange(
        Target {
            addr: door.addr,
            root: &door.ca.root,
            sni: HOST,
        },
        "POST",
        Some(HOST),
        "/mcp",
        &all,
        TOOLS_LIST,
    )
    .await;
    (reply.status, reply.body)
}

fn device_token(w: &World) -> (String, String) {
    let minted = hq_db::web_auth::mint(&w.state.db, "phone", crate::oauth::now(), None).unwrap();
    (minted.id, minted.token)
}

/// Mutation: `refuse_connector_paths` returns the router unchanged. The first loop then finds
/// `/.well-known/...` answering 200 on door 1.
#[tokio::test]
async fn the_own_devices_door_never_serves_connector_paths() {
    let w = world();
    let (addr, _task) = own_devices_door(w.router.clone()).await;
    let grant = mint_grant(&w.state);

    // Control: the same router, unguarded, does serve them, and door 1 serves its own API.
    let (open, _) = own_unguarded(&w, "/.well-known/oauth-protected-resource").await;
    assert_eq!(
        open, 200,
        "the control: the connector's routes exist in this server"
    );
    assert_eq!(own(addr, "GET", "/api/tree", Some(WEB_TOKEN)).await.0, 200);

    let spellings = CONNECTOR_PATHS.iter().copied().chain([
        "/mcp/",
        "/mcp/extra",
        "/oauth/anything",
        "/.well-known/anything",
        "/.well-known/oauth-protected-resource/extra",
    ]);
    for path in spellings {
        for method in ["GET", "POST"] {
            for credential in [None, Some(WEB_TOKEN), Some(grant.access_token.as_str())] {
                let (status, body) = own(addr, method, path, credential).await;
                assert_eq!(status, 404, "{method} {path} with {credential:?}: {body}");
            }
        }
    }
}

async fn own_unguarded(w: &World, path: &str) -> (u16, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = w.router.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    own(addr, "GET", path, None).await
}

/// Mutation: the gate's `scrub_credentials` is skipped (the request goes on untouched) and the
/// session token below becomes a `/mcp` credential that works on the public door; the web and
/// device tokens stay refused only because `/mcp` never read them, so the session token is the
/// one that shows it.
#[tokio::test]
async fn the_connector_door_never_accepts_the_credentials_of_the_own_devices_door() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    let (_, device) = device_token(&w);
    let session = w
        .state
        .db
        .with_conn(|c| hq_db::session_tokens::mint(c, "sess-1"))
        .unwrap();
    let grant = mint_grant(&w.state);
    let paired_key = crate::mcp_http::paired_device_key(&w.state.db, "aa11");
    let (own_addr, _task) = own_devices_door(w.router.clone()).await;

    // What door 1 accepts today.
    for token in [WEB_TOKEN, device.as_str()] {
        assert_eq!(own(own_addr, "GET", "/api/tree", Some(token)).await.0, 200);
    }
    // The session token is a credential `/mcp` honours when it arrives on a trusted door.
    let (status, _) = {
        let router = w.router.clone();
        let res = tower::ServiceExt::oneshot(
            router,
            axum::http::Request::post("/mcp")
                .header("authorization", format!("Bearer {session}"))
                .header("content-type", "application/json")
                .header("host", "localhost")
                .body(axum::body::Body::from(TOOLS_LIST))
                .unwrap(),
        )
        .await
        .unwrap();
        (res.status().as_u16(), ())
    };
    assert_eq!(
        status, 200,
        "the control: a session token works on the local door"
    );

    // On the connector door none of them is anything.
    for (label, headers) in [
        (
            "web token",
            vec![("authorization", format!("Bearer {WEB_TOKEN}"))],
        ),
        (
            "device token",
            vec![("authorization", format!("Bearer {device}"))],
        ),
        (
            "session token",
            vec![("authorization", format!("Bearer {session}"))],
        ),
        (
            "session token as a key",
            vec![("x-api-key", session.clone())],
        ),
        (
            "web token as a key",
            vec![("x-api-key", WEB_TOKEN.to_string())],
        ),
        (
            "paired device key",
            vec![("authorization", format!("Bearer {paired_key}"))],
        ),
        (
            "paired device key as a key",
            vec![("x-api-key", paired_key.clone())],
        ),
        (
            "session token beside a good grant",
            vec![
                ("x-api-key", session.clone()),
                ("authorization", format!("Bearer {}", grant.access_token)),
            ],
        ),
    ] {
        let borrowed: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let (status, body) = connector_mcp(&door, &borrowed).await;
        assert_eq!(status, 401, "{label}: {body}");
    }
    // The grant's own token is what the door is for.
    let ok = format!("Bearer {}", grant.access_token);
    let (status, body) = connector_mcp(&door, &[("authorization", ok.as_str())]).await;
    assert_eq!(status, 200, "{body}");

    // And the owner's API is not on this door at all, credential or not.
    for token in [WEB_TOKEN, device.as_str(), grant.access_token.as_str()] {
        let bearer = format!("Bearer {token}");
        let reply = get(&door, "/api/tree", &[("authorization", &bearer)]).await;
        assert_eq!(reply.status, 404);
    }
}

/// Mutation: `ConnectorTls::for_state` ignores `state.connector` being unset (so it builds a
/// door with the connector off). The "connector off" half then finds a door that exists.
#[tokio::test]
async fn turning_one_door_off_leaves_the_other_unchanged() {
    let w = world();
    let (own_addr, own_task) = own_devices_door(w.router.clone()).await;
    let door = door(&w).await;
    door.tls.order_now().await;
    let grant = mint_grant(&w.state);
    let ok = format!("Bearer {}", grant.access_token);
    let (_, device) = device_token(&w);

    let own_before = [
        own(own_addr, "GET", "/health", None).await,
        own(own_addr, "GET", "/api/tree", Some(WEB_TOKEN)).await,
        own(own_addr, "GET", "/api/tree", Some(&device)).await,
    ];
    let door_before = connector_mcp(&door, &[("authorization", ok.as_str())]).await;
    assert_eq!(door_before.0, 200);

    // Door 2 off: the listener that feeds it stops and the certificate loop never runs again.
    // Door 1 answers what it answered.
    door.accept.abort();
    let _ = (&mut { door.accept }).await;
    let gone = connect(
        door.addr,
        HOST,
        client_config(&door.ca.root, &[b"http/1.1"]),
    )
    .await;
    assert!(gone.is_err(), "door 2 is down");
    let own_after_connector_stopped = [
        own(own_addr, "GET", "/health", None).await,
        own(own_addr, "GET", "/api/tree", Some(WEB_TOKEN)).await,
        own(own_addr, "GET", "/api/tree", Some(&device)).await,
    ];
    assert_eq!(own_before, own_after_connector_stopped);

    // Door 1 off: the connector door answers what it answered.
    let w2 = world();
    let door2 = self::door(&w2).await;
    door2.tls.order_now().await;
    let grant2 = mint_grant(&w2.state);
    let ok2 = format!("Bearer {}", grant2.access_token);
    let (own2, task2) = own_devices_door(w2.router.clone()).await;
    let before = connector_mcp(&door2, &[("authorization", ok2.as_str())]).await;
    let wk_before = get(&door2, "/.well-known/oauth-protected-resource", &[])
        .await
        .body;
    task2.abort();
    let _ = task2.await;
    assert!(
        reqwest::get(format!("http://{own2}/health")).await.is_err(),
        "door 1 is down"
    );
    assert_eq!(
        connector_mcp(&door2, &[("authorization", ok2.as_str())]).await,
        before
    );
    assert_eq!(
        get(&door2, "/.well-known/oauth-protected-resource", &[])
            .await
            .body,
        wk_before
    );

    // The connector itself off: no door can be built, and door 1 is the same server as before.
    let off = world_without_connector();
    let cfg = hq_core::config::McpConnectorConfig {
        enabled: true,
        public_url: Some(format!("https://{HOST}")),
        tls: hq_core::config::ConnectorTlsConfig {
            enabled: true,
            agree_to_ca_terms: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let refused = crate::tls::ConnectorTls::for_state(&off.state, off.router.clone(), &cfg)
        .err()
        .expect("no door without the connector");
    assert!(refused.contains("connector to be on"), "{refused}");
    let (off_addr, _t) = own_devices_door(off.router.clone()).await;
    assert_eq!(
        own(off_addr, "GET", "/api/tree", Some(WEB_TOKEN)).await.0,
        200
    );
    assert_eq!(
        own(off_addr, "GET", "/health", None).await.0,
        own_before[0].0
    );
    own_task.abort();
}

/// Mutation: `revoke_grant` is replaced by a call that signs out every device token as well. The
/// second assertion block then finds the device token dead after a grant was revoked.
#[tokio::test]
async fn revoking_one_leaves_the_other_alone() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    let (own_addr, _task) = own_devices_door(w.router.clone()).await;
    let (device_id, device) = device_token(&w);
    let grant = mint_grant(&w.state);
    let bearer = format!("Bearer {}", grant.access_token);

    // Revoke the connector grant: Claude loses access, the devices keep theirs.
    w.state
        .db
        .with_conn(|c| {
            hq_db::mcp_connector::revoke_grant(c, &grant.grant_id, "test", crate::oauth::now())
        })
        .unwrap();
    assert_eq!(
        connector_mcp(&door, &[("authorization", bearer.as_str())])
            .await
            .0,
        401
    );
    assert_eq!(
        own(own_addr, "GET", "/api/tree", Some(&device)).await.0,
        200
    );
    assert_eq!(
        own(own_addr, "GET", "/api/tree", Some(WEB_TOKEN)).await.0,
        200
    );

    // A new grant, then revoke the device: the device is cut, the grant is not.
    let grant = mint_grant(&w.state);
    let bearer = format!("Bearer {}", grant.access_token);
    assert!(hq_db::web_auth::delete(&w.state.db, &device_id).unwrap());
    assert_eq!(
        own(own_addr, "GET", "/api/tree", Some(&device)).await.0,
        401
    );
    assert_eq!(
        connector_mcp(&door, &[("authorization", bearer.as_str())])
            .await
            .0,
        200
    );

    // And the kill switch for every grant leaves the owner's own token alone.
    w.state
        .db
        .with_conn(|c| hq_db::mcp_connector::revoke_all(c, "test", crate::oauth::now()))
        .unwrap();
    assert_eq!(
        connector_mcp(&door, &[("authorization", bearer.as_str())])
            .await
            .0,
        401
    );
    assert_eq!(
        own(own_addr, "GET", "/api/tree", Some(WEB_TOKEN)).await.0,
        200
    );
}
