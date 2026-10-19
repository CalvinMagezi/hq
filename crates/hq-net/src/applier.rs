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
//! What this program does not decide is who may be admitted: anything running as the input's owner
//! can add a device within the subnet. It limits what such a file can do, and it records the
//! peers it applied, by address and a short key fingerprint, so the journal shows every change.

use crate::grammar::{self, MAX_INPUT_BYTES, Policy, Reject};
use crate::subnet::{Subnet, parse_canonical_ipv4};
use crate::wgconf;
use std::ffi::{CString, OsStr};
use std::fmt;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

const USAGE: &str = "usage: hq-net-apply --interface NAME --subnet CIDR --hub ADDRESS --owner-uid UID --input /ABSOLUTE/PATH [--wg /ABSOLUTE/PATH]";
const MAX_INTERFACE_NAME: usize = 15;
/// Where `wg` may be, in order: the distribution's own directories only. The first that exists
/// and is root-owned and not writable by others is used; no `PATH` search ever happens. A `wg` in
/// `/usr/local` (writable by the `staff` group on Debian) needs an explicit `--wg`.
const WG_CANDIDATES: [&str; 4] = ["/usr/bin/wg", "/usr/sbin/wg", "/bin/wg", "/sbin/wg"];
/// How many times a run applies again because the input changed while it was applying. A change
/// after the last re-read is caught by the timer, not by this loop.
const MAX_ROUNDS: usize = 3;
const FINGERPRINT_CHARS: usize = 8;
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
}

pub fn valid_interface(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= MAX_INTERFACE_NAME
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
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
    let mut slots: [Option<String>; 6] = Default::default();
    let flags = [
        "--interface",
        "--subnet",
        "--hub",
        "--owner-uid",
        "--input",
        "--wg",
    ];
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let Some(index) = flags.iter().position(|f| *f == arg) else {
            return Err(format!("unknown argument {arg:?}"));
        };
        take_once(&mut slots[index], &arg, args.next())?;
    }
    let [interface, subnet, hub, owner, input, wg] = slots;
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
    Ok(Config {
        interface,
        policy,
        input,
        owner_uid,
        wg,
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
fn check_parent(dir: &OwnedFd, owner_uid: u32) -> Result<(), InputError> {
    let copy = dir.try_clone().map_err(|e| InputError::Io(e.to_string()))?;
    let meta = File::from(copy)
        .metadata()
        .map_err(|e| InputError::Io(e.to_string()))?;
    let owner_ok = meta.uid() == ROOT_UID || meta.uid() == owner_uid;
    if !owner_ok || meta.mode() & GROUP_OTHER_WRITE != 0 {
        return Err(InputError::ParentUnsafe);
    }
    Ok(())
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
    if !meta.file_type().is_file() {
        return Err(InputError::NotRegular);
    }
    if meta.uid() != owner_uid {
        return Err(InputError::WrongOwner);
    }
    if meta.mode() & GROUP_OTHER_ACCESS != 0 {
        return Err(InputError::TooOpen);
    }
    if meta.nlink() != 1 {
        return Err(InputError::OtherNames);
    }
    if meta.len() > MAX_INPUT_BYTES as u64 {
        return Err(InputError::TooLarge);
    }
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
            .command(&[subcommand.arg(), interface, "/dev/stdin"])
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

#[derive(Debug)]
pub enum ApplyError {
    Input(InputError),
    Rejected(Reject),
    Wg(String),
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApplyError::Input(e) => e.fmt(f),
            ApplyError::Rejected(e) => e.fmt(f),
            ApplyError::Wg(e) => f.write_str(e),
        }
    }
}

/// One round: read the input, validate it, and only if every line is acceptable ask `wg`. A refused
/// file never causes `wg` to be started.
fn apply_once(config: &Config, wg: &dyn Wg) -> Result<(Vec<u8>, Vec<grammar::Peer>), ApplyError> {
    let bytes = read_input(&config.input, config.owner_uid).map_err(ApplyError::Input)?;
    let peers = grammar::parse(&bytes, &config.policy).map_err(ApplyError::Rejected)?;
    let current = wg.show_config(&config.interface).map_err(ApplyError::Wg)?;
    let (subcommand, text) =
        wgconf::build(&peers, &current).map_err(|e| ApplyError::Wg(e.to_string()))?;
    wg.apply(&config.interface, subcommand, &text)
        .map_err(ApplyError::Wg)?;
    Ok((bytes, peers))
}

/// Applies the input, then reads it again: systemd stops watching the file while this runs, so a
/// change made meanwhile would otherwise wait for the timer. Returns the peers the interface now
/// has. A refusal changes nothing; a failure inside `wg` can leave a partial update (a large
/// peer list goes to the kernel in more than one message), which the next run repairs.
pub fn run(config: &Config, wg: &dyn Wg) -> Result<Vec<grammar::Peer>, ApplyError> {
    let (mut applied_bytes, mut peers) = apply_once(config, wg)?;
    for _ in 1..MAX_ROUNDS {
        let latest = read_input(&config.input, config.owner_uid).map_err(ApplyError::Input)?;
        if latest == applied_bytes {
            break;
        }
        (applied_bytes, peers) = apply_once(config, wg)?;
    }
    Ok(peers)
}

/// A line for the journal per peer: the address and the first characters of the public key.
fn peer_log_line(peer: &grammar::Peer) -> String {
    let fingerprint: String = peer.public_key.chars().take(FINGERPRINT_CHARS).collect();
    format!("hq-net-apply:   {} {fingerprint}", peer.address)
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
    match run(&config, &wg) {
        Ok(peers) => {
            println!(
                "hq-net-apply: {} peer(s) applied to {}",
                peers.len(),
                config.interface
            );
            for peer in &peers {
                println!("{}", peer_log_line(peer));
            }
            0
        }
        Err(why) => {
            eprintln!("hq-net-apply: nothing applied: {why}");
            1
        }
    }
}

#[cfg(test)]
mod tests;
