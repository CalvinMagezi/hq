//! `hq suite sink`, `offload`, `restore-drill`: cold storage for originals, over a folder the owner
//! keeps. The recovery code is read here (from the owner, never from the model) and handed to the
//! pure sink code; it is not stored anywhere.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use hq_suite_shrink::Shrinker;
use hq_suite_sinks::{FolderSink, Listable, Meta, restore_drill};
use hq_suite_store::ColdStore;
use hq_suite_store::{Disposition, Journal, SuitePaths, fsx};

use crate::peer::{PeerSink, parse_addr};
use serde::{Deserialize, Serialize};

use crate::shrink::{VaultNotes, friendly, shrinker};
use crate::text;

const REGISTRY: &str = "_data/suite/sinks.json";
/// The environment variable a script may give the recovery code in, instead of the prompt.
pub const CODE_ENV: &str = "HQ_SUITE_RECOVERY_CODE";
const DEFAULT_SAMPLE: usize = 20;

#[derive(Debug, Default, Serialize, Deserialize)]
struct Registry {
    sinks: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    id: String,
    /// A folder sink's folder. Empty for a peer sink.
    #[serde(default)]
    path: PathBuf,
    /// Set for a sink kept by a paired machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    peer: Option<PeerEntry>,
}

/// Where a peer's store is reached (a loopback forward) and the sink's open description, which
/// holds no secret: without the recovery code it opens nothing.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PeerEntry {
    addr: String,
    meta: Meta,
}

/// A sink as the commands use it: what offload and restore need, and what a drill lists.
trait Sink: ColdStore + Listable {}
impl<T: ColdStore + Listable> Sink for T {}

fn registry_path(paths: &SuitePaths) -> Result<PathBuf> {
    Ok(paths.resolve(REGISTRY)?)
}

fn load(paths: &SuitePaths) -> Result<Registry> {
    match std::fs::read(registry_path(paths)?) {
        Ok(b) => serde_json::from_slice(&b).context("the list of sinks is damaged"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Registry::default()),
        Err(e) => Err(e.into()),
    }
}

fn save(paths: &SuitePaths, reg: &Registry) -> Result<()> {
    fsx::atomic_write(&registry_path(paths)?, &serde_json::to_vec_pretty(reg)?)?;
    Ok(())
}

fn entry(paths: &SuitePaths, id: &str) -> Result<Entry> {
    load(paths)?
        .sinks
        .into_iter()
        .find(|e| e.id == id)
        .with_context(|| format!("there is no sink called {id}; see hq suite sink list"))
}

/// The code from the environment, else typed at the prompt, else the first line of a pipe.
fn read_code() -> Result<String> {
    if let Ok(code) = std::env::var(CODE_ENV)
        && !code.trim().is_empty()
    {
        return Ok(code);
    }
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        print!("{}", text::ask_recovery_code());
        std::io::stdout().flush()?;
    }
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;
    if line.trim().is_empty() {
        bail!("no recovery code was given");
    }
    Ok(line)
}

fn open(paths: &SuitePaths, id: &str) -> Result<Box<dyn Sink>> {
    open_with_code(paths, id, &read_code()?)
}

/// The sink `id`, opened with a recovery code the caller already has (the phone asks the person and
/// sends it for this one call; nothing keeps it).
fn open_with_code(paths: &SuitePaths, id: &str, code: &str) -> Result<Box<dyn Sink>> {
    let e = entry(paths, id)?;
    if let Some(peer) = e.peer {
        let addr = parse_addr(&peer.addr).map_err(|e| anyhow::anyhow!("{e}"))?;
        let sink = PeerSink::open(&peer.meta, code, addr).map_err(|e| anyhow::anyhow!("{e}"))?;
        return Ok(Box::new(sink));
    }
    let sink = FolderSink::open(&e.path, code).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(Box::new(sink))
}

/// `hq suite sink register <folder>`: adds a sink that already exists (made on another machine, or
/// before the phone was lost) to this vault's list, after checking its recovery code.
pub fn register(vault: &Path, folder: &Path) -> Result<()> {
    let paths = SuitePaths::new(vault);
    let folder = std::fs::canonicalize(folder)
        .with_context(|| format!("{} cannot be reached", folder.display()))?;
    let sink = FolderSink::open(&folder, &read_code()?).map_err(|e| anyhow::anyhow!("{e}"))?;
    let id = sink.id().to_string();
    let mut reg = load(&paths)?;
    // A vault restored on another machine already lists the sink under the old machine's path;
    // registering points that entry at the folder as it is here.
    match reg.sinks.iter_mut().find(|e| e.id == id) {
        Some(existing) => existing.path = folder,
        None => reg.sinks.push(Entry {
            id: id.clone(),
            path: folder,
            peer: None,
        }),
    }
    save(&paths, &reg)?;
    print!("{}", text::sink_registered(&id));
    Ok(())
}

/// `hq suite restore-originals --sink <id>`: every original the vault's notes say was sealed into
/// the sink, put back where it was.
pub fn restore_all(vault: &Path, sink: &str) -> Result<()> {
    let paths = SuitePaths::new(vault);
    let store = open(&paths, sink)?;
    let r = crate::restore::restore_originals(vault, &*store)?;
    print!("{}", text::restored(&r));
    if r.failed.is_empty() && r.in_the_way.is_empty() {
        Ok(())
    } else {
        bail!("some originals could not be put back")
    }
}

/// `hq suite sink add <id> <folder>`: makes the sink and shows its recovery code once.
pub fn add(vault: &Path, id: &str, folder: &Path) -> Result<()> {
    let paths = SuitePaths::new(vault);
    let mut reg = load(&paths)?;
    if reg.sinks.iter().any(|e| e.id == id) {
        bail!("there is already a sink called {id}");
    }
    let folder = std::fs::canonicalize(folder)
        .or_else(|_| std::fs::create_dir_all(folder).and_then(|()| std::fs::canonicalize(folder)))
        .with_context(|| format!("cannot use {}", folder.display()))?;
    if folder.starts_with(vault) {
        bail!(
            "a sink inside the vault would be lost with the phone it is meant to protect; choose a card, a drive or another machine's folder"
        );
    }
    let (_, code) = FolderSink::create(&folder, id).map_err(|e| anyhow::anyhow!("{e}"))?;
    reg.sinks.push(Entry {
        id: id.to_string(),
        path: folder.clone(),
        peer: None,
    });
    save(&paths, &reg)?;
    print!(
        "{}",
        text::sink_made(id, &folder.display().to_string(), &code)
    );
    Ok(())
}

pub fn list(vault: &Path) -> Result<()> {
    let paths = SuitePaths::new(vault);
    let reg = load(&paths)?;
    if reg.sinks.is_empty() {
        print!("{}", text::no_sinks());
    }
    for e in reg.sinks {
        let (place, reachable) = match &e.peer {
            Some(p) => (
                format!("peer {}", p.addr),
                parse_addr(&p.addr).is_ok_and(PeerSink::reachable),
            ),
            None => (
                e.path.display().to_string(),
                e.path.join("suite-sink.json").is_file(),
            ),
        };
        println!(
            "{}  {}  {}",
            e.id,
            place,
            if reachable {
                "reachable"
            } else {
                "not reachable now"
            }
        );
    }
    Ok(())
}

/// `hq suite offload <job> --sink <id>`: seal the original into the sink and remove it from the
/// phone, only after it has been read back.
pub fn offload(vault: &Path, job: &str, sink: &str) -> Result<()> {
    let paths = SuitePaths::new(vault);
    let notes = VaultNotes::new(paths.clone());
    let source = Journal::new(&paths)
        .load(job)
        .map(|j| j.source)
        .unwrap_or_default();
    let sink = open(&paths, sink)?;
    let out = shrinker(&paths, &notes)
        .offload(job, &*sink)
        .map_err(friendly)?;
    print!(
        "{}",
        text::offloaded(&source, sink.id(), out.saved_bytes, job)
    );
    Ok(())
}

/// Undoes a job whose original went to a sink: fetches it back. For a job that went to the trash
/// the caller uses the plain undo.
pub fn undo(vault: &Path, job: &str) -> Result<bool> {
    let paths = SuitePaths::new(vault);
    let loaded = Journal::new(&paths).load(job);
    let Ok(j) = loaded else {
        return Ok(false);
    };
    let Some(Disposition::Sink { sink, .. }) = j.disposition else {
        return Ok(false);
    };
    let notes = VaultNotes::new(paths.clone());
    let store = open(&paths, &sink)?;
    let s: Shrinker<'_> = shrinker(&paths, &notes);
    s.undo_with(job, &*store).map_err(friendly)?;
    print!("{}", text::undone(&j.source));
    Ok(true)
}

/// `hq suite restore-drill <sink>`: fetches a spread of stored originals and checks each opens and
/// hashes to its name.
pub fn drill(vault: &Path, sink: &str, sample: Option<usize>) -> Result<()> {
    let paths = SuitePaths::new(vault);
    let store = open(&paths, sink)?;
    let result = restore_drill(&*store, sample.unwrap_or(DEFAULT_SAMPLE));
    print!("{}", text::drill(&result));
    if result.failed.is_empty() {
        Ok(())
    } else {
        bail!("{} original(s) could not be restored", result.failed.len())
    }
}

/// `hq suite sink add-peer <id> <addr>`: a sink kept by a paired machine, reached through a
/// loopback forward to that machine's `hq suite sink peer-serve`. Shows the recovery code once.
pub fn add_peer(vault: &Path, id: &str, addr: &str) -> Result<()> {
    let code = make_peer(vault, id, addr)?;
    print!(
        "{}",
        text::sink_made(id, &format!("the peer at {addr}"), &code)
    );
    Ok(())
}

/// Makes a peer sink and returns its recovery code, which is shown once and kept nowhere.
pub fn make_peer(vault: &Path, id: &str, addr: &str) -> Result<String> {
    let paths = SuitePaths::new(vault);
    let mut reg = load(&paths)?;
    if reg.sinks.iter().any(|e| e.id == id) {
        bail!("there is already a sink called {id}");
    }
    let sock = parse_addr(addr).map_err(|e| anyhow::anyhow!("{e}"))?;
    if !PeerSink::reachable(sock) {
        bail!(
            "nothing answers at {addr}; start `hq suite sink peer-serve` on the peer and forward its port here, then try again"
        );
    }
    let (meta, _, code) = Meta::make(id).map_err(|e| anyhow::anyhow!("{e}"))?;
    PeerSink::publish_meta(sock, &meta).map_err(|e| anyhow::anyhow!("{e}"))?;
    reg.sinks.push(Entry {
        id: id.to_string(),
        path: PathBuf::new(),
        peer: Some(PeerEntry {
            addr: addr.to_string(),
            meta,
        }),
    });
    save(&paths, &reg)?;
    Ok(code)
}

/// A sink as a screen shows it.
#[derive(Debug, Clone, Serialize)]
pub struct SinkInfo {
    pub id: String,
    /// `folder` or `peer`.
    pub kind: &'static str,
    /// Where it is, in words: a folder path or the peer's forwarded address.
    pub place: String,
    pub reachable: bool,
}

/// Every sink this vault knows and whether it answers now.
pub fn infos(vault: &Path) -> Result<Vec<SinkInfo>> {
    let paths = SuitePaths::new(vault);
    Ok(load(&paths)?
        .sinks
        .into_iter()
        .map(|e| match &e.peer {
            Some(p) => SinkInfo {
                id: e.id.clone(),
                kind: "peer",
                place: p.addr.clone(),
                reachable: parse_addr(&p.addr).is_ok_and(PeerSink::reachable),
            },
            None => SinkInfo {
                id: e.id.clone(),
                kind: "folder",
                place: e.path.display().to_string(),
                reachable: e.path.join("suite-sink.json").is_file(),
            },
        })
        .collect())
}

/// Fetches a spread of the sink's originals and checks each opens and matches, with a recovery
/// code the caller gives. A wrong code is refused before anything is read.
pub fn drill_with_code(
    vault: &Path,
    id: &str,
    code: &str,
    sample: Option<usize>,
) -> Result<hq_suite_sinks::Drill> {
    let paths = SuitePaths::new(vault);
    let store = open_with_code(&paths, id, code)?;
    Ok(restore_drill(&*store, sample.unwrap_or(DEFAULT_SAMPLE)))
}

/// `hq suite sink register-peer <addr>`: adds a peer sink that already exists (this phone was
/// replaced) from the description the peer holds, after checking the recovery code.
pub fn register_peer(vault: &Path, addr: &str) -> Result<()> {
    let paths = SuitePaths::new(vault);
    let sock = parse_addr(addr).map_err(|e| anyhow::anyhow!("{e}"))?;
    let meta = PeerSink::fetch_meta(sock).map_err(|e| anyhow::anyhow!("{e}"))?;
    PeerSink::open(&meta, &read_code()?, sock).map_err(|e| anyhow::anyhow!("{e}"))?;
    let id = meta.id().to_string();
    let mut reg = load(&paths)?;
    let peer = PeerEntry {
        addr: addr.to_string(),
        meta,
    };
    match reg.sinks.iter_mut().find(|e| e.id == id) {
        Some(existing) => existing.peer = Some(peer),
        None => reg.sinks.push(Entry {
            id: id.clone(),
            path: PathBuf::new(),
            peer: Some(peer),
        }),
    }
    save(&paths, &reg)?;
    print!("{}", text::sink_registered(&id));
    Ok(())
}

/// `hq suite sink peer-serve <dir> --port <p>`: keeps sealed originals for another machine's sink
/// on this machine's loopback address, until stopped. HQ's pairing exposes the port to the peer.
pub fn peer_serve(dir: &Path, port: u16) -> Result<()> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))
        .with_context(|| format!("cannot listen on 127.0.0.1:{port}"))?;
    println!(
        "Keeping sealed originals in {} for a paired machine, on 127.0.0.1:{port}. Stop with Ctrl-C.",
        dir.display()
    );
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    crate::peer::serve(dir, listener, stop)?;
    Ok(())
}

#[cfg(test)]
mod screen_tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::{Arc, atomic::AtomicBool};

    #[test]
    fn a_peer_sink_is_made_listed_and_checked_with_its_code_and_a_wrong_code_is_refused() {
        let vault = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let (dir, s2) = (store.path().to_path_buf(), Arc::clone(&stop));
        let server = std::thread::spawn(move || crate::peer::serve(&dir, listener, s2).unwrap());
        let code = make_peer(vault.path(), "phone-backup", &addr).unwrap();
        assert!(
            make_peer(vault.path(), "phone-backup", &addr).is_err(),
            "a name is used once"
        );
        let listed = infos(vault.path()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            (listed[0].id.as_str(), listed[0].kind, listed[0].reachable),
            ("phone-backup", "peer", true)
        );
        let drill = drill_with_code(vault.path(), "phone-backup", &code, None).unwrap();
        assert_eq!(
            (drill.checked, drill.failed.len()),
            (0, 0),
            "nothing is stored yet"
        );
        assert!(
            drill_with_code(
                vault.path(),
                "phone-backup",
                "AAAA-BBBB-CCCC-DDDD-EEEE",
                None
            )
            .is_err()
        );
        assert!(drill_with_code(vault.path(), "nope", &code, None).is_err());
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        server.join().unwrap();
        assert!(
            !infos(vault.path()).unwrap()[0].reachable,
            "a stopped peer is not reachable now"
        );
    }
}
