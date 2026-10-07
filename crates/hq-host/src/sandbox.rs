//! Turns a sandbox request into the wrapped command line of one agent: the
//! egress listener it may connect to, the paths it may touch, and the proxy
//! variables that make its HTTP clients use the listener.

use crate::egress::{Egress, Rule};
use crate::error::HostError;
use crate::server;
use hq_sandbox::{AgentSandbox, Backend, HiddenDir, Network, ReadRestriction, Relay, backend, canonical, wrap};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Directories under HOME that hold credentials and have no place in an agent.
const SECRET_HOME_DIRS: &[&str] = &[
    ".ssh", ".gnupg", ".aws", ".kube", ".hq", ".docker", ".config/gh",
];
/// What an agent may read under HOME. Everything else there is unreadable, so a
/// token or note the operator keeps in some other directory cannot be read and
/// sent to an allowed host. Tool installs, shell and git configuration, and the
/// agent's own state are listed; `herdr.sandbox.readable` adds more.
const HOME_READABLE: &[&str] = &[
    ".local", ".cache", ".config", ".cargo", ".rustup", ".nvm", ".npm", ".bun",
    ".volta", ".pyenv", ".asdf", ".deno", ".gitconfig", ".gitignore_global", ".terminfo", ".zshenv",
    ".zprofile", ".zshrc", ".zlogin", ".bashrc", ".bash_profile", ".profile", ".inputrc", ".oh-my-zsh",
    "Library/Caches", "Library/Preferences", "Library/Keychains",
];
/// Where other people's files live, which an agent has no reason to read: other
/// users' home directories and external volumes. What the agent is allowed
/// under HOME or in its project stays readable.
#[cfg(target_os = "macos")]
const OUTER_READ_ROOTS: &[&str] = &["/Users", "/Volumes"];
/// Shared files other users deliberately leave for everyone.
#[cfg(target_os = "macos")]
const OUTER_READ_SHARED: &[&str] = &["/Users/Shared"];

/// Other Claude profiles (`~/.claude-<name>`) hold their own logins.
const OTHER_PROFILE_PREFIX: &str = ".claude-";
/// Credential files directly under HOME.
const SECRET_HOME_FILES: &[&str] = &[".netrc", ".npmrc", ".git-credentials"];
/// Files and directories the operator's own tools run code from later, outside
/// the sandbox. Writable roots must not let an agent plant anything in them.
/// Inside the Claude config directory (`~/.claude`, or the profile's `CLAUDE_CONFIG_DIR`).
const CLAUDE_DIR_READONLY_FILES: &[&str] = &["settings.json", "settings.local.json", ".claude.json"];
const CLAUDE_DIR_READONLY_DIRS: &[&str] = &["hooks"];
const CLAUDE_CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";
const DEFAULT_CLAUDE_DIR: &str = ".claude";
const PROJECT_READONLY: &[&str] = &[".git/hooks", ".git/config", ".mcp.json", ".claude/settings.json", ".claude/settings.local.json", ".claude/hooks"];
/// Ways out of Seatbelt: starting an app outside it, or scheduling one.
const DENIED_PROGRAMS: &[&str] = &["/usr/bin/open", "/usr/bin/osascript", "/bin/launchctl", "/usr/bin/launchctl"];
const DENIED_SERVICES: &[&str] = &["com.apple.coreservices.launchservicesd", "com.apple.dnssd.service"];
/// What Claude Code needs to write under HOME, besides the project.
/// `~/.claude.json` is deliberately not writable: it holds MCP server commands the
/// operator's own Claude runs later. The host records trust for the project itself.
const HOME_WRITABLE: &[&str] = &[".cache", "Library/Caches"];
/// Temporary directories every agent shares on macOS (Seatbelt lists them writable).
const TMP_ROOTS: &[&str] = &["/tmp", "/var/tmp", "/var/folders", "/dev"];
/// What bubblewrap gives each agent a private empty copy of instead: the shared
/// temporary directories, and /run, where the system's own sockets live.
const PRIVATE_TMP_ROOTS: &[&str] = &["/tmp", "/var/tmp", "/run"];
const PROXY_ENV_NAMES: &[&str] = &["HTTPS_PROXY", "HTTP_PROXY", "https_proxy", "http_proxy"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Run under the platform sandbox with a proxy-only network.
    Process,
    /// No sandbox. Stated explicitly so it is visible in `agent.list`.
    None,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Process => "process",
            Mode::None => "none",
        }
    }
}

/// One host an agent may reach.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Allow {
    pub host: String,
    #[serde(default)]
    pub ports: Vec<u16>,
    /// The name may resolve to a private address (an operator-named endpoint).
    #[serde(default)]
    pub private: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxSpec {
    pub mode: Mode,
    #[serde(default)]
    pub allow: Vec<Allow>,
    /// Extra directories the agent may write under.
    #[serde(default)]
    pub writable: Vec<PathBuf>,
    /// Extra paths under HOME the agent may read.
    #[serde(default)]
    pub readable: Vec<PathBuf>,
}

impl SandboxSpec {
    pub fn none() -> Self {
        Self {
            mode: Mode::None,
            allow: Vec::new(),
            writable: Vec::new(),
            readable: Vec::new(),
        }
    }
}

/// The unix socket a bubblewrap sandbox reaches the egress proxy through, and the
/// binary that relays it onto loopback inside.
pub(crate) struct Bridge {
    pub program: PathBuf,
    pub socket: PathBuf,
}

/// The command to run and the variables to add for a sandboxed agent, plus the
/// egress port to close when it exits.
pub(crate) struct Confined {
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub egress_port: u16,
}

fn rules(allow: &[Allow]) -> Vec<Rule> {
    allow
        .iter()
        .map(|a| {
            let mut rule = Rule::new(&a.host);
            rule.ports = a.ports.clone();
            rule.allow_private = a.private;
            rule
        })
        .collect()
}

/// Where one Claude Code account keeps its state: the default `~/.claude` (with
/// `~/.claude.json` beside it), or the directory a launch profile names with
/// `CLAUDE_CONFIG_DIR` (with `.claude.json` inside it).
struct ClaudeConfig {
    dir: PathBuf,
    /// The directory holding `.claude.json`.
    json_dir: PathBuf,
    is_default: bool,
}

fn claude_config(env: &[(String, String)]) -> Option<ClaudeConfig> {
    let custom = env
        .iter()
        .find(|(k, _)| k == CLAUDE_CONFIG_DIR_ENV)
        .map(|(_, v)| PathBuf::from(v))
        .filter(|p| p.is_absolute() && p.is_dir());
    if let Some(dir) = custom {
        let dir = dir.canonicalize().unwrap_or(dir);
        return Some(ClaudeConfig { json_dir: dir.clone(), dir, is_default: false });
    }
    let home = home()?;
    let home = home.canonicalize().unwrap_or(home);
    Some(ClaudeConfig {
        dir: home.join(DEFAULT_CLAUDE_DIR),
        json_dir: home,
        is_default: true,
    })
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.is_absolute())
}

fn policy(
    spec: &SandboxSpec,
    name: &str,
    cwd: &Path,
    run_dir: &Path,
    port: u16,
    claude: &ClaudeConfig,
    bridge: Option<&Bridge>,
) -> Result<AgentSandbox, HostError> {
    let claude_dir: &Path = &claude.dir;
    let default_claude = claude.is_default;
    let home = home().ok_or_else(|| HostError::Io("HOME is not set".into()))?;
    let home = home.canonicalize().unwrap_or(home);
    let under = |names: &[&str]| names.iter().map(|n| home.join(n)).collect::<Vec<_>>();
    let bwrap = bridge.is_some();
    let private_tmp = if bwrap {
        canonical(PRIVATE_TMP_ROOTS.iter().map(PathBuf::from))
    } else {
        Vec::new()
    };
    let shared_tmp: Vec<PathBuf> = if bwrap {
        Vec::new()
    } else {
        TMP_ROOTS.iter().map(PathBuf::from).collect()
    };
    let tmp = std::env::temp_dir();
    let own_tmp = canonical([tmp]).into_iter().filter(|t| !private_tmp.iter().any(|p| t.starts_with(p)));
    let writable = canonical(
        under(HOME_WRITABLE)
            .into_iter()
            .chain([claude_dir.to_path_buf()])
            .chain(shared_tmp)
            .chain(own_tmp)
            .chain(spec.writable.iter().cloned()),
    );
    let run_dir = run_dir.canonicalize().map_err(|e| HostError::Io(format!("the host directory: {e}")))?;
    let project = cwd.canonicalize().map_err(|e| HostError::Io(format!("the working directory: {e}")))?;
    let other_profiles = std::fs::read_dir(&home)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(OTHER_PROFILE_PREFIX))
        .map(|e| e.path())
        .filter(|p| p.canonicalize().ok().as_deref() != Some(claude_dir));
    // With a profile's own config directory, the default ~/.claude is another account.
    let default_dir = (!default_claude).then(|| home.join(DEFAULT_CLAUDE_DIR));
    let secret_dirs = canonical(
        under(SECRET_HOME_DIRS)
            .into_iter()
            .chain(other_profiles)
            .chain(default_dir),
    );
    // A writable root that contains HOME, the run directory or a secret directory
    // would let the agent rename or rewrite them, which no read rule survives.
    let guarded = [home.clone(), run_dir.clone()].into_iter().chain(secret_dirs.iter().cloned());
    for root in std::iter::once(&project).chain(&writable) {
        if let Some(inside) = guarded.clone().find(|g| g.starts_with(root)) {
            return Err(HostError::Sandbox(format!(
                "{} is writable but contains {}; start the agent in a project directory",
                root.display(),
                inside.display()
            )));
        }
    }
    if let Some(inside) = private_tmp.iter().find(|p| run_dir.starts_with(p)) {
        return Err(HostError::Sandbox(format!(
            "the host directory {} is under {}, which every agent gets a private copy of; use a directory under your home",
            run_dir.display(),
            inside.display()
        )));
    }
    let own_files = [
        run_dir.join("hooks").join(format!("{name}.json")),
        run_dir.join("mcp").join(format!("{name}.json")),
    ];
    let socket = server::agent_socket_path(&run_dir);
    let mut allow = canonical(own_files);
    allow.push(socket.clone());
    allow.extend(bridge.map(|b| b.socket.clone()));
    // A bubblewrap bind needs the file to exist.
    allow.retain(|p| !bwrap || p.exists());
    let mut hidden = vec![HiddenDir { dir: run_dir, allow }];
    hidden.extend(secret_dirs.into_iter().map(|dir| HiddenDir { dir, allow: Vec::new() }));
    let under_project = |names: &[&str]| names.iter().map(|n| project.join(n)).collect::<Vec<_>>();
    let home_files: Vec<PathBuf> = CLAUDE_DIR_READONLY_FILES.iter().map(|f| claude_dir.join(f)).collect();
    let home_dirs: Vec<PathBuf> = CLAUDE_DIR_READONLY_DIRS.iter().map(|d| claude_dir.join(d)).collect();
    let (project_files, project_dirs): (Vec<_>, Vec<_>) = under_project(PROJECT_READONLY)
        .into_iter()
        .partition(|p| p.extension().is_some() || p.ends_with("config"));
    // Seatbelt denies writes to a path whether or not it exists; a bubblewrap
    // mount needs something to mount over, so only existing paths are protected.
    let keep = |paths: Vec<PathBuf>| -> Vec<PathBuf> {
        paths.into_iter().filter(|p| !bwrap || p.exists()).collect()
    };
    let macos = cfg!(target_os = "macos");
    let read_allow = canonical(
        under(HOME_READABLE)
            .into_iter()
            .chain([claude_dir.to_path_buf()])
            .chain(default_claude.then(|| home.join(".claude.json")))
            .chain(spec.readable.iter().cloned())
            .chain(std::env::current_exe())
            .chain(std::iter::once(project.clone()))
            .chain(writable.iter().cloned()),
    );
    Ok(AgentSandbox {
        project,
        writable,
        writable_files: Vec::new(),
        readonly_files: keep(home_files.into_iter().chain(project_files).collect()),
        readonly_subpaths: keep(home_dirs.into_iter().chain(project_dirs).collect()),
        denied_programs: if macos { DENIED_PROGRAMS.iter().map(PathBuf::from).collect() } else { Vec::new() },
        denied_services: if macos { DENIED_SERVICES.iter().map(|s| s.to_string()).collect() } else { Vec::new() },
        masked_files: keep(under(SECRET_HOME_FILES)),
        hidden_dirs: hidden,
        private_tmp,
        read_restrictions: outer_restrictions(&read_allow)
            .into_iter()
            .chain([ReadRestriction { root: home.clone(), allow: read_allow }])
            .collect(),
        hide_other_processes: true,
        network: Network::Proxy {
            port,
            unix_sockets: vec![socket],
            relay: bridge.map(|b| Relay { program: b.program.clone(), unix_socket: b.socket.clone() }),
        },
    })
}

#[cfg(target_os = "macos")]
fn outer_restrictions(allow: &[PathBuf]) -> Vec<ReadRestriction> {
    OUTER_READ_ROOTS
        .iter()
        .filter_map(|root| Path::new(root).canonicalize().ok())
        .map(|root| ReadRestriction {
            allow: allow
                .iter()
                .cloned()
                .chain(OUTER_READ_SHARED.iter().map(PathBuf::from))
                .filter(|a| a.starts_with(&root))
                .collect(),
            root,
        })
        .collect()
}

/// The bubblewrap backend cannot nest restrictions; it limits HOME only.
#[cfg(not(target_os = "macos"))]
fn outer_restrictions(_allow: &[PathBuf]) -> Vec<ReadRestriction> {
    Vec::new()
}

/// What is being started: the agent's name, command, kind and environment.
#[derive(Clone, Copy)]
pub(crate) struct Launch<'a> {
    pub name: &'a str,
    pub cwd: &'a Path,
    pub run_dir: Option<&'a Path>,
    pub argv: &'a [String],
    pub agent: Option<&'a str>,
    pub env: &'a [(String, String)],
    pub helper: Option<&'a Path>,
}

/// Opens the agent's egress listener and wraps `argv` in the platform sandbox.
/// Fails closed: no sandbox program, or a policy the platform cannot enforce,
/// is an error, never an unsandboxed start.
pub(crate) fn confine(
    egress: &Egress,
    spec: &SandboxSpec,
    launch: &Launch,
) -> Result<Confined, HostError> {
    let Launch { name, cwd, run_dir, argv, agent, env, helper } = *launch;
    let refuse = |why: &str| HostError::Sandbox(why.to_string());
    let run_dir = run_dir.ok_or_else(|| refuse("the host has no run directory to protect"))?;
    let backend = backend().ok_or_else(|| refuse("no sandbox program (sandbox-exec or bwrap) on this machine"))?;
    let claude = claude_config(env);
    if agent == Some("claude")
        && let Some(claude) = &claude
        && let Err(e) = trust_claude_project(&claude.json_dir, cwd)
    {
        eprintln!("hq host: could not record trust for {}: {e}", cwd.display());
    }
    // A bubblewrap sandbox has no network, so it reaches the proxy through a unix
    // socket in the host directory that a relay inside turns back into loopback.
    let bridge = match backend {
        Backend::Bwrap(_) => Some(Bridge {
            program: match helper {
                Some(path) => path.to_path_buf(),
                None => std::env::current_exe().map_err(|e| HostError::Io(format!("this binary: {e}")))?,
            },
            socket: run_dir
                .canonicalize()
                .map_err(|e| HostError::Io(format!("the host directory: {e}")))?
                .join(format!("eg-{name}.sock")),
        }),
        Backend::SandboxExec(_) => None,
    };
    let port = egress
        .open_with_bridge(name, rules(&spec.allow), bridge.as_ref().map(|b| b.socket.as_path()))
        .map_err(|e| HostError::Io(format!("the egress listener: {e}")))?;
    let wrapped = claude
        .ok_or_else(|| HostError::Io("HOME is not set".into()))
        .and_then(|c| policy(spec, name, cwd, run_dir, port, &c, bridge.as_ref()))
        .and_then(|policy| wrap(backend, &policy, argv).map_err(|e| refuse(&e.to_string())));
    let wrapped = match wrapped {
        Ok(w) => w,
        Err(e) => {
            egress.close_port(name, port);
            return Err(e);
        }
    };
    let program = wrapped.program.to_string_lossy().into_owned();
    let args = wrapped.args.into_iter().map(|a| a.into_string());
    let argv = std::iter::once(Ok(program))
        .chain(args)
        .collect::<Result<Vec<String>, _>>()
        .map_err(|_| refuse("a sandbox path is not valid UTF-8"));
    let argv = match argv {
        Ok(a) => a,
        Err(e) => {
            egress.close_port(name, port);
            return Err(e);
        }
    };
    let url = format!("http://127.0.0.1:{port}");
    let mut env: Vec<(String, String)> = PROXY_ENV_NAMES.iter().map(|n| (n.to_string(), url.clone())).collect();
    env.push(("DISABLE_AUTOUPDATER".into(), "1".into()));
    Ok(Confined { argv, env, egress_port: port })
}

/// Records that the operator trusts `cwd` in `<home>/.claude.json`, which the
/// sandboxed agent cannot write, so Claude Code does not stop at its trust
/// dialog on every launch. Starting an agent there is the operator's choice.
pub(crate) fn trust_claude_project(home: &Path, cwd: &Path) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    const FILE_MODE: u32 = 0o600;
    let path = home.join(".claude.json");
    let mut doc: serde_json::Value = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => return Err(e),
    };
    let key = cwd.to_string_lossy().into_owned();
    let projects = doc
        .as_object_mut()
        .ok_or_else(|| std::io::Error::other(".claude.json is not an object"))?
        .entry("projects")
        .or_insert_with(|| serde_json::json!({}));
    let entry = projects
        .as_object_mut()
        .ok_or_else(|| std::io::Error::other("projects is not an object"))?
        .entry(key)
        .or_insert_with(|| serde_json::json!({}));
    let Some(entry) = entry.as_object_mut() else {
        return Err(std::io::Error::other("a project entry is not an object"));
    };
    if entry.get("hasTrustDialogAccepted") == Some(&serde_json::Value::Bool(true)) {
        return Ok(());
    }
    entry.insert("hasTrustDialogAccepted".into(), true.into());
    let tmp = home.join(format!(".claude.json.hq-{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .open(&tmp)?;
    file.write_all(serde_json::to_string_pretty(&doc)?.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_is_recorded_without_disturbing_the_rest() {
        let home = tempfile::tempdir().unwrap();
        let file = home.path().join(".claude.json");
        std::fs::write(&file, r#"{"mcpServers":{"a":{"command":"x"}},"projects":{"/p":{"allowedTools":["t"]}}}"#).unwrap();
        trust_claude_project(home.path(), Path::new("/p")).unwrap();
        trust_claude_project(home.path(), Path::new("/q")).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(doc["mcpServers"]["a"]["command"], "x");
        assert_eq!(doc["projects"]["/p"]["allowedTools"][0], "t");
        assert_eq!(doc["projects"]["/p"]["hasTrustDialogAccepted"], true);
        assert_eq!(doc["projects"]["/q"]["hasTrustDialogAccepted"], true);
    }

    #[test]
    fn a_missing_file_is_created_private_and_a_broken_one_is_left_alone() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        trust_claude_project(home.path(), Path::new("/p")).unwrap();
        let mode = std::fs::metadata(home.path().join(".claude.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let broken = tempfile::tempdir().unwrap();
        std::fs::write(broken.path().join(".claude.json"), "{not json").unwrap();
        assert!(trust_claude_project(broken.path(), Path::new("/p")).is_err());
        assert_eq!(std::fs::read_to_string(broken.path().join(".claude.json")).unwrap(), "{not json");
    }
}
