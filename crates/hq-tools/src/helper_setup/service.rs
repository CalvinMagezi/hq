//! Setting up helpers on a computer HQ has a host on: find what is installed, install what is
//! missing, start each sign-in, and remember what is ready. Everything runs through the host
//! (`HostBackend::run_command`), so it works the same on this machine and on a laptop paired to a
//! phone. Blocking: callers use `agent_host::blocking`.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Result, bail};
use hq_db::Database;
use serde::Serialize;

use super::catalog::{self, CATALOG, Helper};
use super::runner::{Outcome, run_to_end};
use crate::agent_host::{CommandRun, HostBackend};

const INSTALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const DETECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Looks for programs the way a login shell would, since a host started by the system often has
/// a shorter PATH than the person's terminal.
const DETECT_SCRIPT: &str = "PATH=\"$HOME/.local/bin:$HOME/.bun/bin:$HOME/.cargo/bin:$HOME/.npm-global/bin:/opt/homebrew/bin:/usr/local/bin:$PATH\"; for p in \"$@\"; do command -v \"$p\" >/dev/null 2>&1 && echo \"HQ-FOUND:$p\"; done; true";
const FOUND_PREFIX: &str = "HQ-FOUND:";

const SIGNED_OUT_WORDS: &[&str] = &[
    "not logged in",
    "not signed in",
    "logged out",
    "unauthenticated",
    "please log in",
    "login required",
];

static PANE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn pane_name(kind: &str) -> String {
    let n = PANE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{kind}-{}-{n}", std::process::id() % 10_000)
}

/// One setup at a time per computer, so two taps never run two installers over each other.
fn busy() -> &'static Mutex<HashSet<String>> {
    static BUSY: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    BUSY.get_or_init(Default::default)
}

pub struct HostGuard(String);

impl HostGuard {
    pub fn take(host: &str) -> Result<Self> {
        let mut set = busy().lock().unwrap_or_else(|e| e.into_inner());
        if !set.insert(host.to_string()) {
            bail!("A helper setup is already running on '{host}'. Wait for it to finish.");
        }
        Ok(Self(host.to_string()))
    }
}

impl Drop for HostGuard {
    fn drop(&mut self) {
        busy()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HelperReport {
    pub helper: String,
    pub title: String,
    pub installed: bool,
    pub signed_in: Option<bool>,
    /// Installed and signed in: a session can start now.
    pub ready: bool,
    /// What happened, in the owner's words.
    pub message: String,
}

/// Which of `names` are on the computer. The host's own lookup first, then a login-shell style
/// lookup for what that missed.
pub fn detect(host: &dyn HostBackend, names: &[&str]) -> Result<HashSet<String>> {
    let wanted: Vec<String> = names.iter().map(|n| n.to_string()).collect();
    let mut found: HashSet<String> = host
        .installed_programs(&wanted)
        .unwrap_or_default()
        .into_iter()
        .collect();
    let missing: Vec<&String> = wanted.iter().filter(|n| !found.contains(*n)).collect();
    if missing.is_empty() {
        return Ok(found);
    }
    let mut argv = vec![
        "bash".into(),
        "-c".into(),
        DETECT_SCRIPT.into(),
        "hq-detect".into(),
    ];
    argv.extend(missing.into_iter().cloned());
    let run = CommandRun {
        name: pane_name("hqdetect"),
        argv,
        env: Vec::new(),
    };
    let out = run_to_end(host, &run, DETECT_TIMEOUT, |_, _| {})
        .map_err(|e| anyhow::anyhow!("could not look at that computer: {e}"))?;
    found.extend(
        out.tail
            .lines()
            .filter_map(|l| l.trim().strip_prefix(FOUND_PREFIX))
            .map(|n| n.trim().to_string()),
    );
    Ok(found)
}

/// Runs a step, recording it so the apps can follow it. The outcome is returned either way.
fn step(
    host: &dyn HostBackend,
    db: &Database,
    helper: &Helper,
    step: &str,
    argv: Vec<String>,
    timeout: Duration,
) -> Result<Outcome> {
    let run_id = hq_db::helper_status::start_run(db, host.name(), helper.id, step)?;
    let run = CommandRun {
        name: pane_name("hqsetup"),
        argv,
        env: Vec::new(),
    };
    let result = run_to_end(host, &run, timeout, |tail, urls| {
        if let Err(e) = hq_db::helper_status::note_run(db, run_id, tail, urls) {
            tracing::warn!(error = %e, "recording a setup step's progress failed");
        }
    });
    match result {
        Ok(out) => {
            hq_db::helper_status::finish_run(db, run_id, out.ok(), &out.tail, &out.urls)?;
            Ok(out)
        }
        Err(e) => {
            let text = format!("Could not run it on that computer: {e}");
            hq_db::helper_status::finish_run(db, run_id, false, &text, &[])?;
            bail!(text)
        }
    }
}

fn says_signed_out(text: &str) -> bool {
    let lower = text.to_lowercase();
    SIGNED_OUT_WORDS.iter().any(|w| lower.contains(w))
}

/// Looks at one helper: is it there, and is it signed in where HQ can ask.
pub fn check(host: &dyn HostBackend, db: &Database, helper: &Helper) -> Result<HelperReport> {
    let found = detect(host, helper.programs)?;
    let installed = helper.programs.iter().any(|p| found.contains(*p));
    let mut signed_in = None;
    let mut error = None;
    if installed && let Some(status) = &helper.status {
        let argv = status.argv.iter().map(|a| a.to_string()).collect();
        match step(host, db, helper, "check", argv, CHECK_TIMEOUT) {
            Ok(out) if out.timed_out => {
                error = Some("Asking it whether it is signed in timed out.")
            }
            Ok(out) => signed_in = Some(out.ok() && !says_signed_out(&out.tail)),
            Err(_) => error = Some("Could not ask it whether it is signed in."),
        }
    }
    hq_db::helper_status::record(db, host.name(), helper.id, installed, signed_in, error)?;
    report(db, host.name(), helper)
}

/// Installs one helper with the first recipe whose programs are on the computer.
pub fn install(host: &dyn HostBackend, db: &Database, helper: &Helper) -> Result<HelperReport> {
    let needs: Vec<&str> = helper
        .installs
        .iter()
        .flat_map(|i| i.needs)
        .copied()
        .collect();
    let present = detect(host, &needs)?;
    let Some(recipe) = helper
        .installs
        .iter()
        .find(|i| i.needs.iter().all(|n| present.contains(*n)))
    else {
        let missing = helper
            .installs
            .iter()
            .map(|i| i.needs.join(" and "))
            .collect::<Vec<_>>()
            .join(", or ");
        let message = format!(
            "{} cannot be installed from here yet: that computer needs {missing}.",
            helper.title
        );
        hq_db::helper_status::record(db, host.name(), helper.id, false, None, Some(&message))?;
        return report(db, host.name(), helper);
    };
    let argv = vec!["bash".into(), "-c".into(), recipe.script.to_string()];
    let out = step(host, db, helper, "install", argv, INSTALL_TIMEOUT)?;
    let found = detect(host, helper.programs)?;
    let installed = helper.programs.iter().any(|p| found.contains(*p));
    let error = match (installed, out.ok()) {
        (true, _) => {
            let seen = host
                .installed_programs(
                    &helper
                        .programs
                        .iter()
                        .map(|p| p.to_string())
                        .collect::<Vec<_>>(),
                )
                .is_none_or(|f| !f.is_empty());
            (!seen).then_some(
                "Installed, but HQ on that computer does not see it yet. Restart HQ there.",
            )
        }
        (false, _) if out.timed_out => Some("The install took too long and was stopped."),
        (false, _) => {
            Some("The installer finished but the helper is not there. See what it printed.")
        }
    };
    hq_db::helper_status::record(db, host.name(), helper.id, installed, None, error)?;
    report(db, host.name(), helper)
}

/// Starts the helper's sign-in on the computer and waits for the person to finish it.
pub fn login(host: &dyn HostBackend, db: &Database, helper: &Helper) -> Result<HelperReport> {
    let argv = helper.login.iter().map(|a| a.to_string()).collect();
    let out = step(host, db, helper, "login", argv, LOGIN_TIMEOUT)?;
    let (signed_in, error) = if out.ok() {
        (Some(true), None)
    } else if out.timed_out {
        (
            None,
            Some("The sign-in was not finished in time. Start it again when you are ready."),
        )
    } else {
        (
            None,
            Some("The sign-in did not finish. See what it printed."),
        )
    };
    hq_db::helper_status::record(db, host.name(), helper.id, true, signed_in, error)?;
    report(db, host.name(), helper)
}

/// Installs the helper if it is missing and signs it in if it is not signed in.
pub fn setup_one(host: &dyn HostBackend, db: &Database, helper: &Helper) -> Result<HelperReport> {
    let mut r = check(host, db, helper)?;
    if !r.installed {
        r = install(host, db, helper)?;
    }
    if r.installed && r.signed_in != Some(true) {
        r = login(host, db, helper)?;
    }
    Ok(r)
}

fn report(db: &Database, host: &str, helper: &Helper) -> Result<HelperReport> {
    let row = hq_db::helper_status::list(db)?
        .into_iter()
        .find(|s| s.host == host && s.helper == helper.id);
    let (installed, signed_in, error) = row
        .map(|s| (s.installed, s.signed_in, s.last_error))
        .unwrap_or((false, None, None));
    let ready = installed && signed_in == Some(true);
    let message = match (installed, signed_in, error) {
        (_, _, Some(e)) => e,
        (false, _, None) => format!("{} is not installed on {host}.", helper.title),
        (true, Some(true), None) => {
            format!("{} is installed and signed in on {host}.", helper.title)
        }
        (true, Some(false), None) => {
            format!("{} is installed on {host} but not signed in.", helper.title)
        }
        (true, None, None) => format!(
            "{} is installed on {host}. HQ cannot tell whether it is signed in.",
            helper.title
        ),
    };
    Ok(HelperReport {
        helper: helper.id.into(),
        title: helper.title.into(),
        installed,
        signed_in,
        ready,
        message,
    })
}

/// The helpers the owner named, from the catalog. An empty list means all of them.
pub fn pick(ids: &[String]) -> Result<Vec<&'static Helper>> {
    if ids.is_empty() {
        return Ok(CATALOG.iter().collect());
    }
    ids.iter()
        .map(|id| {
            catalog::find(id).ok_or_else(|| {
                anyhow::anyhow!(
                    "no helper called '{id}' (known: {})",
                    CATALOG.iter().map(|h| h.id).collect::<Vec<_>>().join(", ")
                )
            })
        })
        .collect()
}

/// What HQ last saw, helper by helper, for each computer it has a reading for.
pub fn known(db: &Database) -> Result<HashMap<String, Vec<hq_db::helper_status::HelperStatus>>> {
    let mut by_host: HashMap<String, Vec<_>> = HashMap::new();
    for s in hq_db::helper_status::list(db)? {
        by_host.entry(s.host.clone()).or_default().push(s);
    }
    Ok(by_host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_out_wording_is_recognised() {
        assert!(says_signed_out("Status: Not logged in"));
        assert!(says_signed_out("Error: Login required"));
        assert!(!says_signed_out("Logged in as someone@example.com"));
    }

    #[test]
    fn one_setup_at_a_time_per_computer() {
        let first = HostGuard::take("laptop-test").unwrap();
        assert!(HostGuard::take("laptop-test").is_err());
        assert!(HostGuard::take("other-test").is_ok());
        drop(first);
        assert!(HostGuard::take("laptop-test").is_ok());
    }

    #[test]
    fn naming_an_unknown_helper_lists_the_known_ones() {
        let err = pick(&["nope".into()]).unwrap_err().to_string();
        assert!(err.contains("cursor") && err.contains("codex"), "{err}");
        assert_eq!(pick(&[]).unwrap().len(), CATALOG.len());
    }

    #[test]
    fn pane_names_are_valid_host_names_and_unique() {
        let a = pane_name("hqsetup");
        let b = pane_name("hqsetup");
        assert_ne!(a, b);
        assert!(
            a.len() <= 32
                && a.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        );
    }
}
