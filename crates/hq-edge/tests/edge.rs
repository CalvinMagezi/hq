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
use tokio_rustls::TlsConnector;

const SECOND: Duration = Duration::from_secs(1);

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
async fn a_held_connection_that_stops_answering_is_replaced_within_one_dial_budget() {
    let (rig, device, connector) = http_world(&[]).await;
    let (first, _) = https_get(rig.addr(), &connector, &device.name()).await;
    assert!(first.ends_with(BODY));
    assert_eq!(device.connections.lock().unwrap().len(), 1);

    // The device's network changes: the connection the edge holds is still open, but nothing on
    // it is answered any more. A new connection from the edge works.
    device
        .silent_upto
        .store(1, std::sync::atomic::Ordering::SeqCst);
    let started = std::time::Instant::now();
    let (second, _) = https_get(rig.addr(), &connector, &device.name()).await;
    let took = started.elapsed();
    assert!(second.ends_with(BODY), "got {second:?}");
    assert!(
        took < Duration::from_secs(8),
        "the stale connection cost {took:?}: the redial must fit in the 10 s dial budget"
    );
    assert_eq!(rig.edge.stats().redialed, 1);
    assert_eq!(device.connections.lock().unwrap().len(), 2);

    // The edge keeps using the new connection: no more waiting, no more redials.
    let (third, _) = https_get(rig.addr(), &connector, &device.name()).await;
    assert!(third.ends_with(BODY));
    assert_eq!(rig.edge.stats().redialed, 1);
    assert_eq!(device.connections.lock().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_that_goes_down_costs_one_redial_and_then_the_dial_budget_not_a_loop() {
    let (rig, device, connector) = http_world(&[]).await;
    let (first, _) = https_get(rig.addr(), &connector, &device.name()).await;
    assert!(first.ends_with(BODY));
    // Every connection, old and new, stops answering: the device is down but its endpoint is up.
    device
        .silent_upto
        .store(u64::MAX, std::sync::atomic::Ordering::SeqCst);
    let mut tcp = rig.connect().await;
    tcp.write_all(&client_hello(&device.name())).await.unwrap();
    let (got, took) = read_until_closed(&mut tcp, 14 * SECOND).await;
    assert!(got.is_empty());
    assert!(
        took < Duration::from_secs(12),
        "given up at {took:?}, expected within the 10 s dial budget"
    );
    let stats = rig.edge.stats();
    assert_eq!(stats.redialed, 1, "{stats:?}");
    assert_eq!(stats.dial_failed, 1, "{stats:?}");
    // One held connection retried once on one fresh connection: two connections, two streams
    // (the successful first request plus this one's retry makes three).
    assert_eq!(device.connections.lock().unwrap().len(), 2);
    assert_eq!(device.streams(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_that_refuses_a_client_does_not_cost_the_connection_the_edge_holds() {
    let (rig, device, hello) = raw_world(Behavior::Refuse, |_| {}).await;
    for _ in 0..3 {
        let mut tcp = rig.connect().await;
        tcp.write_all(&hello).await.unwrap();
        let (got, took) = read_until_closed(&mut tcp, 6 * SECOND).await;
        assert!(got.is_empty());
        assert!(took < Duration::from_secs(3), "refusal took {took:?}");
    }
    // A refusal is an answer: the path is fine, so nothing was redialed or replaced.
    assert_eq!(rig.edge.stats().redialed, 0);
    assert_eq!(device.connections.lock().unwrap().len(), 1);
    assert_eq!(device.streams(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_device_also_cuts_a_connection_the_edge_had_put_aside() {
    let (rig, device, hello) = raw_world(Behavior::Greet, |_| {}).await;
    // Client A is greeted and stays open on the first connection.
    let mut a = rig.connect().await;
    a.write_all(&hello).await.unwrap();
    let mut byte = [0u8; 1];
    tokio::time::timeout(10 * SECOND, a.read_exact(&mut byte))
        .await
        .expect("the greeting")
        .unwrap();

    // The first connection stops answering new streams. Client B finds that out, the edge puts
    // the connection aside and serves B on a fresh one. A is still running on the old one.
    device
        .silent_upto
        .store(1, std::sync::atomic::Ordering::SeqCst);
    let mut b = rig.connect().await;
    b.write_all(&hello).await.unwrap();
    tokio::time::timeout(10 * SECOND, b.read_exact(&mut byte))
        .await
        .expect("B's greeting on the fresh connection")
        .unwrap();
    assert_eq!(rig.edge.stats().redialed, 1);
    assert!(
        stays_open(&mut a, Duration::from_millis(500)).await,
        "putting the old connection aside must not cut the clients still on it"
    );

    assert!(revoke_device(&rig.list, &device.id()).unwrap());
    let (_, took_a) = read_until_closed(&mut a, 8 * SECOND).await;
    let (_, took_b) = read_until_closed(&mut b, 8 * SECOND).await;
    assert!(
        took_a < Duration::from_secs(5),
        "A was cut after {took_a:?}"
    );
    assert!(
        took_b < Duration::from_secs(5),
        "B was cut after {took_b:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_revoked_while_a_client_waits_on_its_stale_connection_is_not_dialed_again() {
    let (rig, device, hello) = raw_world(Behavior::Greet, |_| {}).await;
    let mut a = rig.connect().await;
    a.write_all(&hello).await.unwrap();
    let mut byte = [0u8; 1];
    tokio::time::timeout(10 * SECOND, a.read_exact(&mut byte))
        .await
        .expect("the greeting")
        .unwrap();
    device
        .silent_upto
        .store(1, std::sync::atomic::Ordering::SeqCst);

    // B waits on the held connection, which is silent; the device is revoked meanwhile.
    let mut b = rig.connect().await;
    b.write_all(&hello).await.unwrap();
    eventually(
        5 * SECOND,
        "B's hello to reach the stale connection",
        || device.streams() == 2,
    )
    .await;
    assert!(revoke_device(&rig.list, &device.id()).unwrap());
    let (got, took) = read_until_closed(&mut b, 8 * SECOND).await;
    assert!(got.is_empty());
    assert!(took < Duration::from_secs(5), "B was held for {took:?}");
    // No fresh connection was dialed for a device that is no longer registered.
    assert_eq!(device.connections.lock().unwrap().len(), 1);
    assert_eq!(rig.edge.stats().redialed, 0);
    let _ = a;
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
    ];
    for (what, bytes) in &cases {
        let mut tcp = rig.connect().await;
        tcp.write_all(bytes).await.unwrap();
        // The socket stays open on our side: only the edge can end this.
        let Some((got, took)) = try_read_until_closed(&mut tcp, 3 * SECOND).await else {
            panic!("{what}: still open after 3 s");
        };
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
        took >= Duration::from_millis(800) && took < Duration::from_millis(3500),
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
        took < Duration::from_millis(3500),
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
    // The edge frees the slot before it counts the closed client, so the count says it is free.
    eventually(5 * SECOND, "the edge to notice the closed client", || {
        rig.edge.stats().hello_bad == 1
    })
    .await;
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
    let (rig, device, hello) = raw_world(Behavior::Greet, |c| c.limits.idle_timeout_secs = 1).await;
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
        took < Duration::from_millis(5000),
        "gave up late at {took:?}"
    );
    assert_eq!(rig.edge.stats().dial_failed, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_device_cuts_its_open_connections_and_refuses_new_ones() {
    let (rig, device, hello) = raw_world(Behavior::Greet, |_| {}).await;
    let mut tcp = rig.connect().await;
    tcp.write_all(&hello).await.unwrap();
    let mut greeting = [0u8; 1];
    tokio::time::timeout(10 * SECOND, tcp.read_exact(&mut greeting))
        .await
        .expect("the device's greeting")
        .unwrap();
    assert!(stays_open(&mut tcp, Duration::from_millis(500)).await);

    assert!(revoke_device(&rig.list, &device.id()).unwrap());
    let (_, took) = read_until_closed(&mut tcp, 6 * SECOND).await;
    assert!(
        took < Duration::from_millis(4500),
        "revocation took {took:?}"
    );
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

/// A listener that stands in for the relay: it counts connections and the bytes they send, and
/// answers each with a fixed greeting.
struct FakeRelay {
    addr: SocketAddr,
    accepted: std::sync::Arc<std::sync::atomic::AtomicU64>,
    received: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
}

async fn fake_relay() -> FakeRelay {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = FakeRelay {
        addr: listener.local_addr().unwrap(),
        accepted: Arc::new(AtomicU64::new(0)),
        received: Arc::new(Mutex::new(Vec::new())),
    };
    let (accepted, received) = (relay.accepted.clone(), relay.received.clone());
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((mut tcp, _)) = listener.accept().await {
            accepted.fetch_add(1, Ordering::SeqCst);
            let _ = tcp.write_all(b"relay says hi").await;
            let received = received.clone();
            let (mut read, write) = tcp.into_split();
            held.push(write);
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                while let Ok(n) = read.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    received.lock().unwrap().extend_from_slice(&buf[..n]);
                }
            });
        }
    });
    relay
}

fn stand_in(relay: &FakeRelay) -> hq_edge::RelayHost {
    hq_edge::RelayHost::at(
        RELAY_NAME,
        relay.addr,
        "http://127.0.0.1:1".parse().unwrap(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn the_relay_name_is_routed_to_the_relay_with_the_hello_intact() {
    let fake = fake_relay().await;
    let rig = rig_with(&[], Some(stand_in(&fake)), |_| {}).await;

    let mut tcp = rig.connect().await;
    let mut bytes = client_hello(RELAY_NAME);
    let hello_len = bytes.len();
    bytes.extend_from_slice(b"and more");
    tcp.write_all(&bytes).await.unwrap();
    let mut greeting = [0u8; 13];
    tokio::time::timeout(5 * SECOND, tcp.read_exact(&mut greeting))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&greeting, b"relay says hi");
    eventually(5 * SECOND, "the relay to receive the bytes", || {
        fake.received.lock().unwrap().len() == bytes.len()
    })
    .await;
    assert_eq!(
        *fake.received.lock().unwrap(),
        bytes,
        "the relay must see exactly what was sent"
    );
    assert!(bytes.len() > hello_len);
    assert_eq!(rig.edge.stats().relay_forwarded, 1);
    assert_eq!(rig.edge.stats().unknown_name, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_relay_connection_cap_closes_the_extra_connection() {
    const CAP: usize = 2;
    let fake = fake_relay().await;
    let rig = rig_with(&[], Some(stand_in(&fake)), |c| {
        c.limits.max_relay_connections = CAP;
        c.limits.max_pending_per_source = 100;
    })
    .await;
    let hello = client_hello(RELAY_NAME);
    let mut held = Vec::new();
    for _ in 0..CAP {
        let mut tcp = rig.connect().await;
        tcp.write_all(&hello).await.unwrap();
        held.push(tcp);
    }
    eventually(5 * SECOND, "both relay connections", || {
        fake.accepted.load(std::sync::atomic::Ordering::SeqCst) == CAP as u64
    })
    .await;
    let mut extra = rig.connect().await;
    extra.write_all(&hello).await.unwrap();
    let (got, took) = read_until_closed(&mut extra, 3 * SECOND).await;
    assert!(got.is_empty());
    assert!(took < SECOND, "held for {took:?}");
    assert_eq!(rig.edge.stats().refused_busy, 1);
    assert_eq!(
        fake.accepted.load(std::sync::atomic::Ordering::SeqCst),
        CAP as u64
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn bytes_sent_with_the_hello_reach_the_device_with_it() {
    let (rig, device, hello) = raw_world(Behavior::Sink, |_| {}).await;
    let mut tcp = rig.connect().await;
    let mut bytes = hello.clone();
    bytes.extend_from_slice(&[9u8; 100]);
    tcp.write_all(&bytes).await.unwrap();
    eventually(10 * SECOND, "all bytes to arrive", || {
        device.sunk() == bytes.len() as u64
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_hello_is_charged_to_the_devices_byte_budget() {
    // 20 connections, each with a hello of a few hundred bytes, against 4 KB/s and a 2 KB burst:
    // without the charge every hello arrives at once.
    const CONNECTIONS: usize = 20;
    let (rig, device, hello) = raw_world(Behavior::Sink, |c| {
        c.limits.bytes_per_second = 4_000;
        c.limits.burst_bytes = 2_000;
        c.limits.max_connections_per_device = 64;
    })
    .await;
    // One hello first, so the first dial (the slow part on a cold start) is not in the timing, and
    // a pause for the bucket to refill to its burst.
    let mut warm = rig.connect().await;
    warm.write_all(&hello).await.unwrap();
    eventually(20 * SECOND, "the first hello to arrive", || {
        device.sunk() == hello.len() as u64
    })
    .await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    let before = device.sunk();
    let started = std::time::Instant::now();
    let mut open = Vec::new();
    for _ in 0..CONNECTIONS {
        let mut tcp = rig.connect().await;
        tcp.write_all(&hello).await.unwrap();
        open.push(tcp);
    }
    let total = before + (CONNECTIONS * hello.len()) as u64;
    eventually(30 * SECOND, "every hello to arrive", || {
        device.sunk() == total
    })
    .await;
    let total = (CONNECTIONS * hello.len()) as u64;
    let floor = Duration::from_secs_f64((total as f64 - 2_000.0) / 4_000.0 * 0.8);
    assert!(
        started.elapsed() >= floor,
        "{CONNECTIONS} hellos took {:?}, the budget needs at least {floor:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_removed_device_loses_its_edge_connection_and_its_relay_session() {
    let (rig, device, connector) = http_world(&[]).await;
    let relay = rig.edge.relay().unwrap();
    let (response, _) = https_get(rig.addr(), &connector, &device.name()).await;
    assert!(response.ends_with(BODY));
    assert!(
        relay.is_connected(&device.id()),
        "the device should be on the relay"
    );
    assert_eq!(rig.edge.stats().revoked_closed, 0);

    revoke_device(&rig.list, &device.id()).unwrap();
    eventually(
        8 * SECOND,
        "the edge to close its connection to the device",
        || rig.edge.stats().revoked_closed >= 1,
    )
    .await;
    eventually(8 * SECOND, "the relay to cut the device's session", || {
        !relay.is_connected(&device.id())
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_cannot_open_streams_toward_the_edge() {
    let (rig, device, connector) = http_world(&[]).await;
    let (response, _) = https_get(rig.addr(), &connector, &device.name()).await;
    assert!(response.ends_with(BODY));
    let conn = device
        .connections
        .lock()
        .unwrap()
        .first()
        .cloned()
        .expect("the edge connected");
    let opened = tokio::time::timeout(2 * SECOND, conn.open_bi()).await;
    assert!(
        opened.is_err(),
        "the edge granted a device stream credit it never uses"
    );
    drop(rig);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_list_the_edge_cannot_read_refuses_new_connections_but_does_not_cut_open_ones() {
    let (rig, device, connector) = http_world(&[]).await;
    let (response, _) = https_get(rig.addr(), &connector, &device.name()).await;
    assert!(response.ends_with(BODY));
    assert!(rig.edge.relay().unwrap().is_connected(&device.id()));

    // An edit leaves a line that is not an endpoint id: the whole list is refused.
    let good = std::fs::read_to_string(&rig.list).unwrap();
    std::fs::write(&rig.list, format!("{good}not-an-id\n")).unwrap();
    tokio::time::sleep(Duration::from_millis(3500)).await;
    let mut fresh = rig.connect().await;
    fresh
        .write_all(&client_hello(&device.name()))
        .await
        .unwrap();
    let (got, _) = read_until_closed(&mut fresh, 3 * SECOND).await;
    assert!(
        got.is_empty(),
        "a refused list must not forward new connections"
    );
    assert_eq!(rig.edge.stats().unknown_name, 1);
    assert_eq!(
        rig.edge.stats().revoked_closed,
        0,
        "an unreadable list is not a revocation"
    );
    assert!(
        rig.edge.relay().unwrap().is_connected(&device.id()),
        "the device lost its relay session because the list was unreadable"
    );

    // Fixed, and now without the device: that is a revocation.
    std::fs::write(&rig.list, "").unwrap();
    eventually(8 * SECOND, "the removal to take effect", || {
        rig.edge.stats().revoked_closed >= 1
    })
    .await;
}
