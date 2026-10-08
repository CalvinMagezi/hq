//! OS sandbox for the `bash` tool: bubblewrap on Linux, sandbox-exec on
//! macOS. The policy checks in `bash_policy` and governance stay as defense
//! in depth; this is the layer that holds when a command slips past them.
//! Behaviour and failure modes: docs/security/BASH_SANDBOX.md.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use hq_core::config::{BashConfig, BashSandboxMode};
use tracing::warn;

/// Files under `~/.ssh` that clients need and that hold no private key.
const SSH_PUBLIC_FILES: &[&str] = &["known_hosts", "config", "authorized_keys"];

/// HQ key files and the default config location, masked even when
/// `HQ_CONFIG_PATH` points the daemon somewhere else.
const HQ_KEY_FILES: &[&str] = &[
    ".hq/config.yaml",
    ".hq/wallet.enc",
    ".hq/seed.enc",
    ".hq/secret.key",
];

const DOCKER_SOCKETS: &[&str] = &["/var/run/docker.sock", "/run/docker.sock"];

/// Relative to `XDG_RUNTIME_DIR`.
const RUNTIME_SOCKETS: &[&str] = &["systemd/private", "bus", "docker.sock"];

/// Always-writable scratch roots, besides HOME, the cwd and TMPDIR.
const SCRATCH_ROOTS: &[&str] = &["/tmp", "/var/tmp"];

/// macOS per-user temp and cache roots that toolchains write to.
const MACOS_SCRATCH_ROOTS: &[&str] = &["/private/tmp", "/private/var/folders", "/dev"];

/// Where macOS ships sandbox-exec.
const SANDBOX_EXEC_PATH: &str = "/usr/bin/sandbox-exec";

/// Resolved `governance.bash` settings plus the session's writable roots.
#[derive(Debug, Clone)]
pub struct BashSettings {
    pub env_passthrough: Vec<String>,
    pub sandbox: BashSandboxMode,
    pub network: bool,
    /// Extra writable roots (vault, working dir) on top of HOME, cwd and tmp.
    pub writable_paths: Vec<PathBuf>,
    /// Leave only scratch space writable: no HOME, cwd or extra roots. Used by
    /// sessions that may look at the machine but never change it.
    pub read_only: bool,
}

impl Default for BashSettings {
    fn default() -> Self {
        Self::from_config(&BashConfig::default(), Vec::new())
    }
}

impl BashSettings {
    pub fn from_config(config: &BashConfig, writable_paths: Vec<PathBuf>) -> Self {
        Self {
            env_passthrough: config.env_passthrough.clone(),
            sandbox: config.sandbox,
            network: config.network,
            writable_paths,
            read_only: false,
        }
    }

    /// Investigation-only shell: always sandboxed (a missing backend refuses
    /// the command), no network, nothing durable writable.
    pub fn read_only(config: &BashConfig) -> Self {
        Self {
            sandbox: BashSandboxMode::Required,
            network: false,
            read_only: true,
            ..Self::from_config(config, Vec::new())
        }
    }
}

/// Everything a sandbox backend needs, gathered once per command.
#[derive(Debug, Clone)]
pub struct SandboxContext {
    /// Investigation-only: also closes local IPC and the programs that start other apps.
    pub read_only: bool,
    pub cwd: PathBuf,
    pub writable: Vec<PathBuf>,
    pub masked_files: Vec<PathBuf>,
    /// Files inside a writable root that must stay read-only (persistence
    /// targets such as `~/.ssh/authorized_keys`).
    pub readonly_files: Vec<PathBuf>,
    pub network: bool,
}

impl SandboxContext {
    /// Context for this process: HOME, cwd and tmp are writable, credential
    /// files are masked.
    pub fn for_process(settings: &BashSettings) -> Self {
        let home = dirs::home_dir();
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let mut writable: Vec<PathBuf> = Vec::new();
        if !settings.read_only {
            writable.extend(home.iter().cloned());
            writable.push(cwd.clone());
            writable.extend(settings.writable_paths.iter().cloned());
        }
        writable.push(std::env::temp_dir());
        writable.extend(SCRATCH_ROOTS.iter().map(PathBuf::from));
        Self {
            read_only: settings.read_only,
            cwd,
            writable: existing_canonical(writable),
            masked_files: existing_canonical(masked_files(home.as_deref())),
            readonly_files: existing_canonical(readonly_files(home.as_deref())),
            network: settings.network,
        }
    }
}

/// Files a sandboxed command must never read. Deliberately short: masking
/// something a CLI depends on (gh's hosts.yml, gws config, here.now
/// credentials) would break that CLI, and governance already denies those
/// paths to the agent's own tools.
fn masked_files(home: Option<&Path>) -> Vec<PathBuf> {
    let mut files = hq_config_files();
    files.extend(control_sockets());
    if let Some(home) = home {
        files.extend(HQ_KEY_FILES.iter().map(|rel| home.join(rel)));
        files.extend(ssh_private_keys(&home.join(".ssh")));
    }
    files
}

/// Files that stay readable but must not be written, since a write would give
/// the agent a foothold that outlives the sandbox.
fn readonly_files(home: Option<&Path>) -> Vec<PathBuf> {
    let Some(home) = home else {
        return Vec::new();
    };
    let ssh = home.join(".ssh");
    let Ok(entries) = std::fs::read_dir(&ssh) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_authorized_keys(p))
        .collect()
}

fn is_authorized_keys(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with("authorized_keys"))
}

/// The HQ config, its backups, and `*.env` files beside it.
fn hq_config_files() -> Vec<PathBuf> {
    let config = hq_core::config::HqConfig::config_read_path();
    let mut files = vec![config.clone(), hq_core::approval_key::key_path()];
    let (Some(dir), Some(name)) = (config.parent(), config.file_name()) else {
        return files;
    };
    let name = name.to_string_lossy().into_owned();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let entry_name = entry.file_name().to_string_lossy().into_owned();
        let is_backup = entry_name.starts_with(&name);
        if (is_backup || entry_name.ends_with(".env")) && entry.path().is_file() {
            files.push(entry.path());
        }
    }
    files
}

/// Sockets that hand out control of the host: the container runtime and the
/// systemd user manager. A masked path reads as an empty file, so connecting fails.
fn control_sockets() -> Vec<PathBuf> {
    let mut sockets: Vec<PathBuf> = DOCKER_SOCKETS.iter().map(PathBuf::from).collect();
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        let runtime = PathBuf::from(runtime);
        sockets.extend(RUNTIME_SOCKETS.iter().map(|rel| runtime.join(rel)));
    }
    sockets
}

fn ssh_private_keys(ssh_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(ssh_dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let public =
                name.ends_with(".pub") || SSH_PUBLIC_FILES.iter().any(|f| name.starts_with(f));
            !public
        })
        .collect()
}

fn existing_canonical(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = paths
        .into_iter()
        .filter_map(|p| p.canonicalize().ok())
        .collect();
    out.sort();
    out.dedup();
    // Parents first, so a bind of a nested root is not shadowed by its parent.
    out.sort_by_key(|p| p.components().count());
    out
}

/// A sandbox program available on this host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    Bwrap(PathBuf),
    SandboxExec(PathBuf),
}

/// How the bash tool should start one command.
#[derive(Debug)]
pub enum Launch {
    Direct,
    Wrapped {
        program: PathBuf,
        args: Vec<OsString>,
    },
    Refused(String),
}

/// Decide how to launch `command` under `settings`.
pub fn plan_launch(settings: &BashSettings, command: &str) -> Launch {
    plan_launch_with(settings, command, available_backend_for(settings.network))
}

/// `plan_launch` with the probed backend injected.
pub fn plan_launch_with(
    settings: &BashSettings,
    command: &str,
    backend: Option<&Backend>,
) -> Launch {
    if settings.sandbox == BashSandboxMode::Off {
        return Launch::Direct;
    }
    match backend {
        Some(backend) => wrap(backend, &SandboxContext::for_process(settings), command),
        None if settings.sandbox == BashSandboxMode::Required => {
            Launch::Refused(REFUSAL_NO_SANDBOX.to_string())
        }
        None => Launch::Direct,
    }
}

const REFUSAL_NO_SANDBOX: &str = "Command refused: no working sandbox (bubblewrap on Linux, \
    sandbox-exec on macOS) is available on this host, and governance.bash.sandbox is `required` \
    (the default). Install bubblewrap, or ask the owner to set governance.bash.sandbox to \
    `best_effort` or `off` to accept running shell commands without isolation.";

/// What the sandbox settings mean on this host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxStatus {
    /// Every command runs wrapped.
    Active(String),
    /// Sandbox disabled by the operator.
    Disabled,
    /// No backend and `required`: bash is refused.
    Refusing,
    /// No backend and `best_effort`: bash runs unwrapped.
    Unwrapped,
}

pub fn sandbox_status(settings: &BashSettings, backend: Option<&Backend>) -> SandboxStatus {
    match (settings.sandbox, backend) {
        (BashSandboxMode::Off, _) => SandboxStatus::Disabled,
        (_, Some(Backend::Bwrap(_))) => SandboxStatus::Active("bubblewrap".into()),
        (_, Some(Backend::SandboxExec(_))) => SandboxStatus::Active("sandbox-exec".into()),
        (BashSandboxMode::Required, None) => SandboxStatus::Refusing,
        (BashSandboxMode::BestEffort, None) => SandboxStatus::Unwrapped,
    }
}

impl SandboxStatus {
    /// Doctor label (`ok`, `warn`, `FAIL`) and one explanatory line.
    pub fn describe(&self) -> (&'static str, String) {
        match self {
            Self::Active(name) => ("ok", format!("bash commands run inside {name}")),
            Self::Disabled => (
                "warn",
                "governance.bash.sandbox is `off`: bash commands run with no isolation".into(),
            ),
            Self::Refusing => (
                "FAIL",
                "no sandbox backend on this host, so the bash tool refuses every command. \
                 Install bubblewrap (Linux), or set governance.bash.sandbox to `best_effort` \
                 or `off` to run unwrapped"
                    .into(),
            ),
            Self::Unwrapped => (
                "warn",
                "no sandbox backend on this host and governance.bash.sandbox is `best_effort`: \
                 bash commands run with the env allowlist and policy checks only"
                    .into(),
            ),
        }
    }
}

/// Probe-backed status for the current host, for `hq doctor`.
pub fn host_sandbox_status(settings: &BashSettings) -> SandboxStatus {
    sandbox_status(settings, available_backend_for(settings.network))
}

/// When bash is being refused, log an error and file one owner notification
/// for this boot (`boot_id` keeps it once per start, not once ever). Returns
/// whether the sandbox is refusing.
pub fn report_refusal(
    settings: &BashSettings,
    backend: Option<&Backend>,
    db: &hq_db::Database,
    boot_id: &str,
) -> anyhow::Result<bool> {
    if sandbox_status(settings, backend) != SandboxStatus::Refusing {
        return Ok(false);
    }
    let body = format!(
        "{} To keep running unwrapped instead, set governance.bash.sandbox to `best_effort` in the HQ config.",
        SandboxStatus::Refusing.describe().1
    );
    tracing::error!("bash sandbox: {body}");
    let item = hq_core::types::ValueItem::new(
        "bash-sandbox",
        hq_core::types::ValueKind::ActionNeeded,
        "Agent shell commands are disabled: no sandbox on this host",
        body,
    )
    .with_dedup_key(format!("bash-sandbox-refusing-{boot_id}"));
    hq_db::value_items::emit(db, &item)?;
    Ok(true)
}

/// One loud warning per process when the sandbox is not protecting bash.
pub fn warn_if_unprotected_once(settings: &BashSettings) {
    static WARNED: OnceLock<()> = OnceLock::new();
    let status = host_sandbox_status(settings);
    if matches!(status, SandboxStatus::Active(_)) {
        return;
    }
    WARNED.get_or_init(|| warn!("bash sandbox: {}", status.describe().1));
}

pub fn wrap(backend: &Backend, ctx: &SandboxContext, command: &str) -> Launch {
    match backend {
        Backend::Bwrap(program) => Launch::Wrapped {
            program: program.clone(),
            args: bwrap_args(ctx, command),
        },
        Backend::SandboxExec(program) => Launch::Wrapped {
            program: program.clone(),
            args: vec![
                "-p".into(),
                seatbelt_profile(ctx).into(),
                "bash".into(),
                "-c".into(),
                command.into(),
            ],
        },
    }
}

/// Mark this process non-dumpable on Linux, once. Same-uid processes (every
/// bash child) then lose `/proc/<pid>/environ` and `ps e` on the daemon,
/// which still holds the provider keys the env allowlist keeps from them.
/// Children regain dumpability on exec, so their own tools are unaffected.
/// A no-op elsewhere; on macOS `ps eww` of the daemon stays readable.
pub fn hide_process_environment() {
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(set_non_dumpable);
}

#[cfg(target_os = "linux")]
fn set_non_dumpable() {
    // SAFETY: PR_SET_DUMPABLE takes plain integer arguments and touches no
    // memory owned by Rust.
    let rc = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
    if rc != 0 {
        warn!("could not mark the process non-dumpable; bash children can read its environment");
    }
}

#[cfg(not(target_os = "linux"))]
fn set_non_dumpable() {}

/// The working sandbox backend on this host, probed once per process.
pub fn available_backend() -> Option<&'static Backend> {
    available_backend_for(true)
}

/// Like [`available_backend`], probing the same flags the real argv will use,
/// so a container that cannot `--unshare-net` counts as unavailable when the
/// network is meant to be off.
pub fn available_backend_for(network: bool) -> Option<&'static Backend> {
    static WITH_NET: OnceLock<Option<Backend>> = OnceLock::new();
    static NO_NET: OnceLock<Option<Backend>> = OnceLock::new();
    let cell = if network { &WITH_NET } else { &NO_NET };
    cell.get_or_init(|| probe_backend(network)).as_ref()
}

fn bwrap_probe_args(network: bool) -> Vec<&'static str> {
    let mut args = vec![
        "--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--unshare-pid",
    ];
    if !network {
        args.push("--unshare-net");
    }
    args.push("true");
    args
}

fn probe_backend(network: bool) -> Option<Backend> {
    if cfg!(target_os = "macos") {
        let program = PathBuf::from(SANDBOX_EXEC_PATH);
        let ok = run_probe(
            &program,
            &["-p", "(version 1)(allow default)", "/usr/bin/true"],
        );
        return ok.then_some(Backend::SandboxExec(program));
    }
    let program = which::which("bwrap").ok()?;
    // A present binary is not enough: user namespaces may be disabled.
    let probe = bwrap_probe_args(network);
    run_probe(&program, &probe).then_some(Backend::Bwrap(program))
}

fn run_probe(program: &Path, args: &[&str]) -> bool {
    std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// bubblewrap argv: read-only root, fresh /dev and /proc in a new pid
/// namespace (so the daemon's `/proc/<pid>/environ` is out of reach),
/// writable roots bound back in, credential files masked with /dev/null.
pub fn bwrap_args(ctx: &SandboxContext, command: &str) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "--ro-bind",
        "/",
        "/",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--unshare-pid",
        "--die-with-parent",
        "--new-session",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    for dir in &ctx.writable {
        args.extend(["--bind".into(), dir.into(), dir.into()]);
    }
    for file in &ctx.readonly_files {
        args.extend(["--ro-bind".into(), file.into(), file.into()]);
    }
    if !ctx.network {
        args.push("--unshare-net".into());
    }
    for file in &ctx.masked_files {
        args.extend(["--ro-bind".into(), "/dev/null".into(), file.into()]);
    }
    args.extend(["--chdir".into(), ctx.cwd.clone().into()]);
    args.extend(["bash".into(), "-c".into(), command.into()]);
    args
}

/// sandbox-exec profile: writes only under the writable roots, masked files
/// unreadable, and outbound IP denied when the network is off.
pub fn seatbelt_profile(ctx: &SandboxContext) -> String {
    let writable: Vec<String> = ctx
        .writable
        .iter()
        .map(|p| format!("(subpath {})", sbpl_string(p)))
        .chain(
            MACOS_SCRATCH_ROOTS
                .iter()
                .map(|p| format!("(subpath \"{p}\")")),
        )
        .collect();
    let mut profile = format!(
        "(version 1)(allow default)(deny file-write*)(allow file-write* {})",
        writable.join(" ")
    );
    if !ctx.readonly_files.is_empty() {
        let files: Vec<String> = ctx
            .readonly_files
            .iter()
            .map(|p| format!("(literal {})", sbpl_string(p)))
            .collect();
        profile.push_str(&format!("(deny file-write* {})", files.join(" ")));
    }
    if !ctx.masked_files.is_empty() {
        let masked: Vec<String> = ctx
            .masked_files
            .iter()
            .map(|p| format!("(literal {})", sbpl_string(p)))
            .collect();
        profile.push_str(&format!(
            "(deny file-read* file-write* {})",
            masked.join(" ")
        ));
    }
    if !ctx.network {
        profile.push_str("(deny network-outbound (remote ip))");
    }
    if ctx.read_only {
        // Unix sockets (launchd, ssh-agent, docker) stay open under the rule above.
        profile.push_str("(deny network*)");
        let programs: Vec<String> = READ_ONLY_DENIED_PROGRAMS
            .iter()
            .map(|p| format!("(literal \"{p}\")"))
            .collect();
        profile.push_str(&format!("(deny process-exec {})", programs.join(" ")));
        // A copied launcher in scratch space would dodge the list above, so nothing writable may be run.
        let scratch: Vec<String> = ctx
            .writable
            .iter()
            .map(|p| format!("(subpath {})", sbpl_string(p)))
            .chain(MACOS_SCRATCH_ROOTS.iter().map(|p| format!("(subpath \"{p}\")")))
            .collect();
        profile.push_str(&format!("(deny process-exec {})", scratch.join(" ")));
    }
    profile
}

/// Programs that start other apps or change another process's settings, which a read-only shell has no use for.
const READ_ONLY_DENIED_PROGRAMS: &[&str] = &[
    "/bin/launchctl",
    "/usr/bin/launchctl",
    "/usr/bin/open",
    "/usr/bin/osascript",
    "/usr/bin/defaults",
    "/usr/bin/security",
    "/usr/bin/screencapture",
    "/usr/bin/say",
    "/usr/bin/shortcuts",
    "/usr/bin/automator",
];

fn sbpl_string(path: &Path) -> String {
    let raw = path.to_string_lossy();
    format!("\"{}\"", raw.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests;
