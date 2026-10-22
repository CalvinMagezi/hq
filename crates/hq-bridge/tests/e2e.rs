//! Pairing end to end on loopback: real iroh endpoints, no relay, the real gate, the real migrated
//! database. A second loopback address (`::1`) stands in for a second source.

use hq_bridge::{
    Bridge, BridgeConfig, JoinOptions, JoinResult, OpenTicket, PairingSpec, Purpose, Registrar,
    SERVICE_ALPN, Ticket, Tuning, code, echo, echo_handler,
    host::Joiner,
    limits::LimitConfig,
    wire::{self, Final, Hello, Outcome, Reply, Reveal},
};
use hq_db::{Database, bridge as peers};
use iroh::{
    Endpoint, EndpointAddr, RelayMode, SecretKey,
    endpoint::{Connection, presets},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::oneshot;

const V4: &str = "127.0.0.1:0";
const V6: &str = "[::1]:0";
const GIVE_UP: Duration = Duration::from_secs(20);
const GONE: Duration = Duration::from_secs(5);

async fn with_timeout<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(GIVE_UP, f)
        .await
        .expect("took too long")
}

struct Machine {
    bridge: Bridge,
    db: Database,
    _dir: tempfile::TempDir,
}

impl Machine {
    async fn new(name: &str, binds: &[&str], serve: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open_memory().unwrap();
        let bridge = Bridge::start(
            BridgeConfig {
                key: dir.path().join("bridge.key"),
                bind: binds.iter().map(|b| b.parse().unwrap()).collect(),
                relay: RelayMode::Disabled,
                name: name.into(),
                with_addrs: true,
                advertise: vec![],
            },
            db.clone(),
            serve.then(echo_handler),
        )
        .await
        .unwrap();
        Self {
            bridge,
            db,
            _dir: dir,
        }
    }

    /// A machine that listens on both loopback families, like the one showing a QR code.
    async fn host() -> Self {
        Self::new("desk", &[V4, V6], true).await
    }

    /// A machine on IPv4 loopback only.
    async fn phone() -> Self {
        Self::new("phone", &[V4], true).await
    }

    fn id(&self) -> String {
        self.bridge.id().to_string()
    }

    fn peers(&self) -> Vec<String> {
        peers::list(&self.db, false)
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect()
    }
}

fn tuning() -> Tuning {
    Tuning {
        ttl: Duration::from_secs(60),
        ..Tuning::default()
    }
}

/// Answers the first pairing request the way the person at the host would, after typing the digits
/// they read off the other device. Returns the ticket back and the digits the host expected.
fn answer_with(
    mut open: OpenTicket,
    accept: bool,
) -> tokio::task::JoinHandle<(OpenTicket, Option<String>)> {
    tokio::spawn(async move {
        let mut expected = None;
        if let Some(request) = open.requests.recv().await {
            expected = Some(request.sas.clone());
            let _ = request.decide.send(accept);
        }
        (open, expected)
    })
}

async fn join(machine: &Machine, ticket: &Ticket) -> anyhow::Result<JoinResult> {
    with_timeout(machine.bridge.join(ticket, &tuning(), |_| {})).await
}

fn paired(result: &anyhow::Result<JoinResult>) -> bool {
    matches!(result, Ok(JoinResult::Paired(_)))
}

fn address_of(machine: &Machine) -> EndpointAddr {
    let card = machine.bridge.card();
    let mut addr = EndpointAddr::new(machine.bridge.id());
    for a in &card.addrs {
        addr = addr.with_ip_addr(a.parse().unwrap());
    }
    addr
}

async fn raw_client(bind: &str) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .secret_key(SecretKey::generate())
        .relay_mode(RelayMode::Disabled)
        .clear_ip_transports()
        .bind_addr(bind.parse::<SocketAddr>().unwrap())
        .unwrap()
        .bind()
        .await
        .unwrap()
}

fn ticket_addr(ticket: &Ticket) -> EndpointAddr {
    let mut addr = EndpointAddr::new(ticket.endpoint_id().unwrap());
    for a in &ticket.addrs {
        addr = addr.with_ip_addr(a.parse().unwrap());
    }
    addr
}

/// Whether a dial ends in a usable connection within a short time.
async fn dials(client: &Endpoint, addr: EndpointAddr, alpn: &[u8]) -> bool {
    match tokio::time::timeout(GONE, client.connect(addr, alpn)).await {
        Ok(Ok(conn)) => echo(&conn, b"hello").await.is_ok(),
        _ => false,
    }
}

/// Opens a pairing connection by hand, as a hostile or stalled client would, and sends a hello.
struct RawPair {
    conn: Connection,
    send: iroh::endpoint::SendStream,
    recv: iroh::endpoint::RecvStream,
    nonce: code::Nonce,
}

async fn raw_pair(client: &Endpoint, ticket: &Ticket, code_text: &str, version: u16) -> RawPair {
    let alpn = ticket.k.alpn(3);
    let conn = with_timeout(client.connect(ticket_addr(ticket), &alpn))
        .await
        .unwrap();
    let (mut send, recv) = conn.open_bi().await.unwrap();
    let nonce = code::new_nonce();
    wire::send_line(
        &mut send,
        &Hello {
            v: version,
            code: code_text.into(),
            commit: wire::b32(&code::commit(&alpn, &nonce)),
        },
    )
    .await
    .unwrap();
    RawPair {
        conn,
        send,
        recv,
        nonce,
    }
}

async fn reply_of(raw: &mut RawPair) -> Reply {
    serde_json::from_str(&with_timeout(wire::read_line(&mut raw.recv)).await.unwrap()).unwrap()
}

fn wrong_code() -> String {
    wire::b32(&[7u8; code::CODE_BYTES])
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scanned_link_pairs_and_then_the_key_alone_reconnects_both_ways() {
    let (host, phone) = (Machine::host().await, Machine::phone().await);
    let open = host.bridge.open_ticket(tuning()).await.unwrap();
    let link = open.ticket.link();
    let ticket = Ticket::parse(&link).unwrap();
    assert!(
        !ticket.addrs.is_empty(),
        "the ticket tells the joiner where to dial"
    );
    assert_ne!(
        ticket.id,
        host.id(),
        "the ticket names a throwaway endpoint, not the long-term identity"
    );
    assert!(
        !link.contains(&host.id()),
        "the long-term id is nowhere in the link"
    );
    let answering = answer_with(open, true);

    let mut shown = String::new();
    let result = with_timeout(
        phone
            .bridge
            .join(&ticket, &tuning(), |sas| shown = sas.into()),
    )
    .await
    .unwrap();
    let JoinResult::Paired(joined) = result else {
        panic!("{result:?}")
    };
    assert_eq!(joined.sas, shown);
    assert_eq!(joined.sas.len(), 6);
    assert_eq!(
        joined.card.id,
        host.id(),
        "the phone learned the long-term id, not the ticket's"
    );
    let (open, host_digits) = answering.await.unwrap();
    assert_eq!(host_digits.as_deref(), Some(joined.sas.as_str()));
    assert_eq!(host.peers(), [phone.id()]);
    assert_eq!(phone.peers(), [host.id()]);

    // No code, no ticket: just the keys. The phone dials the host, and the host dials the phone.
    let conn = with_timeout(phone.bridge.connect(&host.id()))
        .await
        .unwrap();
    with_timeout(echo(&conn, b"after pairing")).await.unwrap();
    let conn_back = with_timeout(host.bridge.connect(&phone.id()))
        .await
        .unwrap();
    with_timeout(echo(&conn_back, b"and back")).await.unwrap();
    assert_eq!(host.bridge.reached_service.load(Ordering::SeqCst), 1);
    assert_eq!(open.stats.refused.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_key_nobody_paired_is_closed_before_any_handler() {
    let host = Machine::host().await;
    let stranger = raw_client(V4).await;
    assert!(!dials(&stranger, address_of(&host), SERVICE_ALPN).await);
    assert_eq!(
        host.bridge.reached_service.load(Ordering::SeqCst),
        0,
        "closed at the gate: the handler never saw the stranger"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_long_term_endpoint_does_not_speak_the_pairing_protocol() {
    let host = Machine::host().await;
    let open = host.bridge.open_ticket(tuning()).await.unwrap();
    let client = raw_client(V4).await;
    // Even the right ticket's protocol name gets nothing from the long-term id.
    for purpose in [Purpose::Pair, Purpose::Register] {
        assert!(!dials(&client, address_of(&host), &purpose.alpn(3)).await);
    }
    assert_eq!(open.stats.reached.load(Ordering::SeqCst), 0);
    assert_eq!(host.bridge.reached_service.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_declined_device_is_not_paired_and_cannot_reconnect() {
    let (host, phone) = (Machine::host().await, Machine::phone().await);
    let open = host.bridge.open_ticket(tuning()).await.unwrap();
    let ticket = open.ticket.clone();
    let answering = answer_with(open, false);
    let result = join(&phone, &ticket).await;
    assert!(
        matches!(result, Ok(JoinResult::Declined { .. })),
        "{result:?}"
    );
    answering.await.unwrap();
    assert!(host.peers().is_empty());
    assert!(
        phone.peers().is_empty(),
        "nothing is stored for a host that did not accept"
    );
    assert!(
        with_timeout(phone.bridge.connect(&host.id()))
            .await
            .is_err(),
        "the phone has no record to dial"
    );
    // It kept the host's id and dials anyway.
    let conn = with_timeout(
        phone
            .bridge
            .endpoint()
            .connect(address_of(&host), SERVICE_ALPN),
    )
    .await
    .unwrap();
    assert!(echo(&conn, b"let me in").await.is_err());
    assert_eq!(host.bridge.reached_service.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ticket_works_once_and_its_endpoint_is_gone_afterwards() {
    let (host, phone, intruder) = (
        Machine::host().await,
        Machine::phone().await,
        Machine::phone().await,
    );
    let open = host.bridge.open_ticket(tuning()).await.unwrap();
    let ticket = open.ticket.clone();
    let answering = answer_with(open, true);
    assert!(paired(&join(&phone, &ticket).await));
    let (mut open, _) = answering.await.unwrap();
    with_timeout(open.closed()).await;
    assert!(!open.is_open());

    // A photograph of the screen: nothing answers on that endpoint any more.
    let second = tokio::time::timeout(GONE, intruder.bridge.join(&ticket, &tuning(), |_| {})).await;
    assert!(
        !matches!(second, Ok(Ok(JoinResult::Paired(_)))),
        "{second:?}"
    );
    assert_eq!(
        host.peers(),
        [phone.id()],
        "only the first device was paired"
    );
    assert_eq!(open.stats.reached.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_global_backstop_burns_the_ticket_for_the_real_device_too() {
    let (host, phone, guesser) = (
        Machine::host().await,
        Machine::phone().await,
        Machine::phone().await,
    );
    let open = host
        .bridge
        .open_ticket(Tuning {
            global_attempts: 2,
            limits: LimitConfig {
                ip_strikes: 100,
                ..LimitConfig::default()
            },
            ..tuning()
        })
        .await
        .unwrap();
    let real = open.ticket.clone();
    let mut wrong = real.clone();
    wrong.code = wrong_code();
    for _ in 0..2 {
        assert!(!paired(&join(&guesser, &wrong).await));
    }
    let late = tokio::time::timeout(GONE, phone.bridge.join(&real, &tuning(), |_| {})).await;
    assert!(!matches!(late, Ok(Ok(JoinResult::Paired(_)))), "{late:?}");
    assert!(host.peers().is_empty());
    assert_eq!(
        open.stats.reached.load(Ordering::SeqCst),
        2,
        "once burned, the gate closes"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn one_source_that_guesses_is_shut_out_while_another_source_still_pairs() {
    let host = Machine::host().await;
    let guesser = Machine::phone().await; // 127.0.0.1
    let friend = Machine::new("friend", &[V6], true).await; // ::1
    let open = host
        .bridge
        .open_ticket(Tuning {
            limits: LimitConfig {
                ip_strikes: 3,
                ..LimitConfig::default()
            },
            ..tuning()
        })
        .await
        .unwrap();
    let real = open.ticket.clone();
    let mut wrong = real.clone();
    wrong.code = wrong_code();
    for _ in 0..3 {
        assert!(!paired(&join(&guesser, &wrong).await));
    }
    // The guesser's address is shut out, even with the right code now.
    let blocked = tokio::time::timeout(GONE, guesser.bridge.join(&real, &tuning(), |_| {})).await;
    assert!(
        !matches!(blocked, Ok(Ok(JoinResult::Paired(_)))),
        "{blocked:?}"
    );
    assert_eq!(
        open.stats.reached.load(Ordering::SeqCst),
        3,
        "the fourth try never reached the handler"
    );
    assert!(open.stats.refused.load(Ordering::SeqCst) >= 1);

    assert!(
        open.is_open(),
        "guessing from one address did not burn the ticket"
    );
    let answering = answer_with(open, true);
    assert!(
        paired(&join(&friend, &real).await),
        "a different source pairs"
    );
    answering.await.unwrap();
    assert_eq!(host.peers(), [friend.id()]);
}

#[tokio::test(flavor = "multi_thread")]
async fn one_source_cannot_fill_the_concurrency_cap() {
    let host = Machine::host().await;
    let friend = Machine::new("friend", &[V6], true).await;
    let open = host
        .bridge
        .open_ticket(Tuning {
            hello_timeout: Duration::from_secs(4),
            limits: LimitConfig {
                per_source_active: 1,
                max_active: 2,
                ..LimitConfig::default()
            },
            ..tuning()
        })
        .await
        .unwrap();
    let ticket = open.ticket.clone();
    let loiterer = raw_client(V4).await;
    let first = with_timeout(loiterer.connect(ticket_addr(&ticket), &ticket.k.alpn(3)))
        .await
        .unwrap();
    let second = with_timeout(loiterer.connect(ticket_addr(&ticket), &ticket.k.alpn(3)))
        .await
        .unwrap();
    // The second one is turned away at once, long before the hello timeout.
    tokio::time::timeout(Duration::from_secs(2), second.closed())
        .await
        .expect("a second slot for one source");
    assert!(
        first.close_reason().is_none(),
        "the first still holds its place"
    );

    let answering = answer_with(open, true);
    assert!(
        paired(&join(&friend, &ticket).await),
        "another source is not locked out"
    );
    answering.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pairing_connection_that_never_speaks_is_closed() {
    let host = Machine::host().await;
    let hello_timeout = Duration::from_secs(1);
    let open = host
        .bridge
        .open_ticket(Tuning {
            hello_timeout,
            ..tuning()
        })
        .await
        .unwrap();
    let client = raw_client(V4).await;
    let ticket = open.ticket.clone();
    let conn = with_timeout(client.connect(ticket_addr(&ticket), &ticket.k.alpn(3)))
        .await
        .unwrap();
    let started = std::time::Instant::now();
    with_timeout(conn.closed()).await;
    let waited = started.elapsed();
    assert!(
        waited >= hello_timeout && waited < hello_timeout * 5,
        "closed after {waited:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn taking_a_device_off_the_list_closes_the_connection_it_already_has_at_once() {
    let (host, phone) = (Machine::host().await, Machine::phone().await);
    let open = host.bridge.open_ticket(tuning()).await.unwrap();
    let ticket = open.ticket.clone();
    let answering = answer_with(open, true);
    assert!(paired(&join(&phone, &ticket).await));
    answering.await.unwrap();

    let conn = with_timeout(phone.bridge.connect(&host.id()))
        .await
        .unwrap();
    with_timeout(echo(&conn, b"still welcome")).await.unwrap();
    // Nothing is sent from here on: only the host's revoke can end this.
    assert!(host.bridge.revoke(&phone.bridge.id()).await.unwrap());
    tokio::time::timeout(Duration::from_secs(2), conn.closed())
        .await
        .expect("a revoked device kept its connection");
    assert!(echo(&conn, b"after revoking").await.is_err());
    let again = with_timeout(phone.bridge.connect(&host.id())).await;
    let welcomed = match again {
        Ok(conn) => echo(&conn, b"again").await.is_ok(),
        Err(_) => false,
    };
    assert!(!welcomed, "a revoked device got in again");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_revoke_made_elsewhere_stops_the_next_request_and_the_next_handshake() {
    let (host, phone) = (Machine::host().await, Machine::phone().await);
    let open = host.bridge.open_ticket(tuning()).await.unwrap();
    let ticket = open.ticket.clone();
    let answering = answer_with(open, true);
    assert!(paired(&join(&phone, &ticket).await));
    answering.await.unwrap();

    let conn = with_timeout(phone.bridge.connect(&host.id()))
        .await
        .unwrap();
    with_timeout(echo(&conn, b"welcome")).await.unwrap();
    // `hq bridge revoke` from another process only changes the database.
    assert!(peers::revoke(&host.db, &phone.id()).unwrap());
    assert!(
        with_timeout(echo(&conn, b"after the revoke"))
            .await
            .is_err(),
        "a revoked device was served on its open connection"
    );
    // The phone still has the host on its own list, so it dials; the host's gate is what refuses.
    let welcomed = match with_timeout(phone.bridge.connect(&host.id())).await {
        Ok(conn) => echo(&conn, b"again").await.is_ok(),
        Err(_) => false,
    };
    assert!(!welcomed, "a revoked device got in again");
    assert_eq!(
        host.bridge.reached_service.load(Ordering::SeqCst),
        1,
        "only the first connection got past the gate"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_database_that_fails_closes_the_gate() {
    let (host, phone) = (Machine::host().await, Machine::phone().await);
    let open = host.bridge.open_ticket(tuning()).await.unwrap();
    let ticket = open.ticket.clone();
    let answering = answer_with(open, true);
    assert!(paired(&join(&phone, &ticket).await));
    answering.await.unwrap();
    let conn = with_timeout(phone.bridge.connect(&host.id()))
        .await
        .unwrap();
    with_timeout(echo(&conn, b"fine")).await.unwrap();

    host.db
        .with_conn(|c| Ok(c.execute("DROP TABLE bridge_peers", [])?))
        .unwrap();
    assert!(
        with_timeout(echo(&conn, b"no list")).await.is_err(),
        "an unreadable list let a request through"
    );
    let raw = with_timeout(
        phone
            .bridge
            .endpoint()
            .connect(address_of(&host), SERVICE_ALPN),
    )
    .await;
    let welcomed = match raw {
        Ok(conn) => echo(&conn, b"x").await.is_ok(),
        Err(_) => false,
    };
    assert!(!welcomed, "an unreadable list let a handshake through");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_late_answer_pairs_nobody() {
    let (host, phone) = (Machine::host().await, Machine::phone().await);
    let mut open = host
        .bridge
        .open_ticket(Tuning {
            decision_timeout: Duration::from_secs(1),
            ..tuning()
        })
        .await
        .unwrap();
    let ticket = open.ticket.clone();
    let slow_person = tokio::spawn(async move {
        let request = open.requests.recv().await.unwrap();
        tokio::time::sleep(Duration::from_millis(1_800)).await;
        let late = request.decide.send(true).is_err();
        (open, late)
    });
    let result = join(&phone, &ticket).await;
    assert!(
        matches!(result, Ok(JoinResult::Declined { .. })),
        "{result:?}"
    );
    let (_open, was_late) = with_timeout(slow_person).await.unwrap();
    assert!(was_late, "the host was told its answer came too late");
    assert!(host.peers().is_empty(), "a late yes must not pair");
    assert!(phone.peers().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn two_devices_racing_for_one_ticket_pair_exactly_one() {
    let (host, one, two) = (
        Machine::host().await,
        Machine::phone().await,
        Machine::phone().await,
    );
    let open = host.bridge.open_ticket(tuning()).await.unwrap();
    let ticket = open.ticket.clone();
    let answering = answer_with(open, true);
    let (a, b) = tokio::join!(join(&one, &ticket), join(&two, &ticket));
    let count = [&a, &b].iter().filter(|r| paired(r)).count();
    assert_eq!(count, 1, "{a:?} / {b:?}");
    answering.await.unwrap();
    assert_eq!(host.peers().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_code_learns_nothing_about_the_long_term_identity() {
    let host = Machine::host().await;
    let open = host.bridge.open_ticket(tuning()).await.unwrap();
    let client = raw_client(V4).await;
    let mut raw = raw_pair(&client, &open.ticket, &wrong_code(), 3).await;
    let reply = reply_of(&mut raw).await;
    assert!(!reply.ok);
    assert!(reply.nonce.is_empty() && reply.bind.is_empty());
    assert!(!reply.why.contains(&host.id()));
    assert!(open.is_open(), "one wrong guess does not burn the ticket");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unsupported_version_is_refused_without_costing_an_attempt() {
    let (host, phone) = (Machine::host().await, Machine::phone().await);
    // One attempt only: if the bad version counted as a guess, the ticket would be burned.
    let open = host
        .bridge
        .open_ticket(Tuning {
            global_attempts: 1,
            ..tuning()
        })
        .await
        .unwrap();
    let ticket = open.ticket.clone();
    let client = raw_client(V4).await;
    let mut raw = raw_pair(&client, &ticket, &ticket.code, 99).await;
    let reply = reply_of(&mut raw).await;
    assert!(!reply.ok);
    assert!(reply.why.contains("version"), "{}", reply.why);
    assert!(open.is_open());

    // A ticket from a host that speaks only versions this build does not is refused before dialing.
    let mut future = ticket.clone();
    future.proto = vec![42];
    let err = join(&phone, &future).await.unwrap_err().to_string();
    assert!(err.contains("newer") || err.contains("older"), "{err}");
    assert_eq!(
        open.stats.reached.load(Ordering::SeqCst),
        1,
        "the refusal needed no dial"
    );

    let answering = answer_with(open, true);
    assert!(paired(&join(&phone, &ticket).await));
    answering.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reveal_that_does_not_match_the_commitment_never_reaches_the_person() {
    let host = Machine::host().await;
    let mut open = host.bridge.open_ticket(tuning()).await.unwrap();
    let client = raw_client(V4).await;
    let ticket = open.ticket.clone();
    let mut raw = raw_pair(&client, &ticket, &ticket.code, 3).await;
    assert!(reply_of(&mut raw).await.ok);
    let other = code::new_nonce();
    assert_ne!(other, raw.nonce);
    wire::send_line(
        &mut raw.send,
        &Reveal {
            nonce: wire::b32(&other),
            ..Reveal::default()
        },
    )
    .await
    .unwrap();
    with_timeout(raw.conn.closed()).await;
    assert!(
        open.requests.try_recv().is_err(),
        "the person was asked about a cheating device"
    );
    assert!(host.peers().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_name_with_a_terminal_escape_reaches_the_person_clean() {
    let (host, phone) = (
        Machine::host().await,
        Machine::new("\u{1b}[2Jevil\u{7}", &[V4], true).await,
    );
    let mut open = host.bridge.open_ticket(tuning()).await.unwrap();
    let ticket = open.ticket.clone();
    let asked = tokio::spawn(async move {
        let request = open.requests.recv().await.unwrap();
        let name = request.name.clone();
        let _ = request.decide.send(true);
        (open, name)
    });
    assert!(paired(&join(&phone, &ticket).await));
    let (_open, name) = asked.await.unwrap();
    assert!(!name.chars().any(char::is_control), "{name:?}");
    assert!(name.contains("evil"));
}

struct Recording {
    seen: Mutex<Vec<String>>,
    calls: AtomicUsize,
}

impl Registrar for Recording {
    fn accept(&self, joiner: &Joiner) -> anyhow::Result<Option<serde_json::Value>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(joiner.id.to_string());
        Ok(Some(
            serde_json::json!({ "hostname": "device-1.edge.example.com" }),
        ))
    }
}

async fn open_register(host: &Machine, registrar: Arc<Recording>) -> OpenTicket {
    OpenTicket::open(PairingSpec {
        purpose: Purpose::Register,
        card: host.bridge.card(),
        registrar,
        bind: vec![V4.parse().unwrap()],
        relay: RelayMode::Disabled,
        with_addrs: true,
        advertise: vec![],
        tuning: tuning(),
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_registers_with_an_edge_and_the_two_purposes_never_cross() {
    let (edge, phone) = (Machine::host().await, Machine::phone().await);
    let registrar = Arc::new(Recording {
        seen: Mutex::default(),
        calls: AtomicUsize::new(0),
    });
    let mut open = open_register(&edge, registrar.clone()).await;
    let ticket = open.ticket.clone();
    assert_eq!(ticket.k, Purpose::Register);
    let confirming = tokio::spawn(async move {
        let request = open.requests.recv().await.unwrap();
        let _ = request.decide.send(true);
        open
    });
    let opts = JoinOptions::new("phone", vec![], vec![]);
    let result = with_timeout(hq_bridge::join::join(
        phone.bridge.endpoint(),
        &ticket,
        &opts,
        |_| {},
    ))
    .await
    .unwrap();
    let JoinResult::Paired(joined) = result else {
        panic!("{result:?}")
    };
    assert_eq!(
        joined.info.unwrap()["hostname"],
        "device-1.edge.example.com"
    );
    assert_eq!(joined.card.id, edge.id());
    assert_eq!(registrar.seen.lock().unwrap().as_slice(), [phone.id()]);
    let _open = confirming.await.unwrap();
    assert!(
        edge.peers().is_empty(),
        "registering is not pairing: no peer was added"
    );

    // A registration ticket relabelled as a pairing ticket finds no door, and the other way round.
    let registrar = Arc::new(Recording {
        seen: Mutex::default(),
        calls: AtomicUsize::new(0),
    });
    let open = open_register(&edge, registrar.clone()).await;
    let mut relabelled = open.ticket.clone();
    relabelled.k = Purpose::Pair;
    let tried = tokio::time::timeout(GONE, phone.bridge.join(&relabelled, &tuning(), |_| {})).await;
    assert!(!matches!(tried, Ok(Ok(JoinResult::Paired(_)))), "{tried:?}");
    assert_eq!(registrar.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        open.stats.reached.load(Ordering::SeqCst),
        0,
        "closed at the protocol name"
    );

    let pairing = edge.bridge.open_ticket(tuning()).await.unwrap();
    let mut relabelled = pairing.ticket.clone();
    relabelled.k = Purpose::Register;
    let tried = tokio::time::timeout(
        GONE,
        hq_bridge::join::join(phone.bridge.endpoint(), &relabelled, &opts, |_| {}),
    )
    .await;
    assert!(!matches!(tried, Ok(Ok(JoinResult::Paired(_)))), "{tried:?}");
    assert_eq!(pairing.stats.reached.load(Ordering::SeqCst), 0);
    let direct =
        tokio::time::timeout(GONE, phone.bridge.join(&open.ticket, &tuning(), |_| {})).await;
    assert!(
        direct.map_or(true, |r| r.is_err()),
        "Bridge::join refuses a registration ticket"
    );
}

/// A host that behaves until the digits are confirmed, then names a different long-term identity
/// than the one it promised.
#[derive(Clone, Debug)]
struct SwappingHost {
    promised: iroh::EndpointId,
    delivered: iroh::EndpointId,
    code: code::Code,
    seen: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

impl ProtocolHandler for SwappingHost {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let alpn = conn.alpn().to_vec();
        let (mut send, mut recv) = conn.accept_bi().await?;
        let hello: Hello =
            serde_json::from_str(&wire::read_line(&mut recv).await.unwrap()).unwrap();
        assert_eq!(wire::unb32(&hello.code), self.code);
        let nonce = code::new_nonce();
        let binding = code::bind_host(&self.promised, &nonce);
        wire::send_line(
            &mut send,
            &Reply {
                ok: true,
                nonce: wire::b32(&nonce),
                bind: wire::b32(&binding),
                ..Reply::default()
            },
        )
        .await
        .unwrap();
        let _reveal: Reveal =
            serde_json::from_str(&wire::read_line(&mut recv).await.unwrap()).unwrap();
        let card = wire::Card {
            id: self.delivered.to_string(),
            name: "x".into(),
            relays: vec![],
            addrs: vec![],
        };
        wire::send_line(
            &mut send,
            &Final {
                result: Outcome::Accepted,
                card: Some(card),
                info: None,
            },
        )
        .await
        .unwrap();
        send.finish().unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(2), conn.closed()).await;
        let _ = alpn;
        if let Some(tx) = self.seen.lock().unwrap().take() {
            let _ = tx.send(());
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_that_swaps_its_identity_after_the_digits_is_refused() {
    let phone = Machine::phone().await;
    let rogue = Endpoint::builder(presets::Minimal)
        .secret_key(SecretKey::generate())
        .relay_mode(RelayMode::Disabled)
        .clear_ip_transports()
        .bind_addr(V4.parse::<SocketAddr>().unwrap())
        .unwrap()
        .bind()
        .await
        .unwrap();
    let (promised, delivered) = (
        SecretKey::generate().public(),
        SecretKey::generate().public(),
    );
    let code = code::new_code();
    let (tx, rx) = oneshot::channel();
    let _router = Router::builder(rogue.clone())
        .accept(
            Purpose::Pair.alpn(3),
            SwappingHost {
                promised,
                delivered,
                code,
                seen: Arc::new(Mutex::new(Some(tx))),
            },
        )
        .spawn();
    let ticket = Ticket::new(
        Purpose::Pair,
        rogue.id(),
        &code,
        vec![],
        rogue
            .bound_sockets()
            .iter()
            .map(|a| a.to_string())
            .collect(),
    );
    let result = join(&phone, &ticket).await;
    let _ = rx.await;
    let err = result.unwrap_err().to_string();
    assert!(err.contains("promised"), "{err}");
    assert!(
        phone.peers().is_empty(),
        "a host with a swapped identity was stored"
    );
}
