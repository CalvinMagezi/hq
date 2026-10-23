//! Registering a device with the edge by QR code, on loopback: the edge opens an invitation, a
//! device joins it, the person's typed digits decide, and the allowlist (which the running edge
//! reads again every second) is what changes.
#![cfg(feature = "invite")]

mod common;

use std::time::Duration;

use common::*;
use hq_bridge::{JoinResult, OpenTicket, Purpose, RelayMode, Ticket, Tuning, join::JoinOptions};
use hq_edge::{
    EdgeConfig, Registered, allow_device, default_invite_spec, device_label, list_devices,
    open_invite, set_device_relay,
};
use iroh::{Endpoint, RelayUrl, SecretKey, endpoint::presets};

const SECOND: Duration = Duration::from_secs(1);
/// The edge reads its list again this often; a test that needs the new list waits a little longer.
const RELOAD: Duration = Duration::from_millis(1_200);

/// The edge's config as an operator running its own (registered-devices-only) relay has it: the
/// invitation must still work, because it never goes through that relay. The invitation port is
/// on loopback so the test needs no network.
fn config_for(rig: &Rig) -> EdgeConfig {
    EdgeConfig {
        enabled: true,
        domain: DOMAIN.into(),
        dir: rig.dir.path().to_path_buf(),
        relay_url: Some(rig.relay_url.to_string()),
        invite_bind: "127.0.0.1:0".parse().unwrap(),
        ..EdgeConfig::default()
    }
}

/// A device's own endpoint, made from its key, as the app joins an invitation with it.
async fn joiner(key: &SecretKey) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .secret_key(key.clone())
        .relay_mode(RelayMode::Disabled)
        .clear_ip_transports()
        .bind_addr(std::net::SocketAddr::from(([127, 0, 0, 1], 0)))
        .unwrap()
        .bind()
        .await
        .unwrap()
}

/// The person at the edge answers the first request: the typed digits matched, or did not.
fn confirm(mut open: OpenTicket, accept: bool) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Some(request) = open.requests.recv().await {
            let _ = request.decide.send(accept);
        }
        // Keep the invitation alive until the joiner has read the answer.
        tokio::time::sleep(3 * SECOND).await;
        drop(open);
    })
}

async fn register(ticket: &Ticket, key: &SecretKey, relays: Vec<String>) -> JoinResult {
    let endpoint = joiner(key).await;
    let result = tokio::time::timeout(
        20 * SECOND,
        hq_bridge::join::join(
            &endpoint,
            ticket,
            &JoinOptions::new("phone", relays, vec![]),
            |_| {},
        ),
    )
    .await
    .expect("registering took too long")
    .unwrap();
    endpoint.close().await;
    result
}

async fn invite_and_register(
    rig: &Rig,
    key: &SecretKey,
    accept: bool,
    relays: Vec<String>,
    config: &EdgeConfig,
) -> JoinResult {
    let spec = default_invite_spec(config, Tuning::default()).unwrap();
    let open = open_invite(config, spec).await.unwrap();
    let ticket = open.ticket.clone();
    assert_eq!(ticket.k, Purpose::Register);
    let confirming = confirm(open, accept);
    let result = register(&ticket, key, relays).await;
    confirming.abort();
    let _ = rig;
    result
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_that_scans_the_invitation_is_listed_and_told_its_name() {
    let rig = rig(&[], |_| {}).await;
    let key = SecretKey::generate();
    let result = invite_and_register(&rig, &key, true, vec![], &config_for(&rig)).await;
    let JoinResult::Paired(joined) = result else {
        panic!("{result:?}")
    };
    assert_eq!(joined.card.id, rig.edge_id.to_string());
    assert_eq!(joined.card.name, DOMAIN);
    assert_eq!(
        joined.info.unwrap()["hostname"],
        format!("{}.{DOMAIN}", device_label(&key.public()))
    );
    let listed = list_devices(&rig.list).unwrap();
    assert_eq!(
        listed,
        vec![Registered {
            id: key.public(),
            relay: None
        }]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_declined_device_is_not_listed() {
    let rig = rig(&[], |_| {}).await;
    let key = SecretKey::generate();
    let result = invite_and_register(&rig, &key, false, vec![], &config_for(&rig)).await;
    assert!(matches!(result, JoinResult::Declined { .. }), "{result:?}");
    assert!(list_devices(&rig.list).unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn registering_again_with_the_same_key_succeeds_and_lists_it_once() {
    let rig = rig(&[], |_| {}).await;
    let config = config_for(&rig);
    let key = SecretKey::generate();
    for _ in 0..2 {
        let result = invite_and_register(&rig, &key, true, vec![], &config).await;
        assert!(matches!(result, JoinResult::Paired(_)), "{result:?}");
    }
    assert_eq!(list_devices(&rig.list).unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_relay_the_operator_listed_is_recorded_for_a_device_that_names_it() {
    let rig = rig(&[], |_| {}).await;
    let other = "https://other-relay.edge.test";
    let mut config = config_for(&rig);
    config.device_relays = vec![other.to_string()];
    let other_url: RelayUrl = other.parse().unwrap();

    let away = SecretKey::generate();
    let result = invite_and_register(&rig, &away, true, vec![other.to_string()], &config).await;
    assert!(matches!(result, JoinResult::Paired(_)), "{result:?}");
    eventually(5 * SECOND, "the device's relay to be recorded", || {
        list_devices(&rig.list)
            .unwrap()
            .iter()
            .any(|d| d.id == away.public() && d.relay.as_ref() == Some(&other_url))
    })
    .await;

    // A device that names the edge's own relay needs no entry.
    let home = SecretKey::generate();
    let own = rig.relay_url.to_string();
    let result = invite_and_register(&rig, &home, true, vec![own], &config).await;
    assert!(matches!(result, JoinResult::Paired(_)), "{result:?}");
    tokio::time::sleep(SECOND).await;
    let listed = list_devices(&rig.list).unwrap();
    let entry = listed.iter().find(|d| d.id == home.public()).unwrap();
    assert_eq!(entry.relay, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_relay_the_operator_did_not_list_is_never_recorded_whatever_it_looks_like() {
    let rig = rig(&[], |_| {}).await;
    let mut config = config_for(&rig);
    config.device_relays = vec!["https://listed.edge.test".to_string()];
    // A registered device chose these and the edge would connect to whatever is recorded. A name
    // cannot be vetted from outside (it can resolve anywhere), so only listed relays count.
    for bad in [
        "https://relay.example.org",
        "http://listed.edge.test",
        "https://listed.edge.test:6379",
        "https://127.0.0.1:8443",
        "https://169.254.169.254",
        "https://[::1]",
        "https://localhost.",
        "https://x.localhost",
        "https://db.internal.",
        "https://127.0.0.1.nip.io:6379",
        "https://user:pass@listed.edge.test",
    ] {
        let key = SecretKey::generate();
        let result = invite_and_register(&rig, &key, true, vec![bad.to_string()], &config).await;
        assert!(matches!(result, JoinResult::Paired(_)), "{bad}: {result:?}");
        tokio::time::sleep(Duration::from_millis(600)).await;
        let listed = list_devices(&rig.list).unwrap();
        let entry = listed.iter().find(|d| d.id == key.public()).unwrap();
        assert_eq!(entry.relay, None, "{bad} was recorded as a device's relay");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_registered_by_qr_is_reached_through_the_edge() {
    let rig = rig(&[], |_| {}).await;
    let key = SecretKey::generate();
    let result = invite_and_register(&rig, &key, true, vec![], &config_for(&rig)).await;
    let JoinResult::Paired(joined) = result else {
        panic!("{result:?}")
    };
    let hostname = joined.info.unwrap()["hostname"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(hostname, name_of(&key.public()));

    // The running edge picks the list up on its next reload; then the device can be served.
    tokio::time::sleep(RELOAD).await;
    let (acceptor, connector) = tls_pair(&hostname, &[]);
    let device = start_device(key, &rig.relay_url, rig.edge_id, Behavior::Http, acceptor).await;
    let (response, _) = https_get(rig.addr(), &connector, &hostname).await;
    assert!(response.ends_with(BODY), "got {response:?}");
    assert_eq!(device.streams(), 1);
}

#[test]
fn a_device_relay_is_replaced_in_place_and_the_rest_of_the_list_is_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("devices.txt");
    let (a, b) = (
        SecretKey::generate().public(),
        SecretKey::generate().public(),
    );
    for id in [a, b] {
        allow_device(&list, Registered { id, relay: None }).unwrap();
    }
    let relay: RelayUrl = "https://relay.edge.test".parse().unwrap();
    assert!(set_device_relay(&list, &a, Some(&relay)).unwrap());
    let listed = list_devices(&list).unwrap();
    assert_eq!(listed[0].relay.as_ref(), Some(&relay));
    assert_eq!(listed[1], Registered { id: b, relay: None });

    assert!(set_device_relay(&list, &a, None).unwrap());
    assert_eq!(list_devices(&list).unwrap()[0].relay, None);
    assert!(!set_device_relay(&list, &SecretKey::generate().public(), Some(&relay)).unwrap());
    assert!(
        !dir.path().join("devices.tmp").exists(),
        "no temporary file is left behind"
    );
}

#[test]
fn a_note_on_a_device_line_survives_a_relay_change() {
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("devices.txt");
    let id = SecretKey::generate().public();
    allow_device(&list, Registered { id, relay: None }).unwrap();
    let mut text = std::fs::read_to_string(&list).unwrap();
    text = text.replace('\n', " # the kitchen tablet\n");
    std::fs::write(&list, text).unwrap();
    let relay: RelayUrl = "https://relay.edge.test".parse().unwrap();
    assert!(set_device_relay(&list, &id, Some(&relay)).unwrap());
    let after = std::fs::read_to_string(&list).unwrap();
    assert!(after.contains("# the kitchen tablet"), "{after:?}");
    assert_eq!(list_devices(&list).unwrap()[0].relay.as_ref(), Some(&relay));
}

#[test]
fn writers_of_the_list_at_the_same_time_lose_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let list = std::sync::Arc::new(dir.path().join("devices.txt"));
    let ids: Vec<_> = (0..12).map(|_| SecretKey::generate().public()).collect();
    let handles: Vec<_> = ids
        .iter()
        .map(|id| {
            let (list, id) = (list.clone(), *id);
            std::thread::spawn(move || {
                allow_device(&list, Registered { id, relay: None }).unwrap();
                let relay: RelayUrl = "https://relay.edge.test".parse().unwrap();
                assert!(set_device_relay(&list, &id, Some(&relay)).unwrap());
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    let listed = list_devices(&list).unwrap();
    assert_eq!(listed.len(), ids.len(), "a concurrent write lost a device");
    assert!(
        listed.iter().all(|d| d.relay.is_some()),
        "a relay record was lost"
    );
}
