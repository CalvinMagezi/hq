//! The commands under `hq net` after `init`. Each one says what it is about to change, then
//! changes it, so the owner reads the intent first. Output goes to a writer so tests can read it.

use crate::device;
use crate::files;
use crate::status;
use crate::store::{
    Kind, NetError, NewPeer, RETIRED_FILE, Result, Store, add_link, add_peer, remove_link,
    render_applier_input, revoke_peer,
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
    /// Seconds between the device's keepalive packets; see `PeerRecord::keepalive`.
    pub keepalive: Option<u16>,
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
        Ok(Some(current)) => match crate::grammar::parse_file(current.as_bytes(), &net.policy()?) {
            Err(e) => warnings.push(format!(
                "the applier's current input is not a valid peers file ({e}), so I cannot check whether this drops a device the hub has"
            )),
            Ok(current) => {
                // A version 1 file says nothing about forwarding, so the applier would leave the
                // pairs of the version 2 file it replaces open.
                let open_pairs = current.links.as_ref().map_or(0, Vec::len);
                if open_pairs > 0 && !peers.uses_links() {
                    let why = format!(
                        "the hub's current input links {open_pairs} pair(s) of devices, and this one is version 1, which the applier does not read as \"no links\": those pairs would stay open. peers.toml has lost its links (restored from an older copy?). Restore the newer one, or run `hq net link remove` for each pair after adding them back"
                    );
                    if guard == Guard::Enforce {
                        return crate::store::fail(format!("refusing to write the applier's input: {why}"));
                    }
                    warnings.push(why);
                }
                let dropped = removals_without_a_revoke(&current.peers, &peers.peer, retired);
                if !dropped.is_empty() && guard == Guard::Enforce {
                    return crate::store::fail(guard_refusal(&dropped, &peers.peer));
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

/// Loads `peers.toml` and prints anything the load had to tell the owner (a migration that met a
/// damaged `retired` file).
fn load_peers_noted(store: &Store, out: &mut dyn Write) -> Result<crate::store::PeersFile> {
    let mut peers = store.load_peers()?;
    say_warnings(out, &std::mem::take(&mut peers.notices))?;
    Ok(peers)
}

/// The refusal for an input that would drop devices nobody revoked here, with the way out for each
/// address: `forget` for one nobody owns, and for one a device in `peers.toml` owns (under another
/// key than the hub has) correcting `peers.toml` or revoking that device, because `forget` refuses
/// an address a record owns.
fn guard_refusal(dropped: &[String], records: &[crate::store::PeerRecord]) -> String {
    let (mut unowned, mut owned) = (Vec::new(), Vec::new());
    for address in dropped {
        match records.iter().find(|r| r.address == *address) {
            Some(owner) => owned.push(format!(
                "{address} is the address of \"{0}\" in peers.toml, but the hub has another key there: correct peers.toml, or run `hq net peer revoke {0}`",
                owner.label
            )),
            None => unowned.push(format!("`hq net peer forget {address}`")),
        }
    }
    let mut text = format!(
        "refusing to write the applier's input: it would remove {} from the hub, and none of them was revoked here. If peers.toml was damaged or restored from an older copy, restore the newer one.",
        dropped.join(", ")
    );
    if !unowned.is_empty() {
        text.push_str(&format!(
            " To remove entries no device owns, run {} and then `hq net apply`.",
            unowned.join(" and ")
        ));
    }
    for line in owned {
        text.push_str(&format!(" {line}."));
    }
    text
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
    let peers = load_peers_noted(store, out)?;
    let target = net.spool_file(spool_dir);
    let (retired, damaged) = if guard == Guard::Enforce {
        (store.load_retired()?, false)
    } else {
        let read = store.read_retired_salvage();
        (read.addresses, read.damaged)
    };
    let checked = checked_input(&net, &peers, &retired, &target, guard)?;
    if damaged {
        say_warnings(
            out,
            &["the retired record could not be read in full, so the check below may miss an address revoked earlier".to_string()],
        )?;
    }
    say_warnings(out, &checked.warnings)?;
    let links = if peers.uses_links() {
        format!(" and {} link(s)", peers.link.len())
    } else {
        String::new()
    };
    say(
        out,
        &format!(
            "Writing {} device(s){links} to {} for the applier.\n",
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
    let mut peers = load_peers_noted(store, out)?;
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
            keepalive: request.keepalive,
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
    let mut peers = load_peers_noted(store, out)?;
    let (record, dropped_links) = revoke_peer(&mut peers, label)?;
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
    for link in &dropped_links {
        say(
            out,
            &format!(
                "Its link with \"{}\" goes with it.\n",
                if link.a == record.label {
                    &link.b
                } else {
                    &link.a
                }
            ),
        )?;
    }
    // A damaged retired file must not stop a device being cut off: it is rebuilt from what can be read.
    // Recording the address as retired must not stop a device being cut off. If it cannot be
    // recorded, the revoke goes on and says so; `peer add` reads the record strictly and refuses
    // while it is damaged, and `hq net peer forget` records the address once the cause is fixed.
    match store.retire_lenient(&record.address) {
        Ok(outcome) if outcome.damaged => {
            say_warnings(out, &[store.damaged_notice(&outcome)])?;
        }
        Ok(_) => {}
        Err(e) => say_warnings(
            out,
            &[format!(
                "{} could NOT be recorded as retired ({e}). The revoke goes on, because a lost device must be cut off. Do not add devices until that is fixed; then run `hq net peer forget {}` to record it",
                record.address, record.address
            )],
        )?,
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
    let peers = load_peers_noted(store, out)?;
    if let Some(owner) = peers.peer.iter().find(|p| p.address == addr.to_string()) {
        return crate::store::fail(format!(
            "{addr} belongs to the device \"{0}\" in peers.toml; use `hq net peer revoke {0}` for it. If the hub has a different key at that address than peers.toml records for it, correct peers.toml first, or revoke {0}, which retires the address",
            owner.label
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
    let outcome = store.retire_lenient(&addr.to_string())?;
    if outcome.damaged {
        say_warnings(out, &[store.damaged_notice(&outcome)])?;
    }
    // Enforce: each entry the hub has that nobody can name needs a forget of its own. The address
    // above is retired already, so it is not what stops this.
    if let Err(e) = write_applier_input(store, spool_dir, out, false, Guard::Enforce) {
        return Err(NetError(format!(
            "{addr} is retired but is NOT yet removed from the hub: {e}\nFix that, then run: hq net apply"
        )));
    }
    say(out, "Check that it is gone with: sudo wg show hq0 peers\n")
}

/// What a device must add to the tunnel it already has so that it can reach the devices linked to
/// it: their addresses next to the hub's, as the one `AllowedIPs` line the WireGuard app shows.
fn allowed_ips_line(hub: &str, linked: &[String]) -> String {
    let mut all = vec![format!("{hub}/32")];
    all.extend(linked.iter().map(|a| format!("{a}/32")));
    all.join(", ")
}

fn device_lines(
    net: &crate::store::Net,
    peers: &crate::store::PeersFile,
    labels: [&str; 2],
) -> String {
    let mut text = String::new();
    for label in labels {
        let Some(record) = peers.peer.iter().find(|p| p.label == label) else {
            continue;
        };
        let linked = peers.linked_addresses(label);
        text.push_str(&format!(
            "  {label} ({}): AllowedIPs = {}\n",
            record.address,
            allowed_ips_line(&net.hub, &linked)
        ));
    }
    text
}

/// Joins two devices so the hub forwards between them. Every other pair of devices stays unable to
/// reach each other.
///
/// The first link turns the applier's input into a version 2 file, which an applier built before
/// links existed refuses whole, a revocation included. So the first link needs the owner to say the
/// applier on the hub is new enough (`applier_updated`).
pub fn link_add(
    store: &Store,
    spool_dir: &Path,
    a: &str,
    b: &str,
    applier_updated: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let net = store.require_net()?;
    let mut peers = load_peers_noted(store, out)?;
    if !peers.links_enabled && !applier_updated {
        return crate::store::fail(
            "the first link makes the applier's input a version 2 file, and an applier built before links existed refuses a version 2 file whole, so even a revocation would not reach the hub. Build and install the new applier, load the firewall set (docs/PRIVATE_NETWORK.md, \"Linking two devices\"), then run this again with --applier-updated",
        );
    }
    let record = add_link(&mut peers, a, b)?;
    let retired = store.load_retired()?;
    // Checked before anything is saved, as `peer add` does.
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
            "Linking \"{}\" and \"{}\". This changes {}/peers.toml and {} (the applier's input), which becomes a version 2 file that only an applier built with links accepts. The hub then forwards between those two devices, in both directions and on every port, and between no other pair.\n",
            record.a,
            record.b,
            store.dir().display(),
            net.spool_file(spool_dir).display()
        ),
    )?;
    store.save_peers(&peers)?;
    if let Err(e) = apply(store, spool_dir, out) {
        say(
            out,
            &format!(
                "The link is saved in peers.toml, but it is NOT yet on the hub, because the applier's input was not written: {e}\nFix that, then run: hq net apply\n"
            ),
        )?;
    }
    say(
        out,
        &format!(
            "Each device must also route the other's address into its tunnel. In the WireGuard app, change the hub peer's allowed addresses to:\n{}The hub needs the firewall set and an applier that knows links first: see docs/PRIVATE_NETWORK.md, \"Linking two devices\". A device that is behind a mobile network also needs PersistentKeepalive = 25 to stay reachable (set \"Persistent keepalive\" in its app; a device added with `--keepalive 25` already has it).\n",
            device_lines(&net, &peers, [&record.a, &record.b])
        ),
    )
}

/// Takes a link away; the two devices can no longer reach each other through the hub.
pub fn link_remove(
    store: &Store,
    spool_dir: &Path,
    a: &str,
    b: &str,
    out: &mut dyn Write,
) -> Result<()> {
    let net = store.require_net()?;
    let mut peers = load_peers_noted(store, out)?;
    let record = remove_link(&mut peers, a, b)?;
    say(
        out,
        &format!(
            "Unlinking \"{}\" and \"{}\". This changes {}/peers.toml and {}.\n",
            record.a,
            record.b,
            store.dir().display(),
            net.spool_file(spool_dir).display()
        ),
    )?;
    store.save_peers(&peers)?;
    // A pair is closed on the hub whether or not the guard would have objected to something else.
    if let Err(e) = write_applier_input(store, spool_dir, out, false, Guard::Warn) {
        return Err(NetError(format!(
            "the link is removed from peers.toml but is NOT yet closed on the hub: {e}\nFix that, then run: hq net apply"
        )));
    }
    say(
        out,
        &format!(
            "You can take the other device's address out of each device's AllowedIPs; it no longer routes anywhere useful:\n{}",
            device_lines(&net, &peers, [&record.a, &record.b])
        ),
    )
}

pub fn link_list(store: &Store, out: &mut dyn Write) -> Result<()> {
    let peers = load_peers_noted(store, out)?;
    let address = |label: &str| {
        peers
            .peer
            .iter()
            .find(|p| p.label == label)
            .map_or("?", |p| p.address.as_str())
            .to_string()
    };
    let mut text = format!(
        "{:<16} {:<12} {:<16} {}\n",
        "DEVICE", "ADDRESS", "DEVICE", "ADDRESS"
    );
    for link in &peers.link {
        text.push_str(&format!(
            "{:<16} {:<12} {:<16} {}\n",
            link.a,
            address(&link.a),
            link.b,
            address(&link.b)
        ));
    }
    text.push_str(&format!(
        "{} link(s) in peers.toml. `sudo nft list set inet hq_net links_hq0` shows what the hub allows.\n",
        peers.link.len()
    ));
    say(out, &text)
}

pub fn peer_list(store: &Store, out: &mut dyn Write) -> Result<()> {
    let net = store.require_net()?;
    let peers = load_peers_noted(store, out)?;
    let mut text = format!(
        "{:<16} {:<7} {:<12} {:<10} {:<10} {}\n",
        "DEVICE", "KIND", "ADDRESS", "KEY", "PRESHARED", "KEEPALIVE"
    );
    for p in &peers.peer {
        let short: String = p.public_key.chars().take(FINGERPRINT_CHARS).collect();
        let psk = if p.preshared_key.is_some() {
            "yes"
        } else {
            "no"
        };
        let keepalive = device::keepalive_secs(p).map_or("none".to_string(), |s| format!("{s} s"));
        text.push_str(&format!(
            "{:<16} {:<7} {:<12} {short:<10} {psk:<10} {keepalive}\n",
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
            keepalive: None,
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
                err.contains("hq net peer forget 10.77.0.3"),
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
        let key = crate::store::tests::device_key();
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
        let key = crate::store::tests::device_key();
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
    fn a_keepalive_reaches_the_device_config_and_never_the_applier_input() {
        let f = fixture();
        let key = crate::store::tests::device_key();
        let mut out = Vec::new();
        let req = AddRequest {
            keepalive: Some(25),
            ..request("pixel", &key, Kind::Phone)
        };
        peer_add(&f.store, &f.spool, &req, &mut out).unwrap();
        let printed = String::from_utf8(out).unwrap();
        assert!(printed.contains("PersistentKeepalive = 25"));
        assert!(printed.contains("battery use"));
        let with = applier_input(&f);
        assert!(!with.to_lowercase().contains("keepalive"));
        // The same device without the field gives a byte for byte identical input.
        let mut peers = f.store.load_peers().unwrap();
        peers.peer[0].keepalive = None;
        f.store.save_peers(&peers).unwrap();
        apply(&f.store, &f.spool, &mut Vec::new()).unwrap();
        assert_eq!(applier_input(&f), with);
    }

    #[test]
    fn the_list_shows_each_devices_keepalive() {
        let f = fixture();
        run_add(&f, "mac", Kind::Laptop);
        let key = crate::store::tests::device_key();
        let req = AddRequest {
            keepalive: Some(25),
            ..request("pixel", &key, Kind::Phone)
        };
        peer_add(&f.store, &f.spool, &req, &mut Vec::new()).unwrap();
        run_add(&f, "tablet", Kind::Phone);
        let mut out = Vec::new();
        peer_list(&f.store, &mut out).unwrap();
        let rows: Vec<String> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(String::from)
            .collect();
        let row = |label: &str| rows.iter().find(|l| l.starts_with(label)).unwrap().clone();
        assert!(row("mac").trim_end().ends_with("25 s"));
        assert!(row("pixel").trim_end().ends_with("25 s"));
        assert!(row("tablet").trim_end().ends_with("none"));
    }

    #[test]
    fn the_qr_code_is_printed_only_when_asked() {
        let f = fixture();
        let key = crate::store::tests::device_key();
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

    #[test]
    fn forget_needs_its_own_forget_for_each_unnamed_entry() {
        let f = fixture();
        run_add(&f, "mac", Kind::Laptop);
        let first = put_rogue_peer_in_the_spool(&f);
        // A second unnamed entry, at another address.
        let net = f.store.require_net().unwrap();
        let mut peers =
            crate::grammar::parse(applier_input(&f).as_bytes(), &net.policy().unwrap()).unwrap();
        let second = crate::store::tests::device_key();
        peers.push(crate::grammar::Peer {
            public_key: second.clone(),
            address: "10.77.0.88".parse().unwrap(),
            preshared_key: None,
        });
        let path = f.spool.join("hq0.conf");
        std::fs::remove_file(&path).unwrap();
        crate::files::write_private_atomic(&path, &crate::grammar::render(&peers)).unwrap();

        let err = peer_forget(&f.store, &f.spool, "10.77.0.77", &mut Vec::new())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("is retired but is NOT yet removed from the hub"),
            "{err}"
        );
        assert!(
            err.contains("hq net peer forget 10.77.0.88"),
            "the other entry is named: {err}"
        );
        assert!(
            applier_input(&f).contains(&first),
            "nothing was removed yet"
        );
        peer_forget(&f.store, &f.spool, "10.77.0.88", &mut Vec::new()).unwrap();
        let after = applier_input(&f);
        assert!(
            !after.contains(&first) && !after.contains(&second),
            "both gone: {after}"
        );
        assert!(after.contains("# mac"));
    }

    #[test]
    fn when_the_hub_has_another_key_at_a_devices_address_the_guard_does_not_send_you_to_forget() {
        let f = fixture();
        run_add(&f, "mac", Kind::Laptop);
        let net = f.store.require_net().unwrap();
        // The hub has a different key at mac's address (a restored peers.toml with an old key).
        let other = crate::store::tests::device_key();
        let swapped = crate::grammar::render(&[crate::grammar::Peer {
            public_key: other.clone(),
            address: "10.77.0.2".parse().unwrap(),
            preshared_key: None,
        }]);
        let path = f.spool.join("hq0.conf");
        std::fs::remove_file(&path).unwrap();
        crate::files::write_private_atomic(&path, &swapped).unwrap();
        let _ = net;

        let err = apply(&f.store, &f.spool, &mut Vec::new())
            .unwrap_err()
            .to_string();
        assert!(
            !err.contains("peer forget"),
            "forget would refuse here: {err}"
        );
        assert!(
            err.contains("correct peers.toml, or run `hq net peer revoke mac`"),
            "{err}"
        );
        let refused = peer_forget(&f.store, &f.spool, "10.77.0.2", &mut Vec::new())
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("correct peers.toml first") && refused.contains("revoke mac"),
            "{refused}"
        );
        // The way out the message names works, and the entry on the retired address goes.
        peer_revoke(&f.store, &f.spool, "mac", &mut Vec::new()).unwrap();
        assert!(!applier_input(&f).contains(&other));
    }

    #[test]
    fn a_migration_notice_about_a_damaged_retired_file_is_printed() {
        let f = fixture();
        run_add(&f, "mac", Kind::Laptop);
        crate::files::write_private_atomic(
            &f.store.dir().join(crate::store::RETIRED_FILE),
            "10.77.0.9\njunk\n",
        )
        .unwrap();
        let path = f.store.dir().join("peers.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        crate::files::write_private_atomic(&path, &format!("retired = [\"10.77.0.5\"]\n{text}"))
            .unwrap();
        let mut out = Vec::new();
        peer_list(&f.store, &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(
            out.contains("Warning:") && out.contains("was damaged"),
            "{out}"
        );
        let retired = f.store.load_retired().unwrap();
        assert!(
            retired.contains(&"10.77.0.9".to_string())
                && retired.contains(&"10.77.0.5".to_string())
        );
    }

    #[test]
    fn a_damaged_retired_file_that_cannot_be_set_aside_is_left_alone_and_a_revoke_still_cuts_the_device_off()
     {
        let f = fixture();
        run_add(&f, "mac", Kind::Laptop);
        run_add(&f, "pixel", Kind::Phone);
        let retired_path = f.store.dir().join(crate::store::RETIRED_FILE);
        let damaged = "10.77.0.9\njunk\n";
        crate::files::write_private_atomic(&retired_path, damaged).unwrap();
        // Every name for a copy is taken.
        std::fs::write(f.store.dir().join("retired.damaged"), "x").unwrap();
        for n in 1..100 {
            std::fs::write(f.store.dir().join(format!("retired.damaged.{n}")), "x").unwrap();
        }
        let mut out = Vec::new();
        peer_revoke(&f.store, &f.spool, "mac", &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("could NOT be recorded as retired"), "{out}");
        assert!(out.contains("peer forget 10.77.0.2"), "{out}");
        assert!(
            !applier_input(&f).contains("# mac"),
            "the device was cut off all the same"
        );
        assert_eq!(
            std::fs::read_to_string(&retired_path).unwrap(),
            damaged,
            "the damaged file is untouched"
        );
        // Adding is refused while the record is damaged, so the revoked address cannot be reused.
        let key = crate::store::tests::device_key();
        assert!(
            peer_add(
                &f.store,
                &f.spool,
                &request("tablet", &key, Kind::Phone),
                &mut Vec::new()
            )
            .is_err()
        );
    }

    fn two_devices(f: &Fixture) {
        run_add(f, "mac", Kind::Laptop);
        run_add(f, "phone", Kind::Phone);
    }

    fn linked_in_input(f: &Fixture) -> crate::grammar::Parsed {
        let net = f.store.require_net().unwrap();
        crate::grammar::parse_file(applier_input(f).as_bytes(), &net.policy().unwrap()).unwrap()
    }

    #[test]
    fn until_a_link_is_added_the_applier_input_stays_version_1() {
        let f = fixture();
        two_devices(&f);
        let input = applier_input(&f);
        assert!(input.starts_with(crate::grammar::HEADER), "{input}");
        assert!(!input.contains("[Link]"));
        assert_eq!(linked_in_input(&f).links, None);
    }

    #[test]
    fn linking_two_devices_writes_a_version_2_file_and_says_what_each_device_must_add() {
        let f = fixture();
        two_devices(&f);
        let mut out = Vec::new();
        link_add(&f.store, &f.spool, "phone", "mac", true, &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.starts_with("Linking \"mac\" and \"phone\"."), "{out}");
        assert!(out.contains("version 2"), "{out}");
        assert!(
            out.contains("mac (10.77.0.2): AllowedIPs = 10.77.0.1/32, 10.77.0.3/32"),
            "{out}"
        );
        assert!(
            out.contains("phone (10.77.0.3): AllowedIPs = 10.77.0.1/32, 10.77.0.2/32"),
            "{out}"
        );
        assert!(out.contains("PersistentKeepalive"), "{out}");

        let parsed = linked_in_input(&f);
        assert_eq!(parsed.peers.len(), 2);
        let expected =
            crate::grammar::Link::new("10.77.0.2".parse().unwrap(), "10.77.0.3".parse().unwrap())
                .unwrap();
        assert_eq!(parsed.links, Some(vec![expected]));

        let peers = f.store.load_peers().unwrap();
        assert!(peers.links_enabled);
        assert_eq!(peers.link.len(), 1);
        assert_eq!(
            (peers.link[0].a.as_str(), peers.link[0].b.as_str()),
            ("mac", "phone")
        );
    }

    #[test]
    fn bad_link_requests_are_refused_and_change_nothing() {
        let f = fixture();
        two_devices(&f);
        let before = applier_input(&f);
        for (a, b) in [
            ("mac", "mac"),
            ("mac", "nobody"),
            ("nobody", "phone"),
            ("", ""),
        ] {
            let err = link_add(&f.store, &f.spool, a, b, true, &mut Vec::new()).unwrap_err();
            assert!(!err.to_string().is_empty());
        }
        assert_eq!(applier_input(&f), before);
        assert!(f.store.load_peers().unwrap().link.is_empty());

        link_add(&f.store, &f.spool, "mac", "phone", true, &mut Vec::new()).unwrap();
        let err = link_add(&f.store, &f.spool, "phone", "mac", true, &mut Vec::new()).unwrap_err();
        assert!(err.to_string().contains("linked already"), "{err}");
    }

    #[test]
    fn removing_the_last_link_keeps_version_2_so_the_hub_is_told_to_forward_nothing() {
        let f = fixture();
        two_devices(&f);
        link_add(&f.store, &f.spool, "mac", "phone", true, &mut Vec::new()).unwrap();
        let mut out = Vec::new();
        link_remove(&f.store, &f.spool, "phone", "mac", &mut out).unwrap();
        assert_eq!(linked_in_input(&f).links, Some(vec![]));
        assert!(applier_input(&f).contains(crate::grammar::HEADER_V2));
        assert!(f.store.load_peers().unwrap().link.is_empty());
        let err = link_remove(&f.store, &f.spool, "mac", "phone", &mut Vec::new()).unwrap_err();
        assert!(err.to_string().contains("not linked"), "{err}");
    }

    #[test]
    fn revoking_a_device_takes_its_links_with_it_in_the_same_write() {
        let f = fixture();
        two_devices(&f);
        run_add(&f, "tablet", Kind::Laptop);
        link_add(&f.store, &f.spool, "mac", "phone", true, &mut Vec::new()).unwrap();
        link_add(&f.store, &f.spool, "phone", "tablet", true, &mut Vec::new()).unwrap();
        let mut out = Vec::new();
        peer_revoke(&f.store, &f.spool, "phone", &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("Its link with \"mac\" goes with it."), "{out}");
        assert!(
            out.contains("Its link with \"tablet\" goes with it."),
            "{out}"
        );
        let parsed = linked_in_input(&f);
        assert_eq!(parsed.peers.len(), 2);
        assert_eq!(
            parsed.links,
            Some(vec![]),
            "no pair names the revoked address"
        );
        assert!(f.store.load_peers().unwrap().link.is_empty());
        // The address is retired, so a new device cannot inherit what the old one was linked to.
        let (_, key) = run_add(&f, "newer", Kind::Phone);
        let peers = f.store.load_peers().unwrap();
        let newer = peers.peer.iter().find(|p| p.public_key == key).unwrap();
        assert_ne!(newer.address, "10.77.0.3");
    }

    #[test]
    fn the_link_list_names_each_pair_with_its_addresses() {
        let f = fixture();
        two_devices(&f);
        link_add(&f.store, &f.spool, "mac", "phone", true, &mut Vec::new()).unwrap();
        let mut out = Vec::new();
        link_list(&f.store, &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("mac"), "{out}");
        assert!(
            out.contains("10.77.0.2") && out.contains("10.77.0.3"),
            "{out}"
        );
        assert!(out.contains("1 link(s) in peers.toml."), "{out}");
    }

    #[test]
    fn a_hand_edited_links_table_that_names_nobody_is_refused_on_load() {
        let f = fixture();
        two_devices(&f);
        link_add(&f.store, &f.spool, "mac", "phone", true, &mut Vec::new()).unwrap();
        let path = f.store.dir().join(crate::store::PEERS_FILE);
        let text = std::fs::read_to_string(&path).unwrap();
        for edited in [
            text.replace("b = \"phone\"", "b = \"ghost\""),
            text.replace("b = \"phone\"", "b = \"mac\""),
            format!("{text}\n[[link]]\na = \"phone\"\nb = \"mac\"\n"),
            format!("{text}\n[[link]]\na = \"mac\"\nb = \"phone\"\nextra = 1\n"),
        ] {
            std::fs::write(&path, &edited).unwrap();
            let err = f.store.load_peers().unwrap_err().to_string();
            assert!(!err.contains("ghost"), "{err}");
            assert!(
                err.contains("link") || err.contains("unknown field"),
                "{err}"
            );
        }
    }

    #[test]
    fn the_first_link_needs_the_owner_to_say_the_applier_is_new_enough() {
        let f = fixture();
        two_devices(&f);
        let before = applier_input(&f);
        let err = link_add(&f.store, &f.spool, "mac", "phone", false, &mut Vec::new()).unwrap_err();
        assert!(err.to_string().contains("--applier-updated"), "{err}");
        assert_eq!(applier_input(&f), before, "a refused link changes nothing");
        assert!(!f.store.load_peers().unwrap().links_enabled);
        link_add(&f.store, &f.spool, "mac", "phone", true, &mut Vec::new()).unwrap();
        // Once links are in use the question was answered.
        run_add(&f, "tablet", Kind::Laptop);
        link_add(&f.store, &f.spool, "mac", "tablet", false, &mut Vec::new()).unwrap();
    }

    #[test]
    fn a_peers_file_that_lost_its_links_cannot_replace_one_that_has_them() {
        let f = fixture();
        two_devices(&f);
        link_add(&f.store, &f.spool, "mac", "phone", true, &mut Vec::new()).unwrap();
        let before = applier_input(&f);
        // peers.toml restored from a copy made before the first link.
        let mut peers = f.store.load_peers().unwrap();
        peers.link.clear();
        peers.links_enabled = false;
        f.store.save_peers(&peers).unwrap();
        let err = apply(&f.store, &f.spool, &mut Vec::new()).unwrap_err();
        assert!(
            err.to_string().contains("those pairs would stay open"),
            "{err}"
        );
        assert_eq!(
            applier_input(&f),
            before,
            "the version 2 input is untouched"
        );

        // A revoke is never held up; it says so instead.
        let mut out = Vec::new();
        peer_revoke(&f.store, &f.spool, "phone", &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("those pairs would stay open"), "{out}");
        assert!(!applier_input(&f).contains("# phone"));
    }
}
