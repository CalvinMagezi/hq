//! `hq web`: host the web UI with one command.
//!
//! `hq web` starts the web server on its own (no daemon, no relays), finds or
//! builds the UI bundle, and opens the browser. It is safe to run twice: if an
//! HQ web server already answers on the port it reports that instead of
//! failing. `--detach` runs it in the background; `hq web status` and
//! `hq web stop` manage that instance. `--json` prints one machine-readable
//! object, so an agent that set HQ up for someone can read back the URL.

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use hq_core::config::HqConfig;
use hq_db::Database;
use hq_vault::VaultClient;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Args, Debug)]
pub struct WebArgs {
    #[command(flatten)]
    start: StartArgs,
    #[command(subcommand)]
    action: Option<WebAction>,
}

impl WebArgs {
    /// Whether this invocation serves the web app (as opposed to `status` or `stop`).
    pub fn serves(&self) -> bool {
        self.action.is_none()
    }
}

#[derive(Args, Debug, Default)]
struct StartArgs {
    /// Port to listen on (default: `ws_port` from config, 5678)
    #[arg(short, long)]
    port: Option<u16>,
    /// Address to bind (default: `web_bind` from config, 127.0.0.1). A
    /// non-loopback address makes `hq web` generate and require a token.
    #[arg(short, long)]
    bind: Option<String>,
    /// Shorthand for `--bind 0.0.0.0`: reach the UI from a phone or another machine
    #[arg(long, conflicts_with = "bind")]
    lan: bool,
    /// Do not open a browser
    #[arg(long)]
    no_open: bool,
    /// Run in the background and return once the server answers
    #[arg(short, long)]
    detach: bool,
    /// Build the web UI from this source checkout first (needs bun)
    #[arg(long)]
    build: bool,
    /// Print one JSON object instead of text; never opens a browser. Implies
    /// `--detach`, so the call returns instead of serving forever.
    #[arg(long)]
    json: bool,
    /// Internal: set on the background child so its log never holds the token
    #[arg(long, hide = true)]
    supervised: bool,
}

impl StartArgs {
    fn detach(&self) -> bool {
        self.detach || self.json
    }

    /// The user named an address, as opposed to taking the configured one.
    fn names_bind(&self) -> bool {
        self.lan || self.bind.is_some()
    }
}

#[derive(Subcommand, Debug)]
enum WebAction {
    /// Show whether the web server is up
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Stop a web server started with `hq web --detach`
    Stop {
        #[arg(long)]
        json: bool,
    },
}

/// What `hq web` writes to `~/.hq/web.json` while a server it started runs.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct WebState {
    pid: u32,
    port: u16,
    bind: String,
}

/// The `--json` payload of a start, and the source of the text output.
#[derive(Debug, Serialize, PartialEq)]
struct Report {
    ok: bool,
    /// An HQ web server was already listening, so nothing new was started.
    already_running: bool,
    url: String,
    /// `url` plus the `#token=` login fragment when a token is required.
    login_url: String,
    port: u16,
    bind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    network_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    static_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    log: Option<String>,
}

pub async fn run(config: &HqConfig, args: WebArgs) -> Result<()> {
    match args.action {
        Some(WebAction::Status { json }) => status(config, json).await,
        Some(WebAction::Stop { json }) => stop(config, json),
        None => {
            let json = args.start.json;
            match start(config, args.start).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({ "ok": false, "error": format!("{e:#}") })
                        );
                    }
                    Err(e)
                }
            }
        }
    }
}

async fn start(config: &HqConfig, mut args: StartArgs) -> Result<()> {
    args.detach = args.detach();
    let port = args.port.unwrap_or(config.ws_port);
    let bind = if args.lan {
        "0.0.0.0".to_string()
    } else {
        args.bind.clone().unwrap_or_else(|| config.web_bind.clone())
    };

    // Ask before doing any work: an instance that is up needs no UI build or token.
    match probe(probe_addr(&bind, port)?).await {
        Probe::Hq => return report_running(config, &args, &bind, port),
        Probe::Other => bail!(
            "port {port} is taken by something that is not HQ. Pick another with `hq web --port <n>`."
        ),
        Probe::Free => {}
    }

    let token = resolve_token(&bind, config.web_auth_token.as_deref())?;
    // Same rule `hq start` applies; with the token above it only fails on a blank one.
    hq_web::auth::check_web_bind(&bind, token.as_deref()).map_err(anyhow::Error::msg)?;

    let static_dir = ensure_ui(config, args.build, args.json)?;
    let mut report = Report::new(&bind, port, token, Some(&static_dir));

    if args.detach {
        let log = log_path();
        report.log = Some(log.display().to_string());
        report.pid = Some(
            spawn_detached(&bind, port, args.lan, &static_dir, &config.vault_path, &log).await?,
        );
        return finish(report, &args);
    }

    let vault =
        Arc::new(VaultClient::new(config.vault_path.clone()).context("failed to open vault")?);
    let db = Arc::new(Database::open(&config.db_path()).context("failed to open database")?);
    hq_agent::install_ledger(db.clone());
    let state = super::start::build_web_state(
        config,
        &vault,
        &db,
        Some(static_dir.clone()),
        report.token.clone(),
        &bind,
    );
    let addr = listen_addr(&bind, port)?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("could not listen on {addr}"))?;

    report.pid = Some(std::process::id());
    write_state(&WebState {
        pid: std::process::id(),
        port,
        bind: bind.clone(),
    });
    finish(report, &args)?;

    let app = hq_web::create_router(state);
    let served = tokio::select! {
        r = axum::serve(listener, app) => r.context("web server stopped"),
        () = hq_agent::shutdown::wait_for_shutdown_signal() => Ok(()),
    };
    let _ = std::fs::remove_file(state_path());
    served
}

/// An HQ server already answers on `port`: describe it as it is, not as this
/// run's flags would have made it. Asking for a different address than the
/// running one is refused rather than silently ignored.
fn report_running(config: &HqConfig, args: &StartArgs, bind: &str, port: u16) -> Result<()> {
    let ours = read_state().filter(|s| s.port == port && super::stop::is_alive(s.pid));
    let running = ours
        .as_ref()
        .map_or_else(|| config.web_bind.clone(), |s| s.bind.clone());
    if args.names_bind() && !same_bind(&running, bind) {
        bail!(
            "HQ web is already running on {running}:{port}, not {bind}. Run `hq web stop` first \
             (or `hq stop` if it came from `hq start all`), then start it again."
        );
    }
    // A server `hq web` did not start (`hq start`, systemd) may hold a token we cannot see;
    // minting a new one here would print a login link that never works.
    let token = if ours.is_some() {
        resolve_token(&running, config.web_auth_token.as_deref())?
    } else {
        config
            .web_auth_token
            .as_deref()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
    };
    let mut report = Report::new(&running, port, token, None);
    report.already_running = true;
    finish(report, args)
}

/// Prints the report (text or JSON) and opens the browser when appropriate.
fn finish(report: Report, args: &StartArgs) -> Result<()> {
    if args.json {
        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }
    print!("{}", render_text(&report, args.detach, args.supervised));
    if !args.no_open && !args.supervised && can_open_browser() {
        open_browser(&report.login_url);
    }
    Ok(())
}

/// The human-readable report. The supervised background child writes it to a
/// log file, so it never includes the token.
fn render_text(report: &Report, detach: bool, supervised: bool) -> String {
    let mut out = String::new();
    if supervised {
        out.push_str(&format!("hq web serving on {}\n", report.url));
        return out;
    }
    if report.already_running {
        out.push_str("HQ web is already running.\n");
    } else if detach {
        out.push_str(&format!(
            "HQ web is running in the background (pid {}).\n",
            report.pid.unwrap_or(0)
        ));
    } else {
        out.push_str("HQ web is starting. Press Ctrl+C to stop.\n");
    }
    out.push_str(&format!("  Local:   {}\n", report.url));
    if let Some(net) = &report.network_url {
        out.push_str(&format!("  Network: {net}\n"));
    }
    if let Some(token) = &report.token {
        out.push_str(&format!("  Token:   {token}\n"));
        out.push_str(&format!(
            "  Sign-in link (keep it private): {}\n",
            report.login_url
        ));
    }
    if let Some(log) = &report.log {
        out.push_str(&format!("  Log:     {log}\n"));
    }
    if detach && !report.already_running {
        out.push_str("Stop it with `hq web stop`.\n");
    }
    out
}

impl Report {
    fn new(bind: &str, port: u16, token: Option<String>, static_dir: Option<&Path>) -> Self {
        let url = format!("http://{}:{port}", local_host(bind));
        let login_url = match &token {
            Some(t) => format!("{url}/#token={t}"),
            None => url.clone(),
        };
        let network_url = is_open_bind(bind).then(|| {
            let host = if bind == "0.0.0.0" || bind == "::" {
                lan_ip().unwrap_or_else(|| bind.to_string())
            } else {
                bind.to_string()
            };
            let host = if host.contains(':') {
                format!("[{host}]")
            } else {
                host
            };
            format!("http://{host}:{port}")
        });
        Report {
            ok: true,
            already_running: false,
            url,
            login_url,
            port,
            bind: bind.to_string(),
            token,
            pid: None,
            network_url,
            static_dir: static_dir.map(|d| d.display().to_string()),
            log: None,
        }
    }
}

async fn status(config: &HqConfig, json: bool) -> Result<()> {
    let state = read_state();
    let port = state.as_ref().map(|s| s.port).unwrap_or(config.ws_port);
    let bind = state
        .as_ref()
        .map(|s| s.bind.clone())
        .unwrap_or_else(|| config.web_bind.clone());
    let running = match probe_addr(&bind, port) {
        Ok(addr) => matches!(probe(addr).await, Probe::Hq),
        Err(_) => false,
    };
    let url = format!("http://{}:{port}", local_host(&bind));
    let pid = state.filter(|s| is_our_server(s.pid)).map(|s| s.pid);
    if json {
        println!(
            "{}",
            serde_json::json!({ "running": running, "url": url, "port": port, "pid": pid })
        );
    } else if running {
        println!(
            "HQ web is running at {url}{}",
            pid.map(|p| format!(" (pid {p})")).unwrap_or_default()
        );
    } else {
        println!("HQ web is not running. Start it with `hq web`.");
    }
    // Exit 1 when down so scripts and agents can branch on it.
    if running {
        Ok(())
    } else {
        std::process::exit(1)
    }
}

fn stop(_config: &HqConfig, json: bool) -> Result<()> {
    // The pid in the state file may have been reused by an unrelated process
    // since the server died, so only a process that still looks like `hq web` is signalled.
    let stopped = match read_state() {
        Some(s) if is_our_server(s.pid) => {
            super::stop::kill_tree(s.pid);
            true
        }
        _ => false,
    };
    let _ = std::fs::remove_file(state_path());
    if json {
        println!("{}", serde_json::json!({ "stopped": stopped }));
    } else if stopped {
        println!("Stopped HQ web.");
    } else {
        println!("No `hq web` server of this user is running. (`hq stop` handles `hq start all`.)");
    }
    Ok(())
}

// ─── UI bundle ──────────────────────────────────────────────────────────

/// The directory holding the built UI.
///
/// A build that already exists is always reused. Building runs `bun install`,
/// which executes package scripts, so it only happens without being asked for
/// the checkout the configured vault lives in (the user's own HQ), or for the
/// checkout in the current directory when `--build` says so.
fn ensure_ui(config: &HqConfig, build: bool, quiet: bool) -> Result<PathBuf> {
    let root = config.vault_path.parent().unwrap_or(&config.vault_path);
    let configured = config.web_static_dir.clone();
    let dir = configured
        .clone()
        .unwrap_or_else(|| super::start::default_static_dir(root));
    let trusted = checkout_from(root);
    let local = trusted.clone().or_else(|| {
        std::env::current_dir()
            .ok()
            .and_then(|cwd| checkout_from(&cwd))
    });

    if !build {
        if dir.join("index.html").exists() {
            return Ok(dir);
        }
        // A configured directory is taken as is; otherwise reuse a checkout's earlier build.
        if configured.is_none() {
            let built = local.as_ref().map(|c| c.join(BUILT_UI));
            if let Some(built) = built.filter(|b| b.join("index.html").exists()) {
                return Ok(built);
            }
        }
    }
    let to_build = if build {
        local.as_ref()
    } else {
        trusted.as_ref()
    };
    let Some(source) = to_build else {
        let hint = match &local {
            Some(c) => format!(
                "A checkout is at {}; run `hq web --build` to build its UI.",
                c.display()
            ),
            None => "Run `hq update --apply` to install the released web files, or run \
                     `hq web --build` from a checkout, or point `web_static_dir` at a built \
                     `apps/hq-web/dist/client`."
                .to_string(),
        };
        bail!(
            "no web UI build found (looked for {}/index.html). {hint}",
            dir.display()
        );
    };
    if which("bun").is_none() {
        bail!(
            "the web UI is not built and bun is not installed. Install bun (https://bun.sh), then \
             run `hq web --build`; or run `hq update --apply` to get the released web files."
        );
    }
    let app = source.join("apps").join("hq-web");
    if !quiet {
        println!(
            "Building the web UI in {} (first run takes a minute)...",
            app.display()
        );
    }
    // --frozen-lockfile: install exactly what the lockfile pins, never newer.
    for step in [&["install", "--frozen-lockfile"][..], &["run", "build"][..]] {
        let status = std::process::Command::new("bun")
            .args(step)
            .current_dir(&app)
            // Build output must not corrupt the one JSON object on stdout.
            .stdout(if quiet {
                std::process::Stdio::null()
            } else {
                std::process::Stdio::inherit()
            })
            .status()
            .context("failed to run bun")?;
        if !status.success() {
            bail!("`bun {}` failed in {}", step.join(" "), app.display());
        }
    }
    // A configured directory wins; otherwise serve the fresh checkout build.
    let dir = configured.unwrap_or_else(|| source.join(BUILT_UI));
    if !dir.join("index.html").exists() {
        bail!(
            "the build finished but {}/index.html is missing",
            dir.display()
        );
    }
    Ok(dir)
}

/// Where `bun run build` leaves the UI, relative to a checkout root.
const BUILT_UI: &str = "apps/hq-web/dist/client";

/// The nearest directory at or above `start` that holds `apps/hq-web/package.json`.
fn checkout_from(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|d| d.join("apps/hq-web/package.json").is_file())
        .map(Path::to_path_buf)
}

fn which(program: &str) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{program}.exe"), format!("{program}.cmd"), program.to_string()]
    } else {
        vec![program.to_string()]
    };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find(|p| p.is_file())
}

// ─── Background server ──────────────────────────────────────────────────

/// Re-runs this binary as `hq web` in its own process group with output going
/// to `log`, then waits until it answers `/health`.
async fn spawn_detached(
    bind: &str,
    port: u16,
    lan: bool,
    static_dir: &Path,
    vault: &Path,
    log: &Path,
) -> Result<u32> {
    let out = open_log(log)?;
    let addr = probe_addr(bind, port)?;
    let mut cmd =
        std::process::Command::new(std::env::current_exe().context("cannot find the hq binary")?);
    cmd.args([
        "web",
        "--no-open",
        "--supervised",
        "--port",
        &port.to_string(),
    ]);
    // The child serves the directory the parent settled on, wherever it started from.
    cmd.env("HQ_WEB_STATIC_DIR", static_dir);
    // `--vault` is an in-memory override, so a fresh child would load the default vault.
    cmd.env("HQ_VAULT_PATH", vault);
    if lan {
        cmd.arg("--lan");
    } else {
        cmd.args(["--bind", bind]);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(out);
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    // No console window, and the server outlives the terminal that started it.
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000 | 0x0000_0200);
    #[cfg(windows)]
    stop_inheriting_std_handles();
    let mut child = cmd
        .spawn()
        .context("failed to start the background server")?;

    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if matches!(probe(addr).await, Probe::Hq) {
            return Ok(child.id());
        }
        if let Ok(Some(status)) = child.try_wait() {
            bail!(
                "the background server exited ({status}). See {}",
                log.display()
            );
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    super::stop::kill_tree(child.id());
    bail!(
        "the background server did not answer within 20s. See {}",
        log.display()
    )
}

/// Windows hands every inheritable handle to a new process, not only the ones named for it, so
/// the background server would hold the pipes this command's own output goes to. A caller that
/// reads those pipes (a script, an agent's shell tool) would then wait until the server exits.
/// The log file given to the child is passed explicitly and is unaffected.
#[cfg(windows)]
fn stop_inheriting_std_handles() {
    use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: plain Win32 calls on this process's own standard handles; a null or invalid
        // handle makes the call fail harmlessly.
        unsafe {
            let handle = GetStdHandle(which);
            if !handle.is_null() {
                SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

// ─── Small helpers ──────────────────────────────────────────────────────

enum Probe {
    Hq,
    Other,
    Free,
}

/// Is an HQ server listening on `addr`? Only an `agent-hq` `/health` counts,
/// so another program on the port is never mistaken for HQ.
async fn probe(addr: SocketAddr) -> Probe {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .no_proxy()
        .build()
    {
        Ok(c) => c,
        Err(_) => return Probe::Free,
    };
    match client.get(format!("http://{addr}/health")).send().await {
        Ok(resp) => {
            let is_hq = resp
                .json::<serde_json::Value>()
                .await
                .ok()
                .is_some_and(|v| v["service"] == "agent-hq");
            if is_hq { Probe::Hq } else { Probe::Other }
        }
        // Refused is the normal "free" case; a timeout means something is there, unless the port
        // can be taken: Windows does not refuse a closed local port at once but retries for
        // longer than the timeout.
        Err(e) if e.is_timeout() => {
            if std::net::TcpListener::bind(addr).is_ok() { Probe::Free } else { Probe::Other }
        }
        Err(_) => Probe::Free,
    }
}

/// Whether `bind` is reachable from other machines, by the same rule `hq start`
/// uses to demand a token. Keeping one definition means the two cannot drift.
fn is_open_bind(bind: &str) -> bool {
    hq_web::auth::check_web_bind(bind, None).is_err()
}

/// The address to listen on. `localhost` is accepted because `web_bind` allows it.
fn listen_addr(bind: &str, port: u16) -> Result<SocketAddr> {
    let ip: IpAddr = if bind == "localhost" {
        Ipv4Addr::LOCALHOST.into()
    } else {
        bind.parse()
            .with_context(|| format!("'{bind}' is not an IP address (try 127.0.0.1 or 0.0.0.0)"))?
    };
    Ok(SocketAddr::new(ip, port))
}

/// Where a client on this machine reaches a server bound to `bind`: a wildcard
/// bind answers on loopback, a specific address only on itself.
fn probe_addr(bind: &str, port: u16) -> Result<SocketAddr> {
    let mut addr = listen_addr(bind, port)?;
    if addr.ip().is_unspecified() {
        addr.set_ip(match addr.ip() {
            IpAddr::V4(_) => Ipv4Addr::LOCALHOST.into(),
            IpAddr::V6(_) => Ipv6Addr::LOCALHOST.into(),
        });
    }
    Ok(addr)
}

/// Host to show for the local URL: `localhost` unless the server only answers
/// on one specific address.
fn local_host(bind: &str) -> String {
    match listen_addr(bind, 0).map(|a| a.ip()) {
        // Only the exact loopback addresses are what "localhost" resolves to; 127.0.0.2 is not.
        Ok(ip) if ip.is_unspecified() || ip == Ipv4Addr::LOCALHOST || ip == Ipv6Addr::LOCALHOST => {
            "localhost".to_string()
        }
        Ok(IpAddr::V6(v6)) => format!("[{v6}]"),
        Ok(ip) => ip.to_string(),
        Err(_) => "localhost".to_string(),
    }
}

fn same_bind(a: &str, b: &str) -> bool {
    match (listen_addr(a, 0), listen_addr(b, 0)) {
        (Ok(x), Ok(y)) => x.ip() == y.ip(),
        _ => a == b,
    }
}

/// Is `pid` still a server this command started? `kill -0` alone would accept
/// any process that inherited a dead server's pid.
fn is_our_server(pid: u32) -> bool {
    if !super::stop::is_alive(pid) {
        return false;
    }
    #[cfg(unix)]
    {
        std::process::Command::new("ps")
            .args(["-o", "command=", "-p", &pid.to_string()])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .is_some_and(|cmd| looks_like_hq_web(&cmd))
    }
    // Windows has no command-line query that needs no extra tools, so the image name has to do:
    // a reused pid would have to be another process with this program's own file name.
    #[cfg(not(unix))]
    {
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .is_some_and(|out| {
                let me = std::env::current_exe()
                    .ok()
                    .and_then(|e| e.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()))
                    .unwrap_or_else(|| "hq.exe".into());
                out.to_ascii_lowercase().starts_with(&format!("\"{me}\""))
            })
    }
}

/// `…/hq web …` (or its `pwa`/`dashboard` aliases) as a process command line.
fn looks_like_hq_web(cmdline: &str) -> bool {
    // The executable path may contain spaces, so grow it word by word until it names `hq`.
    let words: Vec<&str> = cmdline.split_whitespace().collect();
    (1..=words.len()).any(|end| {
        Path::new(&words[..end].join(" "))
            .file_name()
            .is_some_and(|n| n == "hq")
            && words[end..]
                .iter()
                .any(|w| matches!(*w, "web" | "pwa" | "dashboard"))
    })
}

/// The supervised child's log. It is created owner-only and tightened if an
/// older version left it readable, since the log is for the owner alone.
fn open_log(log: &Path) -> Result<std::fs::File> {
    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let file = opts.open(log)?;
    #[cfg(unix)]
    std::fs::set_permissions(log, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    Ok(file)
}

/// The token a server on `bind` needs: the configured one, else for a
/// non-loopback bind a generated one kept in `~/.hq/web.token` so the sign-in
/// link survives restarts. A loopback bind needs no token.
fn resolve_token(bind: &str, configured: Option<&str>) -> Result<Option<String>> {
    let configured = configured.map(str::trim).filter(|t| !t.is_empty());
    if let Some(t) = configured {
        return Ok(Some(t.to_string()));
    }
    if !is_open_bind(bind) {
        return Ok(None);
    }
    let path = hq_dir().join("web.token");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let existing = existing.trim();
        if existing.len() >= 32 {
            return Ok(Some(existing.to_string()));
        }
    }
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    write_private(&path, &token).context("could not save the generated web token")?;
    Ok(Some(token))
}

fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(path)?.write_all(contents.as_bytes())
}

/// Best-effort LAN address: the source address the OS would use to reach the
/// internet. Connecting a UDP socket sends nothing.
fn lan_ip() -> Option<String> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("203.0.113.1:9").ok()?;
    Some(sock.local_addr().ok()?.ip().to_string())
}

/// Where a detached `hq web` writes its log; `hq logs web` reads it.
pub(crate) fn log_path() -> PathBuf {
    hq_dir().join("logs").join("web.log")
}

fn hq_dir() -> PathBuf {
    HqConfig::hq_dir()
}

fn state_path() -> PathBuf {
    hq_dir().join("web.json")
}

fn write_state(state: &WebState) {
    if let Ok(json) = serde_json::to_string(state) {
        let _ = write_private(&state_path(), &json);
    }
}

fn read_state() -> Option<WebState> {
    serde_json::from_str(&std::fs::read_to_string(state_path()).ok()?).ok()
}

/// A browser can only open on a machine with a desktop, not over SSH or on a
/// headless server.
fn can_open_browser() -> bool {
    if std::env::var_os("SSH_CONNECTION").is_some() {
        return false;
    }
    cfg!(target_os = "macos")
        || cfg!(windows)
        || std::env::var_os("DISPLAY").is_some()
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(all(not(target_os = "macos"), not(windows)))]
    let opener = "xdg-open";
    #[cfg(windows)]
    let opener = "rundll32";
    let mut cmd = std::process::Command::new(opener);
    // `rundll32 url.dll,FileProtocolHandler <url>` opens the default browser without a shell, so
    // nothing in the address is read as a command.
    #[cfg(windows)]
    cmd.arg("url.dll,FileProtocolHandler");
    let _ = cmd
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(report: &Report, detach: bool, supervised: bool) -> String {
        render_text(report, detach, supervised)
    }

    #[test]
    fn loopback_report_has_no_token_or_network_url() {
        let r = Report::new("127.0.0.1", 5678, None, Some(Path::new("/ui")));
        assert_eq!(r.url, "http://localhost:5678");
        assert_eq!(r.login_url, r.url);
        assert!(r.network_url.is_none() && r.token.is_none());
    }

    #[test]
    fn ipv6_network_url_is_bracketed() {
        let r = Report::new("2001:db8::1", 9000, Some("t".into()), None);
        assert_eq!(r.network_url.as_deref(), Some("http://[2001:db8::1]:9000"));
    }

    #[test]
    fn token_goes_in_the_url_fragment_never_the_query() {
        let r = Report::new("0.0.0.0", 9000, Some("abc".into()), Some(Path::new("/ui")));
        assert_eq!(r.login_url, "http://localhost:9000/#token=abc");
        assert!(
            r.network_url
                .as_deref()
                .is_some_and(|u| u.ends_with(":9000"))
        );
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["token"], "abc");
        assert!(
            json.get("pid").is_none(),
            "unset options stay out of the JSON"
        );
    }

    #[test]
    fn configured_token_wins_and_loopback_needs_none() {
        assert_eq!(
            resolve_token("0.0.0.0", Some(" s3cret "))
                .unwrap()
                .as_deref(),
            Some("s3cret")
        );
        assert_eq!(resolve_token("127.0.0.1", None).unwrap(), None);
        assert_eq!(resolve_token("localhost", Some("  ")).unwrap(), None);
    }

    #[test]
    fn token_rule_is_the_one_hq_start_enforces() {
        // A bind that `hq start` would refuse without a token must get one here too.
        for bind in [
            "127.0.0.1",
            "::1",
            "localhost",
            "0.0.0.0",
            "::",
            "10.1.2.3",
            "127.0.0.2",
        ] {
            assert_eq!(
                is_open_bind(bind),
                hq_web::auth::check_web_bind(bind, None).is_err(),
                "{bind}"
            );
        }
    }

    #[test]
    fn state_file_round_trips() {
        let s = WebState {
            pid: 42,
            port: 5678,
            bind: "127.0.0.1".into(),
        };
        let back: WebState = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn localhost_and_ipv6_binds_parse() {
        assert_eq!(
            listen_addr("localhost", 80).unwrap().to_string(),
            "127.0.0.1:80"
        );
        assert_eq!(listen_addr("::1", 80).unwrap().to_string(), "[::1]:80");
        assert_eq!(
            listen_addr("0.0.0.0", 80).unwrap().to_string(),
            "0.0.0.0:80"
        );
        assert!(listen_addr("example.com", 80).is_err());
    }

    #[test]
    fn probing_a_wildcard_bind_uses_loopback_and_a_specific_bind_uses_itself() {
        // The background child was killed as "not answering" when this probed the wrong address.
        assert_eq!(probe_addr("0.0.0.0", 7).unwrap().to_string(), "127.0.0.1:7");
        assert_eq!(probe_addr("::", 7).unwrap().to_string(), "[::1]:7");
        assert_eq!(
            probe_addr("127.0.0.2", 7).unwrap().to_string(),
            "127.0.0.2:7"
        );
        assert_eq!(
            probe_addr("192.0.2.5", 7).unwrap().to_string(),
            "192.0.2.5:7"
        );
    }

    #[test]
    fn local_url_names_the_only_address_a_specific_bind_answers_on() {
        assert_eq!(local_host("127.0.0.1"), "localhost");
        assert_eq!(local_host("localhost"), "localhost");
        assert_eq!(local_host("0.0.0.0"), "localhost");
        assert_eq!(local_host("192.0.2.5"), "192.0.2.5");
        assert_eq!(
            local_host("127.0.0.2"),
            "127.0.0.2",
            "not what localhost resolves to"
        );
        assert_eq!(local_host("2001:db8::1"), "[2001:db8::1]");
    }

    #[test]
    fn same_bind_compares_addresses_not_spellings() {
        assert!(same_bind("localhost", "127.0.0.1"));
        assert!(!same_bind("127.0.0.1", "0.0.0.0"));
    }

    #[test]
    fn only_a_process_that_looks_like_hq_web_is_ours_to_stop() {
        assert!(looks_like_hq_web(
            "/usr/local/bin/hq web --no-open --port 5678"
        ));
        assert!(looks_like_hq_web("/home/u/bin/hq pwa"));
        assert!(!looks_like_hq_web("sleep 300"));
        assert!(
            !looks_like_hq_web("/usr/bin/vim web"),
            "right word, wrong program"
        );
        assert!(!looks_like_hq_web("/usr/local/bin/hq start all"));
        assert!(!looks_like_hq_web(""));
        assert!(looks_like_hq_web(
            "/opt/my tools/bin/hq web --no-open --supervised"
        ));
    }

    #[test]
    fn a_stale_pid_is_not_our_server() {
        // This test process is alive but is not `hq web`.
        assert!(!is_our_server(std::process::id()));
    }

    #[test]
    fn the_background_log_never_gets_the_token_and_is_owner_only() {
        let r = Report::new("0.0.0.0", 9000, Some("tok-123".into()), None);
        let child = text(&r, false, true);
        assert!(!child.contains("tok-123"), "{child}");
        let user = text(&r, true, false);
        assert!(
            user.contains("tok-123"),
            "the person who ran it does see it"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("logs/web.log");
            drop(open_log(&log).unwrap());
            assert_eq!(
                std::fs::metadata(&log).unwrap().permissions().mode() & 0o777,
                0o600
            );
            // A log an older version made world-readable is tightened.
            std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o644)).unwrap();
            drop(open_log(&log).unwrap());
            assert_eq!(
                std::fs::metadata(&log).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn json_implies_detach_so_an_agent_call_returns() {
        let json = StartArgs {
            json: true,
            ..Default::default()
        };
        assert!(json.detach());
        assert!(!StartArgs::default().detach());
        assert!(!StartArgs::default().names_bind());
        assert!(
            StartArgs {
                lan: true,
                ..Default::default()
            }
            .names_bind()
        );
    }

    #[test]
    fn already_running_report_omits_the_stop_hint_and_the_build_dir() {
        let mut r = Report::new("127.0.0.1", 5678, None, None);
        r.already_running = true;
        let out = text(&r, true, false);
        assert!(out.contains("already running") && !out.contains("hq web stop"));
        assert!(
            serde_json::to_value(&r)
                .unwrap()
                .get("static_dir")
                .is_none()
        );
    }

    #[test]
    fn checkout_is_found_by_its_web_app_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("apps/hq-web")).unwrap();
        std::fs::write(tmp.path().join("apps/hq-web/package.json"), "{}").unwrap();
        std::fs::create_dir_all(tmp.path().join(".vault")).unwrap();
        assert_eq!(checkout_from(tmp.path()).as_deref(), Some(tmp.path()));
        assert_eq!(
            checkout_from(&tmp.path().join(".vault")).as_deref(),
            Some(tmp.path())
        );
    }

    #[test]
    fn an_existing_build_is_reused_without_bun_or_a_rebuild() {
        let tmp = tempfile::tempdir().unwrap();
        let built = tmp.path().join(BUILT_UI);
        std::fs::create_dir_all(&built).unwrap();
        std::fs::write(built.join("index.html"), "x").unwrap();
        std::fs::create_dir_all(tmp.path().join("apps/hq-web")).unwrap();
        std::fs::write(tmp.path().join("apps/hq-web/package.json"), "{}").unwrap();
        let config = HqConfig {
            vault_path: tmp.path().join(".vault"),
            ..HqConfig::default()
        };
        assert_eq!(ensure_ui(&config, false, true).unwrap(), built);
    }

    #[test]
    fn a_configured_directory_is_never_replaced_by_a_checkout_build() {
        let tmp = tempfile::tempdir().unwrap();
        let ui = tmp.path().join("ui");
        std::fs::create_dir_all(&ui).unwrap();
        std::fs::write(ui.join("index.html"), "x").unwrap();
        let config = HqConfig {
            vault_path: tmp.path().join(".vault"),
            web_static_dir: Some(ui.clone()),
            ..HqConfig::default()
        };
        assert_eq!(ensure_ui(&config, false, true).unwrap(), ui);
    }

    #[tokio::test]
    async fn a_closed_port_probes_as_free() {
        let addr = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap()
        };
        assert!(matches!(probe(addr).await, Probe::Free));
    }

    #[tokio::test]
    async fn a_non_hq_listener_is_not_mistaken_for_hq() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 512];
                let _ = sock.read(&mut buf).await;
                let body = r#"{"service":"other"}"#;
                let _ = sock
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await;
            }
        });
        assert!(matches!(probe(addr).await, Probe::Other));
    }
}
