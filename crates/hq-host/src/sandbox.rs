//! Turns a sandbox request into the wrapped command line of one agent: the
//! egress listener it may connect to, the paths it may touch, and the proxy
//! variables that make its HTTP clients use the listener.

use crate::egress::{Egress, Rule};
use crate::error::HostError;
use crate::server;
use hq_sandbox::{
    AgentSandbox, Backend, HiddenDir, Network, ReadRestriction, Relay, backend, canonical, wrap,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// HQ's own directory under HOME, which no setting reveals.
const HQ_HOME_DIR: &str = ".hq";
/// Directories under HOME that hold credentials and have no place in an agent.
const SECRET_HOME_DIRS: &[&str] = &[".ssh", ".gnupg", ".aws", ".kube", ".docker", ".config/gh"];
/// What an agent may read under HOME. Everything else there is unreadable, so a
/// token or note the operator keeps in some other directory cannot be read and
/// sent to an allowed host. Tool installs, shell and git configuration, and the
/// agent's own state are listed; `agent_host.sandbox.readable` adds more.
const HOME_READABLE: &[&str] = &[
    ".local",
    ".cache",
    ".config",
    ".cargo",
    ".rustup",
    ".nvm",
    ".npm",
    ".bun",
    ".volta",
    ".pyenv",
    ".asdf",
    ".deno",
    ".gitconfig",
    ".gitignore_global",
    ".terminfo",
    ".zshenv",
    ".zprofile",
    ".zshrc",
    ".zlogin",
    ".bashrc",
    ".bash_profile",
    ".profile",
    ".inputrc",
    ".oh-my-zsh",
    "Library/Caches",
    "Library/Preferences",
    "Library/Keychains",
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
const CLAUDE_DIR_READONLY_FILES: &[&str] =
    &["settings.json", "settings.local.json", ".claude.json"];
const CLAUDE_DIR_READONLY_DIRS: &[&str] = &["hooks"];
const CLAUDE_CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";
const DEFAULT_CLAUDE_DIR: &str = ".claude";
const PROJECT_READONLY: &[&str] = &[
    ".git/hooks",
    ".git/config",
    ".mcp.json",
    ".claude/settings.json",
    ".claude/settings.local.json",
    ".claude/hooks",
];
/// The open profile's one guard on writes: what the operator's own tools run
/// from later, outside the sandbox, and so what an agent could plant code in to
/// get past the wall around HQ's directory. Nothing an agent needs for
/// development lives here. Paths that do not exist are skipped under bubblewrap.
const OPEN_READONLY_HOME_FILES: &[&str] = &[
    ".bashrc",
    ".bash_profile",
    ".bash_login",
    ".bash_logout",
    ".profile",
    ".zshenv",
    ".zprofile",
    ".zshrc",
    ".zlogin",
    ".zlogout",
    ".gitconfig",
];
const OPEN_READONLY_HOME_DIRS: &[&str] = &[
    ".config/git",
    ".config/autostart",
    ".config/systemd/user",
    "Library/LaunchAgents",
];
/// Ways out of Seatbelt: starting an app outside it, or scheduling one.
const DENIED_PROGRAMS: &[&str] = &[
    "/usr/bin/open",
    "/usr/bin/osascript",
    "/bin/launchctl",
    "/usr/bin/launchctl",
];
/// The strict profile also denies DNS: names are resolved by its proxy, so a
/// lookup from inside could only be a way to carry data out.
const DENIED_SERVICES: &[&str] = &[
    "com.apple.coreservices.launchservicesd",
    "com.apple.dnssd.service",
];
/// The open profile resolves names itself, so DNS stays; the launcher stays denied.
const DENIED_SERVICES_OPEN: &[&str] = &["com.apple.coreservices.launchservicesd"];
/// What Claude Code needs to write under HOME, besides the project.
/// `~/.claude.json` is deliberately not writable: it holds MCP server commands the
/// operator's own Claude runs later. The host records trust for the project itself.
/// Where each coding agent keeps its own login, history and logs under HOME. An
/// agent may write only its own: the others stay unreadable to it. (Claude Code is
/// handled separately because a launch profile can move its directory.)
const AGENT_STATE_DIRS: &[(&str, &[&str])] = &[
    ("agy", &[".gemini"]),
    ("antigravity", &[".gemini"]),
    ("codex", &[".codex"]),
    ("cursor", &[".cursor", ".config/cursor"]),
    (
        "opencode",
        &[
            ".config/opencode",
            ".local/share/opencode",
            ".local/state/opencode",
        ],
    ),
    ("copilot", &[".copilot", ".config/github-copilot"]),
    ("kimi", &[".kimi"]),
    ("qwen", &[".qwen"]),
    ("pi", &[".pi"]),
    // General-purpose agents keep their config, skills and memory here.
    ("hermes", &[".hermes"]),
    ("openclaw", &[".openclaw"]),
];
/// Hosts an agent needs to sign in and reach its own models, over HTTPS. HQ sends
/// the Anthropic API for every launch; these cover the others, so a launch does not
/// fail on a login the user already has. Only the first row has been run against
/// the real CLI. Agents that talk to whichever model provider the user configures
/// (opencode, kimi, qwen) are left to `agent_host.sandbox.allow_domains`.
const AGENT_DEFAULT_ALLOW: &[(&str, &[&str])] = &[
    (
        "agy",
        &[
            "oauth2.googleapis.com",
            "www.googleapis.com",
            "cloudcode-pa.googleapis.com",
            "daily-cloudcode-pa.googleapis.com",
            "generativelanguage.googleapis.com",
            "play.googleapis.com",
            "antigravity-unleash.goog",
            "accounts.google.com",
            "lh3.googleusercontent.com",
        ],
    ),
    (
        "codex",
        &["api.openai.com", "auth.openai.com", "chatgpt.com"],
    ),
    ("cursor", &["*.cursor.sh"]),
    (
        "copilot",
        &["*.githubcopilot.com", "api.github.com", "github.com"],
    ),
    ("pi", &["pi.dev", "generativelanguage.googleapis.com"]),
];
const HOME_WRITABLE: &[&str] = &[".cache", "Library/Caches"];
/// Temporary directories every agent shares on macOS (Seatbelt lists them writable).
const TMP_ROOTS: &[&str] = &["/tmp", "/var/tmp", "/var/folders", "/dev"];
/// What bubblewrap gives each agent a private empty copy of instead: the shared
/// temporary directories, and /run, where the system's own sockets live.
const PRIVATE_TMP_ROOTS: &[&str] = &["/tmp", "/var/tmp", "/run"];
const NODE_PROXY_SWITCH: &str = "NODE_USE_ENV_PROXY";
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

/// How tightly a sandboxed agent is confined.
///
/// A request that names no profile is `strict`, the only policy there was before
/// profiles, so a client that predates them keeps what it asked for. HQ itself
/// always names one (`agent_host.sandbox.profile`, `open` by default).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    /// Reads under HOME limited to a list, writes to the project, caches and the
    /// agent's own state, a private `/tmp`, no view of other processes, and a
    /// network that is a proxy with a domain allowlist.
    #[default]
    Strict,
    /// The agent works as the user would in a terminal: the whole filesystem to
    /// read and write, the machine's own network, shared `/tmp` and devices. What
    /// stays out of reach is HQ itself: the host directory with its tokens,
    /// `~/.hq`, other processes' environments, other Claude accounts, and (unless
    /// `hide_credentials` is off) the credential stores. A few files the user's own
    /// tools run later, such as shell startup files, stay read-only.
    Open,
}

impl Profile {
    pub fn as_str(self) -> &'static str {
        match self {
            Profile::Strict => "strict",
            Profile::Open => "open",
        }
    }
}

fn yes() -> bool {
    true
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
    /// How tightly the agent is confined; see [`Profile`].
    #[serde(default)]
    pub profile: Profile,
    /// Keep `~/.ssh`, `~/.aws`, `~/.config/gh` and the other credential stores
    /// (and `~/.netrc`, `~/.npmrc`) out of the agent's reach. Only the open
    /// profile can turn this off; the strict one always hides them. `~/.hq` and
    /// other Claude accounts stay hidden either way.
    #[serde(default = "yes")]
    pub hide_credentials: bool,
    /// Hosts the agent may reach (strict only; the open profile has no proxy).
    #[serde(default)]
    pub allow: Vec<Allow>,
    /// Extra directories the agent may write under.
    #[serde(default)]
    pub writable: Vec<PathBuf>,
    /// Extra paths under HOME the agent may read.
    #[serde(default)]
    pub readable: Vec<PathBuf>,
    /// Paths only one kind of agent may read, keyed by agent. A credential
    /// directory named exactly is revealed to that agent, read-only.
    #[serde(default)]
    pub agent_readable: BTreeMap<String, Vec<PathBuf>>,
    /// Loopback ports the agent may connect to directly (macOS only).
    #[serde(default)]
    pub loopback_ports: Vec<u16>,
}

impl SandboxSpec {
    pub fn none() -> Self {
        Self {
            mode: Mode::None,
            profile: Profile::default(),
            hide_credentials: true,
            allow: Vec::new(),
            writable: Vec::new(),
            readable: Vec::new(),
            agent_readable: BTreeMap::new(),
            loopback_ports: Vec::new(),
        }
    }
}

/// The name an agent kind goes by in the tables here and in `agent_readable`.
fn agent_kind(agent: &str) -> &str {
    if agent == "antigravity" { "agy" } else { agent }
}

/// The unix socket a bubblewrap sandbox reaches the egress proxy through, and the
/// binary that relays it onto loopback inside.
pub(crate) struct Bridge {
    pub program: PathBuf,
    pub socket: PathBuf,
}

/// The command to run and the variables to add for a sandboxed agent, plus the
/// egress port to close when it exits (none under the open profile, which has no
/// proxy).
pub(crate) struct Confined {
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub egress_port: Option<u16>,
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
        return Some(ClaudeConfig {
            json_dir: dir.clone(),
            dir,
            is_default: false,
        });
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
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute())
}

/// The launch a policy is for, beyond the spec.
struct Target<'a> {
    name: &'a str,
    cwd: &'a Path,
    run_dir: &'a Path,
    claude: &'a ClaudeConfig,
    /// The user's home directory, resolved.
    home: &'a Path,
    /// The agent's kind, as the tables here name it.
    kind: Option<&'a str>,
    /// The sandbox program is bubblewrap, which can only mount over a path that exists.
    bwrap: bool,
}

/// What both profiles keep out of an agent's hands: the host directory (the
/// operator token and every agent's own token), HQ's directory, the credential
/// stores (unless the open profile turns that off), other Claude accounts, and the
/// Claude config files the user's own Claude runs code from later.
struct Wall {
    project: PathBuf,
    run_dir: PathBuf,
    /// The agent socket: the one way back into the host an agent has.
    socket: PathBuf,
    /// Every directory hidden or revealed, whichever it is.
    secret_dirs: Vec<PathBuf>,
    /// The ones the spec names for this agent, readable and never writable.
    revealed: Vec<PathBuf>,
    /// The paths the spec names for this agent, resolved.
    agent_readable: Vec<PathBuf>,
    hidden: Vec<HiddenDir>,
    masked_files: Vec<PathBuf>,
    claude_files: Vec<PathBuf>,
    claude_dirs: Vec<PathBuf>,
}

fn under(base: &Path, names: &[&str]) -> Vec<PathBuf> {
    names.iter().map(|n| base.join(n)).collect()
}

/// The login and history directories of every coding agent but this one. Two kinds
/// that share a directory (Antigravity's two names) do not hide it from each other.
fn other_agent_state_dirs(home: &Path, kind: Option<&str>) -> Vec<PathBuf> {
    let own: Vec<&str> = AGENT_STATE_DIRS
        .iter()
        .filter(|(k, _)| Some(*k) == kind)
        .flat_map(|(_, dirs)| dirs.iter().copied())
        .collect();
    AGENT_STATE_DIRS
        .iter()
        .flat_map(|(_, dirs)| dirs.iter().copied())
        .filter(|dir| !own.contains(dir))
        .map(|dir| home.join(dir))
        .collect()
}

fn wall(spec: &SandboxSpec, t: &Target, bridge: Option<&Bridge>) -> Result<Wall, HostError> {
    let home = t.home;
    let claude_dir: &Path = &t.claude.dir;
    // The strict profile reads nothing under HOME that it does not list, so it
    // always hides these; the setting is the open profile's.
    let hide_credentials = spec.hide_credentials || spec.profile == Profile::Strict;
    let run_dir = t
        .run_dir
        .canonicalize()
        .map_err(|e| HostError::Io(format!("the host directory: {e}")))?;
    let project = t
        .cwd
        .canonicalize()
        .map_err(|e| HostError::Io(format!("the working directory: {e}")))?;
    let other_profiles = std::fs::read_dir(home)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(OTHER_PROFILE_PREFIX)
        })
        .map(|e| e.path())
        .filter(|p| p.canonicalize().ok().as_deref() != Some(claude_dir));
    // With a profile's own config directory, the default ~/.claude is another account.
    let default_dir = (!t.claude.is_default).then(|| home.join(DEFAULT_CLAUDE_DIR));
    let credentials = if hide_credentials {
        under(home, SECRET_HOME_DIRS)
    } else {
        Vec::new()
    };
    // The strict profile cannot read them anyway, as it reads nothing under HOME that
    // it does not list; the open profile hides the other agents' logins by name.
    let other_agents = if spec.profile == Profile::Open {
        other_agent_state_dirs(home, t.kind)
    } else {
        Vec::new()
    };
    let secret_dirs = canonical(
        credentials
            .into_iter()
            .chain([home.join(HQ_HOME_DIR)])
            .chain(other_profiles)
            .chain(default_dir)
            .chain(other_agents),
    );
    // `confine` keeps only this agent's entries.
    let agent_readable = canonical(spec.agent_readable.values().flatten().cloned());
    // HQ's own directory holds the operator's tokens and is never revealed.
    let hq_dir = home.join(HQ_HOME_DIR);
    let hq_dir = hq_dir.canonicalize().unwrap_or(hq_dir);
    let revealed: Vec<PathBuf> = secret_dirs
        .iter()
        .filter(|d| agent_readable.contains(d) && **d != hq_dir)
        .cloned()
        .collect();
    let own_files = [
        run_dir.join("hooks").join(format!("{}.json", t.name)),
        run_dir.join("mcp").join(format!("{}.json", t.name)),
    ];
    let socket = server::agent_socket_path(&run_dir);
    let mut allow = canonical(own_files);
    allow.push(socket.clone());
    allow.extend(bridge.map(|b| b.socket.clone()));
    // A bubblewrap bind needs the file to exist.
    allow.retain(|p| !t.bwrap || p.exists());
    let mut hidden = vec![HiddenDir {
        dir: run_dir.clone(),
        allow,
    }];
    hidden.extend(
        secret_dirs
            .iter()
            .filter(|d| !revealed.contains(d))
            .map(|dir| HiddenDir {
                dir: dir.clone(),
                allow: Vec::new(),
            }),
    );
    Ok(Wall {
        project,
        run_dir,
        socket,
        secret_dirs,
        revealed,
        agent_readable,
        hidden,
        masked_files: if hide_credentials {
            under(home, SECRET_HOME_FILES)
        } else {
            Vec::new()
        },
        claude_files: CLAUDE_DIR_READONLY_FILES
            .iter()
            .map(|f| claude_dir.join(f))
            .collect(),
        claude_dirs: CLAUDE_DIR_READONLY_DIRS
            .iter()
            .map(|d| claude_dir.join(d))
            .collect(),
    })
}

/// The policy for one agent. `port` is its egress listener, which only the strict
/// profile has.
fn policy(
    spec: &SandboxSpec,
    t: &Target,
    port: Option<u16>,
    bridge: Option<&Bridge>,
) -> Result<AgentSandbox, HostError> {
    let wall = wall(spec, t, bridge)?;
    match (spec.profile, port) {
        (Profile::Open, _) => Ok(open_policy(t, wall)),
        (Profile::Strict, Some(port)) => strict_policy(spec, t, wall, port, bridge),
        (Profile::Strict, None) => Err(HostError::Sandbox(
            "the strict profile needs an egress listener".into(),
        )),
    }
}

fn strict_policy(
    spec: &SandboxSpec,
    t: &Target,
    wall: Wall,
    port: u16,
    bridge: Option<&Bridge>,
) -> Result<AgentSandbox, HostError> {
    let home = t.home;
    let bwrap = t.bwrap;
    let claude_dir: &Path = &t.claude.dir;
    let default_claude = t.claude.is_default;
    let Wall {
        project,
        run_dir,
        socket,
        secret_dirs,
        revealed,
        agent_readable,
        hidden,
        masked_files,
        claude_files,
        claude_dirs,
    } = wall;
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
    let own_tmp = canonical([tmp])
        .into_iter()
        .filter(|d| !private_tmp.iter().any(|p| d.starts_with(p)));
    let writable = canonical(
        under(home, HOME_WRITABLE)
            .into_iter()
            .chain([claude_dir.to_path_buf()])
            .chain(shared_tmp)
            .chain(own_tmp)
            .chain(spec.writable.iter().cloned()),
    );
    // A writable root that contains HOME, the run directory or a secret directory
    // would let the agent rename or rewrite them, which no read rule survives.
    let guarded = [home.to_path_buf(), run_dir.clone()]
        .into_iter()
        .chain(secret_dirs.iter().cloned());
    for root in std::iter::once(&project).chain(&writable) {
        if let Some(inside) = guarded.clone().find(|g| g.starts_with(root)) {
            return Err(HostError::Sandbox(format!(
                "{} is writable but contains {}; start the agent in a project directory",
                root.display(),
                inside.display()
            )));
        }
        // Writing inside one of them is as bad: hiding only stops reads.
        if let Some(outer) = [&run_dir]
            .into_iter()
            .chain(&secret_dirs)
            .find(|g| root.starts_with(g))
        {
            return Err(HostError::Sandbox(format!(
                "{} is writable but lies inside {}, which agents may not write",
                root.display(),
                outer.display()
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
    let (project_files, project_dirs): (Vec<_>, Vec<_>) = under(&project, PROJECT_READONLY)
        .into_iter()
        .partition(|p| p.extension().is_some() || p.ends_with("config"));
    // Seatbelt denies writes to a path whether or not it exists; a bubblewrap
    // mount needs something to mount over, so only existing paths are protected.
    let keep = |paths: Vec<PathBuf>| -> Vec<PathBuf> {
        paths.into_iter().filter(|p| !bwrap || p.exists()).collect()
    };
    let macos = cfg!(target_os = "macos");
    let read_allow = canonical(
        under(home, HOME_READABLE)
            .into_iter()
            .chain([claude_dir.to_path_buf()])
            .chain(default_claude.then(|| home.join(".claude.json")))
            .chain(spec.readable.iter().cloned())
            .chain(agent_readable)
            .chain(std::env::current_exe())
            .chain(std::iter::once(project.clone()))
            .chain(writable.iter().cloned()),
    );
    Ok(AgentSandbox {
        project,
        open_filesystem: false,
        writable,
        writable_files: Vec::new(),
        readonly_files: keep(claude_files.into_iter().chain(project_files).collect()),
        readonly_subpaths: keep(
            claude_dirs
                .into_iter()
                .chain(project_dirs)
                .chain(revealed)
                .collect(),
        ),
        pinned_dirs: Vec::new(),
        denied_programs: if macos {
            DENIED_PROGRAMS.iter().map(PathBuf::from).collect()
        } else {
            Vec::new()
        },
        denied_services: if macos {
            DENIED_SERVICES.iter().map(|s| s.to_string()).collect()
        } else {
            Vec::new()
        },
        masked_files: keep(masked_files),
        hidden_dirs: hidden,
        private_tmp,
        read_restrictions: outer_restrictions(&read_allow)
            .into_iter()
            .chain([ReadRestriction {
                root: home.to_path_buf(),
                allow: read_allow,
            }])
            .collect(),
        hide_other_processes: true,
        network: Network::Proxy {
            port,
            unix_sockets: vec![socket],
            relay: bridge.map(|b| Relay {
                program: b.program.clone(),
                unix_socket: b.socket.clone(),
            }),
            loopback_ports: if bwrap {
                Vec::new()
            } else {
                spec.loopback_ports
                    .iter()
                    .copied()
                    .filter(|p| *p != 0)
                    .collect()
            },
        },
    })
}

/// The open profile: what the user could do in a terminal, except reach HQ
/// itself. No proxy, no read limits, no private temporary directories, writes
/// anywhere. What is left is the wall (see [`Wall`]), the short read-only list of
/// files the user's own tools run later, and the other processes' environments,
/// which hold the daemon's and the other agents' tokens.
fn open_policy(t: &Target, wall: Wall) -> AgentSandbox {
    let home = t.home;
    let bwrap = t.bwrap;
    let Wall {
        project,
        revealed,
        hidden,
        masked_files,
        claude_files,
        claude_dirs,
        ..
    } = wall;
    let keep = |paths: Vec<PathBuf>| -> Vec<PathBuf> {
        with_link_targets(paths)
            .into_iter()
            .filter(|p| !bwrap || p.exists())
            .collect()
    };
    let mut files = claude_files;
    files.extend(under(home, OPEN_READONLY_HOME_FILES));
    // The default account's trust file lies beside its directory, not in it: its own
    // to read but not write, and another account's to neither.
    let mut masked_files = masked_files;
    let default_json = home.join(".claude.json");
    if t.claude.is_default {
        files.push(default_json);
    } else {
        masked_files.push(default_json);
    }
    let mut dirs = claude_dirs;
    dirs.extend(under(home, OPEN_READONLY_HOME_DIRS));
    dirs.extend(revealed);
    let mut pinned_dirs = Vec::new();
    if !bwrap {
        // Seatbelt hides a path by refusing reads of it, so with every write allowed
        // the hidden directories must be closed to writes too, and so must the
        // directories above them. Under bubblewrap a hidden directory is a mount
        // over an empty one, which cannot be renamed, written or carried off.
        dirs.extend(hidden.iter().map(|h| h.dir.clone()));
        pinned_dirs = pinned_parents(&hidden, home);
    }
    let macos = cfg!(target_os = "macos");
    AgentSandbox {
        project,
        open_filesystem: true,
        writable: Vec::new(),
        writable_files: Vec::new(),
        readonly_files: keep(files),
        readonly_subpaths: keep(dirs),
        pinned_dirs,
        denied_programs: if macos {
            DENIED_PROGRAMS.iter().map(PathBuf::from).collect()
        } else {
            Vec::new()
        },
        denied_services: if macos {
            DENIED_SERVICES_OPEN.iter().map(|s| s.to_string()).collect()
        } else {
            Vec::new()
        },
        masked_files: keep(masked_files),
        hidden_dirs: hidden,
        private_tmp: Vec::new(),
        read_restrictions: Vec::new(),
        hide_other_processes: true,
        network: Network::Full,
    }
}

/// The paths, and for any that is a symlink where it leads: Seatbelt judges the
/// resolved path, so a rule on a dotfile linked into a dotfiles repo has to name
/// the repo's copy too.
fn with_link_targets(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for path in paths {
        if let Ok(real) = path.canonicalize()
            && real != path
        {
            out.push(real);
        }
        out.push(path);
    }
    out.sort();
    out.dedup();
    out
}

/// The directories above each hidden one, up to but not including HOME (or the
/// filesystem root): moving one would carry the hidden directory out from under
/// the rule that names it.
fn pinned_parents(hidden: &[HiddenDir], home: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = hidden
        .iter()
        .flat_map(|h| {
            h.dir
                .ancestors()
                .skip(1)
                .take_while(|a| *a != home && a.parent().is_some())
                .map(Path::to_path_buf)
        })
        .collect();
    out.sort();
    out.dedup();
    out
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

/// Stands for the target machine's home directory in launch arguments.
pub(crate) const HOME_PLACEHOLDER: &str = "{home}";

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

/// Wraps `argv` in the platform sandbox, and under the strict profile opens the
/// agent's egress listener first. Fails closed: no sandbox program, or a policy
/// the platform cannot enforce, is an error, never an unsandboxed start.
pub(crate) fn confine(
    egress: &Egress,
    spec: &SandboxSpec,
    launch: &Launch,
) -> Result<Confined, HostError> {
    let Launch {
        name,
        cwd,
        run_dir,
        argv,
        agent,
        env,
        helper,
    } = *launch;
    let refuse = |why: &str| HostError::Sandbox(why.to_string());
    // The caller cannot know this machine's home, so `{home}` in a launch argument is filled in here.
    let home_dir = home()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_default();
    let expanded: Vec<String> = argv
        .iter()
        .map(|a| a.replace(HOME_PLACEHOLDER, &home_dir))
        .collect();
    let argv = expanded.as_slice();
    let run_dir = run_dir.ok_or_else(|| refuse("the host has no run directory to protect"))?;
    let backend = backend()
        .ok_or_else(|| refuse("no sandbox program (sandbox-exec or bwrap) on this machine"))?;
    let claude = claude_config(env);
    let strict = spec.profile == Profile::Strict;
    // Under the strict profile the agent's own state directories are writable
    // (created if its first run has not made them yet, so there is something to
    // allow). The open profile writes anywhere and has no proxy to list hosts in.
    let mut own_spec = spec.clone();
    let kind = agent.map(agent_kind);
    own_spec
        .agent_readable
        .retain(|k, _| Some(agent_kind(k)) == kind);
    if strict && matches!(backend, Backend::Bwrap(_)) && !own_spec.loopback_ports.is_empty() {
        eprintln!(
            "hq host: loopback ports {:?} are not opened under bubblewrap; HTTP to them still goes through the proxy",
            own_spec.loopback_ports
        );
    }
    if strict && let Some(kind) = kind {
        if let Some((_, dirs)) = AGENT_STATE_DIRS.iter().find(|(k, _)| *k == kind) {
            let home = home().ok_or_else(|| refuse("HOME is not set"))?;
            for dir in dirs.iter().map(|d| home.join(d)) {
                let _ = std::fs::create_dir_all(&dir);
                own_spec.writable.push(dir);
            }
        }
        if let Some((_, hosts)) = AGENT_DEFAULT_ALLOW.iter().find(|(k, _)| *k == kind) {
            for host in *hosts {
                if !own_spec.allow.iter().any(|a| a.host == *host) {
                    own_spec.allow.push(Allow {
                        host: host.to_string(),
                        ports: vec![443],
                        private: false,
                    });
                }
            }
        }
    }
    let spec = &own_spec;
    if agent == Some("claude")
        && let Some(claude) = &claude
        && let Err(e) = trust_claude_project(&claude.json_dir, cwd)
    {
        eprintln!("hq host: could not record trust for {}: {e}", cwd.display());
    }
    // A bubblewrap sandbox has no network, so it reaches the proxy through a unix
    // socket in the host directory that a relay inside turns back into loopback.
    // The open profile keeps the machine's own network and has neither.
    let (bridge, port) = if strict {
        let bridge = match backend {
            Backend::Bwrap(_) => Some(Bridge {
                program: match helper {
                    Some(path) => path.to_path_buf(),
                    None => std::env::current_exe()
                        .map_err(|e| HostError::Io(format!("this binary: {e}")))?,
                },
                socket: run_dir
                    .canonicalize()
                    .map_err(|e| HostError::Io(format!("the host directory: {e}")))?
                    .join(format!("eg-{name}.sock")),
            }),
            Backend::SandboxExec(_) => None,
        };
        let port = egress
            .open_with_bridge(
                name,
                rules(&spec.allow),
                bridge.as_ref().map(|b| b.socket.as_path()),
            )
            .map_err(|e| HostError::Io(format!("the egress listener: {e}")))?;
        (bridge, Some(port))
    } else {
        (None, None)
    };
    let close_listener = || {
        if let Some(port) = port {
            egress.close_port(name, port);
        }
    };
    let home_path = match home() {
        Some(h) => h.canonicalize().unwrap_or(h),
        None => {
            close_listener();
            return Err(HostError::Io("HOME is not set".into()));
        }
    };
    let wrapped = claude
        .ok_or_else(|| HostError::Io("HOME is not set".into()))
        .and_then(|claude| {
            let target = Target {
                name,
                cwd,
                run_dir,
                claude: &claude,
                home: &home_path,
                kind,
                bwrap: matches!(backend, Backend::Bwrap(_)),
            };
            policy(spec, &target, port, bridge.as_ref())
        })
        .and_then(|policy| wrap(backend, &policy, argv).map_err(|e| refuse(&e.to_string())));
    let wrapped = match wrapped {
        Ok(w) => w,
        Err(e) => {
            close_listener();
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
            close_listener();
            return Err(e);
        }
    };
    let mut env: Vec<(String, String)> = Vec::new();
    if let Some(port) = port {
        let url = format!("http://127.0.0.1:{port}");
        env.extend(PROXY_ENV_NAMES.iter().map(|n| (n.to_string(), url.clone())));
    }
    env.push(("DISABLE_AUTOUPDATER".into(), "1".into()));
    if port.is_some() {
        // Node's fetch ignores HTTPS_PROXY unless told to read it; without this cursor and pi go direct, are denied, and report "fetch failed".
        env.push((NODE_PROXY_SWITCH.into(), "1".into()));
    }
    Ok(Confined {
        argv,
        env,
        egress_port: port,
    })
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
        std::fs::write(
            &file,
            r#"{"mcpServers":{"a":{"command":"x"}},"projects":{"/p":{"allowedTools":["t"]}}}"#,
        )
        .unwrap();
        trust_claude_project(home.path(), Path::new("/p")).unwrap();
        trust_claude_project(home.path(), Path::new("/q")).unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
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
        let mode = std::fs::metadata(home.path().join(".claude.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let broken = tempfile::tempdir().unwrap();
        std::fs::write(broken.path().join(".claude.json"), "{not json").unwrap();
        assert!(trust_claude_project(broken.path(), Path::new("/p")).is_err());
        assert_eq!(
            std::fs::read_to_string(broken.path().join(".claude.json")).unwrap(),
            "{not json"
        );
    }

    const SECRET: &str = "TOP-SECRET-VALUE";

    /// A home directory shaped like a real one, a host directory and a project,
    /// all under one temporary root. Not under the system temporary directory:
    /// bubblewrap gives agents a private copy of that, and the strict profile
    /// refuses a host directory there.
    struct Fixture {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        home: PathBuf,
        run_dir: PathBuf,
        project: PathBuf,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let home = root.join("home");
        let run_dir = root.join("run");
        let project = root.join("project");
        for dir in [
            home.join(".hq"),
            home.join(".ssh"),
            home.join(".aws"),
            home.join(".config/gh"),
            home.join(".config/git"),
            home.join(".claude"),
            home.join(".claude-work"),
            home.join(".codex"),
            home.join(".gemini"),
            run_dir.join("mcp"),
            run_dir.join("hooks"),
            project.join(".git/hooks"),
        ] {
            std::fs::create_dir_all(dir).unwrap();
        }
        for (path, text) in [
            (home.join(".hq/operator.token"), SECRET),
            (home.join(".ssh/id_test"), SECRET),
            (home.join(".config/gh/hosts.yml"), SECRET),
            (home.join(".claude-work/credentials.json"), SECRET),
            (home.join(".codex/auth.json"), SECRET),
            (home.join(".gemini/oauth.json"), SECRET),
            (home.join(".npmrc"), SECRET),
            (home.join("notes.txt"), SECRET),
            (home.join(".zshrc"), "# rc"),
            (home.join(".gitconfig"), "[user]"),
            (home.join(".config/git/config"), "[core]"),
            (home.join(".claude.json"), "{}"),
            (home.join(".claude/settings.json"), "{}"),
            (run_dir.join("operator.token"), SECRET),
            (run_dir.join("mcp/a.json"), "OWN"),
            (run_dir.join("mcp/b.json"), SECRET),
            (run_dir.join("hooks/a.json"), "{}"),
            (project.join(".git/config"), "[core]"),
            (project.join(".mcp.json"), "{}"),
        ] {
            std::fs::write(path, text).unwrap();
        }
        Fixture {
            _tmp: tmp,
            root,
            home,
            run_dir,
            project,
        }
    }

    impl Fixture {
        fn claude(&self) -> ClaudeConfig {
            ClaudeConfig {
                dir: self.home.join(".claude"),
                json_dir: self.home.clone(),
                is_default: true,
            }
        }

        fn target<'a>(&'a self, claude: &'a ClaudeConfig, bwrap: bool) -> Target<'a> {
            self.target_as(claude, bwrap, Some("claude"))
        }

        fn target_as<'a>(
            &'a self,
            claude: &'a ClaudeConfig,
            bwrap: bool,
            kind: Option<&'a str>,
        ) -> Target<'a> {
            Target {
                name: "a",
                cwd: &self.project,
                run_dir: &self.run_dir,
                claude,
                home: &self.home,
                kind,
                bwrap,
            }
        }
    }

    fn spec_with(profile: Profile) -> SandboxSpec {
        SandboxSpec {
            mode: Mode::Process,
            profile,
            ..SandboxSpec::none()
        }
    }

    fn hidden_dirs(s: &AgentSandbox) -> Vec<&Path> {
        s.hidden_dirs.iter().map(|h| h.dir.as_path()).collect()
    }

    #[test]
    fn a_request_that_names_no_profile_is_strict_and_hides_credentials() {
        let spec: SandboxSpec = serde_json::from_value(serde_json::json!({"mode": "process"}))
            .expect("a request from a client that predates profiles");
        assert_eq!(spec.profile, Profile::Strict);
        assert!(spec.hide_credentials);
        let spec: SandboxSpec = serde_json::from_value(serde_json::json!({
            "mode": "process", "profile": "open", "hide_credentials": false
        }))
        .unwrap();
        assert_eq!(spec.profile, Profile::Open);
        assert!(!spec.hide_credentials);
    }

    #[test]
    fn the_open_policy_walls_off_hq_and_the_credentials_and_limits_nothing_else() {
        let f = fixture();
        let claude = f.claude();
        for bwrap in [true, false] {
            let s = policy(
                &spec_with(Profile::Open),
                &f.target(&claude, bwrap),
                None,
                None,
            )
            .unwrap();
            assert!(s.open_filesystem);
            assert_eq!(s.network, Network::Full, "no proxy, no relay");
            assert!(s.read_restrictions.is_empty() && s.private_tmp.is_empty());
            assert!(s.writable.is_empty(), "nothing to allow when all is open");
            assert!(s.hide_other_processes, "their environments hold tokens");
            let hidden = hidden_dirs(&s);
            for dir in [
                ".hq",
                ".ssh",
                ".aws",
                ".config/gh",
                ".claude-work",
                ".codex",
                ".gemini",
            ] {
                assert!(hidden.contains(&f.home.join(dir).as_path()), "{dir} hidden");
            }
            assert!(hidden.contains(&f.run_dir.as_path()));
            assert!(!hidden.contains(&f.home.join(".config").as_path()));
            assert!(!hidden.contains(&f.home.as_path()));
            assert!(s.masked_files.contains(&f.home.join(".npmrc")));
            // The only files protected from writes are the ones the user's own tools
            // run later; nothing of the project is.
            for file in [
                ".zshrc",
                ".gitconfig",
                ".claude.json",
                ".claude/settings.json",
            ] {
                assert!(
                    s.readonly_files.contains(&f.home.join(file)),
                    "{file} read-only"
                );
            }
            assert!(s.readonly_subpaths.contains(&f.home.join(".config/git")));
            assert!(
                !s.readonly_files
                    .iter()
                    .chain(&s.readonly_subpaths)
                    .any(|p| p.starts_with(&f.project)),
                "the project's own .git/hooks, .git/config and .mcp.json stay writable"
            );
            // Seatbelt needs the hidden directories closed to writes and their parents
            // pinned; a bubblewrap mount needs neither.
            assert_eq!(s.pinned_dirs.is_empty(), bwrap, "{:?}", s.pinned_dirs);
            assert_eq!(s.readonly_subpaths.contains(&f.run_dir), !bwrap);
            if !bwrap {
                assert!(s.pinned_dirs.contains(&f.home.join(".config")));
                assert!(
                    !s.pinned_dirs.contains(&f.home),
                    "home itself is not pinned"
                );
            }
            hq_sandbox::bwrap_args(&s, &["true".to_string()]).expect("no relay is needed");
        }
    }

    #[test]
    fn the_strict_policy_is_the_one_it_always_was() {
        let f = fixture();
        let claude = f.claude();
        let s = policy(
            &spec_with(Profile::Strict),
            &f.target(&claude, false),
            Some(18080),
            None,
        )
        .unwrap();
        assert!(!s.open_filesystem && s.pinned_dirs.is_empty());
        assert!(matches!(s.network, Network::Proxy { port: 18080, .. }));
        assert!(s.read_restrictions.iter().any(|r| r.root == f.home));
        assert!(s.readonly_files.contains(&f.project.join(".git/config")));
        let err = policy(
            &spec_with(Profile::Strict),
            &f.target(&claude, false),
            None,
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("egress listener"), "{err}");
    }

    #[test]
    fn credential_hiding_can_be_turned_off_for_the_open_profile_only() {
        let f = fixture();
        let claude = f.claude();
        let mut open = spec_with(Profile::Open);
        open.hide_credentials = false;
        let s = policy(&open, &f.target(&claude, false), None, None).unwrap();
        let hidden = hidden_dirs(&s);
        for dir in [".ssh", ".aws", ".config/gh"] {
            assert!(!hidden.contains(&f.home.join(dir).as_path()), "{dir}");
        }
        assert!(s.masked_files.is_empty());
        // HQ's own directory and other accounts stay hidden whatever the setting says.
        for dir in [".hq", ".claude-work"] {
            assert!(hidden.contains(&f.home.join(dir).as_path()), "{dir}");
        }
        let mut strict = spec_with(Profile::Strict);
        strict.hide_credentials = false;
        let s = policy(&strict, &f.target(&claude, false), Some(18080), None).unwrap();
        assert!(hidden_dirs(&s).contains(&f.home.join(".ssh").as_path()));
        assert!(s.masked_files.contains(&f.home.join(".npmrc")));
    }

    #[test]
    fn a_credential_store_named_for_the_agent_is_read_only_under_the_open_profile() {
        let f = fixture();
        let claude = f.claude();
        let mut spec = spec_with(Profile::Open);
        spec.agent_readable.insert(
            "claude".into(),
            vec![f.home.join(".config/gh"), f.home.join(".hq")],
        );
        let s = policy(&spec, &f.target(&claude, false), None, None).unwrap();
        let hidden = hidden_dirs(&s);
        assert!(!hidden.contains(&f.home.join(".config/gh").as_path()));
        assert!(s.readonly_subpaths.contains(&f.home.join(".config/gh")));
        assert!(
            hidden.contains(&f.home.join(".hq").as_path()),
            "HQ's directory is never revealed"
        );
    }

    #[test]
    fn each_agent_sees_its_own_login_directory_and_no_other_agents() {
        let f = fixture();
        let claude = f.claude();
        let hidden_for = |kind: Option<&str>| -> Vec<PathBuf> {
            let s = policy(
                &spec_with(Profile::Open),
                &f.target_as(&claude, false, kind),
                None,
                None,
            )
            .unwrap();
            s.hidden_dirs.into_iter().map(|h| h.dir).collect()
        };
        let codex = f.home.join(".codex");
        let gemini = f.home.join(".gemini");
        let as_claude = hidden_for(Some("claude"));
        assert!(as_claude.contains(&codex) && as_claude.contains(&gemini));
        let as_codex = hidden_for(Some("codex"));
        assert!(!as_codex.contains(&codex) && as_codex.contains(&gemini));
        // Antigravity goes by two names and one directory.
        for name in ["agy", "antigravity"] {
            let as_agy = hidden_for(Some(agent_kind(name)));
            assert!(
                !as_agy.contains(&gemini) && as_agy.contains(&codex),
                "{name}"
            );
        }
        let nobody = hidden_for(None);
        assert!(nobody.contains(&codex) && nobody.contains(&gemini));
        // The strict profile reads nothing under HOME it does not list, so it has no list.
        let s = policy(
            &spec_with(Profile::Strict),
            &f.target_as(&claude, false, Some("claude")),
            Some(18080),
            None,
        )
        .unwrap();
        assert!(!hidden_dirs(&s).contains(&codex.as_path()));
    }

    #[test]
    fn a_launch_profile_loses_the_default_accounts_directory_and_trust_file() {
        let f = fixture();
        let own = ClaudeConfig {
            dir: f.home.join(".claude-work"),
            json_dir: f.home.join(".claude-work"),
            is_default: false,
        };
        let s = policy(
            &spec_with(Profile::Open),
            &f.target(&own, false),
            None,
            None,
        )
        .unwrap();
        let hidden = hidden_dirs(&s);
        assert!(hidden.contains(&f.home.join(".claude").as_path()));
        assert!(
            !hidden.contains(&f.home.join(".claude-work").as_path()),
            "its own directory stays"
        );
        assert!(
            s.masked_files.contains(&f.home.join(".claude.json")),
            "the default account's trust file is neither readable nor writable"
        );
        assert!(!s.readonly_files.contains(&f.home.join(".claude.json")));
        // The default account's own file is read-only to it, not masked.
        let default = f.claude();
        let s = policy(
            &spec_with(Profile::Open),
            &f.target(&default, false),
            None,
            None,
        )
        .unwrap();
        assert!(s.readonly_files.contains(&f.home.join(".claude.json")));
        assert!(!s.masked_files.contains(&f.home.join(".claude.json")));
    }

    #[test]
    fn the_open_profile_enforced_by_the_real_sandbox() {
        let Some(backend) = hq_sandbox::backend() else {
            return;
        };
        let f = fixture();
        let claude = f.claude();
        let bwrap = matches!(backend, Backend::Bwrap(_));
        let s = policy(
            &spec_with(Profile::Open),
            &f.target(&claude, bwrap),
            None,
            None,
        )
        .unwrap();
        let (home, run, project, root) = (
            f.home.display(),
            f.run_dir.display(),
            f.project.display(),
            f.root.display(),
        );
        let shared = std::env::temp_dir().join(format!("hq-host-open-{}", std::process::id()));
        let script = format!(
            "t() {{ if eval \"$2\" >/dev/null 2>&1; then echo \"$1=yes\"; else echo \"$1=no\"; fi; }}; \
             sees() {{ grep -q {SECRET} \"$1\"; }}; \
             t hq_token 'sees {home}/.hq/operator.token'; t host_token 'sees {run}/operator.token'; \
             t sibling_mcp 'sees {run}/mcp/b.json'; t own_mcp 'grep -q OWN {run}/mcp/a.json'; \
             t ssh_key 'sees {home}/.ssh/id_test'; t gh_token 'sees {home}/.config/gh/hosts.yml'; \
             t npmrc 'sees {home}/.npmrc'; t other_account 'sees {home}/.claude-work/credentials.json'; \
             t other_agent 'sees {home}/.codex/auth.json'; t other_agent_too 'sees {home}/.gemini/oauth.json'; \
             t read_home 'sees {home}/notes.txt'; \
             t write_home 'echo x > {home}/new.txt'; t write_outside 'echo x > {root}/outside.txt'; \
             t write_zshrc 'echo x >> {home}/.zshrc'; t write_gitconfig 'echo x >> {home}/.gitconfig'; \
             t write_git_dir 'echo x > {home}/.config/git/config'; \
             t write_settings 'echo x > {home}/.claude/settings.json'; t write_claude 'echo x > {home}/.claude/history'; \
             t write_hook 'echo x > {project}/.git/hooks/pre-commit'; t write_git_config 'echo x >> {project}/.git/config'; \
             t write_mcp 'echo x > {project}/.mcp.json'; \
             t write_hq 'echo x > {home}/.hq/new'; t write_run 'echo x > {run}/new'; \
             mv {home}/.config {home}/.config.moved 2>/dev/null; t moved_gh 'sees {home}/.config.moved/gh/hosts.yml'; \
             t shared_tmp 'echo x > {shared}'; echo DONE",
            shared = shared.display()
        );
        let wrapped = hq_sandbox::wrap(backend, &s, &["sh".into(), "-c".into(), script])
            .expect("the policy can be enforced here");
        let out = std::process::Command::new(&wrapped.program)
            .args(&wrapped.args)
            .stdin(std::process::Stdio::null())
            .output()
            .expect("the sandbox program starts");
        let out = String::from_utf8_lossy(&out.stdout).into_owned();
        let wrote_shared = shared.exists();
        let _ = std::fs::remove_file(&shared);
        assert!(out.contains("DONE"), "{out}");
        for (probe, allowed) in [
            ("hq_token", false),
            ("host_token", false),
            ("sibling_mcp", false),
            ("own_mcp", true),
            ("ssh_key", false),
            ("gh_token", false),
            ("npmrc", false),
            ("other_account", false),
            ("other_agent", false),
            ("other_agent_too", false),
            ("read_home", true),
            ("write_home", true),
            ("write_outside", true),
            ("write_zshrc", false),
            ("write_gitconfig", false),
            ("write_git_dir", false),
            ("write_settings", false),
            ("write_claude", true),
            ("write_hook", true),
            ("write_git_config", true),
            ("write_mcp", true),
            ("moved_gh", false),
            ("shared_tmp", true),
        ] {
            let want = format!("{probe}={}", if allowed { "yes" } else { "no" });
            assert!(out.contains(&want), "expected {want}; got:\n{out}");
        }
        assert!(wrote_shared, "the temporary directory is the machine's own");
        // Seatbelt refuses a write into a hidden directory; bubblewrap lets it land
        // in the empty mount that hides it. Either way the real one is untouched.
        assert!(!f.home.join(".hq/new").exists(), "a write reached ~/.hq");
        assert!(
            !f.run_dir.join("new").exists(),
            "a write reached the host dir"
        );
        assert_eq!(
            std::fs::read_to_string(f.home.join(".zshrc")).unwrap(),
            "# rc"
        );
    }
}
