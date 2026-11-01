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

use std::path::{Path, PathBuf};
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
/// `claude mcp get` also checks the server's health, which can take a while.
const CLAUDE_TIMEOUT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(50);
const URL_LABEL: &str = "URL:";
const REDACTED: &str = "***";

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
    fn run(&self, args: &[String]) -> Result<Run>;
}

/// Runs `claude` from `PATH`.
pub struct SystemRunner {
    program: Option<PathBuf>,
}

impl SystemRunner {
    pub fn new() -> Self {
        Self {
            program: find_on_path(CLAUDE_PROGRAM, std::env::var_os("PATH").as_deref()),
        }
    }
}

impl Default for SystemRunner {
    fn default() -> Self {
        Self::new()
    }
}

fn find_on_path(program: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    std::env::split_paths(path?)
        .map(|dir| dir.join(program))
        .find(|p| p.is_file())
}

impl Runner for SystemRunner {
    fn available(&self) -> bool {
        self.program.is_some()
    }

    fn run(&self, args: &[String]) -> Result<Run> {
        let program = self
            .program
            .as_ref()
            .ok_or_else(|| anyhow!("`{CLAUDE_PROGRAM}` is not on PATH"))?;
        let mut child = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("starting `{CLAUDE_PROGRAM}`"))?;
        let deadline = Instant::now() + CLAUDE_TIMEOUT;
        while child.try_wait()?.is_none() {
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                bail!("`{CLAUDE_PROGRAM}` did not finish within {CLAUDE_TIMEOUT:?}");
            }
            std::thread::sleep(POLL);
        }
        let out = child.wait_with_output()?;
        Ok(Run {
            ok: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
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
        return Part::Skipped(format!("`{CLAUDE_PROGRAM}` is not on PATH"));
    }
    code_run(runner, hub, action).unwrap_or_else(|e| Part::Failed(hub.scrub(&format!("{e:#}"))))
}

fn code_run(runner: &dyn Runner, hub: &Hub, action: Action) -> Result<Part> {
    let (ours, free) = find_code_slots(runner, hub)?;
    match (action, ours, free) {
        (Action::Status, Some(name), _) | (Action::Add, Some(name), _) => Ok(Part::Present(name)),
        (Action::Status, None, _) | (Action::Remove, None, _) => Ok(Part::Absent),
        (Action::Add, None, Some(name)) => {
            let key = hub
                .key
                .as_deref()
                .ok_or_else(|| anyhow!("this machine has no key for the hub; pair again"))?;
            claude(runner, hub, &add_args(&name, &hub.url, key))?;
            Ok(Part::Added(name))
        }
        (Action::Add, None, None) => bail!(
            "the names {} are already used in Claude Code for something else",
            hub.server_names().join(" and ")
        ),
        (Action::Remove, Some(name), _) => {
            claude(runner, hub, &remove_args(&name))?;
            Ok(Part::Removed(name))
        }
    }
}

/// The entry that is ours (same URL), and else the first name nobody uses.
fn find_code_slots(runner: &dyn Runner, hub: &Hub) -> Result<(Option<String>, Option<String>)> {
    let mut free = None;
    for name in hub.server_names() {
        let got = runner
            .run(&get_args(&name))
            .map_err(|e| anyhow!(hub.scrub(&format!("{e:#}"))))?;
        if !got.ok {
            free = free.or(Some(name));
        } else if listed_url(&got.stdout) == Some(hub.url.as_str()) {
            return Ok((Some(name), None));
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

fn claude(runner: &dyn Runner, hub: &Hub, args: &[String]) -> Result<()> {
    let out = runner
        .run(args)
        .map_err(|e| anyhow!(hub.scrub(&format!("{e:#}"))))?;
    if out.ok {
        return Ok(());
    }
    let said = hub.scrub(out.stderr.trim());
    bail!(
        "`claude mcp` exited with an error: {}",
        if said.is_empty() { "no message" } else { &said }
    )
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
        .find(|n| server(&root, n).is_some_and(|e| e == entry(hq_bin, hub)))
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
    let tmp = PathBuf::from(format!("{}.hq-tmp", target.display()));
    let mut text = serde_json::to_string_pretty(root)?;
    text.push('\n');
    std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
    if let Some(meta) = original {
        std::fs::set_permissions(&tmp, meta.permissions())?;
    }
    std::fs::rename(&tmp, &target).with_context(|| format!("replacing {}", target.display()))
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

    /// A stand-in for `claude`: the user-scope servers it knows, and every call it got.
    #[derive(Default)]
    struct Fake {
        servers: RefCell<BTreeMap<String, String>>,
        calls: RefCell<Vec<Vec<String>>>,
        missing: bool,
        fail_with: Option<String>,
    }

    impl Runner for Fake {
        fn available(&self) -> bool {
            !self.missing
        }

        fn run(&self, args: &[String]) -> Result<Run> {
            self.calls.borrow_mut().push(args.to_vec());
            let done = |ok, stdout: String| {
                Ok(Run {
                    ok,
                    stdout,
                    stderr: self.fail_with.clone().unwrap_or_default(),
                })
            };
            if let Some(why) = &self.fail_with {
                return Err(anyhow!("{why}"));
            }
            let arg = |i: usize| args[i].as_str();
            match arg(1) {
                "get" => match self.servers.borrow().get(arg(2)) {
                    Some(url) => done(true, format!("{}:\n  Type: http\n  URL: {url}\n", arg(2))),
                    None => done(false, String::new()),
                },
                "add" => {
                    self.servers
                        .borrow_mut()
                        .insert(arg(6).into(), arg(7).into());
                    done(true, String::new())
                }
                "remove" => {
                    self.servers.borrow_mut().remove(arg(4));
                    done(true, String::new())
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
        fake.servers
            .borrow_mut()
            .insert("hq".into(), "http://127.0.0.1:9/mcp".into());
        let t = targets(&fake, None);
        assert_eq!(add(&t, &phone()).code, Part::Added("hq-phone".into()));
        assert_eq!(fake.servers.borrow()["hq"], "http://127.0.0.1:9/mcp");
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

    #[cfg(unix)]
    #[test]
    fn the_config_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, config) = desktop_dir();
        std::fs::write(&config, "{}").unwrap();
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
        let fake = Fake {
            missing: true,
            ..Fake::default()
        };
        add(&targets(&fake, Some(config.clone())), &phone());
        let mode = std::fs::metadata(&config).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
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
