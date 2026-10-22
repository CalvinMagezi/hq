//! Which devices the edge forwards for. The forwarder only sees the [`Registry`] trait, so the
//! pairing protocol's device store can replace the allowlist file without touching it.

use std::{
    collections::{HashMap, HashSet},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

use iroh::{EndpointId, RelayUrl};

use crate::{LABEL_BYTES, identity::ensure_private_dir};

/// How long a read of the allowlist is trusted before the file is checked again. It is also the
/// longest a revoked device keeps a new connection.
pub const RELOAD_EVERY: Duration = Duration::from_secs(1);
const PRIVATE_FILE_MODE: u32 = 0o600;
const GROUP_OTHER_WRITE_BITS: u32 = 0o022;

/// A device the edge may forward to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registered {
    pub id: EndpointId,
    /// The relay this device uses as its home, when it is not the edge's own.
    pub relay: Option<RelayUrl>,
}

/// The lookup the forwarder and the co-hosted relay make. Implementations answer from memory or a
/// cache: both are called on every connection, and `lookup` again every second per open one.
pub trait Registry: Send + Sync + 'static {
    /// The device whose name label is `label` (see [`device_label`]), if it is registered.
    fn lookup(&self, label: &str) -> Option<Registered>;
    /// Whether `id` is a registered device (the relay admits only these and the edge itself).
    fn is_registered(&self, id: &EndpointId) -> bool;
}

/// The DNS label a device is reached at: the first [`LABEL_BYTES`] bytes of its endpoint id as
/// lowercase hex. The device builds its own name with this same function.
pub fn device_label(id: &EndpointId) -> String {
    hex::encode(&id.as_bytes()[..LABEL_BYTES])
}

#[derive(Debug, thiserror::Error)]
pub enum ListError {
    #[error("{0} is not a plain file")]
    NotPlain(PathBuf),
    #[error("{0} is not owned by the user running the edge")]
    WrongOwner(PathBuf),
    #[error("{0} can be written by others; chmod 600 it")]
    WritableByOthers(PathBuf),
    #[error("the directory {0} can be written by others or belongs to another user")]
    UntrustedDir(PathBuf),
    #[error("line {line}: {why}")]
    BadLine { line: usize, why: String },
    #[error("devices {first} and {second} share the name label {label}; remove one of them")]
    Collision {
        first: String,
        second: String,
        label: String,
    },
    #[error("{0} is already on the list")]
    Duplicate(String),
    #[error("could not read or write the list: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Default)]
struct Snapshot {
    by_label: HashMap<String, Registered>,
    ids: HashSet<EndpointId>,
}

/// A text file with one device per line: `<64 hex endpoint id> [relay URL]`, `#` starts a comment.
///
/// The file must be a plain file owned by the user running the edge and not writable by others,
/// and it must parse completely: anything else makes the registry empty (nobody is forwarded)
/// until it is fixed, because a list the edge cannot fully trust is not a list to forward by.
pub struct AllowlistFile {
    path: PathBuf,
    owner: u32,
    state: Mutex<State>,
}

struct State {
    checked: Instant,
    snapshot: Snapshot,
    /// The last refusal that was logged, so a list that stays bad is reported once, not every second.
    last_error: Option<String>,
}

impl AllowlistFile {
    /// Opens the list and reads it once. A bad list is an error here, at start-up, rather than a
    /// log line on the first connection.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, ListError> {
        let path = path.into();
        // SAFETY: geteuid has no preconditions and cannot fail.
        let owner = unsafe { libc::geteuid() };
        let snapshot = read_snapshot(&path, owner)?;
        Ok(Self {
            path,
            owner,
            state: Mutex::new(State {
                checked: Instant::now(),
                snapshot,
                last_error: None,
            }),
        })
    }

    fn with_fresh<T>(&self, f: impl FnOnce(&Snapshot) -> T) -> T {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.checked.elapsed() >= RELOAD_EVERY {
            match read_snapshot(&self.path, self.owner) {
                Ok(snapshot) => {
                    state.snapshot = snapshot;
                    state.last_error = None;
                }
                Err(e) => {
                    let why = e.to_string();
                    if state.last_error.as_deref() != Some(&why) {
                        tracing::warn!("the device list is refused, nobody is forwarded: {why}");
                    }
                    state.last_error = Some(why);
                    state.snapshot = Snapshot::default();
                }
            }
            state.checked = Instant::now();
        }
        f(&state.snapshot)
    }
}

impl Registry for AllowlistFile {
    fn lookup(&self, label: &str) -> Option<Registered> {
        self.with_fresh(|s| s.by_label.get(label).cloned())
    }

    fn is_registered(&self, id: &EndpointId) -> bool {
        self.with_fresh(|s| s.ids.contains(id))
    }
}

fn read_snapshot(path: &Path, owner: u32) -> Result<Snapshot, ListError> {
    build(read_entries(path, owner)?)
}

/// Indexes the entries by label and id, refusing two devices that share a label.
fn build(entries: Vec<Registered>) -> Result<Snapshot, ListError> {
    build_with(entries, device_label)
}

fn build_with(
    entries: Vec<Registered>,
    label_of: impl Fn(&EndpointId) -> String,
) -> Result<Snapshot, ListError> {
    let mut snapshot = Snapshot::default();
    for entry in entries {
        let label = label_of(&entry.id);
        if let Some(other) = snapshot.by_label.get(&label) {
            return Err(ListError::Collision {
                first: other.id.fmt_short().to_string(),
                second: entry.id.fmt_short().to_string(),
                label,
            });
        }
        snapshot.ids.insert(entry.id);
        snapshot.by_label.insert(label, entry);
    }
    Ok(snapshot)
}

/// Opens the list without following a symlink, checks the open file's kind, owner and mode (so
/// there is no gap between the check and the read), then parses it. A missing file is an empty list.
fn read_entries(path: &Path, owner: u32) -> Result<Vec<Registered>, ListError> {
    check_dir(path, owner)?;
    read_file(path, owner)
}

fn read_file(path: &Path, owner: u32) -> Result<Vec<Registered>, ListError> {
    let mut file = match std::fs::OpenOptions::new()
        .read(true)
        // O_NONBLOCK so that a FIFO cannot hang the open; it is refused as not a plain file below.
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
            return Err(ListError::NotPlain(path.to_owned()));
        }
        Err(e) => return Err(e.into()),
    };
    let meta = file.metadata()?;
    if !meta.file_type().is_file() {
        return Err(ListError::NotPlain(path.to_owned()));
    }
    if meta.uid() != owner {
        return Err(ListError::WrongOwner(path.to_owned()));
    }
    if meta.permissions().mode() & GROUP_OTHER_WRITE_BITS != 0 {
        return Err(ListError::WritableByOthers(path.to_owned()));
    }
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    parse(&text)
}

/// The directory holding the list must be one only this user (or root) can change.
fn check_dir(path: &Path, owner: u32) -> Result<(), ListError> {
    let dir = path.parent().unwrap_or(Path::new("."));
    if crate::identity::dir_is_trusted(dir, owner)? {
        return Ok(());
    }
    Err(ListError::UntrustedDir(dir.to_owned()))
}

fn parse(text: &str) -> Result<Vec<Registered>, ListError> {
    let mut entries = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let content = raw.split('#').next().unwrap_or("").trim();
        if content.is_empty() {
            continue;
        }
        let mut fields = content.split_whitespace();
        let bad = |why: &str| ListError::BadLine {
            line,
            why: why.into(),
        };
        let id: EndpointId = fields
            .next()
            .unwrap_or_default()
            .parse()
            .map_err(|_| bad("not an endpoint id (64 hex characters)"))?;
        let relay = fields
            .next()
            .map(|url| url.parse::<RelayUrl>())
            .transpose()
            .map_err(|_| bad("not a relay URL"))?;
        if fields.next().is_some() {
            return Err(bad("expected an endpoint id and at most a relay URL"));
        }
        entries.push(Registered { id, relay });
    }
    Ok(entries)
}

/// Adds a device to the list at `path`, which `hq edge allow` calls. The existing list must be
/// valid and the device new and without a name collision.
pub fn allow_device(path: &Path, device: Registered) -> Result<(), ListError> {
    // SAFETY: geteuid has no preconditions and cannot fail.
    let owner = unsafe { libc::geteuid() };
    let mut entries = read_entries(path, owner)?;
    if entries.iter().any(|e| e.id == device.id) {
        return Err(ListError::Duplicate(device.id.to_string()));
    }
    entries.push(device.clone());
    build(entries)?;
    if let Some(dir) = path.parent() {
        ensure_private_dir(dir).map_err(|e| std::io::Error::other(e.to_string()))?;
    }
    // The list may not end in a newline (an editor left it so): without one, the new id would
    // join the last line, or vanish into a trailing comment.
    let needs_break =
        std::fs::read(path).is_ok_and(|bytes| bytes.last().is_some_and(|b| *b != b'\n'));
    let mut line = String::new();
    if needs_break {
        line.push('\n');
    }
    line.push_str(&device.id.to_string());
    if let Some(relay) = &device.relay {
        line.push(' ');
        line.push_str(&relay.to_string());
    }
    line.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(PRIVATE_FILE_MODE)
        .open(path)?;
    file.write_all(line.as_bytes())?;
    Ok(())
}

/// Removes a device from the list. Returns whether it was there. Open connections to it are cut
/// within a second.
pub fn revoke_device(path: &Path, id: &EndpointId) -> Result<bool, ListError> {
    // SAFETY: geteuid has no preconditions and cannot fail.
    let owner = unsafe { libc::geteuid() };
    read_entries(path, owner)?;
    let text = std::fs::read_to_string(path)?;
    let mut kept = String::new();
    let mut removed = false;
    for raw in text.lines() {
        let first = raw
            .split('#')
            .next()
            .unwrap_or("")
            .split_whitespace()
            .next();
        if first.is_some_and(|f| f.parse::<EndpointId>().is_ok_and(|line_id| line_id == *id)) {
            removed = true;
            continue;
        }
        kept.push_str(raw);
        kept.push('\n');
    }
    if removed {
        let tmp = path.with_extension("tmp");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(PRIVATE_FILE_MODE)
            .open(&tmp)?;
        file.write_all(kept.as_bytes())?;
        std::fs::rename(&tmp, path)?;
    }
    Ok(removed)
}

/// Reads the list for display (`hq edge list`).
pub fn list_devices(path: &Path) -> Result<Vec<Registered>, ListError> {
    // SAFETY: geteuid has no preconditions and cannot fail.
    let owner = unsafe { libc::geteuid() };
    read_entries(path, owner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;
    use std::os::unix::fs::symlink;

    fn id() -> EndpointId {
        SecretKey::generate().public()
    }

    fn write(path: &Path, text: &str, mode: u32) {
        std::fs::write(path, text).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn the_label_is_sixteen_lowercase_hex_characters_of_the_id() {
        let id = id();
        let label = device_label(&id);
        assert_eq!(label.len(), 16);
        assert!(id.to_string().starts_with(&label));
        assert_eq!(label, label.to_ascii_lowercase());
    }

    #[test]
    fn registered_devices_are_found_by_label_and_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.txt");
        let (a, b) = (id(), id());
        write(
            &path,
            &format!("# the phone\n{a}\n\n{b} https://relay.example.com # home\n"),
            0o600,
        );
        let list = AllowlistFile::open(&path).unwrap();
        assert_eq!(list.lookup(&device_label(&a)).unwrap().id, a);
        let found = list.lookup(&device_label(&b)).unwrap();
        assert_eq!(
            found.relay.unwrap().to_string(),
            "https://relay.example.com/"
        );
        assert!(list.is_registered(&a));
        assert!(!list.is_registered(&id()));
        assert!(list.lookup("0000000000000000").is_none());
    }

    #[test]
    fn a_list_that_is_not_trusted_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.txt");
        let a = id();

        write(&path, &format!("{a}\n"), 0o666);
        assert!(matches!(
            AllowlistFile::open(&path),
            Err(ListError::WritableByOthers(_))
        ));

        std::fs::remove_file(&path).unwrap();
        let real = dir.path().join("real.txt");
        write(&real, &format!("{a}\n"), 0o600);
        symlink(&real, &path).unwrap();
        assert!(matches!(
            AllowlistFile::open(&path),
            Err(ListError::NotPlain(_))
        ));
    }

    #[test]
    fn a_list_owned_by_someone_else_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.txt");
        write(&path, "", 0o600);
        // Reading it as though another user ran the edge is the same check as a foreign owner.
        // SAFETY: geteuid has no preconditions and cannot fail.
        let someone_else = unsafe { libc::geteuid() } + 1;
        assert!(matches!(
            read_file(&path, someone_else),
            Err(ListError::WrongOwner(_))
        ));
    }

    #[test]
    fn two_devices_with_one_label_are_refused() {
        // Two real keys sharing a 64-bit prefix cannot be made, so the labeler is replaced by one
        // that gives every device the same label.
        let entries = vec![
            Registered {
                id: id(),
                relay: None,
            },
            Registered {
                id: id(),
                relay: None,
            },
        ];
        assert!(matches!(
            build_with(entries.clone(), |_| "same".into()),
            Err(ListError::Collision { .. })
        ));
        assert!(build_with(entries, device_label).is_ok());
    }

    #[test]
    fn allow_survives_a_list_without_a_final_newline_or_ending_in_a_comment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.txt");
        let (a, b, c) = (id(), id(), id());
        write(&path, &a.to_string(), 0o600); // no trailing newline
        allow_device(&path, Registered { id: b, relay: None }).unwrap();
        let ids: Vec<_> = list_devices(&path).unwrap().iter().map(|d| d.id).collect();
        assert_eq!(ids, vec![a, b], "the new id must not join the last line");

        write(&path, &format!("{a}\n# the phone"), 0o600); // ends in a comment, no newline
        allow_device(&path, Registered { id: c, relay: None }).unwrap();
        let ids: Vec<_> = list_devices(&path).unwrap().iter().map(|d| d.id).collect();
        assert_eq!(
            ids,
            vec![a, c],
            "the new id must not vanish into the comment"
        );
    }

    #[test]
    fn a_list_that_is_not_a_regular_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.txt");
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(
            AllowlistFile::open(&path),
            Err(ListError::NotPlain(_))
        ));
    }

    #[test]
    fn a_list_in_a_directory_others_can_write_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.txt");
        write(&path, &format!("{}\n", id()), 0o600);
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(matches!(
            AllowlistFile::open(&path),
            Err(ListError::UntrustedDir(_))
        ));
    }

    #[test]
    fn a_collision_names_short_ids_not_whole_ones() {
        let (a, b) = (id(), id());
        let entries = vec![
            Registered { id: a, relay: None },
            Registered { id: b, relay: None },
        ];
        let message = build_with(entries, |_| "same".into())
            .err()
            .unwrap()
            .to_string();
        assert!(!message.contains(&a.to_string()), "{message}");
    }

    #[test]
    fn bad_lines_name_their_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.txt");
        write(&path, &format!("{}\nnot-an-id\n", id()), 0o600);
        match AllowlistFile::open(&path) {
            Err(ListError::BadLine { line: 2, .. }) => {}
            other => panic!("expected a bad line 2, got {:?}", other.err()),
        }
    }

    #[test]
    fn allow_then_revoke_round_trips_and_a_change_is_picked_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("devices.txt");
        let a = id();
        allow_device(&path, Registered { id: a, relay: None }).unwrap();
        assert!(matches!(
            allow_device(&path, Registered { id: a, relay: None }),
            Err(ListError::Duplicate(_))
        ));
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            PRIVATE_FILE_MODE
        );
        let live = AllowlistFile::open(&path).unwrap();
        assert!(live.is_registered(&a));

        assert!(revoke_device(&path, &a).unwrap());
        assert!(!revoke_device(&path, &a).unwrap());
        std::thread::sleep(RELOAD_EVERY + Duration::from_millis(100));
        assert!(!live.is_registered(&a), "a revoked device must drop out");
        assert!(list_devices(&path).unwrap().is_empty());
    }
}
