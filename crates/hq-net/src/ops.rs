//! The commands under `hq net` after `init`. Each one says what it is about to change, then
//! changes it, so the owner reads the intent first. Output goes to a writer so tests can read it.

use crate::device;
use crate::files;
use crate::status;
use crate::store::{
    Kind, NetError, NewPeer, RETIRED_FILE, Result, Store, add_peer, render_applier_input,
    revoke_peer,
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
    /// The caller confirms the key is public although it has the shape of a private one.
    pub confirm_public: bool,
}

fn staged_key_error(store: &Store) -> NetError {
    NetError(format!(
        "the hub's private key is still at {}, where the HQ user and its agents can read it. Move it as root first (step 1 of the `hq net init` output), then run this again",
        store.hub_key_path().display()
    ))
}

/// Devices the hub has now (read from the applier's current input) that the new input drops
/// without any record of revoking them. A damaged or emptied `peers.toml` would otherwise turn
/// into "remove everyone" at the next apply.
fn removals_without_a_revoke(
    current: &[crate::grammar::Peer],
    wanted: &[crate::store::PeerRecord],
    retired: &[String],
) -> Vec<String> {
    // By key, not by address: a new device given a revoked-by-damage device's address is still the
    // old one being dropped.
    current
        .iter()
        .filter(|p| !wanted.iter().any(|w| w.public_key == p.public_key))
        .map(|p| p.address.to_string())
        .filter(|address| !retired.contains(address))
        .collect()
}

/// What to do when the new input would drop a device nobody revoked here.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Guard {
    /// Refuse, and name the way out. For adding and applying.
    Enforce,
    /// Go on and say plainly what else was dropped. For cutting a device off: a lost laptop must
    /// not stay admitted because of some other entry the hub has that nobody here can name.
    Warn,
}

struct Checked {
    text: String,
    warnings: Vec<String>,
}

/// The text for the applier's input, after checks that touch nothing: the applier's own grammar
/// accepts it, and it does not drop a device the hub has unless that device was revoked here. If
/// the applier's current input cannot be read or parsed the second check cannot run; that is a
/// warning, not a refusal, so a damaged input can still be replaced.
fn checked_input(
    net: &crate::store::Net,
    peers: &crate::store::PeersFile,
    retired: &[String],
    target: &Path,
    guard: Guard,
) -> Result<Checked> {
    let text = render_applier_input(net, peers)?;
    let mut warnings = Vec::new();
    match files::read_private(target) {
        Ok(None) => {}
        Err(e) => warnings.push(format!(
            "the applier's current input could not be read ({e}), so I cannot check whether this drops a device the hub has"
        )),
        Ok(Some(current)) => match crate::grammar::parse(current.as_bytes(), &net.policy()?) {
            Err(e) => warnings.push(format!(
                "the applier's current input is not a valid peers file ({e}), so I cannot check whether this drops a device the hub has"
            )),
            Ok(current) => {
                let dropped = removals_without_a_revoke(&current, &peers.peer, retired);
                if !dropped.is_empty() && guard == Guard::Enforce {
                    let forget: Vec<String> =
                        dropped.iter().map(|a| format!("`hq net peer forget {a}`")).collect();
                    return crate::store::fail(format!(
                        "refusing to write the applier's input: it would remove {} from the hub, and none of them was revoked here. If peers.toml was damaged or restored from an older copy, restore the newer one. If you mean to remove them (a device nobody can name any more, or one you did not add), run {} and then `hq net apply`",
                        dropped.join(", "),
                        forget.join(" and ")
                    ));
                }
                if !dropped.is_empty() {
                    warnings.push(format!(
                        "this also removes {} from the hub: no device in peers.toml has that key and none was revoked here. If one was a real device, add it again with its public key",
                        dropped.join(", ")
                    ));
                }
            }
        },
    }
    Ok(Checked { text, warnings })
}

fn say_warnings(out: &mut dyn Write, warnings: &[String]) -> Result<()> {
    for warning in warnings {
        say(out, &format!("Warning: {warning}\n"))?;
    }
    Ok(())
}

/// Renders `peers.toml` into the applier's input file in the spool directory, replacing it
/// whole. The applier is started by its path unit when the file changes.
fn write_applier_input(
    store: &Store,
    spool_dir: &Path,
    out: &mut dyn Write,
    require_key_moved: bool,
    guard: Guard,
) -> Result<()> {
    let net = store.require_net()?;
    if require_key_moved && store.hub_key_staged() {
        return Err(staged_key_error(store));
    }
    // An empty list tells the applier to remove every device. That must come from a peers.toml
    // that exists and lists none, not from one that is missing (a wrong HOME, a deleted file).
    if !store.has_peers_file() {
        return crate::store::fail(format!(
            "{} has no peers.toml, so no device has been added here; refusing to write an empty list that would remove every device",
            store.dir().display()
        ));
    }
    let peers = store.load_peers()?;
    let target = net.spool_file(spool_dir);
    let (retired, damaged) = if guard == Guard::Enforce {
        (store.load_retired()?, false)
    } else {
        store.load_retired_lenient()
    };
    let checked = checked_input(&net, &peers, &retired, &target, guard)?;
    if damaged {
        say_warnings(out, &[retired_damaged_warning(store)])?;
    }
    say_warnings(out, &checked.warnings)?;
    say(
        out,
        &format!(
            "Writing {} device(s) to {} for the applier.\n",
            peers.peer.len(),
            target.display()
        ),
    )?;
    files::check_spool_dir(spool_dir)?;
    files::write_private_atomic(&target, &checked.text)?;
    say(
        out,
        &format!(
            "Done. The path unit applies it within a few seconds, and a timer applies it again every five minutes at the latest. To apply now: sudo systemctl start hq-net-apply@{0}.service\n",
            net.interface
        ),
    )
}

fn retired_damaged_warning(store: &Store) -> String {
    format!(
        "{}/{RETIRED_FILE} was damaged; it was rebuilt from the lines that could be read. Addresses revoked earlier whose lines were lost can be handed out again by `hq net peer add`: check `hq net peer list` before adding",
        store.dir().display()
    )
}

/// Writes the applier's input from `peers.toml`. It refuses while the hub's private key is still
/// in the HQ user's directory, and while it would drop a device nobody revoked here.
pub fn apply(store: &Store, spool_dir: &Path, out: &mut dyn Write) -> Result<()> {
    write_applier_input(store, spool_dir, out, true, Guard::Enforce)
}

pub fn peer_add(
    store: &Store,
    spool_dir: &Path,
    request: &AddRequest<'_>,
    out: &mut dyn Write,
) -> Result<()> {
    let net = store.require_net()?;
    if store.hub_key_staged() {
        return Err(staged_key_error(store));
    }
    let mut peers = store.load_peers()?;
    let retired = store.load_retired()?;
    let record = add_peer(
        &net,
        &mut peers,
        &retired,
        NewPeer {
            label: request.label,
            public_key: request.public_key,
            kind: request.kind,
            preshared: request.preshared,
            confirm_public: request.confirm_public,
        },
    )?;
    // Checked before anything is saved, so a missing server address or an input that would drop
    // devices nobody revoked leaves no half-added device and no changed file.
    let description = device::describe(&net, &record, request.with_qr)?;
    let checked = checked_input(
        &net,
        &peers,
        &retired,
        &net.spool_file(spool_dir),
        Guard::Enforce,
    )?;
    say_warnings(out, &checked.warnings)?;
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
                "The device is saved in peers.toml, but it is NOT yet on the hub, because the applier's input was not written: {e}\nFix that, then run: hq net apply\n"
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
            "Revoking \"{}\" ({}, {}). This changes {}/peers.toml, {}/{RETIRED_FILE} and {}. The address is retired and will not be reused by hq net.\n",
            record.label,
            record.kind.name(),
            record.address,
            store.dir().display(),
            store.dir().display(),
            net.spool_file(spool_dir).display()
        ),
    )?;
    // A damaged retired file must not stop a device being cut off: it is rebuilt from what can be read.
    if store.retire_lenient(&record.address)? {
        say_warnings(out, &[retired_damaged_warning(store)])?;
    }
    store.save_peers(&peers)?;
    // Revoking never waits for the hub key to be moved, and never stops for an entry the hub has
    // that nobody here can name: a lost device must be cut off now.
    if let Err(e) = write_applier_input(store, spool_dir, out, false, Guard::Warn) {
        return Err(NetError(format!(
            "\"{}\" is removed from peers.toml but is NOT yet removed from the hub, so it can still connect: {e}\nFix that, then run: hq net apply",
            record.label
        )));
    }
    say(
        out,
        "If this device was lost or stolen, also sign it out of the web UI and rotate any token it held. Check that it is gone with: sudo wg show hq0 peers\n",
    )
}

/// Retires an address that no device in `peers.toml` owns and removes whatever the hub has on it.
/// The way out when the hub has an entry nobody can name (one a writer put straight into the
/// applier's input, or a device lost with a restored `peers.toml`), which `peer revoke` cannot
/// reach because it needs a label.
pub fn peer_forget(
    store: &Store,
    spool_dir: &Path,
    address: &str,
    out: &mut dyn Write,
) -> Result<()> {
    let net = store.require_net()?;
    let subnet = net.subnet()?;
    let addr = crate::subnet::parse_canonical_ipv4(address)
        .filter(|a| subnet.is_usable_host(*a) && *a != net.hub_addr().unwrap_or(*a))
        .ok_or_else(|| {
            NetError(format!(
                "that is not a device address in {}; give an address like {}",
                net.subnet, "10.77.0.9"
            ))
        })?;
    let peers = store.load_peers()?;
    if let Some(owner) = peers.peer.iter().find(|p| p.address == addr.to_string()) {
        return crate::store::fail(format!(
            "{addr} belongs to the device \"{}\"; use `hq net peer revoke {}` for it",
            owner.label, owner.label
        ));
    }
    say(
        out,
        &format!(
            "Forgetting {addr}. This changes {}/{RETIRED_FILE} and {}: the address is retired and anything the hub has on it is removed.\n",
            store.dir().display(),
            net.spool_file(spool_dir).display()
        ),
    )?;
    if store.retire_lenient(&addr.to_string())? {
        say_warnings(out, &[retired_damaged_warning(store)])?;
    }
    if let Err(e) = write_applier_input(store, spool_dir, out, false, Guard::Warn) {
        return Err(NetError(format!(
            "{addr} is retired but is NOT yet removed from the hub: {e}\nFix that, then run: hq net apply"
        )));
    }
    say(out, "Check that it is gone with: sudo wg show hq0 peers\n")
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
        store.load_retired()?.len()
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
            confirm_public: false,
        }
    }

    fn run_add(f: &Fixture, label: &str, kind: Kind) -> (String, String) {
        let key = crate::store::tests::device_key();
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
        assert!(out.contains("saved in peers.toml, but it is NOT yet on the hub"));
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
        // A peers.toml that exists and lists nobody because everyone was revoked is a real list.
        let g = fixture();
        run_add(&g, "mac", Kind::Laptop);
        peer_revoke(&g.store, &g.spool, "mac", &mut Vec::new()).unwrap();
        assert!(applier_input(&g).contains("# hq-net end 0"));
    }

    #[test]
    fn an_emptied_or_damaged_peers_file_cannot_remove_devices_nobody_revoked() {
        let f = fixture();
        run_add(&f, "mac", Kind::Laptop);
        run_add(&f, "pixel", Kind::Phone);
        let before = applier_input(&f);
        let path = f.store.dir().join("peers.toml");
        // A zero byte file, as a crash or a full disk leaves it, reads as no devices.
        std::fs::write(&path, "").unwrap();
        for attempt in [
            apply(&f.store, &f.spool, &mut Vec::new()),
            // Adding to an emptied list must not quietly drop the two devices the hub has.
            peer_add(
                &f.store,
                &f.spool,
                &request("new", &crate::store::tests::device_key(), Kind::Laptop),
                &mut Vec::new(),
            ),
        ] {
            let err = attempt.unwrap_err().to_string();
            assert!(
                err.contains("10.77.0.2") && err.contains("10.77.0.3"),
                "{err}"
            );
            assert!(err.contains("none of them was revoked here"), "{err}");
            assert!(
                err.contains("hq net peer forget 10.77.0.2"),
                "the way out is named: {err}"
            );
            assert_eq!(
                std::fs::read(&path).unwrap(),
                b"",
                "a refused add must not write peers.toml"
            );
            assert_eq!(
                applier_input(&f),
                before,
                "the applier's input must not change"
            );
        }
        // A typo in the table name is an error, not an empty list.
        std::fs::write(&path, "[[peers]]\nlabel = \"mac\"\n").unwrap();
        assert!(apply(&f.store, &f.spool, &mut Vec::new()).is_err());
    }

    #[test]
    fn revoking_one_device_may_remove_exactly_that_device() {
        let f = fixture();
        run_add(&f, "mac", Kind::Laptop);
        run_add(&f, "pixel", Kind::Phone);
        peer_revoke(&f.store, &f.spool, "mac", &mut Vec::new()).unwrap();
        assert!(!applier_input(&f).contains("# mac"));
        assert_eq!(f.store.load_retired().unwrap(), vec!["10.77.0.2"]);
    }

    #[test]
    fn a_lost_retired_record_still_cannot_hand_out_an_address_the_hub_has_removed_by_mistake() {
        // The retired list lives in its own file, so damage to peers.toml cannot take it along.
        let f = fixture();
        run_add(&f, "mac", Kind::Laptop);
        peer_revoke(&f.store, &f.spool, "mac", &mut Vec::new()).unwrap();
        std::fs::write(f.store.dir().join("peers.toml"), "").unwrap();
        let (added, _) = run_add(&f, "tablet", Kind::Phone);
        assert!(
            added.contains("at 10.77.0.3."),
            "10.77.0.2 stays retired: {added}"
        );
    }

    #[test]
    fn a_private_key_pasted_as_the_public_key_is_refused_with_the_way_out() {
        let f = fixture();
        let private = PrivateKey::generate().reveal();
        let err = peer_add(
            &f.store,
            &f.spool,
            &request("mac", &private, Kind::Laptop),
            &mut Vec::new(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("PRIVATE key") && err.contains("--confirm-public"),
            "{err}"
        );
        assert!(f.store.load_peers().unwrap().peer.is_empty());
        assert!(!f.spool.join("hq0.conf").exists());
        let mut out = Vec::new();
        let confirm = AddRequest {
            confirm_public: true,
            ..request("mac", &private, Kind::Laptop)
        };
        peer_add(&f.store, &f.spool, &confirm, &mut out).unwrap();
        assert_eq!(f.store.load_peers().unwrap().peer.len(), 1);
    }

    #[test]
    fn adding_and_applying_wait_for_the_hub_key_to_leave_the_users_directory() {
        let f = fixture();
        run_add(&f, "mac", Kind::Laptop);
        std::fs::write(f.store.hub_key_path(), "staged\n").unwrap();
        let key = crate::store::tests::device_key();
        let err = peer_add(
            &f.store,
            &f.spool,
            &request("pixel", &key, Kind::Phone),
            &mut Vec::new(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("hub's private key is still at") && err.contains("step 1"),
            "{err}"
        );
        assert_eq!(
            f.store.load_peers().unwrap().peer.len(),
            1,
            "nothing was added"
        );
        assert!(apply(&f.store, &f.spool, &mut Vec::new()).is_err());
        // Cutting a device off never waits for that.
        run_add_after_key_moved(&f);
    }

    fn run_add_after_key_moved(f: &Fixture) {
        peer_revoke(&f.store, &f.spool, "mac", &mut Vec::new()).unwrap();
        assert!(!applier_input(f).contains("# mac"));
        std::fs::remove_file(f.store.hub_key_path()).unwrap();
        assert!(apply(&f.store, &f.spool, &mut Vec::new()).is_ok());
    }

    #[test]
    fn a_revoke_that_cannot_reach_the_applier_says_plainly_the_device_is_still_on_the_hub() {
        let f = fixture();
        run_add(&f, "mac", Kind::Laptop);
        std::fs::set_permissions(&f.spool, std::fs::Permissions::from_mode(0o777)).unwrap();
        let err = peer_revoke(&f.store, &f.spool, "mac", &mut Vec::new())
            .unwrap_err()
            .to_string();
        assert!(err.contains("NOT yet removed from the hub"), "{err}");
        assert!(
            err.contains("can still connect") && err.contains("hq net apply"),
            "{err}"
        );
        // peers.toml and the retired record are already updated, so `hq net apply` finishes the job.
        assert!(f.store.load_peers().unwrap().peer.is_empty());
        assert_eq!(f.store.load_retired().unwrap(), vec!["10.77.0.2"]);
        std::fs::set_permissions(&f.spool, std::fs::Permissions::from_mode(0o700)).unwrap();
        apply(&f.store, &f.spool, &mut Vec::new()).unwrap();
        assert!(applier_input(&f).contains("# hq-net end 0"));
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

    /// A peer the hub has that no device in peers.toml owns: someone wrote it straight into the
    /// applier's input.
    fn put_rogue_peer_in_the_spool(f: &Fixture) -> String {
        let net = f.store.require_net().unwrap();
        let current = applier_input(f);
        let rogue_key = crate::store::tests::device_key();
        let mut peers = crate::grammar::parse(current.as_bytes(), &net.policy().unwrap()).unwrap();
        peers.push(crate::grammar::Peer {
            public_key: rogue_key.clone(),
            address: "10.77.0.77".parse().unwrap(),
            preshared_key: None,
        });
        let path = f.spool.join("hq0.conf");
        std::fs::remove_file(&path).unwrap();
        crate::files::write_private_atomic(&path, &crate::grammar::render(&peers)).unwrap();
        rogue_key
    }

    #[test]
    fn a_rogue_peer_in_the_spool_blocks_apply_and_add_but_never_a_revoke_and_forget_removes_it() {
        let f = fixture();
        run_add(&f, "mac", Kind::Laptop);
        run_add(&f, "pixel", Kind::Phone);
        let rogue = put_rogue_peer_in_the_spool(&f);
        let key = crate::store::tests::device_key();
        for refused in [
            apply(&f.store, &f.spool, &mut Vec::new()),
            peer_add(
                &f.store,
                &f.spool,
                &request("tablet", &key, Kind::Phone),
                &mut Vec::new(),
            ),
        ] {
            let err = refused.unwrap_err().to_string();
            assert!(
                err.contains("10.77.0.77") && err.contains("hq net peer forget 10.77.0.77"),
                "{err}"
            );
        }
        assert!(
            applier_input(&f).contains(&rogue),
            "a refusal leaves the input as it was"
        );
        // Revoking a lost laptop goes through even though the hub has an entry nobody can name; the
        // unknown entry is dropped with it and the output says so.
        let mut out = Vec::new();
        peer_revoke(&f.store, &f.spool, "mac", &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(
            out.contains("Warning: this also removes 10.77.0.77"),
            "{out}"
        );
        let after = applier_input(&f);
        assert!(
            !after.contains("# mac") && !after.contains(&rogue),
            "{after}"
        );
        assert!(after.contains("# pixel"));
    }

    #[test]
    fn forget_retires_an_address_nobody_owns_and_removes_it_and_is_the_way_out_of_the_guard() {
        let f = fixture();
        run_add(&f, "mac", Kind::Laptop);
        let rogue = put_rogue_peer_in_the_spool(&f);
        let mut out = Vec::new();
        peer_forget(&f.store, &f.spool, "10.77.0.77", &mut out).unwrap();
        assert!(!applier_input(&f).contains(&rogue));
        assert!(applier_input(&f).contains("# mac"), "real devices stay");
        assert!(
            f.store
                .load_retired()
                .unwrap()
                .contains(&"10.77.0.77".to_string())
        );
        assert!(apply(&f.store, &f.spool, &mut Vec::new()).is_ok());
        // Not for a device that exists, the hub's own address, or anything outside the subnet.
        for bad in [
            "10.77.0.2",
            "10.77.0.1",
            "10.99.0.5",
            "10.77.0.0",
            "nope",
            "10.77.0.077",
        ] {
            assert!(
                peer_forget(&f.store, &f.spool, bad, &mut Vec::new()).is_err(),
                "{bad}"
            );
        }
        let owned = peer_forget(&f.store, &f.spool, "10.77.0.2", &mut Vec::new())
            .unwrap_err()
            .to_string();
        assert!(owned.contains("peer revoke mac"), "{owned}");
    }

    #[test]
    fn a_peers_file_restored_from_an_older_copy_is_stopped_and_forget_is_the_owners_choice() {
        let f = fixture();
        run_add(&f, "mac", Kind::Laptop);
        let backup = std::fs::read_to_string(f.store.dir().join("peers.toml")).unwrap();
        run_add(&f, "pixel", Kind::Phone);
        let before = applier_input(&f);
        // The older copy knows only mac; the hub has pixel as well.
        crate::files::write_private_atomic(&f.store.dir().join("peers.toml"), &backup).unwrap();
        let err = apply(&f.store, &f.spool, &mut Vec::new())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("10.77.0.3") && err.contains("restore the newer one"),
            "{err}"
        );
        assert_eq!(applier_input(&f), before);
        peer_forget(&f.store, &f.spool, "10.77.0.3", &mut Vec::new()).unwrap();
        assert!(!applier_input(&f).contains("# pixel"));
    }

    #[test]
    fn a_damaged_retired_file_does_not_stop_a_revoke_and_is_rebuilt_with_a_warning() {
        let f = fixture();
        run_add(&f, "mac", Kind::Laptop);
        run_add(&f, "pixel", Kind::Phone);
        crate::files::write_private_atomic(
            &f.store.dir().join(crate::store::RETIRED_FILE),
            "10.77.0.9\ngarbage\n",
        )
        .unwrap();
        let key = crate::store::tests::device_key();
        let add_err = peer_add(
            &f.store,
            &f.spool,
            &request("x", &key, Kind::Phone),
            &mut Vec::new(),
        );
        assert!(
            add_err.is_err(),
            "adding with an unreadable retired record is still refused"
        );
        let mut out = Vec::new();
        peer_revoke(&f.store, &f.spool, "mac", &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("was damaged; it was rebuilt"), "{out}");
        assert!(!applier_input(&f).contains("# mac"));
        let retired = f.store.load_retired().unwrap();
        assert_eq!(retired, vec!["10.77.0.9", "10.77.0.2"]);
    }

    #[test]
    fn an_unreadable_or_invalid_spool_file_is_a_warning_not_a_refusal() {
        let f = fixture();
        run_add(&f, "mac", Kind::Laptop);
        std::fs::write(f.spool.join("hq0.conf"), "not a peers file\n").unwrap();
        std::fs::set_permissions(
            f.spool.join("hq0.conf"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let mut out = Vec::new();
        apply(&f.store, &f.spool, &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(
            out.contains("Warning: the applier's current input is not a valid peers file"),
            "{out}"
        );
        assert!(applier_input(&f).contains("# mac"), "it was healed");
        // A copy other users can read cannot be trusted or read at all; that is warned about too.
        std::fs::set_permissions(
            f.spool.join("hq0.conf"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        let mut out = Vec::new();
        apply(&f.store, &f.spool, &mut out).unwrap();
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("Warning: the applier's current input could not be read")
        );
    }

    #[test]
    fn the_device_config_output_tells_the_owner_how_to_check_the_hub_key() {
        let f = fixture();
        let (out, _) = run_add(&f, "mac", Kind::Laptop);
        let net = f.store.require_net().unwrap();
        assert!(out.contains("sudo wg show hq0 public-key"), "{out}");
        assert!(out.contains(&net.hub_public_key));
    }
}
