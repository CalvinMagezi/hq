//! The root applier: reads the peers file the HQ user rendered, checks every byte of it against
//! the grammar, and only then asks `wg` to make the interface match.
//!
//! Linux only, on purpose. It runs as root from systemd units, installed in a root-owned place, and
//! is told everything that matters (interface, subnet, hub address, which user owns the input,
//! where the input is) by the unit, never by the input.
//!
//! The input is opened one path component at a time without following any link, and the handle
//! it ends in is checked before a byte is read: a plain file, owned by the expected user, no other
//! names, no group or other access, bounded in size. The file's text is parsed once and `wg` is
//! given text this program rendered from the parsed peers, never the file. Refusals name a line
//! and a rule, not the line.
//!
//! A version 2 file also lists the pairs of admitted devices the hub may forward between. The
//! applier turns those into one `nft` script that replaces the contents of a set the owner created
//! with the firewall rules (`links`); it never writes a rule. The script goes to `nft` before the
//! peers go to `wg`, so a pair is closed no later than a device is cut off, and a failure of `nft`
//! never stops `wg`: a revocation must not wait for the firewall.
//!
//! What this program does not decide is who may be admitted: anything running as the input's owner
//! can add a device within the subnet, and join admitted devices in pairs. It limits what such a
//! file can do, and it records the peers and pairs it applied, by address and a short key
//! fingerprint, so the journal shows every change.

use crate::grammar::{self, Link, MAX_INPUT_BYTES, Policy, Reject};
use crate::links;
use crate::names::valid_interface;
use crate::subnet::{Subnet, parse_canonical_ipv4};
use crate::wgconf;
use std::ffi::{CString, OsStr};
use std::fmt;
use std::fs::File;
use std::io::{Read, Write};
use std::net::Ipv4Addr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const USAGE: &str = "usage: hq-net-apply --interface NAME --subnet CIDR --hub ADDRESS --owner-uid UID --input /ABSOLUTE/PATH [--wg /ABSOLUTE/PATH] [--nft /ABSOLUTE/PATH]";
/// Where `wg` may be, in order: the distribution's own directories only. The first that exists
/// and is root-owned and not writable by others is used; no `PATH` search ever happens. A `wg` in
/// `/usr/local` (writable by the `staff` group on Debian) needs an explicit `--wg`.
const WG_CANDIDATES: [&str; 4] = ["/usr/bin/wg", "/usr/sbin/wg", "/bin/wg", "/sbin/wg"];
/// The same rule for `nft`, which is only run for a version 2 file.
const NFT_CANDIDATES: [&str; 4] = ["/usr/sbin/nft", "/sbin/nft", "/usr/bin/nft", "/bin/nft"];
/// `nft -f -` reads the script from standard input.
const NFT_STDIN: &str = "-";
/// How many times a run applies again because the input changed while it was applying. A change
/// after the last re-read is caught by the timer, not by this loop.
const MAX_ROUNDS: usize = 3;
/// `wg` reads the config from its own standard input through this path, which exists in every
/// mount namespace (unlike a `/dev/stdin` link a private `/dev` may or may not carry).
const STDIN_PATH: &str = "/proc/self/fd/0";
const GROUP_OTHER_ACCESS: u32 = 0o077;
const GROUP_OTHER_WRITE: u32 = 0o022;
const ROOT_UID: u32 = 0;

/// Everything the unit tells the applier.
#[derive(Debug, Clone)]
pub struct Config {
    pub interface: String,
    pub policy: Policy,
    pub input: PathBuf,
    pub owner_uid: u32,
    pub wg: Option<PathBuf>,
    pub nft: Option<PathBuf>,
}

fn take_once(slot: &mut Option<String>, flag: &str, value: Option<String>) -> Result<(), String> {
    let value = value.ok_or_else(|| format!("{flag} needs a value"))?;
    if slot.replace(value).is_some() {
        return Err(format!("{flag} was given twice"));
    }
    Ok(())
}

/// Reads the unit's arguments. Each flag once, `--flag value` form only, nothing unknown.
pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Config, String> {
    let mut slots: [Option<String>; 7] = Default::default();
    let flags = [
        "--interface",
        "--subnet",
        "--hub",
        "--owner-uid",
        "--input",
        "--wg",
        "--nft",
    ];
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let Some(index) = flags.iter().position(|f| *f == arg) else {
            return Err(format!("unknown argument {arg:?}"));
        };
        take_once(&mut slots[index], &arg, args.next())?;
    }
    let [interface, subnet, hub, owner, input, wg, nft] = slots;
    let need =
        |value: Option<String>, flag: &str| value.ok_or_else(|| format!("{flag} is required"));
    let interface = need(interface, "--interface")?;
    if !valid_interface(&interface) {
        return Err("--interface must be 1 to 15 characters: a lowercase letter, then lowercase letters, digits or _".into());
    }
    let subnet = Subnet::parse(&need(subnet, "--subnet")?).map_err(|e| format!("--subnet: {e}"))?;
    let hub =
        parse_canonical_ipv4(&need(hub, "--hub")?).ok_or("--hub is not a plain IPv4 address")?;
    let policy = Policy::new(subnet, hub).map_err(|e| format!("--subnet and --hub: {e}"))?;
    let owner_uid = need(owner, "--owner-uid")?
        .parse::<u32>()
        .map_err(|_| "--owner-uid is not a number".to_string())?;
    let input = PathBuf::from(need(input, "--input")?);
    let wg = wg.map(PathBuf::from);
    let nft = nft.map(PathBuf::from);
    Ok(Config {
        interface,
        policy,
        input,
        owner_uid,
        wg,
        nft,
    })
}

#[derive(Debug)]
pub enum InputError {
    Path(&'static str),
    Link,
    NotRegular,
    WrongOwner,
    TooOpen,
    OtherNames,
    ParentUnsafe,
    TooLarge,
    Io(String),
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InputError::Path(why) => f.write_str(why),
            InputError::Link => f.write_str("a component of the input path is a link or is not a directory; give the real path"),
            InputError::NotRegular => f.write_str("the input is not a plain file"),
            InputError::WrongOwner => f.write_str("the input is not owned by the expected user"),
            InputError::TooOpen => f.write_str("the input is readable or writable by group or others"),
            InputError::OtherNames => f.write_str("the input has more than one name (a hard link)"),
            InputError::ParentUnsafe => f.write_str("the input's directory is not owned by root or the expected user, or is writable by group or others"),
            InputError::TooLarge => f.write_str("the input is larger than a peers file can be"),
            InputError::Io(why) => write!(f, "cannot read the input: {why}"),
        }
    }
}

fn open_at(dir: RawFd, name: &OsStr, flags: libc::c_int) -> std::io::Result<OwnedFd> {
    let name = CString::new(name.as_bytes())
        .map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
    // SAFETY: `name` is a valid NUL-terminated string that outlives the call, and `dir` is an open
    // directory descriptor owned by the caller.
    let fd = unsafe { libc::openat(dir, name.as_ptr(), flags | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `openat` returned a new descriptor that nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn open_error(error: std::io::Error) -> InputError {
    match error.raw_os_error() {
        Some(libc::ELOOP | libc::ENOTDIR) => InputError::Link,
        _ => InputError::Io(error.to_string()),
    }
}

/// The directory that holds the input must belong to root or the expected user, and no one else
/// may be able to add, remove or rename names in it.
/// What `fstat` says about an open file or directory: the only inputs to the ownership and mode
/// rules, kept apart from the system calls so every rule can be tested without root.
#[derive(Debug, Clone, Copy)]
pub struct Facts {
    pub is_file: bool,
    pub uid: u32,
    pub mode: u32,
    pub nlink: u64,
    pub len: u64,
}

impl Facts {
    fn of(meta: &std::fs::Metadata) -> Self {
        Self {
            is_file: meta.file_type().is_file(),
            uid: meta.uid(),
            mode: meta.mode(),
            nlink: meta.nlink(),
            len: meta.len(),
        }
    }
}

/// The directory that holds the input: root's or the expected user's, and closed to writes by
/// anyone else.
pub fn check_parent_facts(facts: &Facts, owner_uid: u32) -> Result<(), InputError> {
    let owner_ok = facts.uid == ROOT_UID || facts.uid == owner_uid;
    if !owner_ok || facts.mode & GROUP_OTHER_WRITE != 0 {
        return Err(InputError::ParentUnsafe);
    }
    Ok(())
}

/// The input itself: a plain file of the expected user, no group or other access, one name, and
/// small enough to be a peers file.
pub fn check_file_facts(facts: &Facts, owner_uid: u32) -> Result<(), InputError> {
    if !facts.is_file {
        return Err(InputError::NotRegular);
    }
    if facts.uid != owner_uid {
        return Err(InputError::WrongOwner);
    }
    if facts.mode & GROUP_OTHER_ACCESS != 0 {
        return Err(InputError::TooOpen);
    }
    if facts.nlink != 1 {
        return Err(InputError::OtherNames);
    }
    if facts.len > MAX_INPUT_BYTES as u64 {
        return Err(InputError::TooLarge);
    }
    Ok(())
}

fn check_parent(dir: &OwnedFd, owner_uid: u32) -> Result<(), InputError> {
    let copy = dir.try_clone().map_err(|e| InputError::Io(e.to_string()))?;
    let meta = File::from(copy)
        .metadata()
        .map_err(|e| InputError::Io(e.to_string()))?;
    check_parent_facts(&Facts::of(&meta), owner_uid)
}

/// Opens `path` without following a link at any component and reads it, if the open handle is
/// what a peers file must be.
pub fn read_input(path: &Path, owner_uid: u32) -> Result<Vec<u8>, InputError> {
    let mut parts = path.components();
    if parts.next() != Some(Component::RootDir) {
        return Err(InputError::Path("the input path must be absolute"));
    }
    let names: Vec<&OsStr> = parts
        .map(|part| match part {
            Component::Normal(name) => Ok(name),
            _ => Err(InputError::Path(
                "the input path must not contain . or .. components",
            )),
        })
        .collect::<Result<_, _>>()?;
    let Some((file_name, directories)) = names.split_last() else {
        return Err(InputError::Path(
            "the input path names a directory, not a file",
        ));
    };
    // SAFETY: the path is a NUL-terminated literal; the descriptor is wrapped at once.
    let root = unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if root < 0 {
        return Err(InputError::Io(std::io::Error::last_os_error().to_string()));
    }
    // SAFETY: `open` returned a new descriptor that nothing else owns.
    let mut dir = unsafe { OwnedFd::from_raw_fd(root) };
    for name in directories {
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW;
        dir = open_at(dir.as_raw_fd(), name, flags).map_err(open_error)?;
    }
    check_parent(&dir, owner_uid)?;
    // O_NONBLOCK: a FIFO planted here must fail the type check below, not hang root on open.
    let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY;
    let fd = open_at(dir.as_raw_fd(), file_name, flags).map_err(open_error)?;
    let file = File::from(fd);
    let meta = file.metadata().map_err(|e| InputError::Io(e.to_string()))?;
    check_file_facts(&Facts::of(&meta), owner_uid)?;
    let mut bytes = Vec::new();
    file.take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| InputError::Io(e.to_string()))?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(InputError::TooLarge);
    }
    Ok(bytes)
}

/// The two things the applier asks of `wg`. A trait only so tests can stand in for the program.
pub trait Wg {
    fn show_config(&self, interface: &str) -> Result<String, String>;
    fn apply(
        &self,
        interface: &str,
        subcommand: wgconf::Subcommand,
        config: &str,
    ) -> Result<(), String>;
}

/// The real `wg`, run by absolute path with an empty environment and no shell.
pub struct SystemWg {
    path: PathBuf,
}

impl SystemWg {
    /// Finds `wg` and checks that no one but root could have replaced it, since it runs as root.
    pub fn locate(explicit: Option<&Path>) -> Result<Self, String> {
        let candidates: Vec<PathBuf> = match explicit {
            Some(path) => vec![path.to_path_buf()],
            None => WG_CANDIDATES.iter().map(PathBuf::from).collect(),
        };
        let mut last = "wg was not found in a standard location".to_string();
        for path in candidates {
            match trusted_binary(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(why) => last = format!("{}: {why}", path.display()),
            }
        }
        Err(last)
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.path);
        command.args(args).env_clear();
        command
    }
}

fn trusted_binary(path: &Path) -> Result<(), &'static str> {
    if !path.is_absolute() {
        return Err("the path to wg must be absolute");
    }
    let meta = std::fs::metadata(path).map_err(|_| "no such file")?;
    if !meta.is_file() {
        return Err("not a file");
    }
    if meta.uid() != ROOT_UID || meta.mode() & GROUP_OTHER_WRITE != 0 {
        return Err("not owned by root, or writable by group or others");
    }
    Ok(())
}

impl Wg for SystemWg {
    fn show_config(&self, interface: &str) -> Result<String, String> {
        let out = self
            .command(&["showconf", interface])
            .stdin(Stdio::null())
            .stderr(Stdio::inherit())
            .output()
            .map_err(|e| format!("cannot run wg: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "wg showconf {interface} failed ({}); is the interface up?",
                out.status
            ));
        }
        String::from_utf8(out.stdout)
            .map_err(|_| "wg showconf printed bytes that are not text".to_string())
    }

    fn apply(
        &self,
        interface: &str,
        subcommand: wgconf::Subcommand,
        config: &str,
    ) -> Result<(), String> {
        let mut child = self
            .command(&[subcommand.arg(), interface, STDIN_PATH])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("cannot run wg: {e}"))?;
        // The config is far below a pipe's capacity, so this write cannot block on a full pipe.
        let wrote = child
            .stdin
            .take()
            .map(|mut stdin| stdin.write_all(config.as_bytes()))
            .unwrap_or(Ok(()));
        let status = child
            .wait()
            .map_err(|e| format!("cannot wait for wg: {e}"))?;
        if wrote.is_err() || !status.success() {
            return Err(format!(
                "wg {} {interface} failed ({status})",
                subcommand.arg()
            ));
        }
        Ok(())
    }
}

/// The two things the applier asks of `nft`. A trait only so tests can stand in for the program.
pub trait Firewall {
    /// What `nft list set` prints for the interface's set of pairs.
    fn list_links(&self, interface: &str) -> Result<String, String>;
    /// Runs a script made by `links::script`.
    fn set_links(&self, interface: &str, script: &str) -> Result<(), String>;
}

/// How long `nft` may take. The pairs are set before `wg` runs, so a program that hangs instead
/// of failing would hold up a revocation until systemd's own timeout; this is shorter.
const NFT_DEADLINE: Duration = Duration::from_secs(10);
const NFT_POLL: Duration = Duration::from_millis(25);
/// More than any listing of 256 pairs can be.
const NFT_MAX_OUTPUT: u64 = 1024 * 1024;

/// The real `nft`, found and run the way `wg` is: by absolute path, root's own, empty environment.
pub struct SystemNft {
    explicit: Option<PathBuf>,
    /// Set by tests, which cannot own a root-owned program.
    resolved: Option<PathBuf>,
}

impl SystemNft {
    /// Nothing is looked for yet: a version 1 file never needs `nft`, so a hub without it is fine
    /// until a version 2 file arrives.
    pub fn new(explicit: Option<&Path>) -> Self {
        Self {
            explicit: explicit.map(Path::to_path_buf),
            resolved: None,
        }
    }

    fn resolve(&self) -> Result<PathBuf, String> {
        if let Some(path) = &self.resolved {
            return Ok(path.clone());
        }
        let candidates: Vec<PathBuf> = match &self.explicit {
            Some(path) => vec![path.clone()],
            None => NFT_CANDIDATES.iter().map(PathBuf::from).collect(),
        };
        let mut last = "nft was not found in a standard location".to_string();
        for path in candidates {
            match trusted_binary(&path) {
                Ok(()) => return Ok(path),
                Err(why) => last = format!("{}: {why}", path.display()),
            }
        }
        Err(last)
    }

    /// Runs `nft` with `args`, gives it `input` on standard input if there is any, and waits at
    /// most `deadline`. A program that is still running then is killed.
    fn run(
        &self,
        args: &[&str],
        input: Option<&[u8]>,
        quiet: bool,
        deadline: Duration,
    ) -> Result<(std::process::ExitStatus, Vec<u8>), String> {
        let mut command = Command::new(self.resolve()?);
        command
            .args(args)
            .env_clear()
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(if quiet {
                Stdio::null()
            } else {
                Stdio::inherit()
            });
        let mut child = command
            .spawn()
            .map_err(|e| format!("cannot run nft: {e}"))?;
        if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
            // The script is far below a pipe's capacity, so this write cannot block on a full
            // pipe. The pipe closes when `stdin` is dropped, which is what ends nft's read.
            let _ = stdin.write_all(bytes);
        }
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let mut out = Vec::new();
                    if let Some(stdout) = child.stdout.take() {
                        let _ = stdout.take(NFT_MAX_OUTPUT).read_to_end(&mut out);
                    }
                    return Ok((status, out));
                }
                Ok(None) if started.elapsed() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "nft did not finish within {} seconds and was stopped",
                        deadline.as_secs()
                    ));
                }
                Ok(None) => std::thread::sleep(NFT_POLL),
                Err(e) => return Err(format!("cannot wait for nft: {e}")),
            }
        }
    }
}

impl Firewall for SystemNft {
    fn list_links(&self, interface: &str) -> Result<String, String> {
        let args = links::list_args(interface).ok_or("the interface name is not valid")?;
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (status, out) = self.run(&args, None, true, NFT_DEADLINE)?;
        if !status.success() {
            return Err(format!("nft list set failed ({status})"));
        }
        String::from_utf8(out).map_err(|_| "nft printed bytes that are not text".into())
    }

    fn set_links(&self, interface: &str, script: &str) -> Result<(), String> {
        let (status, _) = self.run(
            &["-f", NFT_STDIN],
            Some(script.as_bytes()),
            false,
            NFT_DEADLINE,
        )?;
        if !status.success() {
            return Err(format!(
                "nft failed to set the links for {interface} ({status}); is the table from step 3 loaded? If forwarding is on for {interface} and the table is missing, nothing limits which devices can reach each other"
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum ApplyError {
    Input(InputError),
    Rejected(Reject),
    Wg(String),
    /// The peers were applied but the pairs could not be set. Forwarding between devices is as it
    /// was before the run, which can be more than the file allows.
    Links(String),
    /// A later round failed after an earlier one had already changed the interface. The peers of
    /// that earlier round stay in place, so the message says how many.
    AfterApply {
        applied: usize,
        cause: Box<ApplyError>,
    },
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApplyError::Input(e) => e.fmt(f),
            ApplyError::Rejected(e) => e.fmt(f),
            ApplyError::Wg(e) => f.write_str(e),
            ApplyError::Links(e) => f.write_str(e),
            ApplyError::AfterApply { applied, cause } => write!(
                f,
                "{cause}. The input changed after {applied} peer(s) had already been applied; they stay on the interface until the next run succeeds"
            ),
        }
    }
}

/// What one round applied: the bytes it read, the peers in them, and the changes it made.
struct Round {
    bytes: Vec<u8>,
    peers: Vec<grammar::Peer>,
    changes: Vec<String>,
    /// What happened to the pairs: `None` for a version 1 file, which does not mention them.
    links: Option<LinksOutcome>,
}

struct LinksOutcome {
    count: usize,
    changes: Vec<String>,
    /// Why the set could not be updated, if it could not.
    error: Option<String>,
}

/// Linked addresses whose device has a different key than the interface has now. A pair names an
/// address, not a key, so whoever holds the new key inherits the pair; the journal says so.
fn rekeyed_linked(
    current: &std::collections::BTreeMap<String, String>,
    peers: &[grammar::Peer],
    wanted: &[Link],
) -> Vec<Ipv4Addr> {
    let mut addresses: Vec<Ipv4Addr> = wanted.iter().flat_map(|l| [l.a, l.b]).collect();
    addresses.sort();
    addresses.dedup();
    addresses
        .into_iter()
        .filter(|address| {
            let text = format!("{address}/32");
            let before = current
                .iter()
                .find(|(_, allowed)| allowed.split(',').any(|a| a.trim() == text))
                .map(|(key, _)| key);
            let after = peers.iter().find(|p| p.address == *address);
            matches!((before, after), (Some(old), Some(new)) if *old != new.public_key)
        })
        .collect()
}

/// Puts the pairs of a version 2 file in the firewall's set. The previous contents are read first
/// only to say what changed; nothing is decided from them.
fn set_links(
    config: &Config,
    fw: &dyn Firewall,
    wanted: &[Link],
    rekeyed: &[Ipv4Addr],
) -> LinksOutcome {
    let previous = fw
        .list_links(&config.interface)
        .ok()
        .and_then(|listing| links::current_pairs(&listing));
    let mut changes = links::changes(previous.as_ref(), wanted);
    changes.extend(rekeyed.iter().map(|address| {
        format!("note: the device at {address} is in a link and now has a different key")
    }));
    let error = match links::script(&config.interface, wanted) {
        Some(script) => fw.set_links(&config.interface, &script).err(),
        None => Some("the interface name is not valid".to_string()),
    };
    LinksOutcome {
        count: wanted.len(),
        changes,
        error,
    }
}

/// One round: read the input, validate it, and only if every line is acceptable ask `wg`. A refused
/// file never causes `wg` to be started. Returns the bytes applied, the peers, and the changes
/// made to the interface.
fn apply_once(
    config: &Config,
    wg: &dyn Wg,
    fw: &dyn Firewall,
    out: &mut dyn Write,
) -> Result<Round, ApplyError> {
    let bytes = read_input(&config.input, config.owner_uid).map_err(ApplyError::Input)?;
    let grammar::Parsed { peers, links } =
        grammar::parse_file(&bytes, &config.policy).map_err(ApplyError::Rejected)?;
    let current = wg.show_config(&config.interface).map_err(ApplyError::Wg)?;
    let changes = wgconf::changes(&wgconf::current_peers(&current), &peers);
    let (subcommand, text) =
        wgconf::build(&peers, &current).map_err(|e| ApplyError::Wg(e.to_string()))?;
    // Pairs first: closing one must not wait for anything, and a failure here must not stop wg.
    let links = links.map(|wanted| {
        let rekeyed = rekeyed_linked(&wgconf::current_peers(&current), &peers, &wanted);
        set_links(config, fw, &wanted, &rekeyed)
    });
    if let Err(e) = wg.apply(&config.interface, subcommand, &text) {
        // The kernel may have taken some of the update before wg failed, so what the run meant to
        // change is on record even though it did not finish.
        log_attempt(out, config, &peers, &changes);
        if let Some(outcome) = &links {
            log_links(out, config, outcome);
        }
        return Err(ApplyError::Wg(e));
    }
    Ok(Round {
        bytes,
        peers,
        changes,
        links,
    })
}

/// The journal record of a round that changed the interface: how many peers it has now and what
/// was added, removed or moved. An unchanged interface gets one line, so a timer that runs every
/// few minutes does not fill the journal. It is written the moment `wg` has succeeded, before the
/// input is read again, so a peer that went live is on record even if a later round is refused.
fn log_round(out: &mut dyn Write, config: &Config, peers: &[grammar::Peer], changes: &[String]) {
    let mut text = if changes.is_empty() {
        format!(
            "hq-net-apply: no change, {} peer(s) on {}\n",
            peers.len(),
            config.interface
        )
    } else {
        format!(
            "hq-net-apply: {} peer(s) on {} after this run\n",
            peers.len(),
            config.interface
        )
    };
    for change in changes {
        text.push_str(&format!("hq-net-apply:   {change}\n"));
    }
    // A journal that cannot be written to must not stop a revocation that has already happened.
    let _ = out.write_all(text.as_bytes()).and_then(|()| out.flush());
}

/// The journal record of the pairs: what was allowed or removed, or why the set could not be
/// changed. A run that changed nothing gets one line.
fn log_links(out: &mut dyn Write, config: &Config, outcome: &LinksOutcome) {
    let text = match &outcome.error {
        Some(why) => format!(
            "hq-net-apply: could not set the links on {}: {why}. Forwarding between devices is unchanged and may allow more than the file does; the peers were still applied\n",
            config.interface
        ),
        None if outcome.changes.is_empty() => format!(
            "hq-net-apply: no link change, {} link(s) on {}\n",
            outcome.count, config.interface
        ),
        None => {
            let mut text = format!(
                "hq-net-apply: {} link(s) on {} after this run\n",
                outcome.count, config.interface
            );
            for change in &outcome.changes {
                text.push_str(&format!("hq-net-apply:   {change}\n"));
            }
            text
        }
    };
    let _ = out.write_all(text.as_bytes()).and_then(|()| out.flush());
}

/// The journal record of a run whose `wg` call failed: what it was trying to do. Part of it may
/// be on the interface.
fn log_attempt(out: &mut dyn Write, config: &Config, peers: &[grammar::Peer], changes: &[String]) {
    let mut text = format!(
        "hq-net-apply: wg failed while putting {} peer(s) on {}; part of this may already be on the interface. The run was trying to make these changes:\n",
        peers.len(),
        config.interface
    );
    if changes.is_empty() {
        text.push_str("hq-net-apply:   attempted no change to the peer list\n");
    }
    for change in changes {
        text.push_str(&format!("hq-net-apply:   attempted {change}\n"));
    }
    let _ = out.write_all(text.as_bytes()).and_then(|()| out.flush());
}

/// Applies the input, then reads it again: systemd stops watching the file while this runs, so a
/// change made meanwhile would otherwise wait for the timer. Returns the peers the interface now
/// has. A refusal changes nothing; a failure inside `wg` can leave a partial update (a large
/// peer list goes to the kernel in more than one message), which the next run repairs. If a later
/// round fails, the error says how many peers an earlier round had already applied.
pub fn run(
    config: &Config,
    wg: &dyn Wg,
    fw: &dyn Firewall,
    out: &mut dyn Write,
) -> Result<Vec<grammar::Peer>, ApplyError> {
    let first = apply_once(config, wg, fw, out)?;
    log_round(out, config, &first.peers, &first.changes);
    let mut links_error = log_and_take_error(out, config, first.links.as_ref());
    let (mut applied_bytes, mut peers) = (first.bytes, first.peers);
    for _ in 1..MAX_ROUNDS {
        let after = |cause: ApplyError, peers: &[grammar::Peer]| ApplyError::AfterApply {
            applied: peers.len(),
            cause: Box::new(cause),
        };
        let latest = read_input(&config.input, config.owner_uid)
            .map_err(|e| after(ApplyError::Input(e), &peers))?;
        if latest == applied_bytes {
            break;
        }
        match apply_once(config, wg, fw, out) {
            Ok(next) => {
                log_round(out, config, &next.peers, &next.changes);
                // A later version 1 file does not touch the set, so it does not wash away the
                // failure of an earlier round.
                if next.links.is_some() {
                    links_error = log_and_take_error(out, config, next.links.as_ref());
                }
                (applied_bytes, peers) = (next.bytes, next.peers);
            }
            Err(cause) => return Err(after(cause, &peers)),
        }
    }
    match links_error {
        Some(why) => Err(ApplyError::Links(why)),
        None => Ok(peers),
    }
}

/// Writes the pairs' journal record and returns the error, if the set could not be changed.
fn log_and_take_error(
    out: &mut dyn Write,
    config: &Config,
    outcome: Option<&LinksOutcome>,
) -> Option<String> {
    let outcome = outcome?;
    log_links(out, config, outcome);
    outcome.error.clone()
}

/// The program: returns the exit code. 0 applied, 1 refused or failed, 2 usage.
pub fn main() -> i32 {
    let args: Result<Vec<String>, _> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.into_string())
        .collect();
    let Ok(args) = args else {
        eprintln!("hq-net-apply: arguments must be valid UTF-8\n{USAGE}");
        return 2;
    };
    let config = match parse_args(args) {
        Ok(config) => config,
        Err(why) => {
            eprintln!("hq-net-apply: {why}\n{USAGE}");
            return 2;
        }
    };
    let wg = match SystemWg::locate(config.wg.as_deref()) {
        Ok(wg) => wg,
        Err(why) => {
            eprintln!("hq-net-apply: {why}");
            return 1;
        }
    };
    let nft = SystemNft::new(config.nft.as_deref());
    match run(&config, &wg, &nft, &mut std::io::stdout()) {
        Ok(_) => 0,
        Err(why) => {
            match &why {
                ApplyError::Input(_) | ApplyError::Rejected(_) => {
                    eprintln!("hq-net-apply: nothing applied: {why}")
                }
                ApplyError::Wg(_) => eprintln!("hq-net-apply: failed: {why}"),
                ApplyError::Links(_) => {
                    eprintln!("hq-net-apply: peers applied, links not set: {why}")
                }
                ApplyError::AfterApply { .. } => eprintln!("hq-net-apply: {why}"),
            }
            1
        }
    }
}

#[cfg(test)]
mod tests;
