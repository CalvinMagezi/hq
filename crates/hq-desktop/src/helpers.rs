//! The coding and general helpers on this computer: finding them, checking their version in a
//! sandbox, trying one in the Try it folder, signing one in, and connecting one to HQ.
//!
//! Listing reads state and runs nothing. A check runs a helper only after the person asked for
//! it, never unsandboxed, and never a program that anyone on this computer could have swapped.
//! The table below mirrors `hq-tools` `harness_session::spec`; this crate does not depend on it.

use crate::host;
use anyhow::{Result, bail};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::Semaphore;

/// How many checks run at once.
pub const CHECKS_AT_ONCE: usize = 2;
/// How long one sandboxed version run may take.
pub const CHECK_LIMIT: Duration = Duration::from_secs(2);
/// How long Undo stays offered after a disconnect.
pub const UNDO_WINDOW: Duration = Duration::from_secs(600);
const CONNECT_LIMIT: Duration = Duration::from_secs(20);
const TRY_PREFIX: &str = "hub-try-";
const API_HOST: &str = "api.anthropic.com";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    Coding,
    General,
    App,
}

/// One helper HQ knows how to find.
pub struct Spec {
    pub id: &'static str,
    pub name: &'static str,
    pub program: &'static str,
    pub group: Group,
    /// What the host starts it with, after the program.
    pub args: &'static [&'static str],
    /// Sites its sign-in link may open: each host and its subdomains.
    pub sites: &'static [&'static str],
    /// The `hq mcp` target that connects it, when there is one.
    pub target: Option<&'static str>,
    /// The npm package whose `package.json` names its version.
    pub package: Option<&'static str>,
    /// The line its project publishes, shown to copy. HQ Hub never runs it.
    pub install: Option<&'static str>,
    /// Its own trust prompt is answered by the host, so it is not "needs you".
    pub trusts: bool,
}

const fn spec(
    id: &'static str,
    name: &'static str,
    program: &'static str,
    group: Group,
    args: &'static [&'static str],
) -> Spec {
    Spec {
        id,
        name,
        program,
        group,
        args,
        sites: &[],
        target: None,
        package: None,
        install: None,
        trusts: false,
    }
}

pub static SPECS: &[Spec] = &[
    Spec {
        sites: &["claude.ai", "console.anthropic.com", "platform.claude.com"],
        target: Some("claude-code"),
        package: Some("@anthropic-ai/claude-code"),
        install: Some("npm install -g @anthropic-ai/claude-code"),
        ..spec(
            "claude-code",
            "Claude Code",
            "claude",
            Group::Coding,
            &["--dangerously-skip-permissions"],
        )
    },
    Spec {
        sites: &["auth.openai.com", "chatgpt.com"],
        package: Some("@openai/codex"),
        install: Some("npm install -g @openai/codex"),
        ..spec(
            "codex",
            "Codex",
            "codex",
            Group::Coding,
            &["--no-daemon", "-c", "check_for_update_on_startup=false"],
        )
    },
    Spec {
        sites: &["cursor.com", "cursor.sh"],
        target: Some("cursor"),
        install: Some("curl https://cursor.com/install -fsS | bash"),
        ..spec(
            "cursor",
            "Cursor",
            "cursor-agent",
            Group::Coding,
            &["--force"],
        )
    },
    Spec {
        sites: &["github.com"],
        target: Some("copilot"),
        package: Some("@github/copilot"),
        install: Some("npm install -g @github/copilot"),
        ..spec(
            "github-copilot",
            "GitHub Copilot",
            "copilot",
            Group::Coding,
            &[
                "--allow-all-tools",
                "--deny-tool=shell(git push)",
                "--deny-tool=shell(git stash)",
                "--deny-tool=shell(git reset)",
                "--deny-tool=shell(git clean)",
                "--deny-tool=shell(rm)",
                "--deny-tool=shell(env)",
                "--deny-tool=shell(printenv)",
            ],
        )
    },
    Spec {
        sites: &["accounts.google.com"],
        target: Some("antigravity"),
        trusts: true,
        ..spec(
            "antigravity",
            "Antigravity",
            "agy",
            Group::Coding,
            &["--dangerously-skip-permissions"],
        )
    },
    Spec {
        sites: &["opencode.ai"],
        target: Some("opencode"),
        install: Some("npm install -g opencode-ai"),
        ..spec("opencode", "OpenCode", "opencode", Group::Coding, &[])
    },
    Spec {
        sites: &["chat.qwen.ai", "qwen.ai"],
        package: Some("@qwen-code/qwen-code"),
        install: Some("npm install -g @qwen-code/qwen-code"),
        ..spec(
            "qwen",
            "Qwen Code",
            "qwen",
            Group::Coding,
            &["--auth-type", "qwen-oauth"],
        )
    },
    Spec {
        sites: &["kimi.com", "moonshot.cn", "moonshot.ai"],
        install: Some("uv tool install kimi-cli"),
        ..spec("kimi", "Kimi", "kimi", Group::Coding, &[])
    },
    spec("pi", "Pi", "pi", Group::Coding, &[]),
    spec("hermes", "Hermes", "hermes", Group::General, &["chat"]),
    Spec {
        install: Some("npm install -g openclaw"),
        ..spec(
            "openclaw",
            "OpenClaw",
            "openclaw",
            Group::General,
            &["tui", "--local"],
        )
    },
    Spec {
        target: Some("claude-desktop"),
        ..spec("claude-desktop", "Claude Desktop", "", Group::App, &[])
    },
];

pub fn find_spec(id: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.id == id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Found, and nobody has asked for a check yet.
    Unchecked,
    Checking,
    NotInstalled,
    ProgramNotFound,
    CheckFailed,
    NeedsSignin,
    NeedsYou,
    NotConnected,
    UpdateAvailable,
    Ready,
    Trying,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Version {
    Known(String),
    /// Nothing could be run safely here, so the version is not known.
    Unknown,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct HelperView {
    pub id: &'static str,
    pub name: &'static str,
    pub group: Group,
    pub path: Option<PathBuf>,
    pub state: State,
    pub version: Option<String>,
    /// The version came from a package file rather than a sandboxed run.
    pub version_from_package: bool,
    pub latest: Option<String>,
    /// Found in a place the app had not seen before.
    pub new_here: bool,
    pub can_connect: bool,
    pub connected: bool,
    pub can_sign_in: bool,
    pub install: Option<&'static str>,
}

type Stamp = (u64, Option<SystemTime>);
type Runner = Arc<dyn Fn(&Path) -> Version + Send + Sync>;

/// Checks programs two at a time and remembers each answer by path, size and modified time.
pub struct Checker {
    cache: Mutex<HashMap<PathBuf, (Stamp, Version)>>,
    pending: Mutex<Vec<PathBuf>>,
    gate: Semaphore,
    runner: Runner,
    pub spawns: AtomicUsize,
    active: AtomicUsize,
    pub most_at_once: AtomicUsize,
}

fn stamp_of(path: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.len(), m.modified().ok()))
}

impl Checker {
    pub fn new() -> Self {
        Self::with_runner(Arc::new(|p| version_of(p, CHECK_LIMIT, true)))
    }

    pub fn with_runner(runner: Runner) -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
            pending: Mutex::new(Vec::new()),
            gate: Semaphore::new(CHECKS_AT_ONCE),
            runner,
            spawns: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            most_at_once: AtomicUsize::new(0),
        }
    }

    pub fn cached(&self, path: &Path) -> Option<Version> {
        let stamp = stamp_of(path)?;
        let cache = self.cache.lock().unwrap();
        cache
            .get(path)
            .filter(|(s, _)| *s == stamp)
            .map(|(_, v)| v.clone())
    }

    pub fn is_pending(&self, path: &Path) -> bool {
        self.pending.lock().unwrap().iter().any(|p| p == path)
    }

    /// "Check all again" starts from nothing.
    pub fn forget_all(&self) {
        self.cache.lock().unwrap().clear();
    }

    pub async fn check(self: &Arc<Self>, path: PathBuf) -> Version {
        if let Some(v) = self.cached(&path) {
            return v;
        }
        self.pending.lock().unwrap().push(path.clone());
        let _permit = self.gate.acquire().await.expect("gate never closes");
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.most_at_once.fetch_max(now, Ordering::SeqCst);
        let me = self.clone();
        let p = path.clone();
        let version = tokio::task::spawn_blocking(move || {
            // Looked at before and after, so a file swapped mid-check is not cached as the old one.
            let before = stamp_of(&p);
            me.spawns.fetch_add(1, Ordering::SeqCst);
            let v = (me.runner)(&p);
            (before.filter(|b| stamp_of(&p) == Some(*b)), v)
        })
        .await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        self.pending.lock().unwrap().retain(|x| x != &path);
        match version {
            Ok((Some(stamp), v)) => {
                self.cache.lock().unwrap().insert(path, (stamp, v.clone()));
                v
            }
            Ok((None, v)) => v,
            Err(_) => Version::Failed,
        }
    }
}

impl Default for Checker {
    fn default() -> Self {
        Self::new()
    }
}

/// The first dotted number in a line, without a leading `v`.
pub fn parse_version(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        line.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',')
            .map(|w| w.trim_start_matches('v'))
            .find(|w| {
                w.contains('.')
                    && w.chars().next().is_some_and(|c| c.is_ascii_digit())
                    && w.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
            })
            .map(str::to_string)
    })
}

/// The version a package folder next to the program names, with no run at all.
fn version_from_package(program: &Path) -> Option<String> {
    let real = std::fs::canonicalize(program).ok()?;
    real.ancestors().skip(1).take(6).find_map(|dir| {
        let text = std::fs::read_to_string(dir.join("package.json")).ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        let name = v.get("name")?.as_str()?;
        SPECS
            .iter()
            .filter_map(|s| s.package)
            .any(|p| p == name)
            .then(|| v.get("version")?.as_str().map(str::to_string))
            .flatten()
    })
}

/// The command that runs `program --version` with no network, or none where no sandbox exists.
fn sandboxed(program: &Path) -> Option<std::process::Command> {
    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("/usr/bin/sandbox-exec");
        cmd.args(["-p", "(version 1)(allow default)(deny network*)"])
            .arg(program);
        Some(cmd)
    }
    #[cfg(target_os = "linux")]
    {
        let bwrap = ["/usr/bin/bwrap", "/bin/bwrap"]
            .into_iter()
            .find(|p| Path::new(p).exists())?;
        let mut cmd = std::process::Command::new(bwrap);
        cmd.args([
            "--ro-bind",
            "/",
            "/",
            "--dev",
            "/dev",
            "--unshare-net",
            "--",
        ])
        .arg(program);
        Some(cmd)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = program;
        None
    }
}

/// The version of `program`: from its package folder, else one sandboxed run. `sandbox` is false
/// only in tests, which run harmless scripts of their own.
pub fn version_of(program: &Path, limit: Duration, sandbox: bool) -> Version {
    if let Some(v) = version_from_package(program) {
        return Version::Known(v);
    }
    let cmd = if sandbox {
        match sandboxed(program) {
            Some(mut c) => {
                c.arg("--version");
                c
            }
            None => return Version::Unknown,
        }
    } else {
        let mut c = std::process::Command::new(program);
        c.arg("--version");
        c
    };
    match host::command_output_within(cmd, limit) {
        Some(out) => match parse_version(&String::from_utf8_lossy(&out)) {
            Some(v) => Version::Known(v),
            None => Version::Failed,
        },
        None => Version::Failed,
    }
}

/// Where helpers are looked for, in order.
pub fn search_dirs(home: Option<&Path>, path_var: Option<&std::ffi::OsStr>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(h) = home {
        for rel in [
            ".local/bin",
            ".bun/bin",
            ".npm-global/bin",
            ".cargo/bin",
            ".claude/local",
            "bin",
        ] {
            dirs.push(h.join(rel));
        }
    }
    dirs.extend(
        ["/opt/homebrew/bin", "/usr/local/bin"]
            .iter()
            .map(PathBuf::from),
    );
    if let Some(var) = path_var {
        dirs.extend(std::env::split_paths(var));
    }
    dirs
}

/// What looking for a program found.
#[derive(Debug, PartialEq, Eq)]
pub enum Found {
    At(PathBuf),
    Missing,
    /// Found where anyone on this computer could swap it, so it is not run.
    Unsafe(PathBuf),
}

pub fn locate(program: &str, dirs: &[PathBuf]) -> Found {
    use std::os::unix::fs::PermissionsExt;
    let hit = dirs.iter().map(|d| d.join(program)).find(|p| {
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    });
    match hit {
        None => Found::Missing,
        Some(p) if crate::chooser::world_writable(&p) => Found::Unsafe(p),
        Some(p) => Found::At(p),
    }
}

/// Where each target keeps HQ's entry, and the key it sits under.
pub fn target_files(target: &str, home: &Path) -> Vec<PathBuf> {
    match target {
        "claude-code" => vec![
            home.join(".claude.json"),
            home.join(".claude").join("settings.json"),
        ],
        "claude-desktop" => hq_bridge::claude::desktop_config_path()
            .into_iter()
            .collect(),
        "cursor" => vec![home.join(".cursor/mcp.json")],
        "copilot" => vec![home.join(".copilot/mcp.json")],
        "opencode" => vec![home.join(".config/opencode/opencode.json")],
        "antigravity" => vec![home.join(".gemini/antigravity-cli/mcp/agent-hq.json")],
        _ => Vec::new(),
    }
}

pub fn is_connected(target: &str, home: &Path) -> bool {
    target_files(target, home).iter().any(|f| {
        if target == "antigravity" {
            return f.exists();
        }
        std::fs::read_to_string(f).is_ok_and(|t| t.contains("\"agent-hq\""))
    })
}

type Snapshot = Vec<(PathBuf, Option<Vec<u8>>)>;

fn snapshot(files: &[PathBuf]) -> Snapshot {
    files
        .iter()
        .map(|f| (f.clone(), std::fs::read(f).ok()))
        .collect()
}

struct Undo {
    id: String,
    at: Instant,
    before: Snapshot,
    after: Snapshot,
}

/// The pages and files the helper commands share.
pub struct Helpers {
    pub checker: Arc<Checker>,
    undo: Mutex<Option<Undo>>,
    link: Mutex<Option<String>>,
    seen: Mutex<Vec<String>>,
}

impl Helpers {
    pub fn new() -> Self {
        Self {
            checker: Arc::new(Checker::new()),
            undo: Mutex::new(None),
            link: Mutex::new(None),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub fn with_checker(checker: Arc<Checker>) -> Self {
        Self {
            checker,
            ..Self::new()
        }
    }

    /// Every helper and where it stands, from what is already known. Nothing is run.
    pub fn list(
        &self,
        dirs: &[PathBuf],
        home: &Path,
        latest: &HashMap<&'static str, String>,
        trying: Option<&str>,
        seen_file: &Path,
    ) -> Vec<HelperView> {
        let seen = read_seen(seen_file);
        SPECS
            .iter()
            .map(|s| {
                let target_ok = s.target.is_some();
                let connected = s.target.is_some_and(|t| is_connected(t, home));
                let mut v = HelperView {
                    id: s.id,
                    name: s.name,
                    group: s.group,
                    path: None,
                    state: State::NotInstalled,
                    version: None,
                    version_from_package: false,
                    latest: None,
                    new_here: false,
                    can_connect: target_ok,
                    connected,
                    can_sign_in: !s.sites.is_empty(),
                    install: s.install,
                };
                if s.group == Group::App {
                    let installed = target_files(s.target.unwrap_or(""), home)
                        .first()
                        .and_then(|f| f.parent())
                        .is_some_and(Path::exists);
                    v.state = match (installed, connected) {
                        (false, _) => State::NotInstalled,
                        (true, true) => State::Ready,
                        (true, false) => State::NotConnected,
                    };
                    return v;
                }
                match locate(s.program, dirs) {
                    Found::Missing => {}
                    Found::Unsafe(p) => {
                        v.path = Some(p);
                        v.state = State::ProgramNotFound;
                    }
                    Found::At(p) => {
                        v.new_here =
                            !seen.is_empty() && !seen.iter().any(|x| x == &p.to_string_lossy());
                        v.state = if trying == Some(s.id) {
                            State::Trying
                        } else if self.checker.is_pending(&p) {
                            State::Checking
                        } else {
                            match self.checker.cached(&p) {
                                None => State::Unchecked,
                                Some(Version::Failed) => State::CheckFailed,
                                Some(ver) => {
                                    if let Version::Known(k) = ver {
                                        v.version_from_package = version_from_package(&p).is_some();
                                        v.version = Some(k);
                                    }
                                    let newer = latest.get(s.id).filter(|l| {
                                        v.version.as_deref().is_some_and(|c| is_newer(l, c))
                                    });
                                    if let Some(l) = newer {
                                        v.latest = Some(l.clone());
                                        State::UpdateAvailable
                                    } else if target_ok && !connected {
                                        State::NotConnected
                                    } else {
                                        State::Ready
                                    }
                                }
                            }
                        };
                        v.path = Some(p);
                    }
                }
                v
            })
            .collect()
    }

    /// Remembers where helpers were found, so the next list can say which are new here.
    pub fn remember(&self, views: &[HelperView], seen_file: &Path) {
        let mut seen = self.seen.lock().unwrap();
        *seen = read_seen(seen_file);
        let mut changed = false;
        for p in views.iter().filter_map(|v| v.path.as_ref()) {
            let s = p.to_string_lossy().to_string();
            if !seen.contains(&s) {
                seen.push(s);
                changed = true;
            }
        }
        if changed {
            let _ = std::fs::write(seen_file, seen.join("\n"));
        }
    }

    pub fn set_link(&self, link: Option<String>) {
        *self.link.lock().unwrap() = link;
    }

    pub fn link(&self) -> Option<String> {
        self.link.lock().unwrap().clone()
    }

    /// Connects `id` to HQ with `hq mcp install`, or disconnects it with `hq mcp remove`. A
    /// disconnect keeps what the files held so Undo can put them back.
    pub fn connect(&self, id: &str, hq: &Path, home: &Path, on: bool) -> Result<()> {
        let Some(target) = find_spec(id).and_then(|s| s.target) else {
            bail!("this helper cannot be connected to HQ yet");
        };
        let files = target_files(target, home);
        let before = snapshot(&files);
        let verb = if on { "install" } else { "remove" };
        let args = ["mcp", verb, "--target", target];
        if host::run_within(hq, &args, CONNECT_LIMIT).is_none() {
            bail!("HQ could not change that helper's settings");
        }
        if !on {
            *self.undo.lock().unwrap() = Some(Undo {
                id: id.to_string(),
                at: Instant::now(),
                before,
                after: snapshot(&files),
            });
        }
        Ok(())
    }

    /// Puts a disconnected helper's files back, while the window is open and nothing changed them.
    pub fn restore(&self, id: &str) -> Result<()> {
        let taken = self.undo.lock().unwrap().take();
        let Some(u) = taken.filter(|u| u.id == id && u.at.elapsed() < UNDO_WINDOW) else {
            bail!(RESTORE_FAILED);
        };
        if snapshot(&u.after.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>()) != u.after {
            bail!(RESTORE_FAILED);
        }
        for (path, bytes) in &u.before {
            match bytes {
                Some(b) => std::fs::write(path, b)?,
                None => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn age_undo(&self, by: Duration) {
        if let Some(u) = self.undo.lock().unwrap().as_mut() {
            u.at -= by;
        }
    }
}

impl Default for Helpers {
    fn default() -> Self {
        Self::new()
    }
}

pub const RESTORE_FAILED: &str =
    "Couldn't undo. It changed on another device, or too much time passed.";

fn read_seen(file: &Path) -> Vec<String> {
    std::fs::read_to_string(file)
        .map(|t| t.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// Whether `latest` is a higher dotted number than `current`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let n = |s: &str| -> Vec<u64> { s.split(['.', '-']).map_while(|p| p.parse().ok()).collect() };
    n(latest) > n(current)
}

/// The first https link on the helper's screen, for sign-in. It is held in memory only.
pub fn first_https_link(screen: &str) -> Option<String> {
    let start = screen.find("https://")?;
    // A link wrapped over screen rows is rejoined only across line ends with no spaces.
    let rest = &screen[start..];
    let mut link = String::new();
    for line in rest.lines() {
        let word = line.trim();
        let take = word.split_whitespace().next().unwrap_or("");
        if take.is_empty() || (!link.is_empty() && take.len() != word.len()) {
            break;
        }
        link.push_str(take);
        if take.len() != word.len() {
            break;
        }
    }
    let link = link.trim_end_matches(['.', ',', ')']).to_string();
    (link.len() > "https://".len()).then_some(link)
}

/// What the screen of a trying helper says, in terms of what the person must do.
pub fn judge_screen(state: Option<&str>, exited: bool, spec: &Spec) -> State {
    if exited {
        return State::CheckFailed;
    }
    match state {
        Some("blocked") if spec.trusts => State::Trying,
        Some("blocked") => State::NeedsYou,
        _ => State::Trying,
    }
}

fn kind_of(spec: &Spec) -> &'static str {
    match spec.id {
        "claude-code" => "claude",
        "cursor" => "cursor",
        _ => "other",
    }
}

/// The sandbox request a try sends with its launch: the helper's folder, and its own sign-in site.
pub fn sandbox_plan(spec: &Spec, try_dir: &Path, cfg: &hq_core::config::AgentHostConfig) -> Value {
    use hq_core::config::{SandboxMode, SandboxProfile};
    if cfg.sandbox.mode == SandboxMode::None {
        return json!({ "mode": "none" });
    }
    let mut allow = vec![json!({ "host": API_HOST, "ports": [443] })];
    allow.extend(
        spec.sites
            .iter()
            .map(|h| json!({ "host": h, "ports": [443] })),
    );
    let mut writable = cfg.sandbox.writable.clone();
    writable.push(try_dir.to_string_lossy().into_owned());
    json!({
        "mode": "process",
        "profile": match cfg.sandbox.profile {
            SandboxProfile::Open => "open",
            SandboxProfile::Strict => "strict",
        },
        "hide_credentials": cfg.sandbox.hide_credentials,
        "allow": allow,
        "writable": writable,
        "readable": cfg.sandbox.readable,
        "agent_readable": cfg.sandbox.agent_readable,
        "loopback_ports": [],
    })
}

/// A folder it is safe to empty: absolute, not a root, not a home and not the home's top.
pub fn safe_try_dir(dir: &Path, home: &Path) -> bool {
    dir.is_absolute() && dir.components().count() >= 3 && dir != home && !home.starts_with(dir)
}

/// The file that marks a Try it folder as HQ Hub's own. Only a folder holding it is ever emptied.
pub const TRY_MARKER: &str = ".hq-try-it";

/// Makes `dir` ready for a try and HQ Hub's own: a new folder, or an empty one, gets the marker; a
/// folder that holds files HQ Hub did not put there, or that is a link, is refused, so a try can
/// never empty a folder of the person's.
pub fn prepare_try_dir(dir: &Path) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    match std::fs::symlink_metadata(dir) {
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "the Try it folder is a link or a file",
            ));
        }
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => std::fs::create_dir_all(dir)?,
        Err(e) => return Err(e),
    }
    let marker = dir.join(TRY_MARKER);
    if marker.is_file() {
        return Ok(());
    }
    if std::fs::read_dir(dir)?.next().is_some() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "the Try it folder already holds files HQ Hub did not put there",
        ));
    }
    std::fs::write(
        marker,
        b"HQ Hub's Try it folder. HQ Hub empties it before and after each try.\n",
    )
}

/// Whether `dir` is HQ Hub's own Try it folder: not a link, and holding the marker.
pub fn owns_try_dir(dir: &Path) -> bool {
    std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        && dir.join(TRY_MARKER).is_file()
}

/// Removes everything inside HQ Hub's own `dir` except its marker, and leaves the folder. Links are
/// removed, never followed. A folder without the marker is refused.
pub fn empty_dir(dir: &Path) -> std::io::Result<()> {
    if !owns_try_dir(dir) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "not HQ Hub's Try it folder",
        ));
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.file_name().is_some_and(|n| n == TRY_MARKER) {
            continue;
        }
        if std::fs::symlink_metadata(&path)?.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// Whether a try may start at all. A general helper is asked one question that uses a little of
/// the person's plan, so it waits for a yes.
pub fn may_start(spec: &Spec, confirmed: bool, locked: bool) -> Result<(), &'static str> {
    if spec.group == Group::App {
        return Err("An app is opened by you, so there is nothing to try.");
    }
    if locked {
        return Err("Try it waits until helpers are allowed again on This computer.");
    }
    if spec.group == Group::General && !confirmed {
        return Err("confirm");
    }
    Ok(())
}

/// The result of a try or a sign-in start.
#[derive(Debug, Clone, Serialize)]
pub struct TryOutcome {
    pub state: State,
    /// The last lines of what the helper showed, with keys hidden.
    pub shown: String,
    /// A sign-in link is held, ready to copy or open. The link itself is never sent to the page.
    pub link_ready: bool,
}

/// Starts `spec` in `dir` through the host on `host_dir` and waits for its first answer.
pub fn try_through_host(
    host_dir: &Path,
    spec: &Spec,
    program: &Path,
    dir: &Path,
    sandbox: Value,
    wait: Duration,
) -> Result<(String, TryOutcome, Option<String>)> {
    let name = format!("{TRY_PREFIX}{}", spec.id);
    let mut client = hq_host::Client::connect(host_dir)
        .map_err(|_| anyhow::anyhow!("Coding helpers are not running on this computer."))?;
    client.set_timeout(Some(Duration::from_secs(10)));
    let mut argv = vec![program.to_string_lossy().into_owned()];
    argv.extend(spec.args.iter().map(|s| s.to_string()));
    client
        .call(
            "agent.spawn",
            json!({ "name": name, "argv": argv, "cwd": dir, "agent": kind_of(spec),
                    "sandbox": sandbox, "rows": 40, "cols": 120 }),
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let until = Instant::now() + wait;
    let mut state = State::Trying;
    let mut link = None;
    let mut shown = String::new();
    while Instant::now() < until {
        std::thread::sleep(Duration::from_millis(400));
        let info = client.call("agent.get", json!({ "name": name }));
        let Ok(info) = info else { break };
        let exited = info.get("status").and_then(Value::as_str) == Some("exited");
        if let Ok(r) = client.call("agent.read", json!({ "name": name, "source": "visible" })) {
            shown = r
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            link = link.or_else(|| first_https_link(&shown));
        }
        state = judge_screen(info.get("state").and_then(Value::as_str), exited, spec);
        let settled = exited
            || state == State::NeedsYou
            || (info.get("state").and_then(Value::as_str) == Some("idle")
                && !shown.trim().is_empty());
        if settled {
            break;
        }
    }
    let tail: Vec<&str> = shown.lines().rev().take(12).collect();
    let tail: Vec<&str> = tail.into_iter().rev().collect();
    let shown = host::hide_secrets(&tail.join("\n"));
    let outcome = TryOutcome {
        state,
        shown,
        link_ready: link.is_some(),
    };
    Ok((name, outcome, link))
}

/// Stops a try and removes its pane.
pub fn stop_through_host(host_dir: &Path, name: &str) {
    if let Ok(mut c) = hq_host::Client::connect(host_dir) {
        c.set_timeout(Some(Duration::from_secs(5)));
        let _ = c.call("agent.kill", json!({ "name": name }));
        let _ = c.call("agent.remove", json!({ "name": name }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    // A script just written can fail to start while another test's fork holds its file open.
    static EXEC: Mutex<()> = Mutex::new(());

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn counting(delay: Duration) -> Arc<Checker> {
        Arc::new(Checker::with_runner(Arc::new(move |_| {
            std::thread::sleep(delay);
            Version::Known("1.0.0".into())
        })))
    }

    #[test]
    fn the_table_has_nine_coding_two_general_and_one_app() {
        let n = |g| SPECS.iter().filter(|s| s.group == g).count();
        assert_eq!(
            (n(Group::Coding), n(Group::General), n(Group::App)),
            (9, 2, 1)
        );
    }

    #[test]
    fn a_package_folder_gives_the_version_with_no_run() {
        let _exec = EXEC.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let pkg = dir
            .path()
            .join("lib/node_modules/@anthropic-ai/claude-code");
        std::fs::create_dir_all(pkg.join("bin")).unwrap();
        std::fs::write(
            pkg.join("package.json"),
            r#"{"name":"@anthropic-ai/claude-code","version":"2.3.4"}"#,
        )
        .unwrap();
        // A run would leave this file behind.
        let bin = script(&pkg.join("bin"), "claude", "touch ran");
        assert_eq!(
            version_of(&bin, CHECK_LIMIT, false),
            Version::Known("2.3.4".into())
        );
        assert!(!pkg.join("bin/ran").exists());
    }

    #[test]
    fn a_program_without_a_package_is_run_once_and_a_hang_is_killed() {
        let _exec = EXEC.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let ok = script(dir.path(), "ok", "echo 'tool v3.1.0 (build 9)'");
        assert_eq!(
            version_of(&ok, Duration::from_secs(20), false),
            Version::Known("3.1.0".into())
        );
        let hang = script(dir.path(), "hang", "sleep 30");
        let t = Instant::now();
        assert_eq!(
            version_of(&hang, Duration::from_millis(300), false),
            Version::Failed
        );
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn a_second_check_runs_nothing_until_all_are_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let p = script(dir.path(), "x", "true");
        let c = counting(Duration::ZERO);
        c.check(p.clone()).await;
        c.check(p.clone()).await;
        assert_eq!(c.spawns.load(Ordering::SeqCst), 1);
        c.forget_all();
        c.check(p.clone()).await;
        assert_eq!(c.spawns.load(Ordering::SeqCst), 2);
        std::fs::write(&p, "#!/bin/sh\ntrue # changed\n").unwrap();
        c.check(p).await;
        assert_eq!(c.spawns.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn at_most_two_checks_run_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let c = counting(Duration::from_millis(80));
        let mut all = Vec::new();
        for i in 0..6 {
            let p = script(dir.path(), &format!("p{i}"), "true");
            let c = c.clone();
            all.push(tokio::spawn(async move { c.check(p).await }));
        }
        for t in all {
            t.await.unwrap();
        }
        assert_eq!(c.spawns.load(Ordering::SeqCst), 6);
        assert!(c.most_at_once.load(Ordering::SeqCst) <= CHECKS_AT_ONCE);
    }

    #[test]
    fn a_program_in_a_shared_folder_is_not_found() {
        let dir = crate::chooser::private_test_dir();
        let safe = dir.path().join("safe");
        std::fs::create_dir(&safe).unwrap();
        std::fs::set_permissions(&safe, std::fs::Permissions::from_mode(0o700)).unwrap();
        script(&safe, "claude", "true");
        let open = dir.path().join("open");
        std::fs::create_dir(&open).unwrap();
        script(&open, "codex", "true");
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
        let dirs = [safe.clone(), open.clone()];
        assert_eq!(locate("claude", &dirs), Found::At(safe.join("claude")));
        assert_eq!(locate("codex", &dirs), Found::Unsafe(open.join("codex")));
        assert_eq!(locate("kimi", &dirs), Found::Missing);
    }

    #[test]
    fn listing_runs_nothing_and_says_unchecked() {
        let dir = crate::chooser::private_test_dir();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        script(&bin, "claude", "touch ran");
        let h = Helpers::with_checker(counting(Duration::ZERO));
        let views = h.list(
            std::slice::from_ref(&bin),
            dir.path(),
            &HashMap::new(),
            None,
            &dir.path().join("seen"),
        );
        assert!(!bin.join("ran").exists());
        let me = views.iter().find(|v| v.id == "claude-code").unwrap();
        assert_eq!(me.state, State::Unchecked);
        assert_eq!(
            views.iter().find(|v| v.id == "kimi").unwrap().state,
            State::NotInstalled
        );
    }

    #[test]
    fn only_the_helpers_own_sites_are_trusted() {
        assert!(find_spec("pi").unwrap().sites.is_empty());
        assert_eq!(
            first_https_link("Open this:\nhttps://claude.ai/oauth?code=1\nthen paste"),
            Some("https://claude.ai/oauth?code=1".into())
        );
        assert_eq!(first_https_link("no link here"), None);
    }

    #[test]
    fn a_folder_of_the_persons_is_never_taken_or_emptied() {
        let docs = tempfile::tempdir().unwrap();
        std::fs::write(docs.path().join("thesis.md"), "years of work").unwrap();
        assert!(
            prepare_try_dir(docs.path()).is_err(),
            "a folder with files is refused"
        );
        assert!(empty_dir(docs.path()).is_err(), "and never emptied");
        assert!(docs.path().join("thesis.md").exists());

        let target = tempfile::tempdir().unwrap();
        std::fs::write(target.path().join(TRY_MARKER), "").unwrap();
        std::fs::write(target.path().join("keep"), "x").unwrap();
        let link_parent = tempfile::tempdir().unwrap();
        let link = link_parent.path().join("try");
        std::os::unix::fs::symlink(target.path(), &link).unwrap();
        assert!(prepare_try_dir(&link).is_err(), "a link is refused");
        assert!(empty_dir(&link).is_err());
        assert!(target.path().join("keep").exists());

        let fresh = tempfile::tempdir().unwrap();
        let new_dir = fresh.path().join("try-it");
        prepare_try_dir(&new_dir).unwrap();
        assert!(owns_try_dir(&new_dir));
    }

    #[test]
    fn the_try_folder_is_emptied_but_stays_and_a_home_is_never_emptied() {
        let dir = tempfile::tempdir().unwrap();
        prepare_try_dir(dir.path()).unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        std::fs::write(dir.path().join("a/b/c"), "x").unwrap();
        std::fs::write(dir.path().join("f"), "x").unwrap();
        empty_dir(dir.path()).unwrap();
        assert!(dir.path().exists());
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            left,
            [std::ffi::OsString::from(TRY_MARKER)],
            "only the marker stays"
        );
        let home = Path::new("/Users/you");
        assert!(!safe_try_dir(home, home));
        assert!(!safe_try_dir(Path::new("/Users"), home));
        assert!(!safe_try_dir(Path::new("/"), home));
        assert!(safe_try_dir(&home.join(".hq/desktop/try-it"), home));
    }

    #[test]
    fn newer_versions_compare_by_number() {
        assert!(is_newer("1.10.0", "1.9.9"));
        assert!(!is_newer("1.0.0", "1.0.0"));
    }

    #[test]
    fn a_helper_with_no_target_cannot_be_connected() {
        let dir = tempfile::tempdir().unwrap();
        let h = Helpers::new();
        for id in ["codex", "pi", "kimi", "qwen", "hermes", "openclaw"] {
            assert!(
                h.connect(id, Path::new("/bin/true"), dir.path(), true)
                    .is_err()
            );
        }
    }

    mod try_it {
        use super::*;

        #[test]
        fn a_trust_prompt_needs_the_person_except_where_the_host_answers() {
            let claude = find_spec("claude-code").unwrap();
            let agy = find_spec("antigravity").unwrap();
            assert_eq!(
                judge_screen(Some("blocked"), false, claude),
                State::NeedsYou
            );
            assert_eq!(judge_screen(Some("blocked"), false, agy), State::Trying);
            assert_eq!(judge_screen(Some("idle"), true, claude), State::CheckFailed);
        }

        #[cfg(unix)]
        #[test]
        fn a_disconnect_restores_byte_for_byte_and_too_late_or_changed_does_not() {
            let _exec = EXEC.lock().unwrap_or_else(|e| e.into_inner());
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path();
            let cfg = home.join(".cursor/mcp.json");
            std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
            let original =
                b"{\n  \"mcpServers\": { \"agent-hq\": {\"command\": \"hq\"}, \"x\": 1 }\n}\n";
            std::fs::write(&cfg, original).unwrap();
            // A stand-in `hq` that removes the entry the way the real one rewrites the file.
            let hq = script(
                home,
                "hq",
                &format!("printf '{{\"mcpServers\":{{}}}}' > '{}'", cfg.display()),
            );
            let h = Helpers::new();
            h.connect("cursor", &hq, home, false).unwrap();
            assert!(!is_connected("cursor", home));
            h.restore("cursor").unwrap();
            assert_eq!(std::fs::read(&cfg).unwrap(), original);

            h.connect("cursor", &hq, home, false).unwrap();
            h.age_undo(UNDO_WINDOW + Duration::from_secs(1));
            assert_eq!(h.restore("cursor").unwrap_err().to_string(), RESTORE_FAILED);

            std::fs::write(&cfg, original).unwrap();
            h.connect("cursor", &hq, home, false).unwrap();
            std::fs::write(&cfg, b"{\"changed\":true}").unwrap();
            assert!(h.restore("cursor").is_err());
            assert_eq!(std::fs::read(&cfg).unwrap(), b"{\"changed\":true}");
        }

        #[cfg(unix)]
        #[test]
        fn a_real_host_runs_a_try_and_a_stop_removes_it() {
            let _exec = EXEC.lock().unwrap_or_else(|e| e.into_inner());
            let dir = tempfile::tempdir().unwrap();
            let run = dir.path().join("run");
            let server = hq_host::Server::bind(&run, Arc::new(hq_host::Host::new())).unwrap();
            let stop = server.stop_handle();
            let thread = std::thread::spawn(move || server.serve());
            let prog = script(dir.path(), "claude", "echo ready; sleep 30");
            let try_dir = dir.path().join("try");
            std::fs::create_dir(&try_dir).unwrap();
            let spec = find_spec("claude-code").unwrap();
            let (name, out, _) = try_through_host(
                &run,
                spec,
                &prog,
                &try_dir,
                json!({"mode": "none"}),
                Duration::from_secs(8),
            )
            .unwrap();
            assert!(
                out.shown.contains("ready"),
                "{:?} {:?}",
                out.shown,
                out.state
            );
            stop_through_host(&run, &name);
            let gone = hq_host::Client::connect(&run)
                .unwrap()
                .call("agent.get", json!({ "name": name }));
            assert!(gone.is_err());
            stop.stop();
            let _ = thread.join();
        }

        #[test]
        fn a_general_helper_starts_only_after_a_yes_and_never_during_a_lockdown() {
            let hermes = find_spec("hermes").unwrap();
            let claude = find_spec("claude-code").unwrap();
            assert_eq!(may_start(hermes, false, false), Err("confirm"));
            assert_eq!(may_start(hermes, true, false), Ok(()));
            assert_eq!(may_start(claude, false, false), Ok(()));
            assert!(may_start(claude, true, true).is_err());
            assert!(may_start(find_spec("claude-desktop").unwrap(), true, false).is_err());
        }
    }
}
