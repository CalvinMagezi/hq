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
        idle: SHORT,
        shutdown_grace: SHORT,
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
        "/oauth/authorize",
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
async fn a_forged_host_or_the_consent_path_is_a_404_even_with_the_window_open() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    // The window is open: the consent origin's handler would now answer, with anything but a 404.
    let now = crate::oauth::now();
    w.state
        .db
        .with_conn(|c| hq_db::mcp_connector::arm(c, now + 600))
        .unwrap();
    let authorize = "/oauth/authorize?response_type=code";
    let target = || Target {
        addr: door.addr,
        root: &door.ca.root,
        sni: HOST,
    };

    // The handler answers on its own origin, which proves it is live and only the door keeps it out.
    let direct = tower::ServiceExt::oneshot(
        w.router.clone(),
        axum::http::Request::get(authorize)
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

    // Through the door, under either name, the path is not served.
    for host in [HOST, CONSENT_HOST] {
        let reply = exchange(target(), "GET", Some(host), authorize, &[], "").await;
        assert_eq!(reply.status, 404, "{host}: {reply:?}");
    }
    // And a forged Host on a path the door does serve is refused too.
    let wk = "/.well-known/oauth-protected-resource";
    let forged = exchange(target(), "GET", Some(CONSENT_HOST), wk, &[], "").await;
    assert_eq!(forged.status, 404, "{forged:?}");
    let two = exchange(
        target(),
        "GET",
        Some(HOST),
        wk,
        &[("host", CONSENT_HOST)],
        "",
    )
    .await;
    assert_eq!(two.status, 404, "two Host headers: {two:?}");
    assert_eq!(
        exchange(target(), "GET", Some(HOST), wk, &[], "")
            .await
            .status,
        200
    );
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
            handshake: Duration::from_secs(5),
            ..Timeouts::default()
        };
        t.acceptor_mut().set_max_connections(1);
        t.acceptor_mut().set_max_hellos(2);
    });
    let tls = std::sync::Arc::new(tls);
    let hello = |root: tokio_rustls::rustls::pki_types::CertificateDer<'static>| {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let config = client_config(&root, &[b"http/1.1"]);
        let name = tokio_rustls::rustls::pki_types::ServerName::try_from(HOST.to_string()).unwrap();
        let client = tokio::spawn(async move {
            // Connects and holds the session open without sending a request.
            let conn = tokio_rustls::TlsConnector::from(config)
                .connect(name, client)
                .await;
            tokio::time::sleep(Duration::from_secs(30)).await;
            drop(conn);
        });
        (client, server)
    };

    // One connection takes the only web slot and holds it.
    let (first_client, first_server) = hello(door.ca.root.clone());
    let first = {
        let tls = tls.clone();
        tokio::spawn(async move { tls.serve(first_server).await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;

    // The next one passes its hello and finds no web slot.
    let (second_client, second_server) = hello(door.ca.root.clone());
    let turned_away = tls.serve(second_server).await;
    assert!(
        matches!(turned_away, Err(TlsError::Busy)),
        "{turned_away:?}"
    );

    // Silent peers use hello slots, not web slots: two fill the hello stage, the third is turned away.
    let (_silent_a_client, silent_a) = tokio::io::duplex(64);
    let (_silent_b_client, silent_b) = tokio::io::duplex(64);
    let (_silent_c_client, silent_c) = tokio::io::duplex(64);
    let a = {
        let tls = tls.clone();
        tokio::spawn(async move { tls.serve(silent_a).await })
    };
    let b = {
        let tls = tls.clone();
        tokio::spawn(async move { tls.serve(silent_b).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    let flooded = tls.serve(silent_c).await;
    assert!(matches!(flooded, Err(TlsError::Busy)), "{flooded:?}");
    for task in [first, a, b] {
        task.abort();
    }
    first_client.abort();
    second_client.abort();
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
async fn the_account_and_key_are_stored_privately() {
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
    let pem = files
        .iter()
        .find(|f| f.extension().is_some_and(|e| e == "pem"))
        .expect("the certificate bundle");
    let bundle = std::fs::read_to_string(pem).unwrap();
    assert!(bundle.starts_with("-----BEGIN PRIVATE KEY-----"));
    assert!(bundle.contains("-----BEGIN CERTIFICATE-----"));
}

#[tokio::test]
async fn tls_is_off_by_default_and_refused_without_the_connector_or_under_lite() {
    use hq_core::config::{ConnectorTlsConfig, McpConnectorConfig};
    let cfg = |tls_on: bool| McpConnectorConfig {
        enabled: true,
        public_url: Some(format!("https://{HOST}")),
        tls: ConnectorTlsConfig {
            enabled: tls_on,
            agree_to_ca_terms: true,
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

    let single = world_single_origin();
    let err = crate::tls::ConnectorTls::for_state(&single.state, single.router.clone(), &cfg(true))
        .err()
        .unwrap();
    assert!(err.contains("authorize_url"), "{err}");

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
        .write(
            &store.issued_path(HOST, &door.ca.directory_url),
            bundle.as_bytes(),
        )
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

/// A door with `short_timeouts()` on its own port, serving the certificate the first door ordered.
async fn short_lived_door(w: &World, door: &Door) -> std::net::SocketAddr {
    let tls = std::sync::Arc::new(build_tls(w, &door.ca, &door.store_dir, |t| {
        t.acceptor_mut().timeouts = short_timeouts();
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    serve_forever(listener, tls);
    addr
}

#[tokio::test]
async fn a_client_that_finishes_the_handshake_and_says_nothing_is_closed_under_either_protocol() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    let addr = short_lived_door(&w, &door).await;
    for alpn in [&b"http/1.1"[..], &b"h2"[..]] {
        let mut client = connect(addr, HOST, client_config(&door.ca.root, &[alpn]))
            .await
            .unwrap();
        let mut out = Vec::new();
        let read = tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut out)).await;
        assert!(
            read.is_ok(),
            "{}: the silent session was closed",
            String::from_utf8_lossy(alpn)
        );
    }
}

#[tokio::test]
async fn an_http2_client_that_starts_the_preface_and_stops_is_closed() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    let addr = short_lived_door(&w, &door).await;
    let mut client = connect(addr, HOST, client_config(&door.ca.root, &[b"h2"]))
        .await
        .unwrap();
    client.write_all(b"PRI * HTTP/2.0\r\n\r\nSM").await.unwrap();
    let mut out = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut out)).await;
    assert!(read.is_ok(), "the half-sent preface did not hold the slot");
}

#[tokio::test]
async fn a_request_that_takes_too_long_is_answered_with_a_408() {
    use axum::routing::get as get_route;
    let mut w = world();
    w.router = axum::Router::new().route(
        "/.well-known/oauth-protected-resource",
        get_route(|| async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            "late"
        }),
    );
    let door = door(&w).await;
    door.tls.order_now().await;
    let tls = std::sync::Arc::new(build_tls(&w, &door.ca, &door.store_dir, |t| {
        t.acceptor_mut().timeouts = Timeouts {
            request: SHORT,
            ..Timeouts::default()
        };
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    serve_forever(listener, tls);
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
    assert_eq!(reply.status, 408, "{reply:?}");
}

#[tokio::test]
async fn a_body_past_the_limit_is_refused_before_a_handler_reads_it() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    let grant = mint_grant(&w.state);
    let bearer = format!("Bearer {}", grant.access_token);
    let big = "x".repeat(2 << 20);
    let reply = exchange(
        Target {
            addr: door.addr,
            root: &door.ca.root,
            sni: HOST,
        },
        "POST",
        Some(HOST),
        "/mcp",
        &[
            ("content-type", "application/json"),
            ("authorization", bearer.as_str()),
        ],
        &big,
    )
    .await;
    assert_eq!(reply.status, 413, "{reply:?}");
}

#[tokio::test]
async fn a_connection_that_ends_gives_its_slot_back() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    let tls = build_tls(&w, &door.ca, &door.store_dir, |t| {
        t.acceptor_mut().set_max_connections(1);
        t.acceptor_mut().set_max_hellos(1);
    });
    for round in 0..3 {
        // A client that connects with the right name, then vanishes mid-handshake.
        let (client, server) = tokio::io::duplex(64 * 1024);
        let config = client_config(&door.ca.root, &[b"http/1.1"]);
        let name = tokio_rustls::rustls::pki_types::ServerName::try_from(HOST.to_string()).unwrap();
        let hello = tokio::spawn(async move {
            let _ = tokio_rustls::TlsConnector::from(config)
                .connect(name, client)
                .await;
        });
        let served = tls.serve(server).await;
        hello.abort();
        assert!(
            !matches!(served, Err(TlsError::Busy)),
            "round {round}: the slot was not given back: {served:?}"
        );
    }
}

#[tokio::test]
async fn the_hello_checks_refuse_each_for_its_own_reason() {
    let w = world();
    let door = door(&w).await;
    let acceptor = door.tls.acceptor();
    let alpn = |p: &'static [&'static [u8]]| Some(p.iter().copied());

    // No certificate installed yet: a web hello is refused, a validation with nothing pending too.
    assert!(acceptor.choose(Some(HOST), alpn(&[b"h2"])).is_err());
    door.tls.order_now().await;
    assert!(acceptor.choose(Some(HOST), alpn(&[b"h2"])).is_ok());
    assert!(
        acceptor.choose(None, alpn(&[b"h2"])).is_err(),
        "no server name"
    );
    assert!(
        acceptor
            .choose(Some("other.example.test"), alpn(&[b"h2"]))
            .is_err()
    );
    assert!(
        acceptor.choose(Some(HOST), alpn(&[b"acme-tls/1"])).is_err(),
        "nothing pending"
    );
    door.tls.challenge_slot().arm(&[3u8; 32]).unwrap();
    let (_, is_validation) = acceptor.choose(Some(HOST), alpn(&[b"acme-tls/1"])).unwrap();
    assert!(is_validation);
    assert!(
        acceptor
            .choose(Some("other.example.test"), alpn(&[b"acme-tls/1"]))
            .is_err()
    );
    let (_, mixed) = acceptor
        .choose(Some(HOST), alpn(&[b"acme-tls/1", b"http/1.1"]))
        .unwrap();
    assert!(!mixed, "acme-tls/1 beside real protocols is a web client");
    door.tls.challenge_slot().disarm();
}

#[tokio::test]
async fn a_rate_limit_and_the_failure_count_survive_a_restart() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    door.ca.rate_limit_next_order(Some(2 * 60 * 60));
    door.tls.order_now().await;
    let before = door.tls.status();
    assert_eq!(before.state, TlsState::RateLimited);

    let restarted = build_tls(&w, &door.ca, &door.store_dir, |_| {});
    let after = restarted.status();
    assert_eq!(after.state, TlsState::RateLimited, "{after:?}");
    assert_eq!(after.next_attempt_at, before.next_attempt_at);
    assert!(
        after.not_after.is_some(),
        "the saved certificate still serves"
    );

    // Failures in a row keep counting across a restart too.
    door.ca.reject_validation(true);
    restarted.order_now().await;
    let first = restarted.status().next_attempt_at.unwrap() - crate::oauth::now();
    let again = build_tls(&w, &door.ca, &door.store_dir, |_| {});
    again.order_now().await;
    let second = again.status().next_attempt_at.unwrap() - crate::oauth::now();
    assert!((14 * 60..=15 * 60).contains(&first), "{first}");
    assert!(
        (29 * 60..=30 * 60).contains(&second),
        "the count carried over: {second}"
    );
}

#[tokio::test]
async fn a_certificate_saved_for_one_ca_is_not_taken_for_another() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    assert_eq!(door.tls.status().state, TlsState::Valid);

    // The same state directory, a different ACME directory (staging, then production).
    let other_ca = super::test_ca::TestCa::start(door.addr).await;
    let switched = build_tls(&w, &other_ca, &door.store_dir, |_| {});
    assert_eq!(switched.status().state, TlsState::NoCertificate);
    assert_eq!(switched.status().not_after, None);
}

#[tokio::test]
async fn the_acme_client_does_not_follow_redirects() {
    use axum::response::Redirect;
    use axum::routing::get as get_route;
    let app = axum::Router::new().route(
        "/",
        get_route(|| async { Redirect::temporary("/elsewhere") }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = crate::tls::acme::AcmeHttp::default_client().unwrap();
    let res = client.get(format!("http://{addr}/")).send().await.unwrap();
    assert_eq!(res.status(), 307);
}

#[tokio::test]
async fn a_full_door_still_answers_the_cas_validation() {
    let w = world();
    let door = door(&w).await;
    door.tls.order_now().await;
    let tls = std::sync::Arc::new(build_tls(&w, &door.ca, &door.store_dir, |t| {
        t.acceptor_mut().set_max_connections(1);
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    serve_forever(listener, tls.clone());

    // One web connection takes the only web slot and sits idle.
    let held = connect(addr, HOST, client_config(&door.ca.root, &[b"http/1.1"]))
        .await
        .unwrap();
    let refused = connect(addr, HOST, client_config(&door.ca.root, &[b"http/1.1"])).await;
    assert!(refused.is_err(), "a second web client finds the door full");

    // The CA's validation has slots of its own.
    tls.challenge_slot().arm(&[5u8; 32]).unwrap();
    let validation = connect(addr, HOST, client_config_unverified(&[b"acme-tls/1"])).await;
    assert!(validation.is_ok(), "{:?}", validation.err());
    tls.challenge_slot().disarm();
    drop(held);
}
