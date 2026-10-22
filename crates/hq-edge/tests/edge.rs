//! The edge end to end on loopback: a TLS client, the edge, and a device that can only be reached
//! through the relay. Every guard has a test that fails when the guard is removed.

mod common;

use std::{net::SocketAddr, time::Duration};

use common::*;
use hq_edge::{Registered, allow_device, revoke_device};
use iroh::SecretKey;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tokio_rustls::{TlsConnector, rustls::pki_types::ServerName};

const SECOND: Duration = Duration::from_secs(1);

/// A TLS client for `name` going through the edge at `addr`: returns the HTTP response and the
/// ALPN protocol it negotiated.
async fn https_get(
    addr: SocketAddr,
    connector: &TlsConnector,
    name: &str,
) -> (String, Option<Vec<u8>>) {
    let tcp = TcpStream::connect(addr).await.unwrap();
    let server_name = ServerName::try_from(name.to_owned()).unwrap();
    let mut tls = connector
        .connect(server_name, tcp)
        .await
        .expect("TLS through the edge");
    let negotiated = tls.get_ref().1.alpn_protocol().map(<[u8]>::to_vec);
    tls.write_all(format!("GET / HTTP/1.1\r\nHost: {name}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut response = String::new();
    tls.read_to_string(&mut response).await.unwrap();
    (response, negotiated)
}

/// A registered device that answers HTTP, behind an edge with the default caps.
async fn http_world(alpn: &[&[u8]]) -> (Rig, Device, TlsConnector) {
    let key = SecretKey::generate();
    let rig = rig(&[key.public()], |_| {}).await;
    let (acceptor, connector) = tls_pair(&name_of(&key.public()), alpn);
    let device = start_device(key, &rig.relay_url, rig.edge_id, Behavior::Http, acceptor).await;
    (rig, device, connector)
}

/// A registered device that holds or sinks what it is sent, and a valid hello for its name.
async fn raw_world(
    behavior: Behavior,
    tune: impl FnOnce(&mut hq_edge::EdgeConfig),
) -> (Rig, Device, Vec<u8>) {
    let key = SecretKey::generate();
    let rig = rig(&[key.public()], tune).await;
    let (acceptor, _) = tls_pair(&name_of(&key.public()), &[]);
    let device = start_device(key, &rig.relay_url, rig.edge_id, behavior, acceptor).await;
    let hello = client_hello(&device.name());
    (rig, device, hello)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tls_client_reaches_the_device_behind_the_nat_through_the_edge() {
    let (rig, device, connector) = http_world(&[]).await;
    let (response, _) = https_get(rig.addr(), &connector, &device.name()).await;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "got {response:?}");
    assert!(response.ends_with(BODY));
    assert_eq!(device.streams(), 1);
    assert_eq!(rig.edge.stats().forwarded, 1);

    // A second request reuses the same QUIC connection to the device.
    let (again, _) = https_get(rig.addr(), &connector, &device.name()).await;
    assert!(again.ends_with(BODY));
    assert_eq!(device.streams(), 2);
    assert_eq!(
        device.strangers.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tls_alpn_01_connection_passes_through_to_the_device() {
    let (rig, device, connector) = http_world(&[b"acme-tls/1"]).await;
    let (response, negotiated) = https_get(rig.addr(), &connector, &device.name()).await;
    assert!(response.ends_with(BODY));
    assert_eq!(negotiated.as_deref(), Some(&b"acme-tls/1"[..]));
    assert_eq!(
        device.alpn_seen.lock().unwrap().as_deref(),
        Some(&b"acme-tls/1"[..]),
        "the device must see the ALPN the client offered"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unregistered_or_foreign_name_is_closed_without_any_answer() {
    let (rig, device, _) = http_world(&[]).await;
    let stranger = SecretKey::generate().public();
    let names = [
        name_of(&stranger),                       // right domain, nobody registered
        "0000000000000000.edge.test".to_string(), // well-formed label, no such device
        "not-a-label.edge.test".to_string(),      // wrong shape
        format!("{}.other.test", &name_of(&device.id())[..16]), // a real label under another domain
        "edge.test".to_string(),                  // the bare domain
        "a.b.edge.test".to_string(),              // two labels below the domain
        "192.0.2.7".to_string(),                  // an address: the hello carries no name
    ];
    for name in &names {
        let mut tcp = rig.connect().await;
        tcp.write_all(&client_hello(name)).await.unwrap();
        let (got, took) = read_until_closed(&mut tcp, 3 * SECOND).await;
        assert!(got.is_empty(), "{name} got {} bytes back", got.len());
        assert!(took < SECOND, "{name} was held for {took:?}");
    }
    let stats = rig.edge.stats();
    assert_eq!(stats.unknown_name, 6, "{stats:?}");
    assert_eq!(
        stats.hello_bad, 1,
        "the address hello has no name: {stats:?}"
    );
    assert_eq!(device.streams(), 0, "no stranger may reach a device");
    assert_eq!(stats.forwarded, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn garbage_is_closed_at_once_with_no_answer() {
    let (rig, _device, _) = http_world(&[]).await;
    let mut oversized = vec![0x16, 0x03, 0x01, 0xff, 0xff];
    oversized.extend(std::iter::repeat_n(0x01, 4096));
    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "plain http",
            b"GET / HTTP/1.1\r\nHost: x.edge.test\r\n\r\n".to_vec(),
        ),
        ("zeros", vec![0u8; 512]),
        ("noise", (0..=255u8).cycle().take(2048).collect()),
        ("a record announcing 64 KiB", oversized),
        (
            "a truncated hello",
            client_hello("x.edge.test")[..20].to_vec(),
        ),
    ];
    for (what, bytes) in &cases {
        let mut tcp = rig.connect().await;
        tcp.write_all(bytes).await.unwrap();
        let _ = tcp.shutdown().await;
        let (got, took) = read_until_closed(&mut tcp, 3 * SECOND).await;
        assert!(got.is_empty(), "{what}: the edge answered garbage");
        assert!(took < SECOND, "{what}: held for {took:?}");
    }
    let stats = rig.edge.stats();
    assert_eq!(stats.hello_bad, cases.len() as u64, "{stats:?}");
    assert_eq!(stats.forwarded, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hello_that_would_outgrow_the_cap_is_cut_off_by_the_cap_not_by_the_deadline() {
    // A handshake header that announces 65520 bytes, sent as valid records and then silence. The
    // TLS parser would wait for the rest until the deadline; only the byte cap closes it at once.
    let (rig, _device, _) = raw_world(Behavior::Hold, |c| c.limits.hello_timeout_secs = 5).await;
    const RECORD: usize = 16_000;
    let mut bytes = Vec::new();
    for index in 0..3 {
        let mut payload = vec![0u8; RECORD];
        if index == 0 {
            payload[..4].copy_from_slice(&[0x01, 0x00, 0xff, 0xf0]);
        }
        bytes.extend_from_slice(&[0x16, 0x03, 0x03]);
        bytes.extend_from_slice(&(RECORD as u16).to_be_bytes());
        bytes.extend_from_slice(&payload);
    }
    assert!(bytes.len() > hq_edge::MAX_HELLO_BYTES);
    let mut tcp = rig.connect().await;
    let _ = tcp.write_all(&bytes).await;
    let (got, took) = read_until_closed(&mut tcp, 4 * SECOND).await;
    assert!(got.is_empty());
    assert!(
        took < SECOND,
        "held for {took:?}: the byte cap did not close it"
    );
    assert_eq!(rig.edge.stats().hello_bad, 1);
    assert_eq!(rig.edge.stats().hello_timeout, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_loris_client_is_closed_at_the_deadline_not_per_byte() {
    let (rig, _device, hello) =
        raw_world(Behavior::Hold, |c| c.limits.hello_timeout_secs = 1).await;
    let (mut reader, mut writer) = rig.connect().await.into_split();
    let started = std::time::Instant::now();
    // One byte every 200 ms: each byte is inside any per-read window, together they never finish.
    let drip = async {
        for byte in &hello[..hello.len() - 1] {
            if writer.write_all(&[*byte]).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    };
    let watch = async {
        let mut buf = [0u8; 16];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => return started.elapsed(),
                Ok(_) => {}
            }
        }
    };
    let took = tokio::select! {
        took = watch => took,
        () = drip => panic!("the edge let a slow client finish its hello"),
    };
    assert!(
        took >= Duration::from_millis(800) && took < Duration::from_millis(2500),
        "closed after {took:?}, expected about the 1 s deadline"
    );
    assert_eq!(rig.edge.stats().hello_timeout, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_sends_nothing_is_closed_at_the_deadline() {
    let (rig, _device, _) = raw_world(Behavior::Hold, |c| c.limits.hello_timeout_secs = 1).await;
    let mut tcp = rig.connect().await;
    let (got, took) = read_until_closed(&mut tcp, 4 * SECOND).await;
    assert!(got.is_empty());
    assert!(
        took >= Duration::from_millis(800),
        "closed early at {took:?}"
    );
    assert!(
        took < Duration::from_millis(2500),
        "closed late at {took:?}"
    );
    assert_eq!(rig.edge.stats().hello_timeout, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_global_pre_hello_cap_closes_the_extra_connection_at_once() {
    const CAP: usize = 3;
    let (rig, _device, _) = raw_world(Behavior::Hold, |c| {
        c.limits.hello_timeout_secs = 5;
        c.limits.max_pending = CAP;
        c.limits.max_pending_per_source = 100;
    })
    .await;
    let mut idle = Vec::new();
    for _ in 0..CAP {
        idle.push(rig.connect().await);
    }
    eventually(5 * SECOND, "the idle connections to be accepted", || {
        rig.edge.stats().accepted == CAP as u64
    })
    .await;
    let mut extra = rig.connect().await;
    let (got, took) = read_until_closed(&mut extra, 3 * SECOND).await;
    assert!(got.is_empty());
    assert!(took < SECOND, "the extra connection was held for {took:?}");
    assert_eq!(rig.edge.stats().refused_busy, 1);

    // Freeing a slot lets the next one in.
    drop(idle.pop());
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut next = rig.connect().await;
    assert!(
        stays_open(&mut next, Duration::from_millis(500)).await,
        "a freed slot must admit a new connection"
    );
    assert_eq!(rig.edge.stats().refused_busy, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_per_source_pre_hello_cap_stops_one_address() {
    const CAP: usize = 2;
    let (rig, _device, _) = raw_world(Behavior::Hold, |c| {
        c.limits.hello_timeout_secs = 5;
        c.limits.max_pending = 100;
        c.limits.max_pending_per_source = CAP;
    })
    .await;
    let mut idle = Vec::new();
    for _ in 0..CAP {
        idle.push(rig.connect().await);
    }
    eventually(5 * SECOND, "the idle connections to be accepted", || {
        rig.edge.stats().accepted == CAP as u64
    })
    .await;
    let mut extra = rig.connect().await;
    let (_, took) = read_until_closed(&mut extra, 3 * SECOND).await;
    assert!(took < SECOND);
    assert_eq!(rig.edge.stats().refused_busy, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_cannot_exceed_its_connection_cap() {
    const CAP: usize = 2;
    let (rig, device, hello) = raw_world(Behavior::Hold, |c| {
        c.limits.max_connections_per_device = CAP;
    })
    .await;
    let mut open = Vec::new();
    for _ in 0..CAP {
        let mut tcp = rig.connect().await;
        tcp.write_all(&hello).await.unwrap();
        open.push(tcp);
    }
    eventually(10 * SECOND, "both connections to reach the device", || {
        device.streams() == CAP as u64
    })
    .await;

    let mut extra = rig.connect().await;
    extra.write_all(&hello).await.unwrap();
    let (got, took) = read_until_closed(&mut extra, 3 * SECOND).await;
    assert!(got.is_empty());
    assert!(
        took < SECOND,
        "the over-cap connection was held for {took:?}"
    );
    assert_eq!(rig.edge.stats().device_busy, 1);
    assert_eq!(
        device.streams(),
        CAP as u64,
        "the extra one must not reach the device"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn finished_connections_free_their_slot_for_the_next() {
    const CAP: usize = 2;
    let key = SecretKey::generate();
    let rig = rig(&[key.public()], |c| {
        c.limits.max_connections_per_device = CAP
    })
    .await;
    let (acceptor, connector) = tls_pair(&name_of(&key.public()), &[]);
    let device = start_device(key, &rig.relay_url, rig.edge_id, Behavior::Http, acceptor).await;
    for _ in 0..CAP * 3 {
        let (response, _) = https_get(rig.addr(), &connector, &device.name()).await;
        assert!(response.ends_with(BODY));
        // The slot is released when the pipe ends, just after the client has its answer.
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(rig.edge.stats().device_busy, 0);
    assert_eq!(rig.edge.stats().forwarded, (CAP * 3) as u64);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_byte_rate_limit_slows_a_device_down() {
    const RATE: u64 = 64 * 1024;
    const BURST: u64 = 16 * 1024;
    const SENT: usize = 160 * 1024;
    let (rig, device, hello) = raw_world(Behavior::Sink, |c| {
        c.limits.bytes_per_second = RATE;
        c.limits.burst_bytes = BURST;
    })
    .await;
    let mut tcp = rig.connect().await;
    tcp.write_all(&hello).await.unwrap();
    eventually(10 * SECOND, "the connection to reach the device", || {
        device.streams() == 1
    })
    .await;
    let started = std::time::Instant::now();
    tcp.write_all(&vec![7u8; SENT]).await.unwrap();
    tcp.shutdown().await.unwrap();
    let target = (hello.len() + SENT) as u64;
    eventually(30 * SECOND, "the device to receive everything", || {
        device.sunk() >= target
    })
    .await;
    let took = started.elapsed();
    // (SENT - burst) at RATE: about 2.25 s. Unthrottled loopback does this in well under a second.
    let floor = Duration::from_secs_f64((SENT as u64 - BURST) as f64 / RATE as f64 * 0.8);
    assert!(
        took >= floor,
        "moved {SENT} bytes in {took:?}, a {RATE} B/s limit needs at least {floor:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_idle_forwarded_connection_is_closed_after_the_idle_timeout() {
    let (rig, device, hello) = raw_world(Behavior::Hold, |c| c.limits.idle_timeout_secs = 1).await;
    let mut tcp = rig.connect().await;
    tcp.write_all(&hello).await.unwrap();
    eventually(10 * SECOND, "the connection to reach the device", || {
        device.streams() == 1
    })
    .await;
    let (_, took) = read_until_closed(&mut tcp, 6 * SECOND).await;
    assert!(
        took >= Duration::from_millis(700),
        "closed early at {took:?}"
    );
    assert!(
        took < Duration::from_millis(3500),
        "closed late at {took:?}"
    );
    assert_eq!(rig.edge.stats().ended_idle, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_registered_device_that_is_offline_is_given_up_on_at_the_dial_timeout() {
    // Registered, never started: the relay has never heard of it.
    let ghost = SecretKey::generate();
    let rig = rig(&[ghost.public()], |c| c.limits.dial_timeout_secs = 1).await;
    let mut tcp = rig.connect().await;
    tcp.write_all(&client_hello(&name_of(&ghost.public())))
        .await
        .unwrap();
    let (got, took) = read_until_closed(&mut tcp, 6 * SECOND).await;
    assert!(got.is_empty());
    assert!(
        took >= Duration::from_millis(700),
        "gave up early at {took:?}"
    );
    assert!(
        took < Duration::from_millis(3500),
        "gave up late at {took:?}"
    );
    assert_eq!(rig.edge.stats().dial_failed, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_device_cuts_its_open_connections_and_refuses_new_ones() {
    let (rig, device, hello) = raw_world(Behavior::Hold, |_| {}).await;
    let mut tcp = rig.connect().await;
    tcp.write_all(&hello).await.unwrap();
    eventually(10 * SECOND, "the connection to reach the device", || {
        device.streams() == 1
    })
    .await;
    assert!(stays_open(&mut tcp, Duration::from_millis(500)).await);

    assert!(revoke_device(&rig.list, &device.id()).unwrap());
    let (_, took) = read_until_closed(&mut tcp, 6 * SECOND).await;
    assert!(
        took < Duration::from_millis(4500),
        "revocation took {took:?}"
    );
    assert_eq!(rig.edge.stats().ended_cancelled, 1);

    let mut fresh = rig.connect().await;
    fresh.write_all(&hello).await.unwrap();
    let (got, _) = read_until_closed(&mut fresh, 3 * SECOND).await;
    assert!(got.is_empty());
    assert_eq!(rig.edge.stats().unknown_name, 1);
    assert_eq!(device.streams(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_relay_admits_registered_devices_and_the_edge_and_nobody_else() {
    let (rig, _device, _) = http_world(&[]).await;
    let relay = rig.edge.relay().expect("the rig co-hosts a relay");
    assert_eq!(
        relay.denied(),
        0,
        "the registered device and the edge must be admitted"
    );

    let stranger = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .secret_key(SecretKey::generate())
        .relay_mode(iroh::RelayMode::custom([rig.relay_url.clone()]))
        .clear_ip_transports()
        .bind()
        .await
        .unwrap();
    eventually(10 * SECOND, "the relay to turn the stranger away", || {
        relay.denied() >= 1
    })
    .await;
    // Counting refusals is not enough: the stranger must really be kept off the relay.
    assert!(
        tokio::time::timeout(3 * SECOND, stranger.online())
            .await
            .is_err(),
        "an unregistered endpoint reached its home relay"
    );
    drop(stranger);
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_edge_may_open_streams_on_the_edge_protocol() {
    // The device side of the contract: a peer that is not the edge is turned away. The relay
    // already keeps strangers off, so the caller here is another registered device. The harness
    // device does what the real device's gate must; this pins the two together so a fake edge
    // written against the contract behaves the same.
    let (key, other_key) = (SecretKey::generate(), SecretKey::generate());
    let rig = rig(&[key.public(), other_key.public()], |_| {}).await;
    let (acceptor, _) = tls_pair(&name_of(&key.public()), &[]);
    let device = start_device(key, &rig.relay_url, rig.edge_id, Behavior::Http, acceptor).await;
    let other = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .secret_key(other_key)
        .relay_mode(iroh::RelayMode::custom([rig.relay_url.clone()]))
        .clear_ip_transports()
        .bind()
        .await
        .unwrap();
    let addr = iroh::EndpointAddr::new(device.id()).with_relay_url(rig.relay_url.clone());
    if let Ok(Ok(conn)) =
        tokio::time::timeout(5 * SECOND, other.connect(addr, hq_edge::EDGE_ALPN)).await
    {
        let _ = conn.open_bi().await;
    }
    eventually(10 * SECOND, "the device to turn the caller away", || {
        device.strangers.load(std::sync::atomic::Ordering::SeqCst) >= 1
    })
    .await;
    assert_eq!(device.streams(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_added_to_the_list_while_the_edge_runs_is_reachable() {
    let (rig, _device, _) = http_world(&[]).await;
    let key = SecretKey::generate();
    // The relay admits only registered devices, so register first, then start the device.
    allow_device(
        &rig.list,
        Registered {
            id: key.public(),
            relay: None,
        },
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let (acceptor, connector) = tls_pair(&name_of(&key.public()), &[]);
    let late = start_device(key, &rig.relay_url, rig.edge_id, Behavior::Http, acceptor).await;
    let (response, _) = https_get(rig.addr(), &connector, &late.name()).await;
    assert!(response.ends_with(BODY));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_lets_encrypt_relay_starts_offline_and_binds_loopback_only() {
    // No certificate can be issued here (there is no public name), but the wiring from the config
    // to a listening relay must work: it binds loopback and names the https URL devices use.
    let dir = tempfile::tempdir().unwrap();
    let registry =
        std::sync::Arc::new(hq_edge::AllowlistFile::open(dir.path().join("devices.txt")).unwrap());
    let relay = hq_edge::RelayHost::spawn(
        hq_edge::RelayParams {
            hostname: RELAY_NAME.into(),
            tls: hq_edge::RelayTls::LetsEncrypt {
                email: "owner@example.com".into(),
                production: false,
                cache_dir: dir.path().join("acme"),
            },
            quic_bind: None,
        },
        registry,
        SecretKey::generate().public(),
    )
    .await
    .unwrap();
    assert!(relay.target().ip().is_loopback());
    assert_eq!(relay.url().to_string(), format!("https://{RELAY_NAME}/"));
    assert!(TcpStream::connect(relay.target()).await.is_ok());
}
