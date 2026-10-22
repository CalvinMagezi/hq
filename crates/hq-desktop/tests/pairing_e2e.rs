//! The app's host against a joiner that does what `hq bridge join` does, through the Rust side:
//! real iroh endpoints on loopback with no relay, the real migrated schema, the real gate.

use hq_bridge::{Bridge, BridgeConfig, JoinResult, RelayMode, Ticket, Tuning, echo};
use hq_db::Database;
use hq_desktop::pairing::{
    MAX_DIGIT_TRIES, PairingConfig, PairingController, PairingSnapshot, PendingView, Submitted,
};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;

const GIVE_UP: Duration = Duration::from_secs(20);
const GONE: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(50);

async fn within<T>(what: &str, f: impl Future<Output = T>) -> T {
    tokio::time::timeout(GIVE_UP, f)
        .await
        .unwrap_or_else(|_| panic!("timed out: {what}"))
}

struct App {
    host: Arc<PairingController>,
    _dir: tempfile::TempDir,
}

fn app_with(tuning: Tuning) -> App {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    let host = PairingController::new(
        PairingConfig {
            key: dir.path().join("bridge.key"),
            bind: vec!["127.0.0.1:0".parse().unwrap()],
            relay: RelayMode::Disabled,
            name: "desk".into(),
            with_addrs: true,
            tuning,
        },
        move || Ok(db.clone()),
        None,
    );
    App {
        host: Arc::new(host),
        _dir: dir,
    }
}

fn app() -> App {
    app_with(Tuning {
        ttl: Duration::from_secs(120),
        ..Tuning::default()
    })
}

/// A device that joins the way `hq bridge join` does: its own bridge, the link, the digits shown
/// through a callback.
struct Joiner {
    bridge: Bridge,
    _dir: tempfile::TempDir,
}

impl Joiner {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let bridge = Bridge::start(
            BridgeConfig {
                key: dir.path().join("bridge.key"),
                bind: vec!["127.0.0.1:0".parse().unwrap()],
                relay: RelayMode::Disabled,
                name: "phone".into(),
                with_addrs: true,
                advertise: vec![],
            },
            Database::open_memory().unwrap(),
            None,
        )
        .await
        .unwrap();
        Self { bridge, _dir: dir }
    }

    /// Starts joining. Returns the digits as soon as the device shows them, and the outcome.
    fn join(
        self: &Arc<Self>,
        link: &str,
        tuning: Tuning,
    ) -> (
        oneshot::Receiver<String>,
        tokio::task::JoinHandle<anyhow::Result<JoinResult>>,
    ) {
        let ticket = Ticket::parse(link).unwrap();
        let (digits_tx, digits_rx) = oneshot::channel();
        let me = self.clone();
        let task = tokio::spawn(async move {
            let mut digits_tx = Some(digits_tx);
            me.bridge
                .join(&ticket, &tuning, move |sas| {
                    if let Some(tx) = digits_tx.take() {
                        let _ = tx.send(sas.to_string());
                    }
                })
                .await
        });
        (digits_rx, task)
    }
}

async fn wait_for<T, F, Fut>(what: &str, mut probe: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    within(what, async {
        loop {
            if let Some(found) = probe().await {
                return found;
            }
            tokio::time::sleep(POLL).await;
        }
    })
    .await
}

async fn first_request(host: &Arc<PairingController>) -> PendingView {
    wait_for("a request from the joining device", || async {
        host.snapshot().await.pending.first().cloned()
    })
    .await
}

fn tuning() -> Tuning {
    Tuning {
        ttl: Duration::from_secs(120),
        ..Tuning::default()
    }
}

#[tokio::test]
async fn a_device_pairs_when_the_digits_are_typed_connects_and_is_cut_by_a_revoke() {
    let app = app();
    let joiner = Arc::new(Joiner::new().await);
    let ticket = app.host.begin().await.unwrap();
    assert!(ticket.link.starts_with("hqpair://"));
    assert!(ticket.qr.is_some());

    let (digits, outcome) = joiner.join(&ticket.link, tuning());
    let request = first_request(&app.host).await;
    assert_eq!(request.name, "phone");
    assert_eq!(request.joiner_id, joiner.bridge.id().to_string());
    assert_eq!(request.tries_left, MAX_DIGIT_TRIES);

    let shown = within("the joiner's digits", digits).await.unwrap();
    assert_eq!(
        app.host.submit_digits(request.id, &shown),
        Submitted::Accepted
    );
    let joined = within("the join to finish", outcome)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(joined, JoinResult::Paired(_)));

    let paired = wait_for("the device in the paired list", || async {
        let snap = app.host.snapshot().await;
        snap.peers.iter().any(|p| p.name == "phone").then_some(snap)
    })
    .await;
    assert!(paired.pending.is_empty());
    assert_eq!(paired.peers[0].id, joiner.bridge.id().to_string());

    let host_id = app.host.snapshot().await.host.unwrap().id;
    let conn = within("connecting by key", joiner.bridge.connect(&host_id))
        .await
        .unwrap();
    within("the echo", echo(&conn, b"hello")).await.unwrap();

    assert!(
        app.host
            .revoke(&joiner.bridge.id().to_string())
            .await
            .unwrap()
    );
    tokio::time::timeout(GONE, conn.closed())
        .await
        .expect("the open connection survived the revoke");
    assert!(app.host.snapshot().await.peers.is_empty());
    // The gate closes a key that is off the list right after the handshake, so a dial may still
    // return a connection; what matters is that no request gets through it.
    let again = within("a second connect", joiner.bridge.connect(&host_id)).await;
    if let Ok(conn) = again {
        let reached = tokio::time::timeout(GONE, echo(&conn, b"again")).await;
        assert!(
            !matches!(reached, Ok(Ok(()))),
            "a revoked device got a request through"
        );
    }
    app.host.stop().await;
}

#[tokio::test]
async fn wrong_digits_decline_after_the_allowed_tries_and_pair_nobody() {
    let app = app();
    let joiner = Arc::new(Joiner::new().await);
    let ticket = app.host.begin().await.unwrap();
    let (digits, outcome) = joiner.join(&ticket.link, tuning());
    let request = first_request(&app.host).await;
    let shown = within("digits", digits).await.unwrap();
    let wrong = if shown == "000000" {
        "111111"
    } else {
        "000000"
    };

    assert_eq!(
        app.host.submit_digits(request.id, "12345"),
        Submitted::NotDigits
    );
    for left in (1..MAX_DIGIT_TRIES).rev() {
        assert_eq!(
            app.host.submit_digits(request.id, wrong),
            Submitted::WrongDigits { tries_left: left }
        );
    }
    assert_eq!(
        app.host.submit_digits(request.id, wrong),
        Submitted::Declined
    );
    // The right digits no longer help: the device was declined.
    assert_eq!(
        app.host.submit_digits(request.id, &shown),
        Submitted::Unknown
    );

    let result = within("the join to finish", outcome)
        .await
        .unwrap()
        .unwrap();
    assert!(!matches!(result, JoinResult::Paired(_)), "{result:?}");
    assert!(app.host.snapshot().await.peers.is_empty());
    app.host.stop().await;
}

#[tokio::test]
async fn a_wrong_try_then_the_right_digits_still_pairs() {
    let app = app();
    let joiner = Arc::new(Joiner::new().await);
    let ticket = app.host.begin().await.unwrap();
    let (digits, outcome) = joiner.join(&ticket.link, tuning());
    let request = first_request(&app.host).await;
    let shown = within("digits", digits).await.unwrap();
    let wrong = if shown == "000000" {
        "111111"
    } else {
        "000000"
    };

    assert!(matches!(
        app.host.submit_digits(request.id, wrong),
        Submitted::WrongDigits { .. }
    ));
    assert_eq!(
        app.host
            .submit_digits(request.id, &format!("{} {}", &shown[..3], &shown[3..])),
        Submitted::Accepted
    );
    let result = within("the join to finish", outcome)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, JoinResult::Paired(_)));
    app.host.stop().await;
}

#[tokio::test]
async fn declining_pairs_nobody() {
    let app = app();
    let joiner = Arc::new(Joiner::new().await);
    let ticket = app.host.begin().await.unwrap();
    let (_digits, outcome) = joiner.join(&ticket.link, tuning());
    let request = first_request(&app.host).await;
    assert!(app.host.decline(request.id));
    let result = within("the join to finish", outcome)
        .await
        .unwrap()
        .unwrap();
    assert!(!matches!(result, JoinResult::Paired(_)));
    assert!(app.host.snapshot().await.peers.is_empty());
    app.host.stop().await;
}

#[tokio::test]
async fn a_late_answer_pairs_nobody() {
    let short = Tuning {
        ttl: Duration::from_secs(120),
        decision_timeout: Duration::from_secs(1),
        hello_timeout: Duration::from_secs(2),
        ..Tuning::default()
    };
    let app = app_with(short.clone());
    let joiner = Arc::new(Joiner::new().await);
    let ticket = app.host.begin().await.unwrap();
    let (digits, outcome) = joiner.join(&ticket.link, short);
    let request = first_request(&app.host).await;
    let shown = within("digits", digits).await.unwrap();

    let _ = within("the join to give up", outcome).await;
    let late = app.host.submit_digits(request.id, &shown);
    assert!(
        matches!(late, Submitted::TooLate | Submitted::Unknown),
        "{late:?}"
    );
    assert!(app.host.snapshot().await.peers.is_empty());
    app.host.stop().await;
}

#[tokio::test]
async fn the_screen_never_gets_the_digits_it_has_to_compare() {
    let app = app();
    let joiner = Arc::new(Joiner::new().await);
    let ticket = app.host.begin().await.unwrap();
    let (digits, _outcome) = joiner.join(&ticket.link, tuning());
    let _ = first_request(&app.host).await;
    let shown = within("digits", digits).await.unwrap();

    let snapshot: PairingSnapshot = app.host.snapshot().await;
    let json = serde_json::to_string(&snapshot).unwrap();
    assert!(
        !json.contains(&shown),
        "the expected digits are in the snapshot: {json}"
    );
    assert!(!json.contains("sas"), "{json}");
    app.host.stop().await;
}

#[tokio::test]
async fn a_new_code_replaces_the_old_one_and_stopping_closes_everything() {
    let app = app();
    let first = app.host.begin().await.unwrap();
    let second = app.host.begin().await.unwrap();
    assert_ne!(first.link, second.link);

    let joiner = Arc::new(Joiner::new().await);
    let (_digits, old) = joiner.join(&first.link, tuning());
    // Nothing answers on the replaced code's endpoint, so the join ends in a failure or never ends.
    let result = tokio::time::timeout(GONE, old).await;
    assert!(
        !matches!(result, Ok(Ok(Ok(JoinResult::Paired(_))))),
        "the replaced code still worked"
    );

    app.host.stop().await;
    let snap = app.host.snapshot().await;
    assert!(!snap.running && snap.ticket.is_none());
}
