//! The app as a whole: settings, the host, pairing and the proxy, wired from the HQ config. The Tauri shell holds one of these and calls into it.

use crate::agents;
use crate::chooser;
use crate::claude;
use crate::helpers;
use crate::host::{self, HostController, HostSpec};
use crate::pairing::{HubServices, PairingConfig, PairingController};
use crate::proxy::{self, ProxyConfig, ProxyHandle};
use crate::rename;
use crate::settings::{Settings, create_private_dir};
use crate::window;
use anyhow::{Context, Result, bail};
use hq_bridge::{RelayMode, Tuning};
use hq_core::config::HqConfig;
use hq_db::Database;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as AsyncMutex;

const HOST_LOG: &str = "host.log";
/// Where a hub's gate keys live under `~/.hq`, as the CLI's setup writes them.
const GATE_KEY_DIR: &str = "gate";
/// The longest closing may take in all: the host's grace period, a kill and a little slack.
const SHUTDOWN_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);
/// How long closing the pairing endpoints may take before the app stops waiting for them.
const SHUTDOWN_STEP: std::time::Duration = std::time::Duration::from_secs(5);
const DEFAULT_DEVICE_NAME: &str = "hq-desktop";
const DEFAULT_MACOS_SHELL: &str = "/bin/zsh";

/// The `hq` the app runs, as the tab shows it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProgramView {
    pub path: PathBuf,
    pub version: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LockdownView {
    pub reason: String,
    /// `operator`, `cli` or `breaker`.
    pub source: String,
    pub at: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HelpersSummary {
    pub reachable: bool,
    pub working: Vec<String>,
    pub needs_you: Vec<String>,
}

/// What the This computer tab shows, in one read.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HubReport {
    #[serde(flatten)]
    pub host: host::HostFacts,
    pub usual_secs: Option<u64>,
    /// None when no `hq` was found: the tab shows the card for choosing one.
    pub program: Option<ProgramView>,
    pub serves_hub: bool,
    /// A paired hub's web app replaces HQ on this computer.
    pub hub_web: bool,
    pub free_bytes: Option<u64>,
    pub almost_full: bool,
    pub lockdown: Option<LockdownView>,
    pub helpers: HelpersSummary,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HostLog {
    pub path: PathBuf,
    pub lines: Vec<String>,
    pub version: Option<String>,
    pub exit_code: Option<i32>,
}

/// Bytes free on the disk holding `dir` (or its nearest folder that exists).
fn free_bytes(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let existing = dir.ancestors().find(|p| p.exists())?;
    let c = std::ffi::CString::new(existing.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid C string and `stat` is a zeroed statvfs the call fills in.
    let ok = unsafe { libc::statvfs(c.as_ptr(), &mut stat) } == 0;
    ok.then(|| stat.f_bavail as u64 * stat.f_frsize as u64)
}

pub struct DesktopInit {
    /// The app's own folder (`~/.hq/desktop`): settings and the host's log.
    pub dir: PathBuf,
    /// The long-term pairing key. The same file `hq bridge` uses, so both agree on who this is.
    pub bridge_key: PathBuf,
    pub config: HqConfig,
    /// Where the app's own program lives, to find a bundled `hq` next to it.
    pub app_dir: Option<PathBuf>,
    pub home: Option<PathBuf>,
    pub path_var: Option<std::ffi::OsString>,
    /// The person's login shell, asked for its PATH when the quick places have no `hq`.
    pub login_shell: Option<PathBuf>,
}

impl DesktopInit {
    /// The usual places on this machine.
    pub fn from_environment(config: HqConfig) -> Result<Self> {
        let home = dirs::home_dir().context("no home directory")?;
        Ok(Self {
            dir: home.join(".hq").join("desktop"),
            bridge_key: home.join(".hq").join("bridge").join("bridge.key"),
            app_dir: std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(Path::to_path_buf)),
            home: Some(home),
            path_var: std::env::var_os("PATH"),
            login_shell: login_shell(),
            config,
        })
    }
}

/// `$SHELL` when it is an absolute path, else the system's default shell.
fn login_shell() -> Option<PathBuf> {
    std::env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| cfg!(target_os = "macos").then(|| PathBuf::from(DEFAULT_MACOS_SHELL)))
}

/// The quick places first; the login shell, which can take seconds, only when they have no `hq`.
/// A configured path is never second-guessed.
fn find_hq(init: &DesktopInit, configured: Option<&Path>) -> Result<PathBuf, host::HostError> {
    let candidates = host::search_path(
        init.app_dir.as_deref(),
        init.home.as_deref(),
        init.path_var.as_deref(),
    );
    let quick = host::locate_hq(configured, &candidates);
    if quick.is_ok() || configured.is_some() {
        return quick;
    }
    let Some(login_path) = init.login_shell.as_deref().and_then(host::login_shell_path) else {
        return quick;
    };
    let mut all = candidates;
    all.extend(host::on_path(&login_path));
    let mut seen = std::collections::HashSet::new();
    all.retain(|p| seen.insert(p.clone()));
    host::locate_hq(None, &all)
}

/// What a revoke did, for the window.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Revoked {
    pub revoked: bool,
    /// Set when HQ's entries could not all be taken out of Claude.
    pub claude_warning: Option<String>,
}

pub struct Desktop {
    dir: PathBuf,
    settings: Mutex<Settings>,
    config: HqConfig,
    pub host: HostController,
    pub pairing: PairingController,
    /// The `hq` program the host runs, and the one Claude Desktop is told to start.
    hq_program: Mutex<PathBuf>,
    /// Whether HQ's program was found when the app opened or has been picked since.
    program_found: std::sync::atomic::AtomicBool,
    /// Why no `hq` was found when the app opened, for the window.
    hq_problem: Option<String>,
    proxy: AsyncMutex<Option<ProxyHandle>>,
    upstream: SocketAddr,
    helpers: Arc<helpers::Helpers>,
    /// The helper being tried and its pane in the host.
    trying: Mutex<Option<(String, String)>>,
    latest: Mutex<std::collections::HashMap<&'static str, String>>,
}

/// Everything the Helpers tab and a helper's page show.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HelpersReport {
    pub helpers: Vec<helpers::HelperView>,
    pub hubs: Vec<claude::HubView>,
    pub host_up: bool,
    pub locked: bool,
    pub looks_for_updates: bool,
    pub try_it_dir: PathBuf,
}

/// The paired hub's web forward is not this app's own, so the window is not opened on it.
#[derive(Debug, thiserror::Error)]
#[error("{}", if *held_elsewhere { proxy::HELD_PORT_LINE } else { FORWARD_WAIT_LINE })]
pub struct ForwardNotOurs {
    pub listen: SocketAddr,
    /// Another program had the port first; otherwise pairing is off and nothing holds it here.
    pub held_elsewhere: bool,
}

pub const FORWARD_WAIT_LINE: &str = "Open HQ waits until HQ Hub holds the port itself.";

/// Where the window's HQ is reachable from this machine: the paired hub's web app on its forward
/// when the config has one, else this machine's own web server. The app talks to either only over
/// loopback, so a host that listens on one non-loopback address cannot be used.
pub fn upstream_addr(config: &HqConfig) -> Result<SocketAddr> {
    if let Some(hub) = config.bridge.hub_web_listen() {
        return Ok(hub);
    }
    let bind: IpAddr = config
        .web_bind
        .parse()
        .with_context(|| format!("web_bind {:?} is not an address", config.web_bind))?;
    // The app dials 127.0.0.1, so only a host that answers there is usable: not `::1` alone.
    if bind != IpAddr::V4(Ipv4Addr::LOCALHOST) && !bind.is_unspecified() {
        bail!(
            "HQ listens on {bind} only, which this app does not use; set web_bind to 127.0.0.1 or 0.0.0.0"
        );
    }
    Ok(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), config.ws_port))
}

fn relay_mode(text: &str) -> Result<RelayMode> {
    match text {
        crate::settings::RELAY_NONE => Ok(RelayMode::Disabled),
        crate::settings::RELAY_DEFAULT => Ok(RelayMode::Default),
        url => Ok(RelayMode::custom([url.parse().context("relay URL")?])),
    }
}

/// Cleans up anything setup made outside hq-bridge: Claude entries on a machine, gate key on a hub.
pub fn cleanup_peer_outside_bridge(
    db: &Database,
    hq_program: &Path,
    peer: &str,
) -> Result<Option<hq_bridge::claude::Report>> {
    hq_bridge::setup::remove_machine_gate_key(db, &HqConfig::hq_dir().join(GATE_KEY_DIR), peer);
    let hq_bin = claude::hq_program(hq_program)?;
    let runner = hq_bridge::claude::SystemRunner::new();
    let targets = hq_bridge::claude::Targets {
        runner: &runner,
        desktop_config: hq_bridge::claude::desktop_config_path(),
        hq_bin,
    };
    Ok(hq_bridge::claude::remove_for_peer(db, &targets, peer))
}

impl Desktop {
    pub fn open(init: DesktopInit) -> Result<Self> {
        create_private_dir(&init.dir)?;
        let settings = Settings::load(&init.dir)?;
        let upstream = upstream_addr(&init.config)?;
        // A missing program is reported when the app opens and again by whatever needs it.
        let (program, hq_problem) = match find_hq(&init, settings.hq_path.as_deref()) {
            Ok(program) => (program, None),
            Err(e) => (PathBuf::from("hq"), Some(e.to_string())),
        };
        let hq_program = program.clone();
        let host = HostController::new(HostSpec::for_config(
            program,
            init.dir.join(HOST_LOG),
            &init.config,
        ));
        let name = if settings.device_name.trim().is_empty() {
            hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| DEFAULT_DEVICE_NAME.into())
        } else {
            settings.device_name.clone()
        };
        let db_path = init.config.db_path();
        let prog_for_cb = hq_program.clone();
        let db_path_for_cb = db_path.clone();
        // Only a revoke the peer told us of: [`Desktop::revoke`] cleans up itself, to say how it went.
        let on_revoked: hq_bridge::RevokeCallback = Arc::new(move |peer: &str| {
            if let Ok(db) = Database::open(&db_path_for_cb) {
                let _ = cleanup_peer_outside_bridge(&db, &prog_for_cb, peer);
            }
        });
        let pairing = PairingController::new(
            PairingConfig {
                key: init.bridge_key,
                bind: vec![SocketAddr::new(
                    Ipv4Addr::UNSPECIFIED.into(),
                    settings.bridge_port,
                )],
                relay: relay_mode(&settings.relay)?,
                name,
                with_addrs: true,
                tuning: Tuning::default(),
            },
            move || Database::open(&db_path),
            None,
        )
        .with_on_revoked(on_revoked)
        .with_services(HubServices {
            config: init.config.bridge.clone(),
            ws_port: init.config.ws_port,
        });
        Ok(Self {
            dir: init.dir,
            settings: Mutex::new(settings),
            config: init.config,
            host,
            pairing,
            program_found: std::sync::atomic::AtomicBool::new(hq_problem.is_none()),
            hq_program: Mutex::new(hq_program),
            hq_problem,
            proxy: AsyncMutex::new(None),
            upstream,
            helpers: Arc::new(helpers::Helpers::new()),
            trying: Mutex::new(None),
            latest: Mutex::new(Default::default()),
        })
    }

    /// The coding agents on this machine's host, those that need the person first.
    pub async fn agents(&self) -> agents::AgentsView {
        let dir = hq_core::config::native_host_dir();
        tokio::task::spawn_blocking(move || agents::read(&dir))
            .await
            .unwrap_or(agents::AgentsView {
                reachable: false,
                agents: Vec::new(),
            })
    }

    fn home(&self) -> PathBuf {
        dirs::home_dir().unwrap_or_default()
    }

    fn search_dirs(&self) -> Vec<PathBuf> {
        let path = std::env::var_os("PATH");
        helpers::search_dirs(Some(&self.home()), path.as_deref())
    }

    fn try_dir(&self) -> PathBuf {
        self.settings()
            .try_it_dir
            .unwrap_or_else(|| self.dir.join("try-it"))
    }

    /// Every helper and where it stands, read without running anything.
    pub async fn helpers_list(&self) -> HelpersReport {
        let trying = self.trying.lock().unwrap().as_ref().map(|t| t.0.clone());
        let latest = self.latest.lock().unwrap().clone();
        let seen = self.dir.join("seen-helpers");
        let views = self.helpers.list(
            &self.search_dirs(),
            &self.home(),
            &latest,
            trying.as_deref(),
            &seen,
        );
        let hubs = self.with_claude(claude::views).await.unwrap_or_default();
        let settings = self.settings();
        let dir = hq_core::config::native_host_dir();
        let host_up = self.agents().await.reachable;
        HelpersReport {
            helpers: views,
            hubs,
            host_up,
            locked: hq_host::read_lockdown(&dir).is_some(),
            looks_for_updates: settings.look_for_helper_updates,
            try_it_dir: self.try_dir(),
        }
    }

    /// Checks one helper, or every one found when `id` is none and `again` clears what was learned.
    pub async fn helpers_check(&self, id: Option<String>, again: bool) -> HelpersReport {
        if again {
            self.helpers.checker.forget_all();
        }
        let dirs = self.search_dirs();
        let mut jobs = Vec::new();
        for s in helpers::SPECS
            .iter()
            .filter(|s| id.as_deref().is_none_or(|i| i == s.id))
        {
            if let helpers::Found::At(p) = helpers::locate(s.program, &dirs) {
                let c = self.helpers.checker.clone();
                jobs.push(tokio::spawn(async move { c.check(p).await }));
            }
        }
        for j in jobs {
            let _ = j.await;
        }
        let report = self.helpers_list().await;
        self.helpers
            .remember(&report.helpers, &self.dir.join("seen-helpers"));
        if self.settings().look_for_helper_updates {
            self.look_for_updates(&report.helpers).await;
            return self.helpers_list().await;
        }
        report
    }

    /// Asks the npm registry for the newest version, and only when the person turned that on.
    async fn look_for_updates(&self, views: &[helpers::HelperView]) {
        let Ok(client) = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
        else {
            return;
        };
        for v in views.iter().filter(|v| v.version.is_some()) {
            let Some(pkg) = helpers::find_spec(v.id).and_then(|s| s.package) else {
                continue;
            };
            let url = format!("https://registry.npmjs.org/{pkg}/latest");
            let Ok(r) = client.get(url).send().await else {
                continue;
            };
            if let Ok(j) = r.json::<serde_json::Value>().await
                && let Some(ver) = j.get("version").and_then(|x| x.as_str())
            {
                self.latest.lock().unwrap().insert(v.id, ver.to_string());
            }
        }
    }

    /// Starts a helper in the Try it folder. Hermes and OpenClaw start only when `confirmed`.
    pub async fn helper_try(
        &self,
        id: &str,
        confirmed: bool,
        signin: bool,
    ) -> Result<helpers::TryOutcome> {
        let Some(spec) = helpers::find_spec(id) else {
            bail!("That helper is not known.");
        };
        let locked = hq_host::read_lockdown(&hq_core::config::native_host_dir()).is_some();
        helpers::may_start(spec, confirmed, locked).map_err(|e| anyhow::anyhow!(e))?;
        let helpers::Found::At(program) = helpers::locate(spec.program, &self.search_dirs()) else {
            bail!("That program was not found.");
        };
        let dir = self.try_dir();
        if !helpers::safe_try_dir(&dir, &self.home()) {
            bail!("The Try it folder is not a safe place. Choose another in Settings.");
        }
        self.helper_try_stop().await?;
        let cfg = self.config.agent_host.clone();
        let wait = std::time::Duration::from_secs(cfg.launch_bound_secs.clamp(5, 60));
        let sandbox = helpers::sandbox_plan(spec, &dir, &cfg);
        let host_dir = hq_core::config::native_host_dir();
        let (name, outcome, link) = tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&dir)?;
            helpers::empty_dir(&dir)?;
            helpers::try_through_host(&host_dir, spec, &program, &dir, sandbox, wait)
        })
        .await??;
        self.helpers.set_link(if signin { link } else { None });
        *self.trying.lock().unwrap() = Some((id.to_string(), name));
        Ok(outcome)
    }

    /// Ends the try, removes its pane and empties the Try it folder.
    pub async fn helper_try_stop(&self) -> Result<()> {
        let taken = self.trying.lock().unwrap().take();
        self.helpers.set_link(None);
        let dir = self.try_dir();
        let home = self.home();
        let host_dir = hq_core::config::native_host_dir();
        tokio::task::spawn_blocking(move || {
            if let Some((_, name)) = taken {
                helpers::stop_through_host(&host_dir, &name);
            }
            if helpers::safe_try_dir(&dir, &home) && dir.is_dir() {
                let _ = helpers::empty_dir(&dir);
            }
        })
        .await?;
        Ok(())
    }

    /// Copies the held sign-in link, or opens it when it is on the helper's own site.
    pub fn helper_signin(&self, id: &str, open: bool) -> Result<()> {
        let Some(link) = self.helpers.link() else {
            bail!("There is no sign-in link yet.");
        };
        let Some(target) = window::signin_target(id, &link) else {
            bail!("That link is not on this helper's own sign-in site, so it was not opened.");
        };
        let status = if open {
            std::process::Command::new("/usr/bin/open")
                .arg(&target)
                .status()
        } else {
            use std::io::Write;
            let mut c = std::process::Command::new("/usr/bin/pbcopy")
                .stdin(std::process::Stdio::piped())
                .spawn()?;
            c.stdin.take().map(|mut i| i.write_all(target.as_bytes()));
            c.wait()
        };
        if status?.success() {
            Ok(())
        } else {
            bail!("The system could not do that.")
        }
    }

    /// Connects or disconnects a helper, on this computer or (Claude only) for a paired hub.
    pub async fn helper_connect(
        &self,
        id: &str,
        hub: Option<String>,
        on: bool,
    ) -> Result<Option<claude::HubView>> {
        if let Some(hub) = hub {
            if !matches!(id, "claude-code" | "claude-desktop") {
                bail!("Only Claude can be added to a paired hub.");
            }
            return self
                .with_claude(move |db, t| claude::change(db, t, &hub, on))
                .await
                .map(Some);
        }
        let (id, hq, home) = (
            id.to_string(),
            claude::hq_program(&self.program())?,
            self.home(),
        );
        let h = self.helpers.clone();
        let result = tokio::task::spawn_blocking(move || h.connect(&id, &hq, &home, on)).await?;
        result.map(|_| None)
    }

    /// Undoes a disconnect, within ten minutes and only if nothing changed the files since.
    pub fn helper_restore(&self, id: &str) -> Result<()> {
        self.helpers.restore(id)
    }

    async fn with_claude<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Database, &hq_bridge::claude::Targets) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let bin = claude::hq_program(&self.program())?;
        claude::blocking(self.config.db_path(), bin, work).await
    }

    /// Revokes a device. A hub's entries in Claude go first: the revoke deletes the setup that
    /// names them, and a key that no longer works should not stay in Claude's config.
    pub async fn revoke(&self, id: &str) -> Result<Revoked> {
        let peer = id.to_string();
        let db_path = self.config.db_path();
        let bin = self.program();
        let cleanup = tokio::task::spawn_blocking(move || {
            let db = Database::open(&db_path)?;
            cleanup_peer_outside_bridge(&db, &bin, &peer)
        })
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!("{e}")));
        // Claude being absent or its file being odd must not stop the revoke, only be said.
        let claude_warning = claude::revoke_warning(cleanup);
        let revoked = self.pairing.revoke(id).await?;
        Ok(Revoked {
            revoked,
            claude_warning,
        })
    }

    /// Set when the app opened without finding HQ's program. The window shows a card for it
    /// (see `HubReport::program`), so the text names only what was looked for.
    pub fn hq_problem(&self) -> Option<&str> {
        self.hq_problem.as_deref()
    }

    /// Whether a paired hub's web app replaces HQ on this computer.
    pub fn hub_web_active(&self) -> bool {
        self.config.bridge.hub_web_listen().is_some()
    }

    /// The `hq` this app runs.
    pub fn program(&self) -> PathBuf {
        self.hq_program.lock().unwrap().clone()
    }

    /// Checks a program the person picked and, if it is HQ, uses it from now on and remembers it.
    /// The refusal is one of `refused`, `not_hq` and `not_found`.
    pub async fn pick_program(&self, path: PathBuf) -> Result<chooser::Picked, chooser::Refusal> {
        let picked = tokio::task::spawn_blocking(move || chooser::check_program(&path))
            .await
            .map_err(|_| chooser::Refusal::NotHq)??;
        let mut settings = self.settings();
        settings.hq_path = Some(picked.path.clone());
        self.save_settings(settings)
            .map_err(|_| chooser::Refusal::Refused)?;
        self.host.set_program(picked.path.clone());
        *self.hq_program.lock().unwrap() = picked.path.clone();
        self.program_found
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(picked)
    }

    /// Everything the This computer tab words itself from.
    pub async fn report(&self) -> HubReport {
        let settings = self.settings();
        let facts = self.host.facts(settings.usual_start_secs).await;
        if let Some(secs) = self.host.take_finished_start_secs() {
            let mut s = self.settings();
            s.recent_start_secs.push(secs);
            let skip = s
                .recent_start_secs
                .len()
                .saturating_sub(host::RECENT_STARTS);
            s.recent_start_secs.drain(..skip);
            s.usual_start_secs = host::median(&s.recent_start_secs);
            let _ = self.save_settings(s);
        }
        let dir = hq_core::config::native_host_dir();
        let helpers = self.agents().await;
        let lockdown = hq_host::read_lockdown(&dir).map(|l| LockdownView {
            reason: l.reason,
            source: l.source,
            at: l.at,
        });
        let found = self.program_found.load(std::sync::atomic::Ordering::SeqCst);
        let program = found.then(|| {
            let path = self.program();
            let version = host::hq_version_within(&path, host::VERSION_TIMEOUT);
            ProgramView { path, version }
        });
        let free_bytes = free_bytes(&HqConfig::hq_dir());
        let attention = |need| {
            helpers
                .agents
                .iter()
                .filter(move |a| a.need == need)
                .map(|a| a.name.clone())
                .collect::<Vec<_>>()
        };
        HubReport {
            usual_secs: settings.usual_start_secs,
            host: facts,
            program,
            serves_hub: self.config.bridge.serves_hub(),
            hub_web: self.config.bridge.hub_web_listen().is_some(),
            free_bytes,
            almost_full: free_bytes.is_some_and(|b| b < host::ALMOST_FULL_BYTES),
            lockdown,
            helpers: HelpersSummary {
                reachable: helpers.reachable,
                working: attention(agents::Need::Working),
                needs_you: attention(agents::Need::Blocked),
            },
        }
    }

    /// The last lines of the host's log (keys hidden) for the log sheet and Copy details.
    pub async fn host_log(&self) -> HostLog {
        let path = self.host.log_path().to_path_buf();
        let lines = tokio::task::spawn_blocking(move || host::read_log_tail(&path))
            .await
            .unwrap_or_default();
        let program = self.program();
        let version = tokio::task::spawn_blocking(move || {
            host::hq_version_within(&program, host::VERSION_TIMEOUT)
        })
        .await
        .ok()
        .flatten();
        let exit_code = match self.host.status().await {
            host::HostStatus::Exited { code } => code,
            _ => None,
        };
        HostLog {
            path: self.host.log_path().to_path_buf(),
            lines,
            version,
            exit_code,
        }
    }

    /// Lets coding helpers start again after a lockdown by running `hq host unlock`. It runs the
    /// person's own `hq` with no arguments from the page, and never touches the lockdown file.
    pub async fn host_unlock(&self) -> Result<()> {
        let program = self.program();
        let out = tokio::task::spawn_blocking(move || {
            host::run_within(&program, &["host", "unlock"], host::VERSION_TIMEOUT)
        })
        .await?;
        if out.is_none() {
            bail!("hq host unlock did not finish");
        }
        Ok(())
    }

    /// Points Claude Desktop entries written before the rename at the program's new place.
    /// Runs once; returns the entries it changed.
    pub async fn rewrite_after_rename(&self) -> Vec<String> {
        if self.settings().bundle_rename_handled {
            return Vec::new();
        }
        let program = self.program();
        let Some(config) = hq_bridge::claude::desktop_config_path() else {
            return Vec::new();
        };
        let db_path = self.config.db_path();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let changed = tokio::task::spawn_blocking(move || {
            let hubs: Vec<String> = Database::open(&db_path)
                .ok()
                .and_then(|db| hq_bridge::claude::hubs(&db).ok())
                .map(|h| h.into_iter().map(|h| h.name).collect())
                .unwrap_or_default();
            rename::rewrite_claude_desktop(&config, &program, &hubs, &today)
        })
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
        let mut s = self.settings();
        s.bundle_rename_handled = true;
        let _ = self.save_settings(s);
        changed
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    /// Saves settings. Ports, relay and name take effect the next time the app opens.
    pub fn save_settings(&self, new: Settings) -> Result<()> {
        new.save(&self.dir)?;
        *self.settings.lock().unwrap() = new;
        Ok(())
    }

    /// Starts the loopback proxy if it is not running and returns its origin. With a paired hub,
    /// only while this app holds the hub's web forward itself: a port another program holds
    /// would get the window's sign-in, so it is refused, never used as a fallback.
    pub async fn proxy_origin(&self) -> Result<String> {
        let mut slot = self.proxy.lock().await;
        let hub = self.config.bridge.hub_web_listen();
        let held = hub.map(|listen| (listen, self.pairing.holds_forward(listen)));
        if let Some((listen, ours)) = held
            && ours != Some(true)
        {
            if let Some(running) = slot.take() {
                running.shutdown().await;
            }
            let held_elsewhere = ours == Some(false);
            return Err(ForwardNotOurs {
                listen,
                held_elsewhere,
            }
            .into());
        }
        if slot.is_none() {
            let port = self.settings().proxy_port;
            let gate = hub.map(|listen| self.pairing.forward_gate(listen));
            *slot = Some(
                proxy::start_gated(
                    ProxyConfig {
                        listen: SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port),
                        upstream: self.upstream,
                    },
                    gate,
                )
                .await?,
            );
        }
        Ok(slot.as_ref().expect("just set").origin())
    }

    /// The address of the main window: the proxy, with this machine's web token in the fragment so
    /// its own HQ signs itself in. The fragment never goes to the proxy or HQ; it stays in the
    /// window. A paired hub has its own token, so the person signs in to it once in the window.
    pub async fn main_window_url(&self) -> Result<String> {
        self.main_window_url_at("/").await
    }

    /// The main window's address at one page of HQ's web app (see `window::main_path_allowed`).
    /// A page that is not allowed is refused before the proxy starts.
    pub async fn main_window_url_at(&self, path: &str) -> Result<String> {
        let Some(path) = window::main_path_allowed(path) else {
            bail!("that page cannot be opened from here");
        };
        let origin = self.proxy_origin().await?;
        let local_token = self
            .config
            .web_auth_token
            .as_deref()
            .filter(|t| !t.trim().is_empty() && self.config.bridge.hub_web_listen().is_none());
        Ok(match local_token {
            Some(token) => format!("{origin}{path}#token={token}"),
            None => format!("{origin}{path}"),
        })
    }

    /// Runs once when the app opens: starts the host only if the person's setting says so.
    pub async fn on_launch(&self) {
        if self.settings().start_host_on_launch
            && let Err(e) = self.host.start().await
        {
            tracing::warn!("could not start the host at launch: {e}");
        }
    }

    /// Runs when the app quits: closes what it opened, including the host it started. The three
    /// closes run side by side and each is bounded, so an open event stream or a slow relay
    /// cannot keep the host from being stopped.
    pub async fn shutdown(&self) {
        let _ = tokio::time::timeout(SHUTDOWN_LIMIT, self.close_everything()).await;
    }

    async fn close_everything(&self) {
        let host = async {
            let _ = self.host.stop_for_good().await;
        };
        let pairing = async {
            let _ = tokio::time::timeout(SHUTDOWN_STEP, self.pairing.stop()).await;
        };
        let proxy = async {
            let open = self.proxy.lock().await.take();
            if let Some(proxy) = open {
                proxy.shutdown().await;
            }
        };
        tokio::join!(host, pairing, proxy);
    }
}

/// Resolves when this process is asked to end (SIGTERM or SIGINT), so the app can close what it
/// opened instead of leaving its host behind.
#[cfg(unix)]
pub async fn wait_for_termination() {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut term), Ok(mut int)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) else {
        return std::future::pending().await;
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
}

#[cfg(not(unix))]
pub async fn wait_for_termination() {
    std::future::pending::<()>().await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(bind: &str) -> HqConfig {
        HqConfig {
            web_bind: bind.into(),
            ws_port: 6001,
            ..HqConfig::default()
        }
    }

    #[test]
    fn the_app_reaches_hq_over_loopback_or_not_at_all() {
        for ok in ["127.0.0.1", "0.0.0.0", "::"] {
            assert_eq!(
                upstream_addr(&config(ok)).unwrap(),
                "127.0.0.1:6001".parse().unwrap(),
                "{ok}"
            );
        }
        for bad in [
            "192.168.1.9",
            "100.64.0.5",
            "hq.example.com",
            "::1",
            "127.0.0.2",
        ] {
            assert!(upstream_addr(&config(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_window_shows_the_paired_hub_when_there_is_a_web_forward() {
        let forward = |service: &str, listen: &str| hq_core::config::BridgeForward {
            peer: "abc".into(),
            service: service.into(),
            listen: listen.into(),
            fail_fast_secs: None,
            when: Default::default(),
        };
        let mut with_hub = config("127.0.0.1");
        with_hub.bridge.forwards = vec![
            forward("gate", "127.0.0.1:47490"),
            forward("web", "127.0.0.1:47480"),
        ];
        assert_eq!(
            upstream_addr(&with_hub).unwrap(),
            "127.0.0.1:47480".parse().unwrap()
        );
        // The hub replaces the local HQ, so a local bind the app cannot use no longer matters.
        with_hub.web_bind = "192.168.1.9".into();
        assert!(upstream_addr(&with_hub).is_ok());

        let mut other_forwards_only = config("127.0.0.1");
        other_forwards_only.bridge.forwards = vec![forward("gate", "127.0.0.1:47490")];
        assert_eq!(
            upstream_addr(&other_forwards_only).unwrap(),
            "127.0.0.1:6001".parse().unwrap()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_termination_signal_ends_the_wait() {
        let waiting = tokio::spawn(wait_for_termination());
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        // SAFETY: raises a signal the wait above has just installed a handler for.
        unsafe {
            libc::raise(libc::SIGTERM);
        }
        tokio::time::timeout(std::time::Duration::from_secs(3), waiting)
            .await
            .expect("the wait did not end on SIGTERM")
            .unwrap();
    }

    #[tokio::test]
    async fn a_missing_hq_is_reported_when_the_app_opens() {
        let dir = tempfile::tempdir().unwrap();
        let app_dir = dir.path().join("desktop");
        let missing = dir.path().join("nowhere").join("hq");
        Settings {
            hq_path: Some(missing.clone()),
            ..Settings::default()
        }
        .save(&app_dir)
        .unwrap();
        let desktop = Desktop::open(DesktopInit {
            dir: app_dir,
            bridge_key: dir.path().join("bridge").join("bridge.key"),
            config: HqConfig {
                ws_port: crate::host::tests_support_unused_port(),
                ..HqConfig::default()
            },
            app_dir: None,
            home: None,
            path_var: None,
            login_shell: None,
        })
        .unwrap();
        let problem = desktop.hq_problem().expect("a missing hq is reported");
        assert!(problem.contains(&missing.display().to_string()));
        assert!(!problem.contains("Settings"));
        assert!(desktop.report().await.program.is_none());
    }

    #[tokio::test]
    async fn opening_the_app_starts_nothing_and_saves_nothing_it_was_not_told_to() {
        let dir = tempfile::tempdir().unwrap();
        let desktop = Desktop::open(DesktopInit {
            dir: dir.path().join("desktop"),
            bridge_key: dir.path().join("bridge").join("bridge.key"),
            config: HqConfig {
                ws_port: crate::host::tests_support_unused_port(),
                ..HqConfig::default()
            },
            app_dir: None,
            home: None,
            path_var: None,
            login_shell: None,
        })
        .unwrap();
        assert!(!desktop.settings().start_host_on_launch);
        desktop.on_launch().await;
        assert_eq!(
            desktop.host.status().await,
            crate::host::HostStatus::Stopped
        );
        assert!(!desktop.pairing.snapshot().await.running);
        assert!(desktop.proxy.lock().await.is_none());
        assert!(!dir.path().join("bridge").exists());
        assert!(!dir.path().join("desktop").join("settings.json").exists());
    }

    /// A hub web forward on a port another program holds: the window is refused and that
    /// program is never sent a request (or the sign-in that would ride on it).
    #[tokio::test]
    async fn a_hub_forward_this_app_does_not_hold_is_refused_not_proxied() {
        let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        held.set_nonblocking(true).unwrap();
        let listen = held.local_addr().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut config = HqConfig {
            ws_port: crate::host::tests_support_unused_port(),
            ..HqConfig::default()
        };
        config.bridge.forwards = vec![hq_core::config::BridgeForward {
            peer: "phone".into(),
            service: "web".into(),
            listen: listen.to_string(),
        }];
        let desktop = Desktop::open(DesktopInit {
            dir: dir.path().join("desktop"),
            bridge_key: dir.path().join("bridge").join("bridge.key"),
            config,
            app_dir: None,
            home: None,
            path_var: None,
            login_shell: None,
        })
        .unwrap();
        let err = desktop.proxy_origin().await.unwrap_err();
        let refused = err
            .downcast_ref::<ForwardNotOurs>()
            .expect("ForwardNotOurs");
        assert_eq!(refused.listen, listen);
        assert!(desktop.proxy.lock().await.is_none(), "no proxy was started");
        assert!(desktop.main_window_url().await.is_err());
        assert!(held.accept().is_err(), "nothing reached the other program");
    }

    fn plain_desktop(dir: &Path, program: Option<&Path>) -> Desktop {
        if let Some(program) = program {
            Settings {
                hq_path: Some(program.to_path_buf()),
                ..Settings::default()
            }
            .save(&dir.join("desktop"))
            .unwrap();
        }
        Desktop::open(DesktopInit {
            dir: dir.join("desktop"),
            bridge_key: dir.join("bridge").join("bridge.key"),
            config: HqConfig {
                ws_port: crate::host::tests_support_unused_port(),
                ..HqConfig::default()
            },
            app_dir: None,
            home: None,
            path_var: None,
            login_shell: None,
        })
        .unwrap()
    }

    fn fake_hq(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("hq");
        let record = dir.join("args.txt");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'hq 0.0.0'; else echo \"$@\" >> {}; fi\n",
                record.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[tokio::test]
    async fn a_page_the_window_may_not_open_is_refused_before_anything_starts() {
        let dir = tempfile::tempdir().unwrap();
        let desktop = plain_desktop(dir.path(), None);
        let mut settings = desktop.settings();
        settings.proxy_port = crate::host::tests_support_unused_port();
        desktop.save_settings(settings).unwrap();
        for bad in [
            "https://example.com",
            "/helpers/..",
            "/settings",
            "/chat?x=1",
        ] {
            assert!(desktop.main_window_url_at(bad).await.is_err(), "{bad}");
        }
        assert!(desktop.proxy.lock().await.is_none());
        let url = desktop.main_window_url_at("/helpers/a1").await.unwrap();
        assert!(url.ends_with("/helpers/a1"));
    }

    #[tokio::test]
    async fn unlock_runs_the_programs_own_host_unlock_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let hq = fake_hq(dir.path());
        let desktop = plain_desktop(dir.path(), Some(&hq));
        desktop.host_unlock().await.unwrap();
        let args = std::fs::read_to_string(dir.path().join("args.txt")).unwrap();
        assert_eq!(args.trim(), "host unlock");
    }

    #[tokio::test]
    async fn a_picked_program_is_checked_then_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let desktop = plain_desktop(dir.path(), None);
        assert_eq!(
            desktop
                .pick_program(PathBuf::from("/tmp/hq"))
                .await
                .unwrap_err(),
            chooser::Refusal::Refused
        );
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let hq = fake_hq(&bin);
        let picked = desktop.pick_program(hq.clone()).await.unwrap();
        assert_eq!(picked.version, "hq 0.0.0");
        assert_eq!(desktop.settings().hq_path, Some(hq.clone()));
        assert_eq!(desktop.report().await.program.unwrap().path, hq);
    }

    #[tokio::test]
    async fn the_report_says_a_paired_hub_replaces_hq_on_this_computer() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = HqConfig::default();
        config.bridge.forwards = vec![hq_core::config::BridgeForward {
            peer: "phone".into(),
            service: "web".into(),
            listen: "127.0.0.1:47480".into(),
        }];
        config.bridge.services = [("gate".to_string(), "127.0.0.1:7747".to_string())].into();
        let desktop = Desktop::open(DesktopInit {
            dir: dir.path().join("desktop"),
            bridge_key: dir.path().join("bridge").join("bridge.key"),
            config,
            app_dir: None,
            home: None,
            path_var: None,
            login_shell: None,
        })
        .unwrap();
        let report = desktop.report().await;
        assert!(report.hub_web && report.serves_hub);
    }
}
