//! Adding the paired hub to Claude Code and Claude Desktop on this computer, and taking it out
//! again (`hq claude`, the desktop app's "Use HQ in Claude", `docs/PAIRING.md`).
//!
//! Claude reaches HQ only through this machine: Claude Code gets the hub's MCP endpoint on the
//! forward pairing opened, with the per-device key; Claude Desktop only starts stdio servers, so it
//! gets `hq mcp-serve --remote <hub>`, which relays to the same endpoint and keeps the key in the
//! database. Both are the owner's files: nothing here runs unless a person asked, and `remove`
//! deletes only the entries `add` writes, found by name and by their exact URL or command.
//!
//! The `claude` program is reached through [`Runner`], so tests never start a real one.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use hq_db::{Database, bridge_setup as rows};
use serde::Serialize;
use serde_json::{Value, json};

/// The server name used when nothing else has it.
pub const DEFAULT_SERVER_NAME: &str = "hq";
const NAME_PREFIX: &str = "hq-";
const CLAUDE_PROGRAM: &str = "claude";
const KEY_HEADER: &str = "x-api-key";
const DESKTOP_DIR: &str = "Claude";
const DESKTOP_FILE: &str = "claude_desktop_config.json";
const SERVERS_KEY: &str = "mcpServers";
const SERVE_ARGS: [&str; 2] = ["mcp-serve", "--remote"];
const HQ_PROGRAM: &str = "hq";
/// Where Claude Code's installers put `claude`, for an app started without the shell's `PATH`.
const HOME_INSTALL_DIRS: [&str; 2] = [".local/bin", ".claude/local"];
const SYSTEM_INSTALL_DIRS: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];
#[cfg(unix)]
const EXEC_BITS: u32 = 0o111;
/// How long to wait for a pipe after `claude` exits, when the deadline is already near.
const PIPE_GRACE: Duration = Duration::from_secs(1);
/// `claude mcp get` also checks the server's health, which can take a while.
const CLAUDE_TIMEOUT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(50);
const URL_LABEL: &str = "URL:";
const REDACTED: &str = "***";
/// What Claude Code prints in place of a header value it hides.
const CLAUDE_MASK: &str = "[REDACTED]";

/// A hub this machine was set up with: what to write into Claude.
#[derive(Clone)]
pub struct Hub {
    /// The hub's slug, as setup named it (also the `remote_mcp` name `hq mcp-serve --remote` takes).
    pub name: String,
    pub url: String,
    key: Option<String>,
}

// By hand: the key must never reach a log through `{:?}`.
impl std::fmt::Debug for Hub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hub")
            .field("name", &self.name)
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl Hub {
    pub fn from_setup(setup: &rows::Setup) -> Option<Self> {
        setup.is_hub().then(|| Self {
            name: setup.peer_name.clone(),
            url: format!(
                "http://127.0.0.1:{}{}",
                setup.local_port,
                rows::HUB_MCP_PATH
            ),
            key: setup.mcp_key.clone(),
        })
    }

    /// The names the entries may have, in the order they are tried: `hq`, then `hq-<hub>`.
    fn server_names(&self) -> [String; 2] {
        [
            DEFAULT_SERVER_NAME.to_string(),
            format!("{NAME_PREFIX}{}", self.name),
        ]
    }

    fn scrub(&self, text: &str) -> String {
        match self.key.as_deref().filter(|k| !k.is_empty()) {
            Some(key) => text.replace(key, REDACTED),
            None => text.to_string(),
        }
    }
}

/// Every hub this machine is set up with, oldest first.
pub fn hubs(db: &Database) -> Result<Vec<Hub>> {
    Ok(rows::list(db)?.iter().filter_map(Hub::from_setup).collect())
}

/// The hub asked for by name, or the only one.
pub fn pick(hubs: &[Hub], wanted: Option<&str>) -> Result<Hub> {
    match (hubs, wanted) {
        ([], _) => bail!("this machine is not paired with a hub (see `hq bridge pair`)"),
        (_, Some(name)) => hubs
            .iter()
            .find(|h| h.name == name)
            .cloned()
            .ok_or_else(|| {
                let known: Vec<_> = hubs.iter().map(|h| h.name.as_str()).collect();
                anyhow!(
                    "no paired hub is named `{name}` (known: {})",
                    known.join(", ")
                )
            }),
        ([only], None) => Ok(only.clone()),
        _ => {
            let known: Vec<_> = hubs.iter().map(|h| h.name.as_str()).collect();
            bail!("name the hub: {}", known.join(", "))
        }
    }
}

/// What one run of `claude` gave back.
pub struct Run {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// The seam to the `claude` program.
pub trait Runner {
    fn available(&self) -> bool;
    /// Runs `claude` with `args` in the folder `cwd`.
    fn run(&self, args: &[String], cwd: &Path) -> Result<Run>;
}

/// Runs `claude` from `PATH`, or from where its installers put it.
pub struct SystemRunner {
    program: Option<PathBuf>,
}

impl SystemRunner {
    pub fn new() -> Self {
        let extra = install_dirs(dirs::home_dir().as_deref());
        Self {
            program: find_on_path(CLAUDE_PROGRAM, std::env::var_os("PATH").as_deref(), &extra),
        }
    }
}

impl Default for SystemRunner {
    fn default() -> Self {
        Self::new()
    }
}

/// A GUI app on macOS starts with a minimal `PATH`, so the usual install folders come after it.
fn install_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = home
        .into_iter()
        .flat_map(|h| HOME_INSTALL_DIRS.map(|d| h.join(d)))
        .collect();
    dirs.extend(SYSTEM_INSTALL_DIRS.map(PathBuf::from));
    dirs
}

/// A relative entry (empty, `.`) would run whatever sits in the current folder, so only absolute
/// folders count.
fn find_on_path(program: &str, path: Option<&OsStr>, extra: &[PathBuf]) -> Option<PathBuf> {
    path.map(std::env::split_paths)
        .into_iter()
        .flatten()
        .chain(extra.iter().cloned())
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(program))
        .find(|p| is_executable(p))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & EXEC_BITS != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Reads a pipe to its end on its own thread, so a child that fills it never blocks.
fn drain(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    if let Some(mut pipe) = pipe {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            let _ = tx.send(bytes);
        });
    }
    rx
}

/// What a pipe gave, waited for no later than the deadline: a process `claude` left running may
/// still hold it open.
fn collect(rx: &mpsc::Receiver<Vec<u8>>, deadline: Instant) -> String {
    let wait = deadline
        .saturating_duration_since(Instant::now())
        .max(PIPE_GRACE);
    String::from_utf8_lossy(&rx.recv_timeout(wait).unwrap_or_default()).into_owned()
}

impl Runner for SystemRunner {
    fn available(&self) -> bool {
        self.program.is_some()
    }

    fn run(&self, args: &[String], cwd: &Path) -> Result<Run> {
        let program = self
            .program
            .as_ref()
            .ok_or_else(|| anyhow!("`{CLAUDE_PROGRAM}` is not on PATH"))?;
        let mut child = std::process::Command::new(program)
            .args(args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("starting `{CLAUDE_PROGRAM}`"))?;
        let stdout = drain(child.stdout.take());
        let stderr = drain(child.stderr.take());
        let deadline = Instant::now() + CLAUDE_TIMEOUT;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                bail!("`{CLAUDE_PROGRAM}` did not finish within {CLAUDE_TIMEOUT:?}");
            }
            std::thread::sleep(POLL);
        };
        Ok(Run {
            ok: status.success(),
            stdout: collect(&stdout, deadline),
            stderr: collect(&stderr, deadline),
        })
    }
}

/// Where Claude Desktop keeps its config on this computer.
pub fn desktop_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(DESKTOP_DIR).join(DESKTOP_FILE))
}

/// What to write into, and which program Claude Desktop should start.
pub struct Targets<'a> {
    pub runner: &'a dyn Runner,
    /// `None` when the platform has no such place.
    pub desktop_config: Option<PathBuf>,
    /// This computer's `hq`, as an absolute path.
    pub hq_bin: PathBuf,
}

/// What happened to one of the two places.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "detail", rename_all = "snake_case")]
pub enum Part {
    /// Not applicable here, and why.
    Skipped(String),
    Added(String),
    /// Already there, under this name.
    Present(String),
    Removed(String),
    Absent,
    Failed(String),
}

impl Part {
    /// One line for a terminal or a window.
    pub fn text(&self, place: &str) -> String {
        match self {
            Self::Skipped(why) => format!("{place}: skipped, {why}"),
            Self::Added(name) => format!("{place}: added as `{name}`"),
            Self::Present(name) => format!("{place}: present as `{name}`"),
            Self::Removed(name) => format!("{place}: removed `{name}`"),
            Self::Absent => format!("{place}: not added"),
            Self::Failed(why) => format!("{place}: failed, {why}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub code: Part,
    pub desktop: Part,
}

impl Report {
    pub fn lines(&self) -> [String; 2] {
        [
            self.code.text("Claude Code"),
            self.desktop.text("Claude Desktop"),
        ]
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Add,
    Status,
    Remove,
}

pub fn add(t: &Targets, hub: &Hub) -> Report {
    run_both(t, hub, Action::Add)
}

pub fn status(t: &Targets, hub: &Hub) -> Report {
    run_both(t, hub, Action::Status)
}

pub fn remove(t: &Targets, hub: &Hub) -> Report {
    run_both(t, hub, Action::Remove)
}

/// Takes the hub's entries out of Claude before a revoke deletes the setup that names them. `None`
/// when `peer` is not a hub this machine was set up with.
pub fn remove_for_peer(db: &Database, t: &Targets, peer: &str) -> Option<Report> {
    let hub = rows::get(db, peer)
        .ok()
        .flatten()
        .as_ref()
        .and_then(Hub::from_setup)?;
    Some(remove(t, &hub))
}

fn run_both(t: &Targets, hub: &Hub, action: Action) -> Report {
    Report {
        code: code_part(t.runner, hub, action),
        desktop: desktop_part(t, hub, action),
    }
}

fn code_part(runner: &dyn Runner, hub: &Hub, action: Action) -> Part {
    if !runner.available() {
        return Part::Skipped(format!(
            "`{CLAUDE_PROGRAM}` is not on PATH or where Claude Code installs it"
        ));
    }
    // A folder's local and project servers shadow user scope in `claude mcp get`, so `claude`
    // runs where there are none.
    tempfile::tempdir()
        .context("making an empty folder to run `claude` in")
        .and_then(|cwd| {
            code_run(
                &Code {
                    runner,
                    hub,
                    cwd: cwd.path(),
                },
                action,
            )
        })
        .unwrap_or_else(|e| Part::Failed(hub.scrub(&format!("{e:#}"))))
}

/// One `claude` for one hub, always run in the same empty folder.
struct Code<'a> {
    runner: &'a dyn Runner,
    hub: &'a Hub,
    cwd: &'a Path,
}

/// An entry with the hub's URL, and whether it holds the key this machine has now.
struct Ours {
    name: String,
    current: bool,
}

fn code_run(c: &Code, action: Action) -> Result<Part> {
    let (ours, free) = find_code_slots(c)?;
    match (action, ours, free) {
        (Action::Status | Action::Add, Some(o), _) if o.current => Ok(Part::Present(o.name)),
        (Action::Status, Some(o), _) => bail!(
            "`{}` holds an old key; run `hq claude add {}` to replace it",
            o.name,
            c.hub.name
        ),
        (Action::Status | Action::Remove, None, _) => Ok(Part::Absent),
        // After a new pairing or a rotated key the entry is dead: it is replaced under its name.
        (Action::Add, Some(o), _) => {
            let key = hub_key(c.hub)?;
            c.claude(&remove_args(&o.name))?;
            c.claude(&add_args(&o.name, &c.hub.url, key))?;
            Ok(Part::Added(o.name))
        }
        (Action::Add, None, Some(name)) => {
            c.claude(&add_args(&name, &c.hub.url, hub_key(c.hub)?))?;
            Ok(Part::Added(name))
        }
        (Action::Add, None, None) => bail!(
            "the names {} are already used in Claude Code for something else",
            c.hub.server_names().join(" and ")
        ),
        (Action::Remove, Some(o), _) => {
            c.claude(&remove_args(&o.name))?;
            Ok(Part::Removed(o.name))
        }
    }
}

fn hub_key(hub: &Hub) -> Result<&str> {
    hub.key
        .as_deref()
        .ok_or_else(|| anyhow!("this machine has no key for the hub; pair again"))
}

/// The entry that is ours (same URL), and else the first name nobody uses.
fn find_code_slots(c: &Code) -> Result<(Option<Ours>, Option<String>)> {
    let mut free = None;
    for name in c.hub.server_names() {
        let got = c.run(&get_args(&name))?;
        if !got.ok {
            free = free.or(Some(name));
        } else if listed_url(&got.stdout) == Some(c.hub.url.as_str()) {
            // A hidden value cannot be compared, so only a visibly different key counts as old.
            let listed = listed_key(&got.stdout);
            let current = c.hub.key.is_some()
                && (listed == c.hub.key.as_deref() || listed == Some(CLAUDE_MASK));
            return Ok((Some(Ours { name, current }), None));
        }
    }
    Ok((None, free))
}

/// The address `claude mcp get` shows for a server.
fn listed_url(stdout: &str) -> Option<&str> {
    stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix(URL_LABEL))
        .map(str::trim)
}

/// The key header `claude mcp get` lists for a server, compared in memory and never shown.
fn listed_key(stdout: &str) -> Option<&str> {
    stdout.lines().find_map(|l| {
        let (name, value) = l.trim().split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case(KEY_HEADER)
            .then(|| value.trim())
    })
}

impl Code<'_> {
    fn run(&self, args: &[String]) -> Result<Run> {
        self.runner
            .run(args, self.cwd)
            .map_err(|e| anyhow!(self.hub.scrub(&format!("{e:#}"))))
    }

    fn claude(&self, args: &[String]) -> Result<()> {
        let out = self.run(args)?;
        if out.ok {
            return Ok(());
        }
        let said = self.hub.scrub(out.stderr.trim());
        bail!(
            "`claude mcp` exited with an error: {}",
            if said.is_empty() { "no message" } else { &said }
        )
    }
}

fn get_args(name: &str) -> Vec<String> {
    ["mcp", "get", name].map(String::from).to_vec()
}

fn remove_args(name: &str) -> Vec<String> {
    ["mcp", "remove", "--scope", "user", name]
        .map(String::from)
        .to_vec()
}

fn add_args(name: &str, url: &str, key: &str) -> Vec<String> {
    let mut args: Vec<String> = [
        "mcp",
        "add",
        "--scope",
        "user",
        "--transport",
        "http",
        name,
        url,
    ]
    .map(String::from)
    .to_vec();
    args.extend(["--header".to_string(), format!("{KEY_HEADER}: {key}")]);
    args
}

fn desktop_part(t: &Targets, hub: &Hub, action: Action) -> Part {
    let Some(path) = &t.desktop_config else {
        return Part::Skipped("Claude Desktop has no config location on this system".into());
    };
    if !path.parent().is_some_and(Path::is_dir) {
        return Part::Skipped(format!(
            "Claude Desktop is not installed ({} does not exist)",
            path.parent().unwrap_or(path).display()
        ));
    }
    desktop_run(path, &t.hq_bin, hub, action).unwrap_or_else(|e| Part::Failed(format!("{e:#}")))
}

fn desktop_run(path: &Path, hq_bin: &Path, hub: &Hub, action: Action) -> Result<Part> {
    let mut root = read_config(path)?;
    let server =
        |root: &Value, name: &str| root.get(SERVERS_KEY).and_then(|s| s.get(name)).cloned();
    let names = hub.server_names();
    let ours = names
        .iter()
        .find(|n| server(&root, n).is_some_and(|e| is_ours(&e, hq_bin, hub)))
        .cloned();
    match (action, ours) {
        (Action::Status | Action::Add, Some(name)) => Ok(Part::Present(name)),
        (Action::Status | Action::Remove, None) => Ok(Part::Absent),
        (Action::Add, None) => {
            let name = names
                .iter()
                .find(|n| server(&root, n).is_none())
                .cloned()
                .ok_or_else(|| {
                    anyhow!(
                        "the names {} are already used in Claude Desktop",
                        names.join(" and ")
                    )
                })?;
            servers_mut(&mut root)?.insert(name.clone(), entry(hq_bin, hub));
            write_config(path, &root)?;
            Ok(Part::Added(name))
        }
        (Action::Remove, Some(name)) => {
            servers_mut(&mut root)?.remove(&name);
            write_config(path, &root)?;
            Ok(Part::Removed(name))
        }
    }
}

fn entry(hq_bin: &Path, hub: &Hub) -> Value {
    json!({
        "command": hq_bin.to_string_lossy(),
        "args": [SERVE_ARGS[0], SERVE_ARGS[1], hub.name],
    })
}

/// Ours when it starts an `hq` with exactly these arguments: the CLI and the desktop app may run
/// `hq` from different places, and each must recognise what the other wrote.
fn is_ours(found: &Value, hq_bin: &Path, hub: &Hub) -> bool {
    let wanted = entry(hq_bin, hub);
    let Some(command) = found.get("command").and_then(Value::as_str).map(Path::new) else {
        return false;
    };
    let hq_name = format!("{HQ_PROGRAM}{}", std::env::consts::EXE_SUFFIX);
    found.get("args") == wanted.get("args")
        && (command.file_name() == Some(OsStr::new(&hq_name)) || same_file(command, hq_bin))
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

fn servers_mut(root: &mut Value) -> Result<&mut serde_json::Map<String, Value>> {
    root.as_object_mut()
        .ok_or_else(|| anyhow!("the config is not a JSON object"))?
        .entry(SERVERS_KEY)
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("`{SERVERS_KEY}` is not an object"))
}

/// A missing or empty file is an empty config; one that does not parse is never touched.
fn read_config(path: &Path) -> Result<Value> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&text)
        .with_context(|| format!("{} is not valid JSON, so it was left alone", path.display()))
}

/// Writes next to the file and renames over it, after a dated backup of what was there. The
/// file keeps its mode. Keys come back in alphabetical order, which Claude does not care about.
fn write_config(path: &Path, root: &Value) -> Result<()> {
    // A config kept in a dotfiles repo is usually a link: replace the file it points at.
    let target = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let original = std::fs::metadata(&target).ok();
    if original.is_some() {
        let stamp = chrono::Local::now().format("%Y-%m-%d");
        let backup = PathBuf::from(format!("{}.bak-{stamp}", target.display()));
        // The first backup of the day is the one from before HQ touched it.
        if !backup.exists() {
            std::fs::copy(&target, &backup)
                .with_context(|| format!("backing up to {}", backup.display()))?;
        }
    }
    let dir = target
        .parent()
        .ok_or_else(|| anyhow!("{} has no folder", target.display()))?;
    let prefix = format!(
        ".{}.",
        target
            .file_name()
            .unwrap_or(OsStr::new(DESKTOP_FILE))
            .to_string_lossy()
    );
    // A new name made with O_EXCL, never a fixed one someone could have planted; dropped on error.
    let mut tmp = tempfile::Builder::new()
        .prefix(&prefix)
        .suffix(".hq-tmp")
        .tempfile_in(dir)
        .with_context(|| format!("creating a temporary file in {}", dir.display()))?;
    if let Some(meta) = original {
        tmp.as_file().set_permissions(meta.permissions())?;
    }
    let mut text = serde_json::to_string_pretty(root)?;
    text.push('\n');
    tmp.write_all(text.as_bytes())?;
    tmp.as_file().sync_all()?;
    tmp.persist(&target)
        .map_err(|e| e.error)
        .with_context(|| format!("replacing {}", target.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::bridge as peers;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    const KEY: &str = "hqd_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const PEER: &str = "aa11";

    fn setup_row(role: &str, name: &str, port: u16) -> rows::Setup {
        rows::Setup {
            peer_id: PEER.into(),
            peer_role: role.into(),
            peer_name: name.into(),
            local_port: port,
            peer_offers: true,
            peer_port: None,
            mcp_key: (role == rows::ROLE_HUB).then(|| KEY.to_string()),
            offered: true,
        }
    }

    fn phone() -> Hub {
        Hub::from_setup(&setup_row(rows::ROLE_HUB, "phone", 47_410)).unwrap()
    }

    /// A stand-in for `claude`: the user-scope servers it knows (URL and header line), every call
    /// it got, and the folder each one ran in.
    #[derive(Default)]
    struct Fake {
        servers: RefCell<BTreeMap<String, (String, String)>>,
        calls: RefCell<Vec<Vec<String>>>,
        cwds: RefCell<Vec<(PathBuf, bool)>>,
        missing: bool,
        fail_with: Option<String>,
        /// `claude mcp add` runs but exits with this on stderr.
        add_says: Option<String>,
    }

    impl Fake {
        fn knows(&self, name: &str, url: &str, key: &str) {
            self.servers
                .borrow_mut()
                .insert(name.into(), (url.into(), format!("{KEY_HEADER}: {key}")));
        }
    }

    impl Runner for Fake {
        fn available(&self) -> bool {
            !self.missing
        }

        fn run(&self, args: &[String], cwd: &Path) -> Result<Run> {
            self.calls.borrow_mut().push(args.to_vec());
            let empty = std::fs::read_dir(cwd).is_ok_and(|mut d| d.next().is_none());
            self.cwds.borrow_mut().push((cwd.to_path_buf(), empty));
            let done = |ok, stdout: String, stderr: String| Ok(Run { ok, stdout, stderr });
            if let Some(why) = &self.fail_with {
                return Err(anyhow!("{why}"));
            }
            let arg = |i: usize| args[i].as_str();
            match arg(1) {
                "get" => match self.servers.borrow().get(arg(2)) {
                    Some((url, header)) => done(
                        true,
                        format!(
                            "{}:\n  Scope: User config\n  Type: http\n  URL: {url}\n  Headers:\n    {header}\n",
                            arg(2)
                        ),
                        String::new(),
                    ),
                    None => done(false, String::new(), String::new()),
                },
                "add" if self.add_says.is_some() => {
                    done(false, String::new(), self.add_says.clone().unwrap())
                }
                "add" => {
                    self.servers
                        .borrow_mut()
                        .insert(arg(6).into(), (arg(7).into(), arg(9).into()));
                    done(true, String::new(), String::new())
                }
                "remove" => {
                    self.servers.borrow_mut().remove(arg(4));
                    done(true, String::new(), String::new())
                }
                other => panic!("unexpected claude mcp {other}"),
            }
        }
    }

    fn targets<'a>(fake: &'a Fake, config: Option<PathBuf>) -> Targets<'a> {
        Targets {
            runner: fake,
            desktop_config: config,
            hq_bin: PathBuf::from("/opt/example/bin/hq"),
        }
    }

    fn read(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn claude_code_is_added_at_user_scope_with_the_key_in_a_header() {
        let fake = Fake::default();
        let report = add(&targets(&fake, None), &phone());
        assert_eq!(report.code, Part::Added("hq".into()));
        let added = fake.calls.borrow().last().unwrap().clone();
        assert_eq!(
            added,
            [
                "mcp",
                "add",
                "--scope",
                "user",
                "--transport",
                "http",
                "hq",
                "http://127.0.0.1:47410/mcp",
                "--header",
                &format!("x-api-key: {KEY}"),
            ]
        );
        // Again: nothing more is run, it is already there.
        let again = add(&targets(&fake, None), &phone());
        assert_eq!(again.code, Part::Present("hq".into()));
        assert!(fake.calls.borrow().iter().filter(|c| c[1] == "add").count() == 1);
    }

    #[test]
    fn a_taken_name_moves_to_the_hub_name_and_is_never_overwritten() {
        let fake = Fake::default();
        fake.knows("hq", "http://127.0.0.1:9/mcp", "theirs");
        let t = targets(&fake, None);
        assert_eq!(add(&t, &phone()).code, Part::Added("hq-phone".into()));
        assert_eq!(fake.servers.borrow()["hq"].0, "http://127.0.0.1:9/mcp");
        assert_eq!(status(&t, &phone()).code, Part::Present("hq-phone".into()));
        // Remove touches only ours.
        assert_eq!(remove(&t, &phone()).code, Part::Removed("hq-phone".into()));
        assert_eq!(fake.servers.borrow().len(), 1);
        assert_eq!(remove(&t, &phone()).code, Part::Absent);
    }

    #[test]
    fn no_claude_program_skips_claude_code_with_a_reason() {
        let fake = Fake {
            missing: true,
            ..Fake::default()
        };
        let report = status(&targets(&fake, None), &phone());
        assert!(matches!(&report.code, Part::Skipped(why) if why.contains("PATH")));
        assert!(fake.calls.borrow().is_empty());
    }

    #[test]
    fn the_key_is_never_in_a_message_or_in_debug_output() {
        let fake = Fake {
            fail_with: Some(format!("bad header x-api-key: {KEY}")),
            ..Fake::default()
        };
        let report = add(&targets(&fake, None), &phone());
        let text = report.lines().join("\n");
        assert!(matches!(report.code, Part::Failed(_)), "{text}");
        assert!(!text.contains(KEY), "{text}");
        assert!(!format!("{:?}", phone()).contains(KEY));
        let keyless = Hub {
            key: None,
            ..phone()
        };
        let report = add(&targets(&Fake::default(), None), &keyless);
        assert!(matches!(&report.code, Part::Failed(why) if why.contains("pair again")));
    }

    #[test]
    fn a_stale_key_is_replaced_and_status_says_so_without_showing_either_key() {
        const OLD: &str = "hqd_old0000000000000000000000000000000000000000000000000000000000";
        let fake = Fake::default();
        fake.knows("hq", "http://127.0.0.1:47410/mcp", OLD);
        let t = targets(&fake, None);
        let stale = status(&t, &phone());
        let text = stale.lines().join("\n");
        assert!(
            matches!(&stale.code, Part::Failed(why) if why.contains("old key")),
            "{text}"
        );
        assert!(!text.contains(OLD) && !text.contains(KEY), "{text}");

        assert_eq!(add(&t, &phone()).code, Part::Added("hq".into()));
        let verbs: Vec<String> = fake.calls.borrow().iter().map(|c| c[1].clone()).collect();
        assert!(
            verbs.ends_with(&["remove".into(), "add".into()]),
            "{verbs:?}"
        );
        assert_eq!(fake.servers.borrow()["hq"].1, format!("x-api-key: {KEY}"));
        assert_eq!(status(&t, &phone()).code, Part::Present("hq".into()));

        // A key Claude Code hides is taken as current rather than replaced on every add.
        fake.knows("hq", "http://127.0.0.1:47410/mcp", CLAUDE_MASK);
        assert_eq!(add(&t, &phone()).code, Part::Present("hq".into()));
    }

    #[test]
    fn every_claude_runs_in_an_empty_folder_that_is_not_the_current_one() {
        let fake = Fake::default();
        add(&targets(&fake, None), &phone());
        let here = std::env::current_dir().unwrap();
        let cwds = fake.cwds.borrow();
        assert!(cwds.len() >= 2);
        for (cwd, empty) in cwds.iter() {
            assert!(cwd.is_absolute() && *empty && *cwd != here, "{cwd:?}");
        }
        // The folder is thrown away afterwards.
        assert!(!cwds[0].0.exists());
    }

    #[test]
    fn a_failed_run_is_scrubbed_of_the_key_it_printed() {
        let fake = Fake {
            add_says: Some(format!("Invalid header x-api-key: {KEY}")),
            ..Fake::default()
        };
        let report = add(&targets(&fake, None), &phone());
        let text = report.lines().join("\n");
        assert!(
            matches!(&report.code, Part::Failed(why) if why.contains(REDACTED)),
            "{text}"
        );
        assert!(!text.contains(KEY), "{text}");
    }

    #[cfg(unix)]
    fn plant(dir: &Path, mode: u32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let file = dir.join(CLAUDE_PROGRAM);
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
        file
    }

    #[cfg(unix)]
    #[test]
    fn the_path_search_skips_relative_folders_and_files_that_cannot_run() {
        let plain = tempfile::tempdir().unwrap();
        plant(plain.path(), 0o644);
        let path = std::env::join_paths([plain.path()]).unwrap();
        assert_eq!(find_on_path(CLAUDE_PROGRAM, Some(&path), &[]), None);

        // A folder named relative to the current one, holding a runnable `claude`.
        let local = tempfile::tempdir_in(".").unwrap();
        plant(local.path(), 0o755);
        let named = Path::new(".").join(local.path().file_name().unwrap());
        assert!(named.join(CLAUDE_PROGRAM).is_file());
        let relative = std::env::join_paths([Path::new(""), &named]).unwrap();
        assert_eq!(find_on_path(CLAUDE_PROGRAM, Some(&relative), &[]), None);

        // After `PATH`, the install folders are searched.
        let installed = tempfile::tempdir().unwrap();
        let found = plant(installed.path(), 0o755);
        let extra = [installed.path().to_path_buf()];
        assert_eq!(
            find_on_path(CLAUDE_PROGRAM, Some(&path), &extra),
            Some(found.clone())
        );
        assert_eq!(find_on_path(CLAUDE_PROGRAM, None, &extra), Some(found));
        let dirs = install_dirs(Some(Path::new("/home/example")));
        assert!(dirs.contains(&PathBuf::from("/home/example/.local/bin")));
        assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")));
    }

    #[cfg(unix)]
    #[test]
    fn a_child_that_writes_a_lot_finishes_and_runs_in_the_given_folder() {
        const BIG: usize = 512 * 1024;
        let runner = SystemRunner {
            program: Some(PathBuf::from("/bin/sh")),
        };
        let cwd = tempfile::tempdir().unwrap();
        let script = format!("head -c {BIG} /dev/zero; head -c {BIG} /dev/zero >&2; pwd -P >&2");
        let started = Instant::now();
        let out = runner.run(&["-c".to_string(), script], cwd.path()).unwrap();
        assert!(out.ok);
        assert!(started.elapsed() < CLAUDE_TIMEOUT / 2);
        assert_eq!(out.stdout.len(), BIG);
        let printed = out.stderr.trim_start_matches('\0').trim();
        assert_eq!(Path::new(printed), cwd.path().canonicalize().unwrap());
    }

    fn desktop_dir() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("Claude").join(DESKTOP_FILE);
        std::fs::create_dir(config.parent().unwrap()).unwrap();
        (dir, config)
    }

    #[test]
    fn claude_desktop_gets_a_stdio_entry_and_keeps_everything_else() {
        let (_dir, config) = desktop_dir();
        std::fs::write(
            &config,
            r#"{"theme":"dark","mcpServers":{"other":{"command":"x","args":["1"]}}}"#,
        )
        .unwrap();
        let fake = Fake {
            missing: true,
            ..Fake::default()
        };
        let t = targets(&fake, Some(config.clone()));
        assert_eq!(add(&t, &phone()).desktop, Part::Added("hq".into()));
        let got = read(&config);
        assert_eq!(got["theme"], "dark");
        assert_eq!(
            got["mcpServers"]["other"],
            json!({"command": "x", "args": ["1"]})
        );
        assert_eq!(
            got["mcpServers"]["hq"],
            json!({"command": "/opt/example/bin/hq", "args": ["mcp-serve", "--remote", "phone"]})
        );
        assert!(!std::fs::read_to_string(&config).unwrap().contains(KEY));
        let backups: Vec<_> = std::fs::read_dir(config.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".bak-"))
            .collect();
        assert_eq!(backups.len(), 1);
        let before = std::fs::read_to_string(backups[0].path()).unwrap();
        assert!(before.contains("\"theme\"") && !before.contains("mcp-serve"));
    }

    #[test]
    fn adding_twice_changes_nothing_and_remove_deletes_only_ours() {
        let (_dir, config) = desktop_dir();
        let fake = Fake {
            missing: true,
            ..Fake::default()
        };
        let t = targets(&fake, Some(config.clone()));
        assert_eq!(status(&t, &phone()).desktop, Part::Absent);
        assert_eq!(add(&t, &phone()).desktop, Part::Added("hq".into()));
        let once = std::fs::read_to_string(&config).unwrap();
        assert_eq!(add(&t, &phone()).desktop, Part::Present("hq".into()));
        assert_eq!(std::fs::read_to_string(&config).unwrap(), once);
        assert_eq!(status(&t, &phone()).desktop, Part::Present("hq".into()));

        // A user's own `hq-phone` with another command is theirs, not ours.
        let mut value = read(&config);
        value["mcpServers"]["hq-phone"] = json!({"command": "mine"});
        std::fs::write(&config, serde_json::to_string(&value).unwrap()).unwrap();
        assert_eq!(remove(&t, &phone()).desktop, Part::Removed("hq".into()));
        let left = read(&config);
        assert!(left["mcpServers"].get("hq").is_none());
        assert_eq!(left["mcpServers"]["hq-phone"], json!({"command": "mine"}));
        assert_eq!(remove(&t, &phone()).desktop, Part::Absent);
    }

    #[test]
    fn a_name_taken_by_something_else_in_desktop_falls_back_and_bad_json_is_left_alone() {
        let (_dir, config) = desktop_dir();
        std::fs::write(&config, r#"{"mcpServers":{"hq":{"command":"mine"}}}"#).unwrap();
        let fake = Fake {
            missing: true,
            ..Fake::default()
        };
        let t = targets(&fake, Some(config.clone()));
        assert_eq!(add(&t, &phone()).desktop, Part::Added("hq-phone".into()));
        assert_eq!(
            read(&config)["mcpServers"]["hq"],
            json!({"command": "mine"})
        );

        std::fs::write(&config, "{ not json").unwrap();
        assert!(matches!(add(&t, &phone()).desktop, Part::Failed(_)));
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "{ not json");
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_config_that_does_not_parse_or_has_odd_servers_is_left_untouched() {
        let fake = Fake {
            missing: true,
            ..Fake::default()
        };
        for text in [
            "{ not json",
            r#"{"mcpServers":[]}"#,
            r#"{"mcpServers":"x"}"#,
            "[]",
        ] {
            let (_dir, config) = desktop_dir();
            std::fs::write(&config, text).unwrap();
            let t = targets(&fake, Some(config.clone()));
            for report in [add(&t, &phone()), remove(&t, &phone())] {
                assert!(
                    matches!(report.desktop, Part::Failed(_) | Part::Absent),
                    "{text}: {report:?}"
                );
            }
            assert!(
                matches!(add(&t, &phone()).desktop, Part::Failed(_)),
                "{text}"
            );
            assert_eq!(std::fs::read_to_string(&config).unwrap(), text);
            assert_eq!(files_in(config.parent().unwrap()), [DESKTOP_FILE], "{text}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_config_is_followed_and_the_link_kept() {
        let (dir, config) = desktop_dir();
        let dotfiles = dir.path().join("dotfiles");
        std::fs::create_dir(&dotfiles).unwrap();
        let real = dotfiles.join(DESKTOP_FILE);
        std::fs::write(&real, r#"{"theme":"dark"}"#).unwrap();
        std::os::unix::fs::symlink(&real, &config).unwrap();
        let fake = Fake {
            missing: true,
            ..Fake::default()
        };
        assert_eq!(
            add(&targets(&fake, Some(config.clone())), &phone()).desktop,
            Part::Added("hq".into())
        );
        assert!(
            std::fs::symlink_metadata(&config)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let got = read(&real);
        assert_eq!(got["theme"], "dark");
        assert_eq!(got["mcpServers"]["hq"]["args"][2], "phone");
        // No temporary file is left next to the real one.
        assert!(files_in(&dotfiles).iter().all(|n| !n.contains("hq-tmp")));
    }

    #[test]
    fn the_cli_and_the_app_recognise_each_others_entries() {
        let fake = Fake {
            missing: true,
            ..Fake::default()
        };
        for (wrote, reads) in [
            (
                "/home/example/bin/hq",
                "/Applications/HQ.app/Contents/MacOS/hq",
            ),
            (
                "/Applications/HQ.app/Contents/MacOS/hq",
                "/home/example/bin/hq",
            ),
        ] {
            let (_dir, config) = desktop_dir();
            let mut cli = targets(&fake, Some(config.clone()));
            cli.hq_bin = PathBuf::from(wrote);
            assert_eq!(add(&cli, &phone()).desktop, Part::Added("hq".into()));
            let mut app = targets(&fake, Some(config.clone()));
            app.hq_bin = PathBuf::from(reads);
            assert_eq!(status(&app, &phone()).desktop, Part::Present("hq".into()));
            assert_eq!(add(&app, &phone()).desktop, Part::Present("hq".into()));
            assert_eq!(remove(&app, &phone()).desktop, Part::Removed("hq".into()));
            assert!(read(&config)["mcpServers"].as_object().unwrap().is_empty());
        }
    }

    #[test]
    fn an_entry_is_ours_only_with_our_arguments_and_an_hq_program() {
        let hub = phone();
        let bin = Path::new("/opt/example/bin/hq");
        let with = |command: &str, args: Value| json!({"command": command, "args": args});
        let args = json!(["mcp-serve", "--remote", "phone"]);
        assert!(is_ours(&with("/elsewhere/hq", args.clone()), bin, &hub));
        assert!(!is_ours(
            &with("/elsewhere/not-hq", args.clone()),
            bin,
            &hub
        ));
        assert!(!is_ours(
            &with("/elsewhere/hq", json!(["mcp-serve", "--remote", "vps"])),
            bin,
            &hub
        ));
        assert!(!is_ours(
            &with("/elsewhere/hq", json!(["mcp-serve"])),
            bin,
            &hub
        ));
        assert!(!is_ours(&json!({"args": args}), bin, &hub));
    }

    #[cfg(unix)]
    #[test]
    fn a_program_with_another_name_is_ours_when_it_is_the_same_file() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("hq-desktop-sidecar");
        std::fs::write(&real, "").unwrap();
        let link = dir.path().join("link-to-it");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let found =
            json!({"command": link.to_string_lossy(), "args": ["mcp-serve", "--remote", "phone"]});
        assert!(is_ours(&found, &real, &phone()));
        assert!(!is_ours(
            &found,
            Path::new("/opt/example/bin/other"),
            &phone()
        ));
    }

    #[cfg(unix)]
    #[test]
    fn the_config_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, config) = desktop_dir();
        std::fs::write(&config, "{}").unwrap();
        // Not 0o600, which is what a fresh temporary file would have anyway.
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o640)).unwrap();
        let fake = Fake {
            missing: true,
            ..Fake::default()
        };
        let t = targets(&fake, Some(config.clone()));
        assert_eq!(add(&t, &phone()).desktop, Part::Added("hq".into()));
        let mode = std::fs::metadata(&config).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
        assert_eq!(remove(&t, &phone()).desktop, Part::Removed("hq".into()));
        let mode = std::fs::metadata(&config).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
    }

    #[test]
    fn a_missing_desktop_folder_is_skipped_with_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("Claude").join(DESKTOP_FILE);
        let fake = Fake {
            missing: true,
            ..Fake::default()
        };
        let report = add(&targets(&fake, Some(config.clone())), &phone());
        assert!(matches!(&report.desktop, Part::Skipped(why) if why.contains("not installed")));
        assert!(!config.exists());
    }

    #[test]
    fn revoking_a_hub_removes_its_claude_entries_first() {
        let (_dir, config) = desktop_dir();
        let db = Database::open_memory().unwrap();
        peers::add_peer(&db, PEER, "phone", &[], &[]).unwrap();
        assert!(rows::record(&db, &setup_row(rows::ROLE_HUB, "phone", 47_410), None).unwrap());
        let fake = Fake::default();
        let t = targets(&fake, Some(config.clone()));
        let hub = pick(&hubs(&db).unwrap(), None).unwrap();
        add(&t, &hub);
        assert_eq!(fake.servers.borrow().len(), 1);

        let report = remove_for_peer(&db, &t, PEER).unwrap();
        assert_eq!(report.code, Part::Removed("hq".into()));
        assert_eq!(report.desktop, Part::Removed("hq".into()));
        assert!(fake.servers.borrow().is_empty());
        assert!(read(&config)["mcpServers"].as_object().unwrap().is_empty());
        // A machine peer has nothing to remove.
        assert!(remove_for_peer(&db, &t, "bb22").is_none());
    }

    #[test]
    fn the_hub_is_the_only_one_or_named() {
        let a = phone();
        let b = Hub {
            name: "vps".into(),
            ..phone()
        };
        assert!(pick(&[], None).is_err());
        assert_eq!(pick(std::slice::from_ref(&a), None).unwrap().name, "phone");
        assert!(pick(&[a.clone(), b.clone()], None).is_err());
        assert_eq!(pick(&[a, b], Some("vps")).unwrap().name, "vps");
        assert!(pick(&[phone()], Some("nope")).is_err());
    }
}
