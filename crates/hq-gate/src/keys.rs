//! The caller's signing key and the callee's list of authorized keys.

use crate::proto::{GateError, PUBLIC_KEY_BYTES, decode_exact, encode};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::RngCore;
use std::fs::File;
use std::io::{Read, Write};
use std::net::IpAddr;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

const SEED_BYTES: usize = 32;
const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;
const GROUP_OTHER_WRITE: u32 = 0o022;
const GROUP_OTHER_ANY: u32 = 0o077;
const MAX_KEY_FILE_BYTES: u64 = 1_000_000;

/// The server's key for one host. It lives in a file of mode 0600 as base64url of the 32 byte seed.
pub struct ClientKey {
    signing: SigningKey,
}

impl ClientKey {
    /// Reads the key at `path`, or makes one there (and its directory, mode 0700) if the file does
    /// not exist. A key file that other users can read is refused.
    pub fn load_or_create(path: &Path) -> Result<Self, GateError> {
        match std::fs::symlink_metadata(path) {
            Ok(_) => Self::read(path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::create(path),
            Err(e) => Err(e.into()),
        }
    }

    /// Reads the key at `path` and never makes one: a missing key is a setup mistake to report.
    pub fn load(path: &Path) -> Result<Self, GateError> {
        match std::fs::symlink_metadata(path) {
            Ok(_) => Self::read(path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(GateError::Config(format!(
                "no gate key at {}; make one with `hq host gate-key <host>`",
                path.display()
            ))),
            Err(e) => Err(e.into()),
        }
    }

    pub fn public_key(&self) -> String {
        encode(self.signing.verifying_key().as_bytes())
    }

    pub(crate) fn sign(&self, message: &[u8]) -> Signature {
        self.signing.sign(message)
    }

    fn read(path: &Path) -> Result<Self, GateError> {
        let text = read_trusted(path, GROUP_OTHER_ANY, "readable")?;
        let seed = decode_exact::<SEED_BYTES>(text.trim()).ok_or_else(|| {
            GateError::Config(format!("{} does not hold a gate key", path.display()))
        })?;
        Ok(Self {
            signing: SigningKey::from_bytes(&seed),
        })
    }

    fn create(path: &Path) -> Result<Self, GateError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(DIR_MODE))?;
        }
        let mut seed = [0u8; SEED_BYTES];
        rand::rngs::OsRng.fill_bytes(&mut seed);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(FILE_MODE)
            .open(path)?;
        file.write_all(format!("{}\n", encode(&seed)).as_bytes())?;
        Ok(Self {
            signing: SigningKey::from_bytes(&seed),
        })
    }
}

/// One line of the authorized keys file: `<public-key> [from=<ip>[,<ip>...]] [# label]`.
#[derive(Debug, Clone)]
pub struct AuthorizedKey {
    pub(crate) key: VerifyingKey,
    /// Addresses this key may connect from; empty means any of the listener's allowed sources.
    pub(crate) from: Vec<IpAddr>,
    pub label: String,
}

/// Opens a file the gate trusts and reads it. It must be a plain file (not a link, not a pipe)
/// owned by the user running HQ, with none of the `forbidden` permission bits set. The checks run
/// on the open handle, so the file cannot be swapped between the check and the read.
fn read_trusted(path: &Path, forbidden: u32, too_open_for: &str) -> Result<String, GateError> {
    let before = std::fs::symlink_metadata(path)?;
    if !before.file_type().is_file() {
        return Err(GateError::Config(format!(
            "{} is not a plain file",
            path.display()
        )));
    }
    let file = File::open(path)?;
    let meta = file.metadata()?;
    if meta.dev() != before.dev() || meta.ino() != before.ino() {
        return Err(GateError::Config(format!(
            "{} changed while it was being opened",
            path.display()
        )));
    }
    // SAFETY: geteuid has no preconditions and cannot fail.
    if meta.uid() != unsafe { libc::geteuid() } {
        return Err(GateError::Config(format!(
            "{} is not owned by the user running HQ",
            path.display()
        )));
    }
    if meta.mode() & forbidden != 0 {
        return Err(GateError::Config(format!(
            "{} is {too_open_for} by other users; run chmod 600 on it",
            path.display()
        )));
    }
    let mut text = String::new();
    file.take(MAX_KEY_FILE_BYTES + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > MAX_KEY_FILE_BYTES {
        return Err(GateError::Config(format!(
            "{} is too large",
            path.display()
        )));
    }
    Ok(text)
}

/// Reads the authorized keys. The file must be a plain file of the current user that group and
/// others cannot write, or no key is accepted at all. A line that does not parse authorizes
/// nothing and is skipped.
pub fn load_authorized(path: &Path) -> Result<Vec<AuthorizedKey>, GateError> {
    let text = read_trusted(path, GROUP_OTHER_WRITE, "writable")?;
    Ok(text.lines().filter_map(parse_line).collect())
}

fn parse_line(line: &str) -> Option<AuthorizedKey> {
    let (body, label) = match line.split_once('#') {
        Some((body, label)) => (body, label.trim()),
        None => (line, ""),
    };
    let mut words = body.split_whitespace();
    let bytes = decode_exact::<PUBLIC_KEY_BYTES>(words.next()?)?;
    let key = VerifyingKey::from_bytes(&bytes).ok()?;
    let mut from = Vec::new();
    for option in words {
        let list = option.strip_prefix("from=")?;
        for addr in list.split(',') {
            from.push(addr.parse::<IpAddr>().ok()?);
        }
    }
    Some(AuthorizedKey {
        key,
        from,
        label: label.to_string(),
    })
}

/// Appends a key to the file (created with mode 0600). Returns false if the key is already there
/// with the same source addresses, and an error if it is there with different ones: change those
/// by editing the file, so nothing is widened by accident.
pub fn append_authorized(
    path: &Path,
    public_key: &str,
    from: &[IpAddr],
    label: &str,
) -> Result<bool, GateError> {
    let bytes = decode_exact::<PUBLIC_KEY_BYTES>(public_key)
        .ok_or_else(|| GateError::Config("that is not a gate public key".into()))?;
    VerifyingKey::from_bytes(&bytes)
        .map_err(|_| GateError::Config("that is not a valid Ed25519 public key".into()))?;
    if label.contains(['\n', '\r']) {
        return Err(GateError::Config("the label must be one line".into()));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let existing = match std::fs::symlink_metadata(path) {
        Ok(_) => read_trusted(path, GROUP_OTHER_WRITE, "writable")?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    if let Some(line) = existing
        .lines()
        .find(|l| l.split_whitespace().next() == Some(public_key))
    {
        let mut listed = parse_line(line).map(|k| k.from).unwrap_or_default();
        let mut wanted = from.to_vec();
        listed.sort();
        wanted.sort();
        if listed == wanted {
            return Ok(false);
        }
        return Err(GateError::Config(format!(
            "that key is already in {} with different source addresses; edit the file to change them",
            path.display()
        )));
    }
    let mut line = public_key.to_string();
    if !from.is_empty() {
        let list: Vec<String> = from.iter().map(ToString::to_string).collect();
        line.push_str(&format!(" from={}", list.join(",")));
    }
    if !label.is_empty() {
        line.push_str(&format!(" # {label}"));
    }
    let lead = if existing.is_empty() || existing.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(FILE_MODE)
        .open(path)?;
    writeln!(file, "{lead}{line}")?;
    Ok(true)
}

/// Removes every line of the file whose key and label `drop` picks (`drop(key, label)`, the key as
/// written and the label after `#`, trimmed), keeping all other lines byte for byte. Lines that do
/// not parse are kept. The file gets the same checks as when it is read, and is replaced whole
/// (written beside it with mode 0600, then renamed), so a reader never sees half of it. A missing
/// file has nothing to remove. Returns how many lines went.
pub fn remove_authorized(
    path: &Path,
    drop: impl Fn(&str, &str) -> bool,
) -> Result<usize, GateError> {
    let text = match std::fs::symlink_metadata(path) {
        Ok(_) => read_trusted(path, GROUP_OTHER_WRITE, "writable")?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.into()),
    };
    let mut kept = String::with_capacity(text.len());
    let mut removed = 0;
    for line in text.split_inclusive('\n') {
        let picked = parse_line(line.trim_end_matches(['\n', '\r'])).is_some_and(|k| {
            let written = line.split_whitespace().next().unwrap_or_default();
            drop(written, &k.label)
        });
        if picked {
            removed += 1;
        } else {
            kept.push_str(line);
        }
    }
    if removed == 0 {
        return Ok(0);
    }
    let staged = path.with_extension("hq-new");
    let _ = std::fs::remove_file(&staged);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .open(&staged)?;
    file.write_all(kept.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&staged, path)?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_text(seed: u8) -> String {
        let key = SigningKey::from_bytes(&[seed; 32]);
        encode(key.verifying_key().as_bytes())
    }

    #[test]
    fn a_created_key_is_private_and_reloads_to_the_same_public_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gate/mac.key");
        let made = ClientKey::load_or_create(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let again = ClientKey::load_or_create(&path).unwrap();
        assert_eq!(made.public_key(), again.public_key());
    }

    #[test]
    fn a_key_file_other_users_can_read_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k");
        ClientKey::load_or_create(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(ClientKey::load_or_create(&path).is_err());
    }

    #[test]
    fn a_symlinked_or_garbage_key_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::write(&real, "not a key\n").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(ClientKey::load_or_create(&real).is_err());
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(ClientKey::load_or_create(&link).is_err());
    }

    #[test]
    fn authorized_lines_parse_with_options_and_bad_lines_authorize_nothing() {
        let good = key_text(1);
        let lines = [
            good.clone(),
            format!("{} from=10.77.0.1,10.77.0.9 # mac", key_text(2)),
            format!("# a comment {}", key_text(3)),
            "garbage".to_string(),
            format!("{} from=not-an-ip", key_text(4)),
            format!("{} command=\"x\"", key_text(5)),
            String::new(),
        ];
        let parsed: Vec<_> = lines.iter().filter_map(|l| parse_line(l)).collect();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[1].from.len(), 2);
        assert_eq!(parsed[1].label, "mac");
    }

    #[test]
    fn a_writable_authorized_keys_file_is_refused_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys");
        append_authorized(&path, &key_text(1), &[], "a").unwrap();
        assert_eq!(load_authorized(&path).unwrap().len(), 1);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(load_authorized(&path).is_err());
    }

    #[test]
    fn appending_is_idempotent_private_and_rejects_bad_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys");
        let key = key_text(9);
        let from: Vec<IpAddr> = vec!["10.77.0.1".parse().unwrap()];
        assert!(append_authorized(&path, &key, &from, "server").unwrap());
        assert!(!append_authorized(&path, &key, &from, "again").unwrap());
        assert!(append_authorized(&path, &key, &[], "wider").is_err());
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(append_authorized(&path, "short", &[], "x").is_err());
        assert!(append_authorized(&path, &key_text(8), &[], "two\nlines").is_err());
        assert_eq!(load_authorized(&path).unwrap().len(), 1);
    }
    #[test]
    fn a_file_without_a_trailing_newline_keeps_its_last_key_when_another_is_added() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys");
        std::fs::write(&path, key_text(1)).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(append_authorized(&path, &key_text(2), &[], "").unwrap());
        assert_eq!(load_authorized(&path).unwrap().len(), 2);
    }

    #[test]
    fn a_linked_or_non_regular_authorized_keys_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("keys");
        append_authorized(&real, &key_text(1), &[], "").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(load_authorized(&link).is_err());
        assert!(load_authorized(dir.path()).is_err());
    }

    #[test]
    fn removing_drops_only_the_picked_lines_and_keeps_the_rest_as_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys");
        assert_eq!(remove_authorized(&path, |_, _| true).unwrap(), 0, "no file");
        let (a, b, c) = (key_text(1), key_text(2), key_text(3));
        let text = format!(
            "# owner's note\n{a} from=10.77.0.1 # mac\n{b} from=127.0.0.1 # bridge:aabbccdd\ngarbage line\n{c}  # bridge:aabbccdd-other"
        );
        std::fs::write(&path, &text).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let gone = remove_authorized(&path, |key, label| key == b && label == "bridge:aabbccdd");
        assert_eq!(gone.unwrap(), 1);
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            after,
            format!(
                "# owner's note\n{a} from=10.77.0.1 # mac\ngarbage line\n{c}  # bridge:aabbccdd-other"
            )
        );
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(
            remove_authorized(&path, |_, _| true).is_err(),
            "a writable file is refused"
        );
    }
}
