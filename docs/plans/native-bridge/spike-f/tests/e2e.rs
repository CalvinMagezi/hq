//! Pairing end to end on loopback: two real iroh endpoints, no relay, the real gate.

use iroh::RelayMode;
use pairspike::{
    net::{self, Host, HostConfig, JoinConfig, JoinResult, Stored},
    pairing::Ticket,
};
use std::{path::PathBuf, sync::atomic::Ordering, time::Duration};

const LOOPBACK: &str = "127.0.0.1:0";
const GIVE_UP: Duration = Duration::from_secs(20);

struct Dirs {
    root: tempfile::TempDir,
}

impl Dirs {
    fn new() -> Self {
        Self {
            root: tempfile::tempdir().unwrap(),
        }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }
}

async fn host(dirs: &Dirs, ttl: Duration, attempts: u32) -> Host {
    host_with(
        dirs,
        ttl,
        attempts,
        net::HELLO_TIMEOUT,
        net::DECISION_TIMEOUT,
    )
    .await
}

async fn host_with(
    dirs: &Dirs,
    ttl: Duration,
    attempts: u32,
    hello_timeout: Duration,
    decision_timeout: Duration,
) -> Host {
    Host::start(HostConfig {
        key: dirs.path("host.key"),
        paired: dirs.path("paired.txt"),
        bind: LOOPBACK.parse().unwrap(),
        relay: RelayMode::Disabled,
        ttl,
        attempts,
        advertise: vec![],
        with_addrs: true,
        keepalive: None,
        hello_timeout,
        decision_timeout,
    })
    .await
    .unwrap()
}

fn joiner(dirs: &Dirs, name: &str) -> JoinConfig {
    JoinConfig {
        key: dirs.path(&format!("{name}.key")),
        store: dirs.path(&format!("{name}.json")),
        bind: LOOPBACK.parse().unwrap(),
        relay: RelayMode::Disabled,
        keepalive: None,
    }
}

/// Answers the first pairing request the way the person at the host would, after typing the digits
/// they read off the other device. Returns the digits the host expected.
fn answer_with(mut host: Host, accept: bool) -> tokio::task::JoinHandle<(Host, Option<String>)> {
    tokio::spawn(async move {
        let mut expected = None;
        if let Some(request) = host.requests.recv().await {
            expected = Some(request.sas.clone());
            let _ = request.decide.send(accept);
        }
        (host, expected)
    })
}

fn paired_lines(dirs: &Dirs) -> usize {
    std::fs::read_to_string(dirs.path("paired.txt")).map_or(0, |t| t.lines().count())
}

async fn with_timeout<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(GIVE_UP, f)
        .await
        .expect("took too long")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scanned_ticket_pairs_and_then_the_key_alone_reconnects() {
    let dirs = Dirs::new();
    let host = host(&dirs, Duration::from_secs(60), 5).await;
    let ticket = Ticket::parse(&host.ticket.encode()).unwrap();
    assert!(
        !ticket.addrs.is_empty(),
        "the ticket tells the joiner where to dial"
    );
    let answering = answer_with(host, true);

    let mut shown = String::new();
    let result = with_timeout(net::join(&ticket, joiner(&dirs, "phone"), |sas| {
        shown = sas.to_string()
    }))
    .await
    .unwrap();
    let JoinResult::Paired { sas } = result else {
        panic!("{result:?}")
    };
    assert_eq!(sas, shown);
    assert_eq!(sas.len(), 6);
    let (host, host_digits) = answering.await.unwrap();
    assert_eq!(
        host_digits.as_deref(),
        Some(sas.as_str()),
        "the digits the host expects are the digits the joiner showed"
    );
    assert!(dirs.path("phone.json").exists());
    assert_eq!(paired_lines(&dirs), 1);

    // No code, no ticket: just the stored key and host id.
    let session = with_timeout(net::dial(&joiner(&dirs, "phone")))
        .await
        .unwrap();
    assert_eq!(net::path_kind(&session.conn), "direct");
    with_timeout(net::echo(&session.conn, b"after pairing"))
        .await
        .unwrap();
    drop(session);
    assert_eq!(host.refused.load(Ordering::SeqCst), 0);
    assert_eq!(host.reached_echo.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_key_nobody_paired_is_closed_before_any_handler() {
    let dirs = Dirs::new();
    let host = host(&dirs, Duration::from_secs(60), 5).await;
    let ticket = host.ticket.clone();
    let outcome = with_timeout(net::stranger(
        &ticket,
        LOOPBACK.parse().unwrap(),
        RelayMode::Disabled,
    ))
    .await
    .unwrap();
    assert!(
        outcome.contains("not paired") || outcome.contains("403"),
        "{outcome}"
    );
    assert!(host.refused.load(Ordering::SeqCst) >= 1);
    assert_eq!(
        host.reached_echo.load(Ordering::SeqCst),
        0,
        "closed at the gate: the handler never saw the stranger"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_declined_device_is_not_paired_and_cannot_reconnect() {
    let dirs = Dirs::new();
    let host = host(&dirs, Duration::from_secs(60), 5).await;
    let ticket = host.ticket.clone();
    let answering = answer_with(host, false);
    let result = with_timeout(net::join(&ticket, joiner(&dirs, "phone"), |_| {}))
        .await
        .unwrap();
    assert!(matches!(result, JoinResult::Declined { .. }), "{result:?}");
    let (host, _) = answering.await.unwrap();
    assert!(
        !dirs.path("phone.json").exists(),
        "nothing is stored for a device that was not accepted"
    );
    assert_eq!(paired_lines(&dirs), 0);

    // The device kept the host's id and tries to dial anyway.
    let stored = Stored {
        id: host.endpoint.id().to_string(),
        relays: vec![],
        addrs: ticket.addrs.clone(),
        paired_unix: 0,
    };
    std::fs::write(
        dirs.path("phone.json"),
        serde_json::to_vec(&stored).unwrap(),
    )
    .unwrap();
    let dialed = with_timeout(net::dial(&joiner(&dirs, "phone"))).await;
    assert!(dialed.is_err(), "a declined device got in");
    assert_eq!(host.reached_echo.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ticket_works_once() {
    let dirs = Dirs::new();
    let host = host(&dirs, Duration::from_secs(60), 5).await;
    let ticket = host.ticket.clone();
    let answering = answer_with(host, true);
    assert!(matches!(
        with_timeout(net::join(&ticket, joiner(&dirs, "phone"), |_| {}))
            .await
            .unwrap(),
        JoinResult::Paired { .. }
    ));
    let (host, _) = answering.await.unwrap();
    // A second device with the same ticket (a photograph of the screen) gets nothing, and is closed
    // at the gate: the handler never sees it.
    let second = with_timeout(net::join(&ticket, joiner(&dirs, "intruder"), |_| {})).await;
    assert!(
        !matches!(second, Ok(JoinResult::Paired { .. })),
        "{second:?}"
    );
    assert!(!dirs.path("intruder.json").exists());
    assert_eq!(paired_lines(&dirs), 1, "only the first device was paired");
    assert_eq!(host.reached_pair.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn guessing_the_code_burns_the_ticket_for_the_real_device_too() {
    let dirs = Dirs::new();
    let attempts = 3;
    let host = host(&dirs, Duration::from_secs(60), attempts).await;
    let real = host.ticket.clone();
    let mut wrong = real.clone();
    wrong.code = data_encoding::BASE32_NOPAD.encode(&[7u8; 8]);
    for n in 0..attempts {
        let result = with_timeout(net::join(
            &wrong,
            joiner(&dirs, &format!("guesser{n}")),
            |_| {},
        ))
        .await;
        assert!(!matches!(result, Ok(JoinResult::Paired { .. })));
    }
    let late = with_timeout(net::join(&real, joiner(&dirs, "phone"), |_| {})).await;
    assert!(!matches!(late, Ok(JoinResult::Paired { .. })), "{late:?}");
    assert!(!dirs.path("phone.json").exists());
    // Once burned, the gate closes pairing connections before the handler: only the guesses got in.
    assert_eq!(
        host.reached_pair.load(Ordering::SeqCst),
        u64::from(attempts)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_expired_ticket_pairs_nobody() {
    let dirs = Dirs::new();
    let host = host(&dirs, Duration::from_secs(1), 5).await;
    let ticket = host.ticket.clone();
    tokio::time::sleep(Duration::from_millis(1_300)).await;
    let late = with_timeout(net::join(&ticket, joiner(&dirs, "phone"), |_| {})).await;
    assert!(!matches!(late, Ok(JoinResult::Paired { .. })), "{late:?}");
    assert_eq!(
        host.reached_pair.load(Ordering::SeqCst),
        0,
        "closed at the gate"
    );
}

async fn paired_phone(dirs: &Dirs) -> Host {
    let host = host(dirs, Duration::from_secs(60), 5).await;
    let ticket = host.ticket.clone();
    let answering = answer_with(host, true);
    with_timeout(net::join(&ticket, joiner(dirs, "phone"), |_| {}))
        .await
        .unwrap();
    answering.await.unwrap().0
}

#[tokio::test(flavor = "multi_thread")]
async fn taking_a_device_off_the_list_cuts_it_off_at_the_next_connection() {
    let dirs = Dirs::new();
    let _host = paired_phone(&dirs).await;
    with_timeout(net::dial(&joiner(&dirs, "phone")))
        .await
        .unwrap();
    std::fs::write(dirs.path("paired.txt"), "").unwrap();
    let again = with_timeout(net::dial(&joiner(&dirs, "phone"))).await;
    assert!(again.is_err(), "a revoked device still got in");
}

#[tokio::test(flavor = "multi_thread")]
async fn taking_a_device_off_the_list_closes_the_connection_it_already_has() {
    let dirs = Dirs::new();
    let _host = paired_phone(&dirs).await;
    let session = with_timeout(net::dial(&joiner(&dirs, "phone")))
        .await
        .unwrap();
    with_timeout(net::echo(&session.conn, b"still welcome"))
        .await
        .unwrap();
    std::fs::write(dirs.path("paired.txt"), "").unwrap();
    with_timeout(session.conn.closed()).await;
    assert!(
        net::echo(&session.conn, b"after revoking").await.is_err(),
        "a revoked device kept talking on its open connection"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_late_answer_pairs_nobody() {
    let dirs = Dirs::new();
    let mut host = host_with(
        &dirs,
        Duration::from_secs(60),
        5,
        net::HELLO_TIMEOUT,
        Duration::from_secs(1),
    )
    .await;
    let ticket = host.ticket.clone();
    let slow_person = tokio::spawn(async move {
        let request = host.requests.recv().await.unwrap();
        tokio::time::sleep(Duration::from_millis(1_800)).await;
        request.decide.send(true).is_err()
    });
    let result = with_timeout(net::join(&ticket, joiner(&dirs, "phone"), |_| {}))
        .await
        .unwrap();
    assert!(matches!(result, JoinResult::Declined { .. }), "{result:?}");
    assert!(
        with_timeout(slow_person).await.unwrap(),
        "the host was told its answer came too late"
    );
    assert_eq!(paired_lines(&dirs), 0, "a late yes must not pair");
    assert!(!dirs.path("phone.json").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pairing_connection_that_never_speaks_is_closed() {
    let dirs = Dirs::new();
    let hello_timeout = Duration::from_secs(1);
    let host = host_with(
        &dirs,
        Duration::from_secs(60),
        5,
        hello_timeout,
        net::DECISION_TIMEOUT,
    )
    .await;
    let waited = with_timeout(net::lurk(
        &host.ticket,
        LOOPBACK.parse().unwrap(),
        RelayMode::Disabled,
    ))
    .await
    .unwrap();
    assert!(
        waited >= hello_timeout && waited < hello_timeout * 5,
        "closed after {waited:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn two_devices_racing_for_one_ticket_pair_exactly_one() {
    let dirs = Dirs::new();
    let host = host(&dirs, Duration::from_secs(60), 5).await;
    let ticket = host.ticket.clone();
    let answering = answer_with(host, true);
    let (a, b) = tokio::join!(
        with_timeout(net::join(&ticket, joiner(&dirs, "one"), |_| {})),
        with_timeout(net::join(&ticket, joiner(&dirs, "two"), |_| {})),
    );
    let paired = [&a, &b]
        .iter()
        .filter(|r| matches!(r, Ok(JoinResult::Paired { .. })))
        .count();
    assert_eq!(paired, 1, "{a:?} / {b:?}");
    answering.await.unwrap();
    assert_eq!(paired_lines(&dirs), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paired_list_others_can_write_or_that_is_a_link_is_not_trusted() {
    use std::os::unix::fs::PermissionsExt;
    let dirs = Dirs::new();
    let _host = paired_phone(&dirs).await;
    let list = dirs.path("paired.txt");
    with_timeout(net::dial(&joiner(&dirs, "phone")))
        .await
        .unwrap();

    std::fs::set_permissions(&list, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert!(
        with_timeout(net::dial(&joiner(&dirs, "phone")))
            .await
            .is_err(),
        "a world-writable list was trusted"
    );
    std::fs::set_permissions(&list, std::fs::Permissions::from_mode(0o600)).unwrap();

    let real = dirs.path("real.txt");
    std::fs::rename(&list, &real).unwrap();
    std::os::unix::fs::symlink(&real, &list).unwrap();
    assert!(
        with_timeout(net::dial(&joiner(&dirs, "phone")))
            .await
            .is_err(),
        "a list that is a symlink was trusted"
    );
    std::fs::remove_file(&list).unwrap();
    std::fs::rename(&real, &list).unwrap();
    with_timeout(net::dial(&joiner(&dirs, "phone")))
        .await
        .unwrap();
}
