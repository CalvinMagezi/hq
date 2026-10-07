//! Turns a sandbox request into the wrapped command line of one agent: the
//! egress listener it may connect to, the paths it may touch, and the proxy
//! variables that make its HTTP clients use the listener.

use crate::egress::{Egress, Rule};
use crate::error::HostError;
use crate::server;
use hq_sandbox::{AgentSandbox, HiddenDir, Network, ReadRestriction, backend, canonical, wrap};
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
    ".claude", ".claude.json", ".local", ".cache", ".config", ".cargo", ".rustup", ".nvm", ".npm", ".bun",
    ".volta", ".pyenv", ".asdf", ".deno", ".gitconfig", ".gitignore_global", ".terminfo", ".zshenv",
    ".zprofile", ".zshrc", ".zlogin", ".bashrc", ".bash_profile", ".profile", ".inputrc", ".oh-my-zsh",
    "Library/Caches", "Library/Preferences", "Library/Keychains", ".claude.json.lock",
];
/// Other Claude profiles (`~/.claude-<name>`) hold their own logins.
const OTHER_PROFILE_PREFIX: &str = ".claude-";
/// Credential files directly under HOME.
const SECRET_HOME_FILES: &[&str] = &[".netrc", ".npmrc", ".git-credentials"];
/// Files and directories the operator's own tools run code from later, outside
/// the sandbox. Writable roots must not let an agent plant anything in them.
const HOME_READONLY: &[&str] = &[".claude/settings.json", ".claude/settings.local.json", ".claude/hooks"];
const PROJECT_READONLY: &[&str] = &[".git/hooks", ".git/config", ".mcp.json", ".claude/settings.json", ".claude/settings.local.json", ".claude/hooks"];
/// Ways out of Seatbelt: starting an app outside it, or scheduling one.
const DENIED_PROGRAMS: &[&str] = &["/usr/bin/open", "/usr/bin/osascript", "/bin/launchctl", "/usr/bin/launchctl"];
const DENIED_SERVICES: &[&str] = &["com.apple.coreservices.launchservicesd", "com.apple.dnssd.service"];
/// What Claude Code needs to write under HOME, besides the project.
/// `~/.claude.json` is deliberately not writable: it holds MCP server commands the
/// operator's own Claude runs later. The host records trust for the project itself.
const HOME_WRITABLE: &[&str] = &[".claude", ".cache", "Library/Caches"];
const TMP_ROOTS: &[&str] = &["/tmp", "/var/tmp", "/var/folders", "/dev"];
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

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.is_absolute())
}

fn policy(
    spec: &SandboxSpec,
    name: &str,
    cwd: &Path,
    run_dir: &Path,
    port: u16,
) -> Result<AgentSandbox, HostError> {
    let home = home().ok_or_else(|| HostError::Io("HOME is not set".into()))?;
    let home = home.canonicalize().unwrap_or(home);
    let under = |names: &[&str]| names.iter().map(|n| home.join(n)).collect::<Vec<_>>();
    let tmp = std::env::temp_dir();
    let writable = canonical(
        under(HOME_WRITABLE)
            .into_iter()
            .chain(TMP_ROOTS.iter().map(PathBuf::from))
            .chain([tmp])
            .chain(spec.writable.iter().cloned()),
    );
    let run_dir = run_dir.canonicalize().map_err(|e| HostError::Io(e.to_string()))?;
    let project = cwd.canonicalize().map_err(|e| HostError::Io(e.to_string()))?;
    let other_profiles = std::fs::read_dir(&home)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(OTHER_PROFILE_PREFIX))
        .map(|e| e.path());
    let secret_dirs = canonical(under(SECRET_HOME_DIRS).into_iter().chain(other_profiles));
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
    let own_files = [
        run_dir.join("hooks").join(format!("{name}.json")),
        run_dir.join("mcp").join(format!("{name}.json")),
    ];
    let socket = server::agent_socket_path(&run_dir);
    let mut allow = canonical(own_files);
    allow.push(socket.clone());
    let mut hidden = vec![HiddenDir { dir: run_dir, allow }];
    hidden.extend(secret_dirs.into_iter().map(|dir| HiddenDir { dir, allow: Vec::new() }));
    let under_project = |names: &[&str]| names.iter().map(|n| project.join(n)).collect::<Vec<_>>();
    let (home_files, home_dirs): (Vec<_>, Vec<_>) =
        under(HOME_READONLY).into_iter().partition(|p| p.extension().is_some());
    let (project_files, project_dirs): (Vec<_>, Vec<_>) = under_project(PROJECT_READONLY)
        .into_iter()
        .partition(|p| p.extension().is_some() || p.ends_with("config"));
    let macos = cfg!(target_os = "macos");
    let read_allow = canonical(
        under(HOME_READABLE)
            .into_iter()
            .chain(spec.readable.iter().cloned())
            .chain(std::iter::once(project.clone()))
            .chain(writable.iter().cloned()),
    );
    Ok(AgentSandbox {
        project,
        writable,
        writable_files: Vec::new(),
        readonly_files: home_files.into_iter().chain(project_files).collect(),
        readonly_subpaths: home_dirs.into_iter().chain(project_dirs).collect(),
        denied_programs: if macos { DENIED_PROGRAMS.iter().map(PathBuf::from).collect() } else { Vec::new() },
        denied_services: if macos { DENIED_SERVICES.iter().map(|s| s.to_string()).collect() } else { Vec::new() },
        masked_files: under(SECRET_HOME_FILES),
        hidden_dirs: hidden,
        read_restriction: Some(ReadRestriction {
            root: home.clone(),
            allow: read_allow,
        }),
        hide_other_processes: true,
        network: Network::Proxy {
            port,
            unix_sockets: vec![socket],
        },
    })
}

/// Opens the agent's egress listener and wraps `argv` in the platform sandbox.
/// Fails closed: no sandbox program, or a policy the platform cannot enforce,
/// is an error, never an unsandboxed start.
pub(crate) fn confine(
    egress: &Egress,
    spec: &SandboxSpec,
    name: &str,
    cwd: &Path,
    run_dir: Option<&Path>,
    argv: &[String],
    agent: Option<&str>,
) -> Result<Confined, HostError> {
    let refuse = |why: &str| HostError::Sandbox(why.to_string());
    let run_dir = run_dir.ok_or_else(|| refuse("the host has no run directory to protect"))?;
    let backend = backend().ok_or_else(|| refuse("no sandbox program (sandbox-exec or bwrap) on this machine"))?;
    if agent == Some("claude")
        && let Some(home) = home()
        && let Err(e) = trust_claude_project(&home, cwd)
    {
        eprintln!("hq host: could not record trust for {}: {e}", cwd.display());
    }
    let port = egress
        .open(name, rules(&spec.allow))
        .map_err(|e| HostError::Io(e.to_string()))?;
    let wrapped = policy(spec, name, cwd, run_dir, port)
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
