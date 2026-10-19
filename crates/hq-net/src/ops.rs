//! The commands under `hq net` after `init`. Each one says what it is about to change, then
//! changes it, so the owner reads the intent first. Output goes to a writer so tests can read it.

use crate::device;
use crate::files;
use crate::status;
use crate::store::{
    Kind, NetError, NewPeer, Result, Store, add_peer, render_applier_input, revoke_peer,
};
use std::io::Write;
use std::path::Path;

const FINGERPRINT_CHARS: usize = 8;

fn say(out: &mut dyn Write, text: &str) -> Result<()> {
    out.write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .map_err(NetError::from)
}

pub struct AddRequest<'a> {
    pub label: &'a str,
    pub public_key: &'a str,
    pub kind: Kind,
    pub preshared: bool,
    pub with_qr: bool,
}

/// Renders `peers.toml` into the applier's input file in the spool directory, replacing it
/// whole. The applier is started by its path unit when the file changes.
pub fn apply(store: &Store, spool_dir: &Path, out: &mut dyn Write) -> Result<()> {
    let net = store.require_net()?;
    // An empty list tells the applier to remove every device. That must come from a peers.toml
    // that exists and lists none, not from one that is missing (a wrong HOME, a deleted file).
    if !store.has_peers_file() {
        return crate::store::fail(format!(
            "{} has no peers.toml, so no device has been added here; refusing to write an empty list that would remove every device",
            store.dir().display()
        ));
    }
    let peers = store.load_peers()?;
    let text = render_applier_input(&net, &peers)?;
    let target = net.spool_file(spool_dir);
    say(
        out,
        &format!(
            "Writing {} device(s) to {} for the applier.\n",
            peers.peer.len(),
            target.display()
        ),
    )?;
    files::check_spool_dir(spool_dir)?;
    files::write_private_atomic(&target, &text)?;
    say(
        out,
        &format!(
            "Done. The path unit hq-net-apply@{0}.path applies it within a few seconds, and a timer applies it again every five minutes. To apply now: sudo systemctl start hq-net-apply@{0}.service\n",
            net.interface
        ),
    )
}

pub fn peer_add(
    store: &Store,
    spool_dir: &Path,
    request: &AddRequest<'_>,
    out: &mut dyn Write,
) -> Result<()> {
    let net = store.require_net()?;
    let mut peers = store.load_peers()?;
    let record = add_peer(
        &net,
        &mut peers,
        NewPeer {
            label: request.label,
            public_key: request.public_key,
            kind: request.kind,
            preshared: request.preshared,
        },
    )?;
    // Checked before anything is saved, so a missing server address leaves no half-added device.
    let description = device::describe(&net, &record, request.with_qr)?;
    say(
        out,
        &format!(
            "Adding {} \"{}\" at {}. This changes {}/peers.toml and {} (the applier's input).\n",
            record.kind.name(),
            record.label,
            record.address,
            store.dir().display(),
            net.spool_file(spool_dir).display()
        ),
    )?;
    store.save_peers(&peers)?;
    if let Err(e) = apply(store, spool_dir, out) {
        say(
            out,
            &format!(
                "The device is saved in peers.toml, but the applier's input was not written: {e}\nFix that, then run: hq net apply\n"
            ),
        )?;
    }
    say(out, &format!("\n{description}"))
}

pub fn peer_revoke(
    store: &Store,
    spool_dir: &Path,
    label: &str,
    out: &mut dyn Write,
) -> Result<()> {
    let net = store.require_net()?;
    let mut peers = store.load_peers()?;
    let record = revoke_peer(&mut peers, label)?;
    say(
        out,
        &format!(
            "Revoking \"{}\" ({}, {}). This changes {}/peers.toml and {}. The address is retired and will not be reused.\n",
            record.label,
            record.kind.name(),
            record.address,
            store.dir().display(),
            net.spool_file(spool_dir).display()
        ),
    )?;
    store.save_peers(&peers)?;
    apply(store, spool_dir, out)?;
    say(
        out,
        "If this device was lost or stolen, also sign it out of the web UI and rotate any token it held. Check that it is gone with: sudo wg show hq0 peers\n",
    )
}

pub fn peer_list(store: &Store, out: &mut dyn Write) -> Result<()> {
    let net = store.require_net()?;
    let peers = store.load_peers()?;
    let mut text = format!(
        "{:<16} {:<7} {:<12} {:<10} {}\n",
        "DEVICE", "KIND", "ADDRESS", "KEY", "PRESHARED"
    );
    for p in &peers.peer {
        let short: String = p.public_key.chars().take(FINGERPRINT_CHARS).collect();
        let psk = if p.preshared_key.is_some() {
            "yes"
        } else {
            "no"
        };
        text.push_str(&format!(
            "{:<16} {:<7} {:<12} {short:<10} {psk}\n",
            p.label,
            p.kind.name(),
            p.address
        ));
    }
    text.push_str(&format!(
        "{} device(s) on {} (hub {}); {} address(es) retired.\n",
        peers.peer.len(),
        net.subnet,
        net.hub,
        peers.retired.len()
    ));
    say(out, &text)
}

/// `handshakes` is the output of `wg show <interface> latest-handshakes`, or `None` if it could not
/// be read (it needs root).
pub fn peer_status(
    store: &Store,
    handshakes: Option<&str>,
    now_unix: u64,
    out: &mut dyn Write,
) -> Result<()> {
    let net = store.require_net()?;
    let peers = store.load_peers()?;
    let parsed = handshakes.map(status::parse_handshakes);
    say(out, &status::table(&peers.peer, parsed.as_ref(), now_unix))?;
    if handshakes.is_none() {
        say(
            out,
            &format!(
                "Handshake times need root. Run: sudo wg show {0} latest-handshakes | hq net status --handshakes -\n",
                net.interface
            ),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::PrivateKey;
    use crate::store::tests::sample_net;
    use std::os::unix::fs::PermissionsExt;

    struct Fixture {
        _dir: tempfile::TempDir,
        store: Store,
        spool: std::path::PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("net"));
        store.ensure_dir().unwrap();
        store.save_net(&sample_net()).unwrap();
        let spool = dir.path().join("spool");
        std::fs::create_dir(&spool).unwrap();
        std::fs::set_permissions(&spool, std::fs::Permissions::from_mode(0o700)).unwrap();
        Fixture {
            _dir: dir,
            store,
            spool,
        }
    }

    fn request<'a>(label: &'a str, key: &'a str, kind: Kind) -> AddRequest<'a> {
        AddRequest {
            label,
            public_key: key,
            kind,
            preshared: false,
            with_qr: false,
        }
    }

    fn run_add(f: &Fixture, label: &str, kind: Kind) -> (String, String) {
        let key = PrivateKey::generate().public();
        let mut out = Vec::new();
        peer_add(&f.store, &f.spool, &request(label, &key, kind), &mut out).unwrap();
        (String::from_utf8(out).unwrap(), key)
    }

    fn applier_input(f: &Fixture) -> String {
        std::fs::read_to_string(f.spool.join("hq0.conf")).unwrap()
    }

    #[test]
    fn adding_a_device_says_what_it_changes_then_writes_both_files_and_prints_the_config() {
        let f = fixture();
        let (out, key) = run_add(&f, "mac", Kind::Laptop);
        assert!(out.starts_with("Adding laptop \"mac\" at 10.77.0.2."));
        assert!(out.find("This changes").unwrap() < out.find("[Interface]").unwrap());
        assert!(out.contains("AllowedIPs = 10.77.0.1/32"));
        assert!(applier_input(&f).contains(&format!("PublicKey = {key}")));
        assert_eq!(f.store.load_peers().unwrap().peer.len(), 1);
        let mode = std::fs::metadata(f.spool.join("hq0.conf"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn the_input_the_applier_reads_is_accepted_by_the_applier_grammar() {
        let f = fixture();
        run_add(&f, "mac", Kind::Laptop);
        run_add(&f, "pixel", Kind::Phone);
        let net = f.store.require_net().unwrap();
        let parsed =
            crate::grammar::parse(applier_input(&f).as_bytes(), &net.policy().unwrap()).unwrap();
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn revoking_removes_the_device_from_the_input_and_retires_the_address() {
        let f = fixture();
        let (_, key) = run_add(&f, "mac", Kind::Laptop);
        run_add(&f, "pixel", Kind::Phone);
        let mut out = Vec::new();
        peer_revoke(&f.store, &f.spool, "mac", &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.starts_with("Revoking \"mac\""));
        assert!(!applier_input(&f).contains(&key));
        assert!(applier_input(&f).contains("# pixel"));
        let (added, _) = run_add(&f, "tablet", Kind::Phone);
        assert!(
            added.contains("at 10.77.0.4."),
            "10.77.0.2 stays retired: {added}"
        );
        let mut out = Vec::new();
        assert!(peer_revoke(&f.store, &f.spool, "nobody", &mut out).is_err());
    }

    #[test]
    fn revoking_the_last_device_writes_a_valid_empty_peer_list() {
        let f = fixture();
        run_add(&f, "mac", Kind::Laptop);
        peer_revoke(&f.store, &f.spool, "mac", &mut Vec::new()).unwrap();
        let input = applier_input(&f);
        assert_eq!(
            input,
            format!(
                "{}\n{}0\n",
                crate::grammar::HEADER,
                crate::grammar::TRAILER_PREFIX
            )
        );
    }

    #[test]
    fn a_missing_spool_directory_keeps_the_device_and_says_how_to_finish() {
        let f = fixture();
        std::fs::remove_dir(&f.spool).unwrap();
        let (out, _) = run_add(&f, "mac", Kind::Laptop);
        assert!(out.contains("saved in peers.toml, but the applier's input was not written"));
        assert!(out.contains("hq net apply"));
        assert_eq!(f.store.load_peers().unwrap().peer.len(), 1);
    }

    #[test]
    fn a_missing_peers_file_is_never_turned_into_an_empty_list() {
        let f = fixture();
        run_add(&f, "mac", Kind::Laptop);
        let before = applier_input(&f);
        std::fs::remove_file(f.store.dir().join("peers.toml")).unwrap();
        let err = apply(&f.store, &f.spool, &mut Vec::new()).unwrap_err();
        assert!(err.to_string().contains("no peers.toml"), "{err}");
        assert_eq!(
            applier_input(&f),
            before,
            "the applier's input must not change"
        );
        // A peers.toml that exists and lists nobody (everyone revoked) is a real, empty list.
        let g = fixture();
        run_add(&g, "mac", Kind::Laptop);
        peer_revoke(&g.store, &g.spool, "mac", &mut Vec::new()).unwrap();
        assert!(applier_input(&g).contains("# hq-net end 0"));
    }

    #[test]
    fn a_spool_directory_others_can_write_to_is_refused() {
        let f = fixture();
        run_add(&f, "mac", Kind::Laptop);
        std::fs::set_permissions(&f.spool, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(apply(&f.store, &f.spool, &mut Vec::new()).is_err());
    }

    #[test]
    fn without_a_server_address_nothing_is_saved() {
        let f = fixture();
        let mut net = sample_net();
        net.endpoint = None;
        f.store.save_net(&net).unwrap();
        let key = PrivateKey::generate().public();
        let err = peer_add(
            &f.store,
            &f.spool,
            &request("mac", &key, Kind::Laptop),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("--endpoint"));
        assert!(f.store.load_peers().unwrap().peer.is_empty());
        assert!(!f.spool.join("hq0.conf").exists());
    }

    #[test]
    fn list_and_status_show_devices_and_say_when_handshakes_are_unreadable() {
        let f = fixture();
        let (_, key) = run_add(&f, "mac", Kind::Laptop);
        let mut out = Vec::new();
        peer_list(&f.store, &mut out).unwrap();
        let list = String::from_utf8(out).unwrap();
        assert!(list.contains("mac") && list.contains("10.77.0.2") && list.contains(&key[..8]));
        assert!(!list.contains(&key), "only a short fingerprint is listed");
        let mut out = Vec::new();
        peer_status(&f.store, None, 100, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("unknown") && text.contains("sudo wg show hq0 latest-handshakes"));
        let mut out = Vec::new();
        peer_status(&f.store, Some(&format!("{key}\t90\n")), 100, &mut out).unwrap();
        assert!(String::from_utf8(out).unwrap().contains("10s ago"));
    }

    #[test]
    fn a_preshared_key_flows_to_the_input_and_the_device_config() {
        let f = fixture();
        let key = PrivateKey::generate().public();
        let mut out = Vec::new();
        let req = AddRequest {
            preshared: true,
            ..request("mac", &key, Kind::Laptop)
        };
        peer_add(&f.store, &f.spool, &req, &mut out).unwrap();
        let psk = f.store.load_peers().unwrap().peer[0]
            .preshared_key
            .clone()
            .unwrap();
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains(&format!("PresharedKey = {psk}"))
        );
        assert!(applier_input(&f).contains(&format!("PresharedKey = {psk}")));
    }

    #[test]
    fn the_qr_code_is_printed_only_when_asked() {
        let f = fixture();
        let key = PrivateKey::generate().public();
        let mut out = Vec::new();
        let req = AddRequest {
            with_qr: true,
            ..request("pixel", &key, Kind::Phone)
        };
        peer_add(&f.store, &f.spool, &req, &mut out).unwrap();
        assert!(String::from_utf8(out).unwrap().contains("as a QR code"));
        let (plain, _) = run_add(&f, "other", Kind::Phone);
        assert!(!plain.contains("as a QR code"));
    }
}
