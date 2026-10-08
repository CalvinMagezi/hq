//! A process sandbox for one coding agent: what it may read, write, see and
//! reach, turned into the command line of the platform's sandbox program
//! (`sandbox-exec` on macOS, `bwrap` on Linux). The agent runs as the same user
//! as the host, so this is what keeps it away from the host's own secrets (its
//! operator token, the other agents' tokens, their environments) and from every
//! network but the one the policy names.
//!
//! The policy is plain data. Callers resolve it to real paths first (macOS
//! matches the resolved path, so `/tmp` must be `/private/tmp`); `canonical`
//! does that for a list. Nothing here runs the agent.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Where macOS ships sandbox-exec.
const SANDBOX_EXEC_PATH: &str = "/usr/bin/sandbox-exec";

/// A directory whose contents are unreadable except for the listed files. Its
/// entries stay visible to `stat` (Claude Code stats every parent of a file it
/// reads and refuses a path it cannot examine), but a listing and a read are
/// refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenDir {
    pub dir: PathBuf,
    /// Files (or sockets) inside it the agent may still use.
    pub allow: Vec<PathBuf>,
}

/// A tree the agent may read only where listed: everything under `root` is
/// unreadable except the `allow` paths. Hidden directories and masked files are
/// applied after it, so they win inside an allowed path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadRestriction {
    pub root: PathBuf,
    pub allow: Vec<PathBuf>,
}

/// What the agent may reach over the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Network {
    /// No restriction from the sandbox.
    Full,
    /// No outbound connections at all.
    Off,
    /// Outbound only to `localhost:<port>` (an egress proxy that applies a
    /// domain allowlist) and to the listed unix sockets. Seatbelt can allow one
    /// loopback port; bubblewrap has no network at all in its new namespace, so
    /// it needs a `relay` that listens on that port inside the sandbox and
    /// forwards to a unix socket the host serves.
    Proxy {
        port: u16,
        unix_sockets: Vec<PathBuf>,
        relay: Option<Relay>,
    },
}

/// The in-sandbox half of the bridge to the egress proxy: `program host
/// sandbox-init` starts a relay on the proxy port that forwards to
/// `unix_socket`, then runs the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relay {
    pub program: PathBuf,
    pub unix_socket: PathBuf,
}

/// Everything one sandboxed agent is allowed, as resolved paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSandbox {
    /// Working directory of the agent; always writable.
    pub project: PathBuf,
    /// Directories the agent may write under (besides `project`).
    pub writable: Vec<PathBuf>,
    /// Files the agent may write, with their temporary siblings
    /// (`<file>.*`), for programs that replace a file atomically.
    pub writable_files: Vec<PathBuf>,
    /// Files inside a writable root that must stay read-only.
    pub readonly_files: Vec<PathBuf>,
    /// Directories inside a writable root that must stay read-only.
    pub readonly_subpaths: Vec<PathBuf>,
    /// Programs the agent may not start (macOS: a way out of the sandbox).
    pub denied_programs: Vec<PathBuf>,
    /// Mach services the agent may not look up (macOS).
    pub denied_services: Vec<String>,
    /// Files the agent can neither read nor write.
    pub masked_files: Vec<PathBuf>,
    pub hidden_dirs: Vec<HiddenDir>,
    /// Directories given a private empty tmpfs (bubblewrap only; Seatbelt lists
    /// the shared temporary directories as writable instead).
    pub private_tmp: Vec<PathBuf>,
    /// Reads under each root limited to listed paths, outermost root first (a
    /// restriction inside an earlier one narrows it).
    pub read_restrictions: Vec<ReadRestriction>,
    /// Hide other processes: their environments and their details.
    pub hide_other_processes: bool,
    pub network: Network,
}

/// A sandbox program available on this host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    SandboxExec(PathBuf),
    Bwrap(PathBuf),
}

/// A command ready to run: the sandbox program and its arguments, ending with
/// the agent's own command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wrapped {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

/// Why a policy cannot be enforced by this backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported(pub String);

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unsupported {}

/// Wraps `argv` (the agent's program and arguments) for `backend`.
pub fn wrap(
    backend: &Backend,
    sandbox: &AgentSandbox,
    argv: &[String],
) -> Result<Wrapped, Unsupported> {
    match backend {
        Backend::SandboxExec(program) => {
            let mut args: Vec<OsString> = vec!["-p".into(), seatbelt_profile(sandbox).into()];
            args.extend(argv.iter().map(OsString::from));
            Ok(Wrapped {
                program: program.clone(),
                args,
            })
        }
        Backend::Bwrap(program) => Ok(Wrapped {
            program: program.clone(),
            args: bwrap_args(sandbox, argv)?,
        }),
    }
}

/// The sandbox-exec profile. Rules later in the profile win, so the broad
/// denials come first and the narrow allowances after them.
pub fn seatbelt_profile(s: &AgentSandbox) -> String {
    let mut p = String::from("(version 1)(allow default)(deny file-write*)");

    let subpaths: Vec<String> = std::iter::once(&s.project)
        .chain(&s.writable)
        .map(|d| format!("(subpath {})", sbpl_string(d)))
        .collect();
    let files: Vec<String> = s
        .writable_files
        .iter()
        .flat_map(|f| {
            [
                format!("(literal {})", sbpl_string(f)),
                format!("(regex #\"^{}\\..*\")", regex_escape(f)),
            ]
        })
        .collect();
    p.push_str(&format!(
        "(allow file-write* {} {})",
        subpaths.join(" "),
        files.join(" ")
    ));

    if !s.readonly_files.is_empty() {
        p.push_str(&format!("(deny file-write* {})", literals(&s.readonly_files)));
    }
    for dir in &s.readonly_subpaths {
        p.push_str(&format!("(deny file-write* (subpath {}))", sbpl_string(dir)));
    }
    for r in &s.read_restrictions {
        p.push_str(&format!("(deny file-read-data (subpath {}))", sbpl_string(&r.root)));
        let allowed: Vec<String> = r
            .allow
            .iter()
            .map(|a| format!("(subpath {})", sbpl_string(a)))
            .collect();
        if !allowed.is_empty() {
            p.push_str(&format!("(allow file-read-data {})", allowed.join(" ")));
        }
    }
    // Every denial first, then every allowance: a later deny of an enclosing
    // directory would otherwise cancel an earlier allow for a file inside it.
    for hidden in &s.hidden_dirs {
        p.push_str(&format!(
            "(deny file-read-data (subpath {}))",
            sbpl_string(&hidden.dir)
        ));
    }
    for hidden in s.hidden_dirs.iter().filter(|h| !h.allow.is_empty()) {
        p.push_str(&format!("(allow file-read-data {})", literals(&hidden.allow)));
    }
    for program in &s.denied_programs {
        p.push_str(&format!("(deny process-exec (literal {}))", sbpl_string(program)));
    }
    for service in &s.denied_services {
        p.push_str(&format!("(deny mach-lookup (global-name {}))", sbpl_string(Path::new(service))));
    }
    if !s.masked_files.is_empty() {
        p.push_str(&format!(
            "(deny file-read* file-write* {})",
            literals(&s.masked_files)
        ));
    }
    if s.hide_other_processes {
        p.push_str("(deny process-info* (target others))");
    }
    match &s.network {
        Network::Full => {}
        Network::Off => p.push_str("(deny network-outbound (remote ip))"),
        Network::Proxy { port, unix_sockets, .. } => {
            let mut allowed = vec![format!("(remote ip \"localhost:{port}\")")];
            allowed.extend(
                unix_sockets
                    .iter()
                    .map(|sock| format!("(literal {})", sbpl_string(sock))),
            );
            p.push_str(&format!(
                "(deny network-outbound)(allow network-outbound {})",
                allowed.join(" ")
            ));
        }
    }
    p
}

/// The bubblewrap argv: read-only root, fresh `/dev` and `/proc` in a new pid
/// namespace (so other processes' environments are out of reach), writable
/// roots bound back in, hidden directories replaced by an empty tmpfs with only
/// the allowed files bound back.
pub fn bwrap_args(s: &AgentSandbox, argv: &[String]) -> Result<Vec<OsString>, Unsupported> {
    let relay = match &s.network {
        Network::Proxy { relay: None, .. } => {
            return Err(Unsupported(
                "a domain-allowlisted network needs a relay inside the bubblewrap sandbox, and \
                 none was given"
                    .into(),
            ));
        }
        Network::Proxy { port, relay: Some(relay), .. } => Some((*port, relay)),
        _ => None,
    };
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
    for dir in &s.private_tmp {
        args.extend(["--tmpfs".into(), dir.into()]);
    }
    for r in &s.read_restrictions {
        args.extend(["--tmpfs".into(), (&r.root).into()]);
        for path in &r.allow {
            args.extend(["--ro-bind".into(), path.into(), path.into()]);
        }
    }
    for dir in std::iter::once(&s.project).chain(&s.writable) {
        args.extend(["--bind".into(), dir.into(), dir.into()]);
    }
    for path in s.readonly_files.iter().chain(&s.readonly_subpaths) {
        args.extend(["--ro-bind".into(), path.into(), path.into()]);
    }
    if s.network == Network::Off || relay.is_some() {
        args.push("--unshare-net".into());
    }
    // Outermost first: a later mount shadows an earlier one, so an inner hidden
    // directory has to be mounted after the one that contains it.
    let mut hidden: Vec<&HiddenDir> = s.hidden_dirs.iter().collect();
    hidden.sort_by_key(|h| h.dir.components().count());
    for hidden in hidden {
        // Private, like the real directory: programs inside check that before trusting it.
        args.extend(["--perms".into(), "0700".into(), "--tmpfs".into(), (&hidden.dir).into()]);
        for file in &hidden.allow {
            args.extend(["--bind".into(), file.into(), file.into()]);
        }
    }
    for file in &s.masked_files {
        args.extend(["--ro-bind".into(), "/dev/null".into(), file.into()]);
    }
    args.extend(["--chdir".into(), (&s.project).into(), "--".into()]);
    if let Some((port, relay)) = relay {
        args.extend([
            relay.program.clone().into_os_string(),
            "host".into(),
            "sandbox-init".into(),
            "--port".into(),
            port.to_string().into(),
            "--unix".into(),
            relay.unix_socket.clone().into_os_string(),
            "--".into(),
        ]);
    }
    args.extend(argv.iter().map(OsString::from));
    Ok(args)
}

/// The sandbox program on this host, probed once per process: a present binary
/// is not enough (user namespaces may be off), so it is run on a trivial command.
pub fn backend() -> Option<&'static Backend> {
    static FOUND: OnceLock<Option<Backend>> = OnceLock::new();
    FOUND.get_or_init(probe).as_ref()
}

fn probe() -> Option<Backend> {
    if cfg!(target_os = "macos") {
        let program = PathBuf::from(SANDBOX_EXEC_PATH);
        let ok = run(&program, &["-p", "(version 1)(allow default)", "/usr/bin/true"]);
        return ok.then_some(Backend::SandboxExec(program));
    }
    let program = which::which("bwrap").ok()?;
    let ok = run(
        &program,
        &["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--unshare-pid", "true"],
    );
    ok.then_some(Backend::Bwrap(program))
}

fn run(program: &Path, args: &[&str]) -> bool {
    std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Resolves paths to their real location (macOS matches on that), dropping any
/// that do not exist, sorted with parents before children and without repeats.
pub fn canonical(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = paths
        .into_iter()
        .filter_map(|p| p.canonicalize().ok())
        .collect();
    out.sort();
    out.dedup();
    out.sort_by_key(|p| p.components().count());
    out
}

fn literals(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| format!("(literal {})", sbpl_string(p)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn sbpl_string(path: &Path) -> String {
    let raw = path.to_string_lossy();
    format!("\"{}\"", raw.replace('\\', "\\\\").replace('"', "\\\""))
}

/// `path` as the body of a Seatbelt regex literal.
fn regex_escape(path: &Path) -> String {
    let mut out = String::new();
    for c in path.to_string_lossy().chars() {
        if "\\.^$|?*+()[]{}\"#".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests;
