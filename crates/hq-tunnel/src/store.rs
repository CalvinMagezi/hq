//! The files a tunnel keeps in one private directory, and the two decisions about them that
//! protect the device: a join text from a hub this device has not seen before has to be confirmed
//! by someone who can see the hub's key, and it takes the serve and reach maps with it.
//!
//! ```text
//! join.conf    the join text as `hq net` wrote it (it may hold a preshared key)
//! maps.toml    the serve and reach entries and the limits; written by the owner, never by import
//! key          the private key, for a desktop run (an app passes it on stdin instead)
//! status.json  what the peer reports; events.log the bounded log beside it
//! ```

use crate::config::{MAX_CONFIG_BYTES, TunnelConfig};
use crate::error::{Error, Result, invalid};
use crate::fsutil::{self, Access};
use crate::join::{JoinConfig, MAX_JOIN_BYTES, parse};
use std::io;
use std::path::{Path, PathBuf};

pub const JOIN_FILE: &str = "join.conf";
pub const MAPS_FILE: &str = "maps.toml";
pub const KEY_FILE: &str = "key";
pub const STATUS_FILE: &str = "status.json";
pub const EVENTS_FILE: &str = "events.log";
/// The status file is a few hundred bytes; this only bounds a file someone else replaced.
const MAX_STATUS_BYTES: u64 = 16 * 1024;

/// What `import` did, or why it did nothing.
#[derive(Debug, PartialEq, Eq)]
pub enum Import {
    /// The join text was saved. `maps_cleared` is set when the hub is a new one and the serve
    /// and reach entries written for the old one were removed.
    Saved {
        hub_key: String,
        endpoint: String,
        same_hub: bool,
        maps_cleared: bool,
    },
    /// Nothing was written. This hub is new to the device and the caller did not name its key.
    NeedsConfirmation { hub_key: String, endpoint: String },
}

pub struct TunnelDir {
    root: PathBuf,
}

impl TunnelDir {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Creates the directory if it is missing. One that exists must be a real directory that
    /// belongs to this user and is closed to others, or it is refused.
    pub fn ensure(&self) -> Result<()> {
        fsutil::ensure_private_dir(&self.root)
            .map_err(|e| Error::Invalid(format!("cannot use {}: {e}", self.root.display())))
    }

    /// The same check without making anything, run before a file in the directory is trusted. A
    /// missing directory passes: the file read after it says what is missing.
    fn verify_dir(&self) -> Result<()> {
        match fsutil::check_private_dir(&self.root) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => invalid(format!("cannot use {}: {e}", self.root.display())),
            Ok(()) => Ok(()),
        }
    }

    pub fn load_join(&self) -> Result<JoinConfig> {
        self.verify_dir()?;
        let path = self.path(JOIN_FILE);
        let bytes =
            fsutil::read_private(&path, MAX_JOIN_BYTES as u64, Access::OwnerOnly).map_err(|e| {
                if e.kind() == io::ErrorKind::NotFound {
                    Error::Invalid(format!(
                        "no join text yet: import one with `hq net tunnel import` ({})",
                        path.display()
                    ))
                } else {
                    Error::Invalid(format!("cannot read {}: {e}", path.display()))
                }
            })?;
        parse(&String::from_utf8_lossy(&bytes))
    }

    /// The serve and reach entries, checked against the join text they will run with. A missing
    /// file is an empty map: the tunnel then answers nothing and forwards nothing.
    pub fn load_maps(&self, join: &JoinConfig) -> Result<TunnelConfig> {
        self.verify_dir()?;
        let path = self.path(MAPS_FILE);
        let config = match fsutil::read_private(&path, MAX_CONFIG_BYTES as u64, Access::Owner) {
            Ok(bytes) => TunnelConfig::from_toml(&String::from_utf8_lossy(&bytes))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => TunnelConfig::default(),
            Err(e) => return invalid(format!("cannot read {}: {e}", path.display())),
        };
        config.validate(join)?;
        Ok(config)
    }

    /// The status file the peer last wrote, as the JSON it holds.
    pub fn read_status(&self) -> Result<serde_json::Value> {
        self.verify_dir()?;
        let path = self.path(STATUS_FILE);
        let bytes = fsutil::read_private(&path, MAX_STATUS_BYTES, Access::Anyone)
            .map_err(|e| Error::Invalid(format!("no status at {}: {e}", path.display())))?;
        serde_json::from_slice(&bytes)
            .map_err(|_| Error::Invalid(format!("{} is not a status file", path.display())))
    }

    /// Takes a join text. The hub's key and endpoint decide who this device talks to, so a hub
    /// the device has not seen has to be confirmed with its key (`confirm_hub`), read off the
    /// hub's own `hq net` output. The same hub again needs no confirmation and keeps the maps. A
    /// key that is named but is not the text's hub key is refused whether or not the hub is new.
    ///
    /// The old maps go first: if that fails nothing has changed, and if the device stops between
    /// the two steps it is left with the old hub and no maps, which serves and reaches nothing.
    pub fn import(&self, text: &str, confirm_hub: Option<&str>) -> Result<Import> {
        if text.len() > MAX_JOIN_BYTES {
            return invalid("the join text is too long");
        }
        self.verify_dir()?;
        let join = parse(text)?;
        let hub_key = hq_net::key::encode(&join.hub_key);
        let endpoint = join.endpoint.to_string();
        if confirm_hub.is_some_and(|named| named != hub_key) {
            return invalid("the key given with --confirm-hub is not the hub key in the join text");
        }
        // A join text already here that cannot be read counts as no hub at all.
        let same_hub = self.load_join().is_ok_and(|old| same_hub(&old, &join));
        if !same_hub && confirm_hub.is_none() {
            return Ok(Import::NeedsConfirmation { hub_key, endpoint });
        }
        self.ensure()?;
        let maps_cleared = !same_hub && self.clear_maps()?;
        fsutil::write_private_atomic(&self.path(JOIN_FILE), text.as_bytes())?;
        Ok(Import::Saved {
            hub_key,
            endpoint,
            same_hub,
            maps_cleared,
        })
    }

    /// Removes the maps file. Returns whether there was one.
    fn clear_maps(&self) -> Result<bool> {
        match std::fs::remove_file(self.path(MAPS_FILE)) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// Makes a private key in the directory's `key` file, which must not exist, and returns the
    /// public half.
    #[cfg(feature = "keygen")]
    pub fn keygen_to_file(&self) -> Result<String> {
        use std::io::Write;
        self.ensure()?;
        let (private, public) = keygen_pair();
        let path = self.path(KEY_FILE);
        let mut file = fsutil::create_new_private(&path).map_err(|e| {
            if e.kind() == io::ErrorKind::AlreadyExists {
                Error::Invalid(format!(
                    "there is already a key at {}; remove it to make a new one",
                    path.display()
                ))
            } else {
                e.into()
            }
        })?;
        file.write_all(private.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(public)
    }
}

/// The same hub: the same key at the same host and port (a host name has no case).
fn same_hub(a: &JoinConfig, b: &JoinConfig) -> bool {
    a.hub_key == b.hub_key
        && a.endpoint.port() == b.endpoint.port()
        && a.endpoint.host().eq_ignore_ascii_case(b.endpoint.host())
}

/// A new private key as text, wiped when dropped, and its public half. A key whose public half
/// happens to have the shape of a private key (one in sixteen) is skipped: `hq net peer add` refuses
/// such a key without `--confirm-public`, and a person who has just made a key should not meet
/// that.
#[cfg(feature = "keygen")]
pub fn keygen_pair() -> (zeroize::Zeroizing<String>, String) {
    loop {
        let key = hq_net::key::PrivateKey::generate();
        let public = key.public();
        if !hq_net::key::looks_like_private_key(&public) {
            return (zeroize::Zeroizing::new(key.reveal()), public);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_net::key::encode;
    use std::os::unix::fs::PermissionsExt;

    fn join_text(hub: u8, endpoint: &str, psk: Option<u8>) -> String {
        let psk = psk
            .map(|p| format!("PresharedKey = {}\n", encode(&[p; 32])))
            .unwrap_or_default();
        format!(
            "[Interface]\nAddress = 10.77.0.4/32\n\n[Peer]\nPublicKey = {}\n{psk}Endpoint = {endpoint}\nAllowedIPs = 10.77.0.1/32\n",
            encode(&[hub; 32])
        )
    }

    fn dir() -> (tempfile::TempDir, TunnelDir) {
        let root = tempfile::tempdir().unwrap();
        let tunnel = TunnelDir::new(root.path().join("tunnel"));
        (root, tunnel)
    }

    const MAPS: &str = "[[serve]]\nport = 8443\nto = \"127.0.0.1:9000\"\nfrom = [\"10.77.0.1\"]\n";

    fn write_maps(tunnel: &TunnelDir) {
        std::fs::write(tunnel.path(MAPS_FILE), MAPS).unwrap();
    }

    fn saved(outcome: Import) -> (bool, bool) {
        match outcome {
            Import::Saved {
                same_hub,
                maps_cleared,
                ..
            } => (same_hub, maps_cleared),
            other => panic!("expected a save, got {other:?}"),
        }
    }

    #[test]
    fn a_first_hub_is_not_taken_until_its_key_is_named() {
        let (_root, tunnel) = dir();
        let text = join_text(3, "vpn.example.com:51820", None);
        let outcome = tunnel.import(&text, None).unwrap();
        assert_eq!(
            outcome,
            Import::NeedsConfirmation {
                hub_key: encode(&[3; 32]),
                endpoint: "vpn.example.com:51820".into()
            }
        );
        assert!(!tunnel.root().exists(), "nothing was written");
        let wrong = tunnel.import(&text, Some(&encode(&[4; 32]))).unwrap_err();
        assert!(wrong.to_string().contains("--confirm-hub"), "{wrong}");
        assert!(!tunnel.root().exists(), "a wrong key writes nothing");
        let (same, cleared) = saved(tunnel.import(&text, Some(&encode(&[3; 32]))).unwrap());
        assert!(!same && !cleared);
        assert_eq!(tunnel.load_join().unwrap().address.to_string(), "10.77.0.4");
    }

    #[test]
    fn the_same_hub_again_keeps_the_maps_and_needs_no_confirmation() {
        let (_root, tunnel) = dir();
        let text = join_text(3, "vpn.example.com:51820", None);
        tunnel.import(&text, Some(&encode(&[3; 32]))).unwrap();
        write_maps(&tunnel);
        // Same key; the host differs only in case; a new preshared key and address come with it.
        let again =
            join_text(3, "VPN.Example.COM:51820", Some(9)).replace("10.77.0.4", "10.77.0.5");
        let (same, cleared) = saved(tunnel.import(&again, None).unwrap());
        assert!(same && !cleared);
        assert!(tunnel.path(MAPS_FILE).exists(), "the maps stayed");
        let join = tunnel.load_join().unwrap();
        assert_eq!(join.address.to_string(), "10.77.0.5");
        assert!(join.preshared_key.is_some());
    }

    #[test]
    fn another_hub_key_or_endpoint_needs_its_key_and_takes_the_maps_away() {
        for other in [
            join_text(4, "vpn.example.com:51820", None),
            join_text(3, "vpn.example.com:51821", None),
            join_text(3, "other.example.com:51820", None),
            join_text(3, "192.0.2.7:51820", None),
        ] {
            let (_root, tunnel) = dir();
            let first = join_text(3, "vpn.example.com:51820", None);
            tunnel.import(&first, Some(&encode(&[3; 32]))).unwrap();
            write_maps(&tunnel);
            let hub = hq_net::key::decode(
                other
                    .lines()
                    .find_map(|l| l.strip_prefix("PublicKey = "))
                    .unwrap(),
            )
            .unwrap();
            // Without the key nothing changes, not even the maps.
            assert!(matches!(
                tunnel.import(&other, None).unwrap(),
                Import::NeedsConfirmation { .. }
            ));
            assert!(tunnel.path(MAPS_FILE).exists());
            assert!(
                tunnel.load_join().unwrap().endpoint.to_string() == "vpn.example.com:51820",
                "the old join text is still in force"
            );
            let (same, cleared) = saved(tunnel.import(&other, Some(&encode(&hub))).unwrap());
            assert!(!same && cleared, "{other}");
            assert!(
                !tunnel.path(MAPS_FILE).exists(),
                "the maps went with the hub"
            );
            let join = tunnel.load_join().unwrap();
            assert!(tunnel.load_maps(&join).unwrap().serve.is_empty());
        }
    }

    #[test]
    fn a_wrong_key_is_refused_even_for_the_hub_already_known() {
        let (_root, tunnel) = dir();
        let text = join_text(3, "vpn.example.com:51820", None);
        tunnel.import(&text, Some(&encode(&[3; 32]))).unwrap();
        let err = tunnel.import(&text, Some(&encode(&[9; 32]))).unwrap_err();
        assert!(err.to_string().contains("--confirm-hub"), "{err}");
        // The right key for the same hub is fine, and so is none.
        assert!(tunnel.import(&text, Some(&encode(&[3; 32]))).is_ok());
        assert!(tunnel.import(&text, None).is_ok());
    }

    #[test]
    fn a_map_that_cannot_be_removed_stops_the_import_before_the_hub_changes() {
        let (_root, tunnel) = dir();
        let first = join_text(3, "vpn.example.com:51820", None);
        tunnel.import(&first, Some(&encode(&[3; 32]))).unwrap();
        // A directory where the file should be: removing it as a file fails, as an unremovable
        // file would after a stop between the two steps.
        let maps = tunnel.path(MAPS_FILE);
        std::fs::create_dir(&maps).unwrap();
        std::fs::write(maps.join("keeps-it-from-being-removed"), "x").unwrap();
        let before = std::fs::read(tunnel.path(JOIN_FILE)).unwrap();
        let other = join_text(5, "vpn.example.com:51820", None);
        assert!(tunnel.import(&other, Some(&encode(&[5; 32]))).is_err());
        assert_eq!(
            std::fs::read(tunnel.path(JOIN_FILE)).unwrap(),
            before,
            "the old hub is still the hub"
        );
    }

    #[test]
    fn nothing_in_a_directory_that_is_open_to_others_is_trusted() {
        let (_root, tunnel) = dir();
        let text = join_text(3, "vpn.example.com:51820", None);
        tunnel.import(&text, Some(&encode(&[3; 32]))).unwrap();
        write_maps(&tunnel);
        std::fs::write(tunnel.path(STATUS_FILE), "{}").unwrap();
        let join = tunnel.load_join().unwrap();
        std::fs::set_permissions(tunnel.root(), std::fs::Permissions::from_mode(0o755)).unwrap();
        for result in [
            tunnel.load_join().map(|_| ()),
            tunnel.load_maps(&join).map(|_| ()),
            tunnel.read_status().map(|_| ()),
            tunnel.ensure(),
            tunnel.import(&text, None).map(|_| ()),
        ] {
            assert!(result.unwrap_err().to_string().contains("chmod 700"));
        }
    }

    #[test]
    fn a_hand_written_map_file_needs_to_be_yours_but_not_closed_to_others() {
        let (_root, tunnel) = dir();
        let text = join_text(3, "vpn.example.com:51820", None);
        tunnel.import(&text, Some(&encode(&[3; 32]))).unwrap();
        write_maps(&tunnel);
        std::fs::set_permissions(
            tunnel.path(MAPS_FILE),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        let join = tunnel.load_join().unwrap();
        assert_eq!(tunnel.load_maps(&join).unwrap().serve.len(), 1);
    }

    #[test]
    fn a_join_text_that_cannot_be_read_back_counts_as_no_hub() {
        let (_root, tunnel) = dir();
        tunnel.ensure().unwrap();
        std::fs::write(tunnel.path(JOIN_FILE), "garbage").unwrap();
        std::fs::set_permissions(
            tunnel.path(JOIN_FILE),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        write_maps(&tunnel);
        let text = join_text(3, "vpn.example.com:51820", None);
        assert!(matches!(
            tunnel.import(&text, None).unwrap(),
            Import::NeedsConfirmation { .. }
        ));
        let (same, cleared) = saved(tunnel.import(&text, Some(&encode(&[3; 32]))).unwrap());
        assert!(!same && cleared);
    }

    #[test]
    fn a_hostile_join_text_changes_nothing() {
        let (_root, tunnel) = dir();
        let text = join_text(3, "vpn.example.com:51820", None);
        tunnel.import(&text, Some(&encode(&[3; 32]))).unwrap();
        write_maps(&tunnel);
        let before = std::fs::read(tunnel.path(JOIN_FILE)).unwrap();
        let with_private_key = format!("{text}PrivateKey = {}\n", encode(&[1; 32]));
        let long = format!("{text}{}", "#".repeat(MAX_JOIN_BYTES));
        for bad in [with_private_key, long, "".into(), "[Interface]\n".into()] {
            let err = tunnel.import(&bad, Some(&encode(&[3; 32]))).unwrap_err();
            assert!(!err.to_string().contains(&encode(&[1; 32])), "{err}");
            assert_eq!(std::fs::read(tunnel.path(JOIN_FILE)).unwrap(), before);
            assert!(tunnel.path(MAPS_FILE).exists());
        }
    }

    #[test]
    fn the_join_text_is_private_and_a_link_at_its_path_is_replaced() {
        let (_root, tunnel) = dir();
        tunnel.ensure().unwrap();
        let target = tunnel.root().join("precious");
        std::fs::write(&target, "keep me").unwrap();
        std::os::unix::fs::symlink(&target, tunnel.path(JOIN_FILE)).unwrap();
        let text = join_text(3, "vpn.example.com:51820", Some(5));
        tunnel.import(&text, Some(&encode(&[3; 32]))).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep me");
        let meta = std::fs::symlink_metadata(tunnel.path(JOIN_FILE)).unwrap();
        assert!(meta.is_file());
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        let dir_mode = std::fs::metadata(tunnel.root())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(dir_mode & 0o777, 0o700);
    }

    #[test]
    fn the_maps_are_read_with_the_same_strictness_as_the_config() {
        let (_root, tunnel) = dir();
        let text = join_text(3, "vpn.example.com:51820", None);
        tunnel.import(&text, Some(&encode(&[3; 32]))).unwrap();
        let join = tunnel.load_join().unwrap();
        assert!(
            tunnel.load_maps(&join).unwrap().serve.is_empty(),
            "missing is empty"
        );
        for bad in [
            "[[serve]]\nport = 1\nto = \"192.0.2.1:80\"\nfrom = [\"10.77.0.1\"]\n",
            "surprise = true\n",
            "not toml at all [",
        ] {
            std::fs::write(tunnel.path(MAPS_FILE), bad).unwrap();
            assert!(tunnel.load_maps(&join).is_err(), "{bad}");
        }
        std::fs::write(tunnel.path(MAPS_FILE), MAPS).unwrap();
        assert_eq!(tunnel.load_maps(&join).unwrap().serve.len(), 1);
    }

    #[test]
    fn the_status_is_read_as_json_and_nothing_else() {
        let (_root, tunnel) = dir();
        tunnel.ensure().unwrap();
        assert!(tunnel.read_status().is_err());
        std::fs::write(tunnel.path(STATUS_FILE), "{\"state\":\"up\"}").unwrap();
        assert_eq!(tunnel.read_status().unwrap()["state"], "up");
        std::fs::write(tunnel.path(STATUS_FILE), "no").unwrap();
        assert!(tunnel.read_status().is_err());
    }

    #[cfg(feature = "keygen")]
    #[test]
    fn a_generated_public_key_never_looks_like_a_private_one() {
        // One key in sixteen would, so without the filter this fails with near certainty.
        for _ in 0..400 {
            let (private, public) = keygen_pair();
            assert!(!hq_net::key::looks_like_private_key(&public));
            assert_eq!(hq_net::key::public_key_of(&private).unwrap(), public);
        }
    }

    #[cfg(feature = "keygen")]
    #[test]
    fn a_generated_key_is_private_new_and_its_public_half_matches() {
        let (_root, tunnel) = dir();
        let public = tunnel.keygen_to_file().unwrap();
        let path = tunnel.path(KEY_FILE);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let secret = crate::keyin::read_key_file(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(hq_net::key::public_key_of(text.trim()).unwrap(), public);
        assert_eq!(hq_net::key::decode(text.trim()).unwrap(), secret.bytes());
        let again = tunnel.keygen_to_file().unwrap_err().to_string();
        assert!(
            again.contains("already a key") && !again.contains(text.trim()),
            "{again}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            text,
            "never overwritten"
        );
    }
}
