//! The same order against Pebble, Let's Encrypt's own ACME test server, which is a different
//! implementation from the in-process CA in `test_ca.rs` and validates TLS-ALPN-01 the way the
//! real CA does. It needs Docker, so it is `#[ignore]`d; run it with
//! `scripts/test-connector-tls-pebble.sh`, which starts Pebble, points this test at it and cleans up.

use super::support::*;
use crate::tls::acme::{AcmeHttp, AcmeSettings};
use crate::tls::gate::Gate;
use crate::tls::store::PrivateDir;
use crate::tls::{ConnectorTls, Parts, TlsState};
use std::sync::Arc;
use tokio_rustls::rustls::pki_types::CertificateDer;
use tokio_rustls::rustls::pki_types::pem::PemObject;

fn env(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is not set; run scripts/test-connector-tls-pebble.sh"))
}

fn root_from_pem(pem: &[u8]) -> CertificateDer<'static> {
    CertificateDer::from_pem_slice(pem).expect("a PEM certificate")
}

#[tokio::test]
#[ignore = "needs Docker and Pebble: scripts/test-connector-tls-pebble.sh"]
async fn pebble_issues_a_certificate_through_the_acme_flow_and_the_door_serves_it() {
    let directory = env("HQ_PEBBLE_DIRECTORY");
    let minica = std::fs::read(env("HQ_PEBBLE_MINICA")).unwrap();
    let management = env("HQ_PEBBLE_MANAGEMENT");
    let listen = env("HQ_PEBBLE_LISTEN");

    // Pebble's ACME endpoints and management API are served under its own test CA.
    let acme_client = reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(&minica).unwrap())
        .build()
        .unwrap();

    let w = world();
    let store_dir = tempfile::TempDir::new().unwrap();
    let tls = Arc::new(
        ConnectorTls::from_parts(Parts {
            acme: AcmeSettings {
                host: HOST.into(),
                directory_url: directory,
                contact_email: None,
            },
            http: AcmeHttp::new(acme_client.clone()),
            store: PrivateDir::at(store_dir.path().join("connector_tls")),
            gate: Gate::new(HOST, live_grant_check(&w.state)),
            router: w.router.clone(),
        })
        .unwrap(),
    );
    // Pebble dials the name on its TLS-ALPN-01 port; the door is what answers.
    let listener = tokio::net::TcpListener::bind(&listen).await.unwrap();
    let addr = listener.local_addr().unwrap();
    serve_forever(listener, tls.clone());

    tls.order_now().await;
    let status = tls.status();
    assert_eq!(status.state, TlsState::Valid, "{status:?}");

    // The chain Pebble signed ends at its root, which the management API publishes.
    let root_pem = acme_client
        .get(format!("{management}/roots/0"))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let root = root_from_pem(&root_pem);

    let connect_to = std::net::SocketAddr::from(([127, 0, 0, 1], addr.port()));
    let reply = exchange(
        Target {
            addr: connect_to,
            root: &root,
            sni: HOST,
        },
        "GET",
        Some(HOST),
        "/.well-known/oauth-protected-resource",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    let doc: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(doc["resource"], format!("https://{HOST}/mcp"));

    let hidden = exchange(
        Target {
            addr: connect_to,
            root: &root,
            sni: HOST,
        },
        "GET",
        Some(HOST),
        "/api/tree",
        &[],
        "",
    )
    .await;
    assert_eq!(hidden.status, 404);
}
