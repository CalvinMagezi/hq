//! `hq suite sink`, `offload`, `restore-drill`: cold storage for originals, over a folder the owner
//! keeps. The recovery code is read here (from the owner, never from the model) and handed to the
//! pure sink code; it is not stored anywhere.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use hq_suite_shrink::Shrinker;
use hq_suite_sinks::{FolderSink, restore_drill};
use hq_suite_store::{Disposition, Journal, SuitePaths, fsx};
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
    path: PathBuf,
}

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

fn open(paths: &SuitePaths, id: &str) -> Result<FolderSink> {
    let e = entry(paths, id)?;
    FolderSink::open(&e.path, &read_code()?).map_err(|e| anyhow::anyhow!("{e}"))
}

/// `hq suite sink register <folder>`: adds a sink that already exists (made on another machine, or
/// before the phone was lost) to this vault's list, after checking its recovery code.
pub fn register(vault: &Path, folder: &Path) -> Result<()> {
    let paths = SuitePaths::new(vault);
    let folder = std::fs::canonicalize(folder)
        .with_context(|| format!("{} cannot be reached", folder.display()))?;
    let sink = FolderSink::open(&folder, &read_code()?).map_err(|e| anyhow::anyhow!("{e}"))?;
    let id = sink_id(&sink).to_string();
    let mut reg = load(&paths)?;
    // A vault restored on another machine already lists the sink under the old machine's path;
    // registering points that entry at the folder as it is here.
    match reg.sinks.iter_mut().find(|e| e.id == id) {
        Some(existing) => existing.path = folder,
        None => reg.sinks.push(Entry {
            id: id.clone(),
            path: folder,
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
    let r = crate::restore::restore_originals(vault, &store)?;
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
        let reachable = e.path.join("suite-sink.json").is_file();
        println!(
            "{}  {}  {}",
            e.id,
            e.path.display(),
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
        .offload(job, &sink)
        .map_err(friendly)?;
    print!(
        "{}",
        text::offloaded(&source, sink_id(&sink), out.saved_bytes, job)
    );
    Ok(())
}

fn sink_id(s: &FolderSink) -> &str {
    use hq_suite_store::ColdStore;
    s.id()
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
    s.undo_with(job, &store).map_err(friendly)?;
    print!("{}", text::undone(&j.source));
    Ok(true)
}

/// `hq suite restore-drill <sink>`: fetches a spread of stored originals and checks each opens and
/// hashes to its name.
pub fn drill(vault: &Path, sink: &str, sample: Option<usize>) -> Result<()> {
    let paths = SuitePaths::new(vault);
    let store = open(&paths, sink)?;
    let result = restore_drill(&store, sample.unwrap_or(DEFAULT_SAMPLE));
    print!("{}", text::drill(&result));
    if result.failed.is_empty() {
        Ok(())
    } else {
        bail!("{} original(s) could not be restored", result.failed.len())
    }
}
