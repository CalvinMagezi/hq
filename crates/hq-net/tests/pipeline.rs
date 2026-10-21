#![cfg(all(unix, feature = "tools"))]
//! The whole unprivileged-to-privileged path without root: `hq net init`, `peer add` and
//! `revoke` write the applier's input; the applier's grammar reads that same file; the text that
//! would go to `wg` is built from it. No interface is touched and nothing needs a privilege.

use hq_net::grammar::{self, Policy};
use hq_net::init::{InitContext, InitOptions, init};
use hq_net::key::PrivateKey;
use hq_net::links;
use hq_net::ops::{AddRequest, apply, link_add, link_remove, peer_add, peer_revoke};
use hq_net::store::{Kind, Store};
use hq_net::wgconf;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

struct Plane {
    _dir: tempfile::TempDir,
    store: Store,
    spool: PathBuf,
    /// The hub's private key as `init` staged it, before the owner moved it.
    hub_private: String,
}

impl Plane {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("net"));
        let spool = dir.path().join("spool");
        std::fs::create_dir(&spool).unwrap();
        std::fs::set_permissions(&spool, std::fs::Permissions::from_mode(0o700)).unwrap();
        let ctx = InitContext {
            routed: Some(vec![]),
            ip_forward: Some("0".into()),
            owner_uid: 1000,
            owner_gid: 1000,
            spool_dir: spool.clone(),
        };
        let options = InitOptions {
            endpoint: Some("vpn.example.com:51820".into()),
            ..Default::default()
        };
        init(&store, &options, &ctx).unwrap();
        // Step 1 of the printed steps: the owner moves the hub key out of the user's directory.
        let hub_private = std::fs::read_to_string(store.hub_key_path()).unwrap();
        std::fs::remove_file(store.hub_key_path()).unwrap();
        Self {
            _dir: dir,
            store,
            spool,
            hub_private,
        }
    }

    fn add(&self, label: &str, kind: Kind, preshared: bool) -> String {
        let key = device_key();
        let request = AddRequest {
            label,
            public_key: &key,
            kind,
            preshared,
            keepalive: None,
            with_qr: false,
            confirm_public: false,
        };
        peer_add(&self.store, &self.spool, &request, &mut Vec::new()).unwrap();
        key
    }

    fn policy(&self) -> Policy {
        self.store.require_net().unwrap().policy().unwrap()
    }

    fn input(&self) -> Vec<u8> {
        std::fs::read(self.spool.join("hq0.conf")).unwrap()
    }
}

/// A public key that does not happen to look like a private one.
fn device_key() -> String {
    loop {
        let key = PrivateKey::generate().public();
        if !hq_net::key::looks_like_private_key(&key) {
            return key;
        }
    }
}

const SHOWCONF: &str =
    "[Interface]\nListenPort = 51820\nPrivateKey = AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=\n";

#[test]
fn what_hq_net_writes_is_exactly_what_the_applier_accepts() {
    let plane = Plane::new();
    let laptop = plane.add("mac", Kind::Laptop, false);
    let phone = plane.add("pixel", Kind::Phone, true);
    let peers = grammar::parse(&plane.input(), &plane.policy()).unwrap();
    assert_eq!(peers.len(), 2);
    assert_eq!(peers[0].public_key, laptop);
    assert_eq!(peers[1].public_key, phone);
    assert!(peers[0].preshared_key.is_none() && peers[1].preshared_key.is_some());
    assert_eq!(peers[0].address.to_string(), "10.77.0.2");
}

#[test]
fn the_text_for_wg_holds_the_interface_then_only_the_three_allowed_settings() {
    let plane = Plane::new();
    plane.add("mac", Kind::Laptop, false);
    plane.add("pixel", Kind::Phone, true);
    let peers = grammar::parse(&plane.input(), &plane.policy()).unwrap();
    let (subcommand, text) = wgconf::build(&peers, SHOWCONF).unwrap();
    assert_eq!(subcommand, wgconf::Subcommand::Syncconf);
    let mut section = "";
    for line in text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        match line {
            "[Interface]" | "[Peer]" => section = line,
            _ => {
                let name = line.split(" = ").next().unwrap();
                let allowed: &[&str] = if section == "[Interface]" {
                    &["PrivateKey", "ListenPort", "FwMark"]
                } else {
                    &["PublicKey", "PresharedKey", "AllowedIPs"]
                };
                assert!(
                    allowed.contains(&name),
                    "{line:?} does not belong in {section}"
                );
            }
        }
    }
    assert_eq!(text.matches("[Peer]").count(), 2);
    assert!(
        text.contains("AllowedIPs = 10.77.0.2/32") && text.contains("AllowedIPs = 10.77.0.3/32")
    );
}

#[test]
fn revoking_a_device_removes_it_from_what_wg_receives() {
    let plane = Plane::new();
    let gone = plane.add("mac", Kind::Laptop, false);
    let kept = plane.add("pixel", Kind::Phone, false);
    peer_revoke(&plane.store, &plane.spool, "mac", &mut Vec::new()).unwrap();
    let peers = grammar::parse(&plane.input(), &plane.policy()).unwrap();
    let (_, text) = wgconf::build(&peers, SHOWCONF).unwrap();
    assert!(!text.contains(&gone) && text.contains(&kept));
    peer_revoke(&plane.store, &plane.spool, "pixel", &mut Vec::new()).unwrap();
    let peers = grammar::parse(&plane.input(), &plane.policy()).unwrap();
    let (subcommand, text) = wgconf::build(&peers, SHOWCONF).unwrap();
    assert_eq!(
        subcommand,
        wgconf::Subcommand::Setconf,
        "an empty list must replace, not sync"
    );
    assert!(!text.contains("[Peer]"));
}

#[test]
fn a_broken_hand_edit_of_peers_toml_changes_nothing_that_the_applier_reads() {
    let plane = Plane::new();
    plane.add("mac", Kind::Laptop, false);
    let before = plane.input();
    let path = plane.store.dir().join("peers.toml");
    let edited = std::fs::read_to_string(&path)
        .unwrap()
        .replace("10.77.0.2", "10.77.0.1");
    std::fs::write(&path, edited).unwrap();
    let err = apply(&plane.store, &plane.spool, &mut Vec::new()).unwrap_err();
    assert!(err.to_string().contains("refused"), "{err}");
    assert_eq!(
        plane.input(),
        before,
        "a failed apply must leave the previous input in place"
    );
}

#[test]
fn the_applier_input_is_private_and_leaves_no_temporary_files_in_the_spool() {
    let plane = Plane::new();
    plane.add("mac", Kind::Laptop, false);
    plane.add("pixel", Kind::Phone, false);
    let names: Vec<_> = std::fs::read_dir(&plane.spool)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("hq0.conf")]);
    let mode = std::fs::metadata(plane.spool.join("hq0.conf"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn the_hub_key_is_never_in_the_applier_input_or_the_device_config_output() {
    let plane = Plane::new();
    let key = device_key();
    let mut out = Vec::new();
    let request = AddRequest {
        label: "mac",
        public_key: &key,
        kind: Kind::Laptop,
        preshared: true,
        keepalive: None,
        with_qr: true,
        confirm_public: false,
    };
    peer_add(&plane.store, &plane.spool, &request, &mut out).unwrap();
    let shown = String::from_utf8(out).unwrap();
    for text in [
        shown,
        String::from_utf8(plane.input()).unwrap(),
        std::fs::read_to_string(plane.store.dir().join("net.toml")).unwrap(),
    ] {
        assert!(
            !text.contains(plane.hub_private.trim()),
            "SR-N2: the hub's private key leaked"
        );
    }
}

#[test]
fn nothing_is_added_while_the_hub_key_is_still_staged_and_a_revoke_still_works() {
    let plane = Plane::new();
    plane.add("mac", Kind::Laptop, false);
    std::fs::write(plane.store.hub_key_path(), &plane.hub_private).unwrap();
    let key = device_key();
    let request = AddRequest {
        label: "pixel",
        public_key: &key,
        kind: Kind::Phone,
        preshared: false,
        keepalive: None,
        with_qr: false,
        confirm_public: false,
    };
    assert!(peer_add(&plane.store, &plane.spool, &request, &mut Vec::new()).is_err());
    peer_revoke(&plane.store, &plane.spool, "mac", &mut Vec::new()).unwrap();
    assert!(!String::from_utf8(plane.input()).unwrap().contains("# mac"));
}

#[test]
fn a_link_reaches_the_firewall_script_as_two_directed_pairs_and_nothing_else() {
    let plane = Plane::new();
    plane.add("mac", Kind::Laptop, false);
    plane.add("pixel", Kind::Phone, false);
    plane.add("tablet", Kind::Laptop, false);
    link_add(
        &plane.store,
        &plane.spool,
        "pixel",
        "mac",
        true,
        &mut Vec::new(),
    )
    .unwrap();

    let parsed = grammar::parse_file(&plane.input(), &plane.policy()).unwrap();
    assert_eq!(parsed.peers.len(), 3);
    let wanted = parsed.links.expect("a version 2 file");
    let script = links::script("hq0", &wanted).unwrap();
    assert_eq!(
        script,
        "flush set inet hq_net links_hq0\nadd element inet hq_net links_hq0 { 10.77.0.2 . 10.77.0.3, 10.77.0.3 . 10.77.0.2 }\n",
        "the tablet at 10.77.0.4 is in no pair"
    );
    assert!(
        grammar::parse(&plane.input(), &plane.policy()).is_err(),
        "a reader that does not know links refuses the file"
    );

    link_remove(&plane.store, &plane.spool, "mac", "pixel", &mut Vec::new()).unwrap();
    let parsed = grammar::parse_file(&plane.input(), &plane.policy()).unwrap();
    assert_eq!(parsed.links, Some(vec![]));
    assert_eq!(
        links::script("hq0", &[]).unwrap(),
        "flush set inet hq_net links_hq0\n"
    );
}

#[test]
fn revoking_a_linked_device_leaves_no_pair_for_its_address() {
    let plane = Plane::new();
    plane.add("mac", Kind::Laptop, false);
    plane.add("pixel", Kind::Phone, false);
    link_add(
        &plane.store,
        &plane.spool,
        "mac",
        "pixel",
        true,
        &mut Vec::new(),
    )
    .unwrap();
    peer_revoke(&plane.store, &plane.spool, "pixel", &mut Vec::new()).unwrap();
    let parsed = grammar::parse_file(&plane.input(), &plane.policy()).unwrap();
    assert_eq!(parsed.peers.len(), 1);
    assert_eq!(parsed.links, Some(vec![]));
    // The address is retired: a new device does not get the revoked one's pairs by taking its place.
    let again = plane.add("watch", Kind::Phone, false);
    let parsed = grammar::parse_file(&plane.input(), &plane.policy()).unwrap();
    let newcomer = parsed.peers.iter().find(|p| p.public_key == again).unwrap();
    assert_ne!(newcomer.address.to_string(), "10.77.0.3");
    assert_eq!(parsed.links, Some(vec![]));
}
