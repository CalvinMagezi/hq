//! The real `hq bridge join` program against the app's host. Opt-in, because it needs an `hq`
//! build with the `bridge` feature:
//!
//! ```bash
//! cargo build -p hq-cli --features bridge
//! HQ_BRIDGE_BIN=target/debug/hq cargo test -p hq-desktop --test cli_join -- --ignored
//! ```
//!
//! It runs with its own HOME, vault and config, so it touches nothing of the person's.

use hq_bridge::{RelayMode, Tuning};
use hq_db::Database;
use hq_desktop::pairing::{PairingConfig, PairingController, Submitted};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const GIVE_UP: Duration = Duration::from_secs(60);
const DIGITS_MARK: &str = "Digits: ";
const SAS_LEN: usize = 6;

#[tokio::test]
#[ignore = "needs HQ_BRIDGE_BIN: an hq build with the bridge feature (see the file header)"]
async fn the_real_hq_bridge_join_pairs_with_the_apps_host() {
    let bin = std::env::var("HQ_BRIDGE_BIN")
        .expect("set HQ_BRIDGE_BIN to an hq built with --features bridge");
    let home = tempfile::tempdir().unwrap();
    let key_dir = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    let host = PairingController::new(
        PairingConfig {
            key: key_dir.path().join("bridge.key"),
            bind: vec!["127.0.0.1:0".parse().unwrap()],
            relay: RelayMode::Disabled,
            name: "desk".into(),
            with_addrs: true,
            tuning: Tuning::default(),
        },
        move || Ok(db.clone()),
        None,
    );
    let ticket = host.begin().await.unwrap();

    let mut joiner = tokio::process::Command::new(bin)
        .args(["bridge", "join", "--relay", "none", "--name", "cli-phone"])
        .env("HOME", home.path())
        .env("HQ_VAULT_PATH", home.path().join("vault"))
        .env("HQ_CONFIG_PATH", home.path().join("config.yaml"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("could not run the hq program");
    let mut stdin = joiner.stdin.take().unwrap();
    // The link goes in on standard input, as the program asks, not on its command line.
    stdin
        .write_all(format!("{}\n", ticket.link).as_bytes())
        .await
        .unwrap();
    let mut lines = BufReader::new(joiner.stdout.take().unwrap()).lines();

    let digits = tokio::time::timeout(GIVE_UP, async {
        while let Some(line) = lines.next_line().await.unwrap() {
            if let Some(at) = line.find(DIGITS_MARK) {
                let rest = &line[at + DIGITS_MARK.len()..];
                return rest[..SAS_LEN].to_string();
            }
        }
        panic!("the program ended without showing digits");
    })
    .await
    .expect("the program showed no digits");

    let request = tokio::time::timeout(GIVE_UP, async {
        loop {
            if let Some(p) = host.snapshot().await.pending.first().cloned() {
                return p;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("no request reached the app");
    assert_eq!(request.name, "cli-phone");
    assert_eq!(host.submit_digits(request.id, &digits), Submitted::Accepted);

    let mut rest = String::new();
    while let Ok(Some(line)) = tokio::time::timeout(GIVE_UP, lines.next_line())
        .await
        .unwrap()
    {
        rest.push_str(&line);
        rest.push('\n');
    }
    let status = tokio::time::timeout(GIVE_UP, joiner.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success(), "the program failed: {rest}");
    assert!(rest.contains("Paired with"), "{rest}");
    assert!(
        host.snapshot()
            .await
            .peers
            .iter()
            .any(|p| p.name == "cli-phone")
    );
    host.stop().await;
}
