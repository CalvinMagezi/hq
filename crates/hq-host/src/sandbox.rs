//! Turns a sandbox request into the wrapped command line of one agent: the
//! egress listener it may connect to, the paths it may touch, and the proxy
//! variables that make its HTTP clients use the listener.

use crate::egress::{Egress, Rule};
use crate::error::HostError;
use crate::server;
use hq_sandbox::{AgentSandbox, HiddenDir, Network, backend, canonical, wrap};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Directories under HOME that hold credentials and have no place in an agent.
const SECRET_HOME_DIRS: &[&str] = &[".ssh", ".gnupg", ".aws", ".kube", ".hq"];
/// What Claude Code needs to write under HOME, besides the project.
const HOME_WRITABLE: &[&str] = &[".claude", ".cache", "Library/Caches"];
const HOME_WRITABLE_FILES: &[&str] = &[".claude.json"];
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
}

impl SandboxSpec {
    pub fn none() -> Self {
        Self {
            mode: Mode::None,
            allow: Vec::new(),
            writable: Vec::new(),
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
    let own_files = [
        run_dir.join("hooks").join(format!("{name}.json")),
        run_dir.join("mcp").join(format!("{name}.json")),
    ];
    let socket = server::socket_path(&run_dir);
    let mut allow = canonical(own_files);
    allow.push(socket.clone());
    let mut hidden = vec![HiddenDir { dir: run_dir, allow }];
    hidden.extend(
        canonical(under(SECRET_HOME_DIRS))
            .into_iter()
            .map(|dir| HiddenDir { dir, allow: Vec::new() }),
    );
    Ok(AgentSandbox {
        project: cwd.canonicalize().map_err(|e| HostError::Io(e.to_string()))?,
        writable,
        writable_files: under(HOME_WRITABLE_FILES),
        readonly_files: Vec::new(),
        masked_files: Vec::new(),
        hidden_dirs: hidden,
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
) -> Result<Confined, HostError> {
    let refuse = |why: &str| HostError::Sandbox(why.to_string());
    let run_dir = run_dir.ok_or_else(|| refuse("the host has no run directory to protect"))?;
    let backend = backend().ok_or_else(|| refuse("no sandbox program (sandbox-exec or bwrap) on this machine"))?;
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
