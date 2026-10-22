//! The connector door end to end: a certificate through the ACME flow, served over real TLS,
//! with the allow-list, the Host check, the challenge handling and the timeouts.

use super::support::*;
use crate::tls::{Served, Timeouts, TlsError, TlsState};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const SHORT: Duration = Duration::from_millis(400);

fn short_timeouts() -> Timeouts {
    Timeouts {
        handshake: SHORT,
        request_headers: SHORT,
        ..Timeouts::default()
    }
}

#[tokio::test]
async fn a_certificate_from_the_acme_flow_serves_the_oauth_metadata_over_real_tls() {
    let w = world();
    let door = door(&w).await;
    assert_eq!(door.tls.status().state, TlsState::NoCertificate);

    door.tls.order_now().await;

    let status = door.tls.status();
    assert_eq!(status.state, TlsState::Valid, "{status:?}");
    assert!(status.not_after.unwrap() > crate::oauth::now());
    let stats = door.ca.stats();
    assert_eq!(
        (
            stats.accounts,
            stats.orders,
            stats.validations_ok,
            stats.certificates
        ),
        (1, 1, 1, 1)
    );
    assert_eq!(stats.contacts, ["mailto:owner@example.com"]);

    let reply = get(&door, "/.well-known/oauth-protected-resource", &[]).await;
    assert_eq!(reply.status, 200, "{reply:?}");
    let doc: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(doc["resource"], format!("https://{HOST}/mcp"));
    assert_eq!(doc["authorization_servers"][0], format!("https://{HOST}"));
}

#[tokio::test]
async fn every_machine_path_answers_and_the_rest_of_the_server_is_a_404() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;

    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
        "/.well-known/oauth-authorization-server",
    ] {
        assert_eq!(get(&door, path, &[]).await.status, 200, "{path}");
    }
    // /mcp without a credential is the 401 challenge Claude starts discovery from.
    let challenge = exchange(
        Target {
            addr: door.addr,
            root: &door.ca.root,
            sni: HOST,
        },
        "POST",
        Some(HOST),
        "/mcp",
        &[("content-type", "application/json")],
        "{}",
    )
    .await;
    assert_eq!(challenge.status, 401);
    assert!(
        challenge
            .header("www-authenticate")
            .unwrap()
            .contains("resource_metadata=")
    );

    // The owner's web token is valid on the real server for all of these, and none of them is served.
    let owner = format!("Bearer {WEB_TOKEN}");
    for path in [
        "/",
        "/health",
        "/api/tree",
        "/api/settings",
        "/ws",
        "/connect",
        "/api/oauth/connector",
        "/api/oauth/requests/abc",
        "/mcp/",
        "//mcp",
        "/%6Dcp",
        "/.well-known/",
        "/.well-known/../api/tree",
        "/assets/index.js",
    ] {
        let reply = get(&door, path, &[("authorization", &owner)]).await;
        assert_eq!(reply.status, 404, "{path}: {reply:?}");
        assert_eq!(
            reply.body, "Not found",
            "{path}: a plain refusal, no handler's body"
        );
    }
}

#[tokio::test]
async fn a_refused_request_reaches_no_handler_and_no_middleware() {
    use axum::routing::{any, get as get_route};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let w = world();
    let seen = Arc::new(AtomicUsize::new(0));
    let count = |seen: &Arc<AtomicUsize>| {
        let seen = seen.clone();
        move || {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                "ok"
            }
        }
    };
    let counted = axum::Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get_route(count(&seen)),
        )
        .route("/api/tree", get_route(count(&seen)))
        .fallback(any(count(&seen)))
        .layer(axum::middleware::from_fn({
            let seen = seen.clone();
            move |req: axum::extract::Request, next: axum::middleware::Next| {
                let seen = seen.clone();
                async move {
                    seen.fetch_add(1000, Ordering::SeqCst);
                    next.run(req).await
                }
            }
        }));
    let mut parts_world = w;
    parts_world.router = counted;
    let door = door(&parts_world).await;
    door.tls.order_now().await;

    for path in ["/api/tree", "/", "/anything", "/mcp/"] {
        assert_eq!(get(&door, path, &[]).await.status, 404, "{path}");
    }
    assert_eq!(
        seen.load(Ordering::SeqCst),
        0,
        "nothing inside the router ran"
    );
    assert_eq!(
        get(&door, "/.well-known/oauth-protected-resource", &[])
            .await
            .status,
        200
    );
    assert_eq!(
        seen.load(Ordering::SeqCst),
        1001,
        "an allowed path runs the router once"
    );
}

#[tokio::test]
async fn a_forged_host_cannot_reach_the_handler_that_answers_only_on_the_consent_origin() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    // The window is open: the consent origin's handler would now answer, with anything but a 404.
    let now = crate::oauth::now();
    w.state
        .db
        .with_conn(|c| hq_db::mcp_connector::arm(c, now + 600))
        .unwrap();
    let path = "/oauth/authorize?response_type=code";

    let forged = exchange(
        Target {
            addr: door.addr,
            root: &door.ca.root,
            sni: HOST,
        },
        "GET",
        Some(CONSENT_HOST),
        path,
        &[],
        "",
    )
    .await;
    assert_eq!(forged.status, 404, "{forged:?}");
    let honest = exchange(
        Target {
            addr: door.addr,
            root: &door.ca.root,
            sni: HOST,
        },
        "GET",
        Some(HOST),
        path,
        &[],
        "",
    )
    .await;
    assert_eq!(
        honest.status, 404,
        "the public name never serves authorize: {honest:?}"
    );

    // The same request on the consent origin itself is what the handler answers (a 400 for the
    // missing client), which proves the handler is live and only the door kept it out.
    let router = w.router.clone();
    let direct = tower::ServiceExt::oneshot(
        router,
        axum::http::Request::get(path)
            .header("host", CONSENT_HOST)
            .body(axum::body::Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_ne!(
        direct.status(),
        404,
        "the handler answers on its own origin"
    );

    let both = exchange(
        Target {
            addr: door.addr,
            root: &door.ca.root,
            sni: HOST,
        },
        "GET",
        Some(HOST),
        path,
        &[("host", CONSENT_HOST)],
        "",
    )
    .await;
    assert_eq!(both.status, 404, "two Host headers: {both:?}");
}

#[tokio::test]
async fn a_hello_for_another_name_or_before_a_certificate_is_dropped() {
    let w = world();
    let door = door(&w).await;
    // The client accepts any certificate, so a handshake that fails was refused by the server.
    let any = || client_config_unverified(&[b"http/1.1"]);
    let early = connect(door.addr, HOST, any()).await;
    assert!(early.is_err(), "no certificate yet");

    door.tls.order_now().await;
    for sni in [
        "other.example.test",
        "hq.example.ts.net",
        "device.edge.example.test.evil.test",
    ] {
        let wrong = connect(door.addr, sni, any()).await;
        assert!(wrong.is_err(), "{sni}");
    }
    assert!(
        connect(
            door.addr,
            HOST,
            client_config(&door.ca.root, &[b"http/1.1"])
        )
        .await
        .is_ok()
    );
    assert!(
        connect(
            door.addr,
            "DEVICE.edge.example.test",
            client_config(&door.ca.root, &[b"http/1.1"])
        )
        .await
        .is_ok()
    );
}

#[tokio::test]
async fn http2_is_served_too() {
    use http_body_util::{BodyExt, Empty};
    use hyper_util::rt::{TokioExecutor, TokioIo};

    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    let tls = connect(door.addr, HOST, client_config(&door.ca.root, &[b"h2"]))
        .await
        .unwrap();
    assert_eq!(tls.get_ref().1.alpn_protocol(), Some(&b"h2"[..]));
    let (mut sender, conn) =
        hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(tls))
            .await
            .unwrap();
    tokio::spawn(conn);
    // No Host header: HTTP/2 carries the name as :authority, and the door supplies the Host.
    let request = hyper::Request::get(format!(
        "https://{HOST}/.well-known/oauth-authorization-server"
    ))
    .body(Empty::<bytes::Bytes>::new())
    .unwrap();
    let response = sender.send_request(request).await.unwrap();
    assert_eq!(response.status(), 200);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let doc: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(doc["issuer"], format!("https://{HOST}"));

    let wrong = hyper::Request::get(format!(
        "https://{CONSENT_HOST}/.well-known/oauth-authorization-server"
    ))
    .body(Empty::<bytes::Bytes>::new())
    .unwrap();
    assert_eq!(sender.send_request(wrong).await.unwrap().status(), 404);
}

#[tokio::test]
async fn forged_proxy_headers_do_not_make_a_request_look_local() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    // `HQ_MCP_DEV_NO_AUTH` opens `/mcp` for a request that carries none of these headers, which is
    // how a local client looks. The gate's own test asks the real `mcp_dev_open` about the headers
    // it leaves behind; this one shows a client claiming to be local gets nothing from the door.
    let lie = [
        ("content-type", "application/json"),
        ("x-forwarded-for", "127.0.0.1"),
        ("forwarded", "for=127.0.0.1"),
        ("x-real-ip", "127.0.0.1"),
    ];
    let reply = exchange(
        Target {
            addr: door.addr,
            root: &door.ca.root,
            sni: HOST,
        },
        "POST",
        Some(HOST),
        "/mcp",
        &lie,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
    )
    .await;
    assert_eq!(reply.status, 401, "{reply:?}");
}

#[tokio::test]
async fn a_silent_peer_is_closed_when_the_handshake_times_out() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    let mut tls = build_tls(&w, &door.ca, &door.store_dir, |t| {
        t.acceptor_mut().timeouts = short_timeouts();
    });
    // Same stored certificate, shorter limits.
    assert_eq!(tls.status().state, TlsState::Valid);
    let _ = &mut tls;
    let (mut client, server) = tokio::io::duplex(1024);
    let started = std::time::Instant::now();
    let served = tokio::spawn(async move { tls.serve(server).await });

    // The client opens the stream and says nothing.
    let result = tokio::time::timeout(Duration::from_secs(5), served)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(TlsError::Timeout(_))), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(3));
    let mut buf = [0u8; 8];
    assert_eq!(
        client.read(&mut buf).await.unwrap(),
        0,
        "the server side is closed"
    );
}

#[tokio::test]
async fn a_peer_that_stops_halfway_through_the_hello_is_closed_too() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    let tls = build_tls(&w, &door.ca, &door.store_dir, |t| {
        t.acceptor_mut().timeouts = short_timeouts();
    });
    let (mut client, server) = tokio::io::duplex(1024);
    let served = tokio::spawn(async move { tls.serve(server).await });
    client
        .write_all(&[0x16, 0x03, 0x01, 0x02, 0x00, 0x01])
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), served)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(TlsError::Timeout(_))), "{result:?}");
}

#[tokio::test]
async fn a_connected_client_that_never_finishes_its_request_is_closed() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    let tls = std::sync::Arc::new(build_tls(&w, &door.ca, &door.store_dir, |t| {
        t.acceptor_mut().timeouts = short_timeouts();
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    serve_forever(listener, tls);

    let mut client = connect(addr, HOST, client_config(&door.ca.root, &[b"http/1.1"]))
        .await
        .unwrap();
    client
        .write_all(b"GET /.well-known/oauth-protected-resource HTTP/1.1\r\nHost: ")
        .await
        .unwrap();
    let mut buf = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut buf)).await;
    assert!(
        read.is_ok(),
        "the server closed the connection instead of waiting forever"
    );
}

#[tokio::test]
async fn acme_tls_1_gets_the_challenge_certificate_and_never_reaches_http() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    // Order two certificates' worth of state: arm a challenge by hand and look at it as a CA would.
    let digest = [9u8; 32];
    door.tls.challenge_slot().arm(&digest).unwrap();

    let mut tls = connect(door.addr, HOST, client_config_unverified(&[b"acme-tls/1"]))
        .await
        .unwrap();
    assert_eq!(tls.get_ref().1.alpn_protocol(), Some(&b"acme-tls/1"[..]));
    let leaf = tls.get_ref().1.peer_certificates().unwrap()[0].clone();
    let (_, cert) = x509_parser::parse_x509_certificate(leaf.as_ref()).unwrap();
    let ext = cert
        .extensions()
        .iter()
        .find(|e| e.oid.to_id_string() == "1.3.6.1.5.5.7.1.31")
        .expect("the acmeIdentifier extension");
    assert!(ext.critical && ext.value.ends_with(&digest));

    // Whatever the peer now writes is not read as HTTP: the server shuts the stream down.
    let _ = tls
        .write_all(b"GET /.well-known/oauth-protected-resource HTTP/1.1\r\nHost: x\r\n\r\n")
        .await;
    let mut out = Vec::new();
    let _ = tls.read_to_end(&mut out).await;
    assert!(
        out.is_empty(),
        "no HTTP answer: {:?}",
        String::from_utf8_lossy(&out)
    );
    door.tls.challenge_slot().disarm();
}

#[tokio::test]
async fn acme_tls_1_is_refused_when_no_challenge_is_pending_or_for_another_name() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    let none = connect(door.addr, HOST, client_config_unverified(&[b"acme-tls/1"])).await;
    assert!(none.is_err(), "nothing pending");

    door.tls.challenge_slot().arm(&[1u8; 32]).unwrap();
    let wrong = connect(
        door.addr,
        "other.example.test",
        client_config_unverified(&[b"acme-tls/1"]),
    )
    .await;
    assert!(wrong.is_err(), "another name");
    // A client that offers acme-tls/1 among real protocols is a web client, not the CA.
    let mixed = connect(
        door.addr,
        HOST,
        client_config(&door.ca.root, &[b"acme-tls/1", b"http/1.1"]),
    )
    .await;
    let mixed = mixed.expect("served as an ordinary connection");
    assert_eq!(mixed.get_ref().1.alpn_protocol(), Some(&b"http/1.1"[..]));
    door.tls.challenge_slot().disarm();
}

#[tokio::test]
async fn the_challenge_is_withdrawn_when_the_order_ends() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    assert!(!door.tls.challenge_slot().is_armed());

    door.ca.reject_validation(true);
    door.tls.order_now().await;
    assert!(
        !door.tls.challenge_slot().is_armed(),
        "also after a failed order"
    );
}

#[tokio::test]
async fn too_many_connections_are_turned_away_instead_of_queued() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    let tls = build_tls(&w, &door.ca, &door.store_dir, |t| {
        t.acceptor_mut().timeouts = Timeouts {
            handshake: Duration::from_secs(2),
            ..Timeouts::default()
        };
        t.acceptor_mut().set_max_connections(1);
    });
    let tls = std::sync::Arc::new(tls);
    let (_held_client, held) = tokio::io::duplex(64);
    let first = {
        let tls = tls.clone();
        tokio::spawn(async move { tls.serve(held).await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (_second_client, second) = tokio::io::duplex(64);
    let turned_away = tls.serve(second).await;
    assert!(
        matches!(turned_away, Err(TlsError::Busy)),
        "{turned_away:?}"
    );
    first.abort();
}

#[tokio::test]
async fn a_rate_limit_keeps_the_current_certificate_and_names_when_to_try_again() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    let served_before = get(&door, "/.well-known/oauth-protected-resource", &[])
        .await
        .status;
    assert_eq!(served_before, 200);

    door.ca.rate_limit_next_order(Some(2 * 60 * 60));
    door.tls.order_now().await;

    let status = door.tls.status();
    assert_eq!(status.state, TlsState::RateLimited, "{status:?}");
    let wait = status.next_attempt_at.unwrap() - crate::oauth::now();
    assert!(
        (2 * 60 * 60 - 5..=2 * 60 * 60).contains(&wait),
        "Retry-After is honoured: {wait}"
    );
    assert!(status.last_error.unwrap().contains("rate limit"));
    assert!(
        status.not_after.is_some(),
        "the certificate in use is still known"
    );
    assert_eq!(
        get(&door, "/.well-known/oauth-protected-resource", &[])
            .await
            .status,
        200
    );
    assert_eq!(
        door.ca.stats().accounts,
        1,
        "a limit never buys a new account"
    );
}

#[tokio::test]
async fn a_failed_order_backs_off_and_the_door_keeps_serving() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    door.ca.reject_validation(true);
    door.tls.order_now().await;

    let status = door.tls.status();
    assert_eq!(status.state, TlsState::Failed, "{status:?}");
    let wait = status.next_attempt_at.unwrap() - crate::oauth::now();
    assert!(
        (14 * 60..=15 * 60).contains(&wait),
        "first retry after 15 minutes: {wait}"
    );
    assert_eq!(
        get(&door, "/.well-known/oauth-protected-resource", &[])
            .await
            .status,
        200
    );

    door.ca.reject_validation(false);
    door.tls.order_now().await;
    let healed = door.tls.status();
    assert_eq!(healed.state, TlsState::Valid);
    assert_eq!(healed.next_attempt_at, None);
}

#[tokio::test]
async fn a_restart_serves_the_saved_certificate_without_ordering_again() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    let orders = door.ca.stats().orders;

    let restarted = std::sync::Arc::new(build_tls(&w, &door.ca, &door.store_dir, |_| {}));
    assert_eq!(restarted.status().state, TlsState::Valid);
    assert_eq!(restarted.status().not_after, door.tls.status().not_after);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    serve_forever(listener, restarted);
    let reply = exchange(
        Target {
            addr,
            root: &door.ca.root,
            sni: HOST,
        },
        "GET",
        Some(HOST),
        "/.well-known/oauth-protected-resource",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, 200);
    assert_eq!(door.ca.stats().orders, orders, "no new order");
}

#[tokio::test]
async fn the_account_and_key_are_stored_privately_and_never_printed() {
    use std::os::unix::fs::PermissionsExt;
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    let dir = door.store_dir.path().join("connector_tls");
    assert_eq!(
        std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(
        files.len(),
        2,
        "one account, one certificate bundle: {files:?}"
    );
    for f in &files {
        assert_eq!(
            std::fs::metadata(f).unwrap().permissions().mode() & 0o777,
            0o600,
            "{f:?}"
        );
    }
    let bundle = std::fs::read_to_string(dir.join(format!("{HOST}.pem"))).unwrap();
    assert!(bundle.starts_with("-----BEGIN PRIVATE KEY-----"));
    let key_body: String = bundle
        .lines()
        .skip(1)
        .take_while(|l| !l.starts_with("-----END"))
        .collect();
    let status = format!("{:?}", door.tls.status());
    assert!(
        !status.contains(&key_body[..40]),
        "the status never carries the key"
    );
}

#[tokio::test]
async fn tls_is_off_by_default_and_refused_without_the_connector_or_under_lite() {
    use hq_core::config::{ConnectorTlsConfig, McpConnectorConfig};
    let cfg = |tls_on: bool| McpConnectorConfig {
        enabled: true,
        public_url: Some(format!("https://{HOST}")),
        tls: ConnectorTlsConfig {
            enabled: tls_on,
            ..Default::default()
        },
        ..Default::default()
    };
    let w = world();
    assert!(
        crate::tls::ConnectorTls::for_state(&w.state, w.router.clone(), &cfg(false))
            .unwrap()
            .is_none()
    );
    assert!(
        crate::tls::ConnectorTls::for_state(&w.state, w.router.clone(), &cfg(true))
            .unwrap()
            .is_some()
    );

    let lite = world_with(hq_core::config::Profile::Lite);
    let err = crate::tls::ConnectorTls::for_state(&lite.state, lite.router.clone(), &cfg(true))
        .err()
        .unwrap();
    assert!(err.contains("lite"), "{err}");
    assert!(
        crate::tls::ConnectorTls::for_state(&lite.state, lite.router.clone(), &cfg(false))
            .unwrap()
            .is_none(),
        "off is off under lite too"
    );

    let no_connector = world_without_connector();
    let err = crate::tls::ConnectorTls::for_state(
        &no_connector.state,
        no_connector.router.clone(),
        &cfg(true),
    )
    .err()
    .unwrap();
    assert!(err.contains("connector"), "{err}");

    let other = McpConnectorConfig {
        public_url: Some("https://other.example.test".into()),
        ..cfg(true)
    };
    let err = crate::tls::ConnectorTls::for_state(&w.state, w.router.clone(), &other)
        .err()
        .unwrap();
    assert!(err.contains("different origin"), "{err}");
}

#[tokio::test]
async fn serve_reports_what_kind_of_connection_it_handled() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    let tls = std::sync::Arc::new(build_tls(&w, &door.ca, &door.store_dir, |_| {}));
    let (client, server) = tokio::io::duplex(64 * 1024);
    let handled = {
        let tls = tls.clone();
        tokio::spawn(async move { tls.serve(server).await })
    };
    let mut config = (*client_config(&door.ca.root, &[b"http/1.1"])).clone();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let name = tokio_rustls::rustls::pki_types::ServerName::try_from(HOST.to_string()).unwrap();
    let mut conn = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config))
        .connect(name, client)
        .await
        .unwrap();
    conn.write_all(b"GET /.well-known/oauth-protected-resource HTTP/1.1\r\nHost: device.edge.example.test\r\nConnection: close\r\n\r\n").await.unwrap();
    let mut body = Vec::new();
    let _ = conn.read_to_end(&mut body).await;
    assert!(String::from_utf8_lossy(&body).starts_with("HTTP/1.1 200"));
    assert_eq!(handled.await.unwrap().unwrap(), Served::Http);
}

#[tokio::test]
async fn an_idle_keep_alive_connection_is_closed_instead_of_holding_a_slot() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    let tls = std::sync::Arc::new(build_tls(&w, &door.ca, &door.store_dir, |t| {
        t.acceptor_mut().timeouts = short_timeouts();
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    serve_forever(listener, tls);

    let mut client = connect(addr, HOST, client_config(&door.ca.root, &[b"http/1.1"]))
        .await
        .unwrap();
    let request = format!(
        "GET /.well-known/oauth-protected-resource HTTP/1.1\r\nHost: {HOST}\r\nConnection: keep-alive\r\n\r\n"
    );
    client.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut out)).await;
    assert!(read.is_ok(), "the server closed the idle connection");
    assert!(String::from_utf8_lossy(&out).starts_with("HTTP/1.1 200"));
}

#[tokio::test]
async fn an_idle_http2_connection_ends_when_its_lifetime_runs_out() {
    use hyper_util::rt::{TokioExecutor, TokioIo};

    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    let tls = std::sync::Arc::new(build_tls(&w, &door.ca, &door.store_dir, |t| {
        t.acceptor_mut().timeouts = Timeouts {
            connection: SHORT,
            ..Timeouts::default()
        };
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    serve_forever(listener, tls);

    let io = connect(addr, HOST, client_config(&door.ca.root, &[b"h2"]))
        .await
        .unwrap();
    let (_sender, conn) = hyper::client::conn::http2::handshake::<
        _,
        _,
        http_body_util::Empty<bytes::Bytes>,
    >(TokioExecutor::new(), TokioIo::new(io))
    .await
    .unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(5), conn).await;
    assert!(ended.is_ok(), "the server wound the idle session down");
}

#[tokio::test]
async fn a_saved_certificate_that_has_expired_is_not_served_and_an_order_is_due() {
    let w = world();
    let door = door(&w).await;
    let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let mut params = rcgen::CertificateParams::new(vec![HOST.to_string()]).unwrap();
    params.not_before = rcgen::date_time_ymd(2020, 1, 1);
    params.not_after = rcgen::date_time_ymd(2020, 4, 1);
    let expired = params.self_signed(&pair).unwrap();
    let store = crate::tls::store::PrivateDir::at(door.store_dir.path().join("connector_tls"));
    let bundle = format!("{}{}", pair.serialize_pem(), expired.pem());
    store
        .write(&store.issued_path(HOST), bundle.as_bytes())
        .unwrap();

    let restarted = build_tls(&w, &door.ca, &door.store_dir, |_| {});
    assert_eq!(restarted.status().state, TlsState::NoCertificate);
    assert_eq!(restarted.status().not_after, None);
}

#[tokio::test]
async fn two_orders_at_once_take_turns_and_leave_one_account() {
    let w = world();
    let door = door(&w).await;
    tokio::join!(door.tls.order_now(), door.tls.order_now());
    let stats = door.ca.stats();
    assert_eq!(stats.accounts, 1);
    assert_eq!(stats.orders, 2);
    assert_eq!(
        stats.validations_failed, 0,
        "the second order did not disarm the first's challenge"
    );
    assert_eq!(door.tls.status().state, TlsState::Valid);
}
