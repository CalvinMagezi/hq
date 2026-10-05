use super::{BinaryStatus, MachineProfile, PROBED_BINARIES, WebSearchBackendStatus};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Binaries whose version is worth the extra subprocess.
const VERSIONED_BINARIES: &[&str] = &["gh", "git", "docker", "node", "python3", "cargo", "bun"];

/// Binaries probed on the fallback path, when no cached profile exists yet.
const FAST_PROBE_BINARIES: &[&str] = &["git", "gh", "node", "python3", "cargo", "docker"];

const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const AUTH_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Locate a binary on `PATH`, with the user bin directories prepended.
///
/// launchd spawns the daemon with a minimal `PATH` that omits `~/.local/bin`
/// and `/opt/homebrew/bin`, so a bare `command -v` reports `gh` and `node` as
/// missing under the daemon while finding them from an interactive shell.
/// Every probe in this module must go through here — a profile that
/// disagrees with reality is worse than no profile.
pub fn which_binary(name: &str) -> Option<PathBuf> {
    let valid = !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !valid {
        return None;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let extra = format!("{home}/.local/bin:{home}/bin:/opt/homebrew/bin:/usr/local/bin");
    // The name is a positional parameter, never part of the script text.
    Command::new("sh")
        .args(["-c", &format!("PATH=\"{extra}:$PATH\" command -v \"$1\""), "sh", name])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            String::from_utf8(o.stdout)
                .ok()
                .map(|s| PathBuf::from(s.trim()))
        })
        .filter(|p| !p.as_os_str().is_empty())
}

/// Run a command with the augmented PATH, killing it if it outruns `timeout`.
///
/// `std::process::Command` has no timeout, and a hung `docker info` against a
/// dead daemon would otherwise stall the whole probe.
fn run_with_timeout(script: &str, timeout: Duration) -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let extra = format!("{home}/.local/bin:{home}/bin:/opt/homebrew/bin:/usr/local/bin");
    let mut child = Command::new("sh")
        .args(["-c", &format!("PATH=\"{extra}:$PATH\" {script}")])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()
        .ok()?;

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = child.wait_with_output().ok()?;
                if !status.success() && out.stdout.is_empty() {
                    // Fall back to stderr: `gh auth status` writes there.
                    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    return if err.is_empty() { None } else { Some(err) };
                }
                let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                return if text.is_empty() { None } else { Some(text) };
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return None,
        }
    }
}

/// First line of `<binary> --version`, with the binary name stripped.
fn probe_version(name: &str) -> Option<String> {
    let raw = run_with_timeout(&format!("{name} --version 2>&1"), PROBE_TIMEOUT)?;
    let first = raw.lines().next()?.trim();
    // "git version 2.51.0" -> "2.51.0"; "docker version 27.3.1, build x" -> "27.3.1";
    // node reports "v22.23.1", so the leading `v` is stripped too.
    let is_version = |tok: &&str| {
        let t = tok.strip_prefix('v').unwrap_or(tok);
        t.chars().next().is_some_and(|c| c.is_ascii_digit())
    };
    let cleaned = first
        .split_whitespace()
        .find(is_version)
        .unwrap_or(first)
        .trim_end_matches(',');
    let cleaned = cleaned.strip_prefix('v').unwrap_or(cleaned);
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.to_string())
    }
}

/// Parse `gh auth status` into "account (scope, scope)".
fn probe_gh_auth() -> Option<String> {
    let raw = run_with_timeout("gh auth status 2>&1", AUTH_PROBE_TIMEOUT)?;
    if !raw.contains("Logged in") {
        return None;
    }
    let account = raw
        .lines()
        .find(|l| l.contains("Logged in"))
        .and_then(|l| {
            let after = l.split("account ").nth(1)?;
            after.split_whitespace().next()
        })
        .unwrap_or("unknown")
        .to_string();
    let scopes = raw
        .lines()
        .find(|l| l.contains("Token scopes:"))
        .and_then(|l| l.split("Token scopes:").nth(1))
        .map(|s| s.replace(['\'', '"'], "").trim().to_string())
        .filter(|s| !s.is_empty());
    Some(match scopes {
        Some(s) => format!("{account} ({s})"),
        None => account,
    })
}

fn probe_git_user() -> Option<String> {
    let name = run_with_timeout("git config --global user.name", PROBE_TIMEOUT)?;
    match run_with_timeout("git config --global user.email", PROBE_TIMEOUT) {
        Some(email) => Some(format!("{name} <{email}>")),
        None => Some(name),
    }
}

fn detect_container() -> bool {
    Path::new("/.dockerenv").exists()
        || std::fs::read_to_string("/proc/1/cgroup")
            .map(|c| c.contains("docker") || c.contains("containerd") || c.contains("kubepods"))
            .unwrap_or(false)
}

/// Probe every binary in `names`, returning (present, missing).
///
/// Probes run on parallel threads: 24 `command -v` calls plus 7 version calls
/// are ~2s serial and ~200ms fanned out.
fn probe_binaries(names: &[&str], with_versions: bool) -> (Vec<BinaryStatus>, Vec<String>) {
    let results: Vec<BinaryStatus> = std::thread::scope(|scope| {
        let handles: Vec<_> = names
            .iter()
            .map(|name| {
                scope.spawn(move || {
                    let path = which_binary(name);
                    let version = match (&path, with_versions && VERSIONED_BINARIES.contains(name))
                    {
                        (Some(_), true) => probe_version(name),
                        _ => None,
                    };
                    BinaryStatus {
                        name: (*name).to_string(),
                        path,
                        version,
                    }
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });

    let (mut present, mut absent): (Vec<_>, Vec<_>) =
        results.into_iter().partition(|b| b.path.is_some());
    present.sort_by(|a, b| a.name.cmp(&b.name));
    absent.sort_by(|a, b| a.name.cmp(&b.name));
    (present, absent.into_iter().map(|b| b.name).collect())
}

/// Probe the host. Blocking — call from `spawn_blocking` in async contexts.
pub fn probe_machine(vault_path: Option<&Path>) -> MachineProfile {
    let (binaries, missing) = probe_binaries(PROBED_BINARIES, true);
    let has = |n: &str| binaries.iter().any(|b| b.name == n);

    let gh_auth = if has("gh") { probe_gh_auth() } else { None };
    let git_user = if has("git") { probe_git_user() } else { None };
    let docker_running = has("docker")
        && run_with_timeout(
            "docker info --format '{{.ServerVersion}}' 2>/dev/null",
            AUTH_PROBE_TIMEOUT,
        )
        .is_some();

    let hardware = crate::hardware::detect_hardware();
    let can_build_self = probe_can_build_self(&binaries);
    let (web_search_backend, web_search) = probe_web_search_from_config();

    MachineProfile {
        generated_at: chrono::Utc::now(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        hostname: hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".into()),
        in_container: detect_container(),
        cpu_cores: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1),
        memory_gb: hardware.total_memory_gb,
        home: dirs::home_dir().unwrap_or_default(),
        vault_path: vault_path.map(|p| p.to_path_buf()),
        binaries,
        missing,
        gh_auth,
        deep_probed: true,
        docker_running,
        git_user,
        can_build_self,
        web_search_backend,
        web_search,
    }
}

/// Cheap existence-only probe for the first session on a host where the
/// daemon has not yet written a profile. No version or auth subprocesses.
pub fn probe_machine_fast(vault_path: Option<&Path>) -> MachineProfile {
    let (binaries, missing) = probe_binaries(FAST_PROBE_BINARIES, false);
    let can_build_self = probe_can_build_self(&binaries);
    // Not a subprocess — a config read plus a short TCP connect — so unlike
    // gh_auth/docker_running there's no reason to skip it on the fast path.
    // A fresh-install / daemon-down session is exactly the case FR-005 wants
    // to declare availability for before first use, not leave silent.
    let (web_search_backend, web_search) = probe_web_search_from_config();
    MachineProfile {
        generated_at: chrono::Utc::now(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        hostname: hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".into()),
        in_container: detect_container(),
        cpu_cores: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1),
        memory_gb: crate::hardware::detect_hardware().total_memory_gb,
        home: dirs::home_dir().unwrap_or_default(),
        vault_path: vault_path.map(|p| p.to_path_buf()),
        binaries,
        missing,
        gh_auth: None,
        deep_probed: false,
        docker_running: false,
        git_user: None,
        can_build_self,
        web_search_backend,
        web_search,
    }
}

/// Locate an agent-hq source checkout.
///
/// A daemon-launched process's cwd tells us nothing: launchd/systemd set
/// `WorkingDirectory` to the vault or `/opt/hq`, never the repo (verified
/// against `scripts/com.agent-hq.hq-all.plist.template` and
/// `deploy/hq.service`), so walking up from `current_dir()` would report
/// "no checkout" even on the machine that built this exact binary. Instead:
///
/// 1. `AGENT_HQ_SRC` wins if set (explicit opt-in, never a hardcoded
///    personal path per this repo's own de-personalization rule).
/// 2. `env!("CARGO_MANIFEST_DIR")` is baked in at *compile* time as the path
///    this crate was built from. Checking it at runtime is a check of "does
///    the checkout that built this binary still exist here" — correct
///    whether or not the checkout was later deleted (a copy-only VPS
///    install per `deploy/README.md`'s "skip to step 3") or is still
///    present (a developer building and running from their own clone).
fn find_agent_hq_checkout() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("AGENT_HQ_SRC") {
        let p = PathBuf::from(explicit);
        if p.join("Cargo.toml").is_file() {
            return Some(p);
        }
    }
    // crates/hq-core -> workspace root.
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .parent()?
        .to_path_buf();
    let candidate = workspace_root.join("Cargo.toml");
    if std::fs::read_to_string(&candidate).is_ok_and(|c| c.contains("hq-cli")) {
        return Some(workspace_root);
    }
    None
}

/// Cheap, subprocess-free check: can this host build its own source? All
/// three conditions or none — a host that can half-build isn't a host that
/// can "verify with cargo before shipping".
fn probe_can_build_self(binaries: &[BinaryStatus]) -> bool {
    let has = |n: &str| binaries.iter().any(|b| b.name == n);
    has("cargo") && has("git") && find_agent_hq_checkout().is_some()
}

/// Whether an agent-hq source checkout is reachable on this host, without
/// needing a full `MachineProfile` probe. For callers (like `hq-tools`'s
/// `web_search`) that just need to decide whether it's honest to name a
/// repo-relative script path in an error message.
pub fn agent_hq_checkout_path() -> Option<PathBuf> {
    find_agent_hq_checkout()
}

/// Strip a `http(s)://` scheme and split `host:port` from a URL's authority.
/// ponytail: hand-rolled instead of pulling in the `url` crate for one field;
/// doesn't handle IPv6-bracket or userinfo forms, fine for a local config
/// value — upgrade to `url` if that ever matters.
pub(super) fn host_port_from_url(url: &str) -> Option<(String, u16)> {
    let (default_port, rest) = if let Some(r) = url.strip_prefix("https://") {
        (443, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (80, r)
    } else {
        (80, url)
    };
    let authority = rest.split('/').next()?;
    match authority.split_once(':') {
        Some((host, port)) => Some((host.to_string(), port.parse().ok()?)),
        // No explicit port — e.g. `https://searx.example.org` — is a
        // perfectly valid, commonly-used `searxng_url` value. Requiring one
        // meant every such config silently probed as unreachable.
        None => Some((authority.to_string(), default_port)),
    }
}

/// `host:port` reachability with the DNS resolve *and* the connect both
/// bounded — `ToSocketAddrs::to_socket_addrs` is a blocking resolver call
/// with no timeout of its own, so `TcpStream::connect_timeout` alone leaves
/// an unresolvable host free to block the prompt-build path for however
/// long the resolver takes. Runs on a side thread with a bounded wait
/// instead; a thread that outlives the deadline is simply not waited on
/// (it sends into a channel nobody reads and exits on its own).
pub(super) fn resolve_and_connect(host: String, port: u16, deadline: Duration) -> bool {
    use std::net::{TcpStream, ToSocketAddrs};

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let ok = (host.as_str(), port)
            .to_socket_addrs()
            .ok()
            .and_then(|mut addrs| addrs.next())
            .is_some_and(|addr| {
                TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
            });
        let _ = tx.send(ok);
    });
    rx.recv_timeout(deadline).unwrap_or(false)
}

const SEARXNG_CONNECT_DEADLINE: Duration = Duration::from_millis(500);

fn probe_web_search_from_config() -> (Option<String>, Vec<WebSearchBackendStatus>) {
    let cfg = crate::config::HqConfig::load().ok();
    let statuses = probe_web_search(
        cfg.as_ref().and_then(|c| c.searxng_url.as_deref()),
        cfg.as_ref().and_then(|c| c.brave_api_key.as_deref()),
    );
    (legacy_backend_summary(&statuses), statuses)
}

/// Passive per-backend health: a bounded TCP connect for SearxNG, key
/// presence only for Brave (a real query would spend paid quota on every
/// refresh). No test query is sent here; `hq doctor` sends one.
pub(super) fn probe_web_search(
    searxng_url: Option<&str>,
    brave_api_key: Option<&str>,
) -> Vec<WebSearchBackendStatus> {
    vec![probe_searxng(searxng_url), brave_key_status(brave_api_key)]
}

fn probe_searxng(searxng_url: Option<&str>) -> WebSearchBackendStatus {
    let Some(url) = searxng_url.map(str::trim).filter(|s| !s.is_empty()) else {
        return web_status("searxng", false, None, "searxng: not configured".into());
    };
    let reachable = host_port_from_url(url)
        .is_some_and(|(host, port)| resolve_and_connect(host, port, SEARXNG_CONNECT_DEADLINE));
    let detail = if reachable {
        format!("searxng ({url}) reachable, not test-queried")
    } else {
        format!("searxng ({url}) configured but unreachable")
    };
    web_status("searxng", true, Some(reachable), detail)
}

fn brave_key_status(brave_api_key: Option<&str>) -> WebSearchBackendStatus {
    let configured = brave_api_key.is_some_and(|k| !k.trim().is_empty());
    let detail = if configured {
        "brave API key configured, not verified"
    } else {
        "brave: not configured"
    };
    web_status("brave", configured, None, detail.into())
}

fn web_status(
    provider: &str,
    configured: bool,
    reachable: Option<bool>,
    detail: String,
) -> WebSearchBackendStatus {
    WebSearchBackendStatus {
        provider: provider.into(),
        configured,
        reachable,
        answered: None,
        detail,
    }
}

/// The single-string `web_search_backend` older readers of `machine.json` expect.
fn legacy_backend_summary(statuses: &[WebSearchBackendStatus]) -> Option<String> {
    let first = statuses.iter().find(|s| s.usable())?;
    Some(match first.provider.as_str() {
        "searxng" => first.detail.clone(),
        other => other.to_string(),
    })
}

#[cfg(test)]
mod checkout_tests {
    use super::find_agent_hq_checkout;

    #[test]
    fn finds_the_checkout_that_built_this_binary() {
        // Regression: the old cwd-based walk reported "no checkout" under a
        // daemon whose WorkingDirectory is the vault, not the repo — even on
        // the machine that built this exact binary. The compile-time
        // CARGO_MANIFEST_DIR path is baked into the binary, so this must
        // resolve independent of whatever the process's cwd happens to be
        // (this test deliberately does not touch cwd, since mutating it is
        // process-global and would race with every other test in this binary).
        assert!(
            find_agent_hq_checkout().is_some(),
            "must find the checkout this binary was built from"
        );
    }
}

#[cfg(test)]
mod which_tests {
    use super::which_binary;

    #[test]
    fn rejects_names_that_are_not_plain_binary_names() {
        assert!(which_binary("sh; echo pwned").is_none());
        assert!(which_binary("$(id)").is_none());
        assert!(which_binary("").is_none());
        assert!(which_binary("sh").is_some());
    }
}
