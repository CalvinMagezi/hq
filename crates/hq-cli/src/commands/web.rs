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
    /// Print one JSON object instead of text; never opens a browser
    #[arg(long)]
    json: bool,
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
    static_dir: String,
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

async fn start(config: &HqConfig, args: StartArgs) -> Result<()> {
    let port = args.port.unwrap_or(config.ws_port);
    let bind = if args.lan {
        "0.0.0.0".to_string()
    } else {
        args.bind.clone().unwrap_or_else(|| config.web_bind.clone())
    };
    let token = resolve_token(&bind, config.web_auth_token.as_deref())?;
    // Same rule `hq start` applies; with the token above it only fails on a blank one.
    hq_web::auth::check_web_bind(&bind, token.as_deref()).map_err(anyhow::Error::msg)?;

    let static_dir = ensure_ui(config, args.build, args.json)?;
    let mut report = Report::new(&bind, port, token, &static_dir);

    match probe(port).await {
        Probe::Hq => {
            report.already_running = true;
            return finish(report, &args);
        }
        Probe::Other => bail!(
            "port {port} is taken by something that is not HQ. Pick another with `hq web --port <n>`."
        ),
        Probe::Free => {}
    }

    if args.detach {
        let log = hq_dir().join("logs").join("web.log");
        report.log = Some(log.display().to_string());
        report.pid = Some(spawn_detached(&bind, port, args.lan, &log).await?);
        return finish(report, &args);
    }

    let vault =
        Arc::new(VaultClient::new(config.vault_path.clone()).context("failed to open vault")?);
    let db = Arc::new(Database::open(&config.db_path()).context("failed to open database")?);
    let state = super::start::build_web_state(
        config,
        &vault,
        &db,
        Some(static_dir.clone()),
        report.token.clone(),
    );
    let addr: std::net::SocketAddr = format!("{bind}:{port}")
        .parse()
        .with_context(|| format!("'{bind}' is not an IP address"))?;
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

/// Prints the report (text or JSON) and opens the browser when appropriate.
fn finish(report: Report, args: &StartArgs) -> Result<()> {
    if args.json {
        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }
    if report.already_running {
        println!("HQ web is already running.");
    } else if args.detach {
        println!(
            "HQ web is running in the background (pid {}).",
            report.pid.unwrap_or(0)
        );
    } else {
        println!("HQ web is starting. Press Ctrl+C to stop.");
    }
    println!("  Local:   {}", report.url);
    if let Some(net) = &report.network_url {
        println!("  Network: {net}");
    }
    if let Some(token) = &report.token {
        println!("  Token:   {token}");
        println!("  Sign-in link (keep it private): {}", report.login_url);
    }
    if let Some(log) = &report.log {
        println!("  Log:     {log}");
    }
    if args.detach {
        println!("Stop it with `hq web stop`.");
    }
    if !args.no_open && can_open_browser() {
        open_browser(&report.login_url);
    }
    Ok(())
}

impl Report {
    fn new(bind: &str, port: u16, token: Option<String>, static_dir: &Path) -> Self {
        let url = format!("http://localhost:{port}");
        let login_url = match &token {
            Some(t) => format!("{url}/#token={t}"),
            None => url.clone(),
        };
        let network_url = (!is_loopback(bind)).then(|| {
            let host = if bind == "0.0.0.0" || bind == "::" {
                lan_ip().unwrap_or_else(|| bind.to_string())
            } else {
                bind.to_string()
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
            static_dir: static_dir.display().to_string(),
            log: None,
        }
    }
}

async fn status(config: &HqConfig, json: bool) -> Result<()> {
    let state = read_state();
    let port = state.as_ref().map(|s| s.port).unwrap_or(config.ws_port);
    let running = matches!(probe(port).await, Probe::Hq);
    let url = format!("http://localhost:{port}");
    let pid = state
        .filter(|s| super::stop::is_alive(s.pid))
        .map(|s| s.pid);
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
    let stopped = match read_state() {
        Some(s) if super::stop::is_alive(s.pid) => {
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

/// The directory holding the built UI, building it from a source checkout when
/// asked (or when it is missing and bun is there to do it).
fn ensure_ui(config: &HqConfig, build: bool, quiet: bool) -> Result<PathBuf> {
    let root = config.vault_path.parent().unwrap_or(&config.vault_path);
    let configured = config.web_static_dir.clone();
    let found = || {
        configured
            .clone()
            .unwrap_or_else(|| super::start::default_static_dir(root))
    };
    let dir = found();
    let source = find_source_checkout(root);
    if !build {
        if dir.join("index.html").exists() {
            return Ok(dir);
        }
        // A configured directory is taken as is; otherwise reuse a checkout's earlier build.
        let built = source.as_ref().map(|s| s.join("apps/hq-web/dist/client"));
        if let Some(built) = built.filter(|b| configured.is_none() && b.join("index.html").exists()) {
            return Ok(built);
        }
    }
    let Some(source) = source else {
        if dir.join("index.html").exists() {
            return Ok(dir);
        }
        bail!(
            "no web UI build found (looked for {}/index.html). Run `hq update --apply` to install \
             the released web files, or build from a checkout with `hq web --build`, or point \
             `web_static_dir` at a built `apps/hq-web/dist/client`.",
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
    for step in [&["install"][..], &["run", "build"][..]] {
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
    let built = app.join("dist").join("client");
    let dir = configured.unwrap_or(built);
    if !dir.join("index.html").exists() {
        bail!(
            "the build finished but {}/index.html is missing",
            dir.display()
        );
    }
    Ok(dir)
}

/// The nearest directory from `root` or the current directory upward that
/// holds `apps/hq-web/package.json`.
fn find_source_checkout(root: &Path) -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok();
    root.ancestors()
        .chain(cwd.iter().flat_map(|c| c.ancestors()))
        .find(|d| d.join("apps/hq-web/package.json").is_file())
        .map(Path::to_path_buf)
}

fn which(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .map(|d| Path::new(d).join(program))
        .find(|p| p.is_file())
}

// ─── Background server ──────────────────────────────────────────────────

/// Re-runs this binary as `hq web` in its own process group with output going
/// to `log`, then waits until it answers `/health`.
async fn spawn_detached(bind: &str, port: u16, lan: bool, log: &Path) -> Result<u32> {
    std::fs::create_dir_all(log.parent().unwrap_or(Path::new(".")))?;
    let out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)?;
    let mut cmd =
        std::process::Command::new(std::env::current_exe().context("cannot find the hq binary")?);
    cmd.args(["web", "--no-open", "--port", &port.to_string()]);
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
    let mut child = cmd
        .spawn()
        .context("failed to start the background server")?;

    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if matches!(probe(port).await, Probe::Hq) {
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

// ─── Small helpers ──────────────────────────────────────────────────────

enum Probe {
    Hq,
    Other,
    Free,
}

/// Is an HQ server listening on `port`? Only an `agent-hq` `/health` counts,
/// so another program on the port is never mistaken for HQ.
async fn probe(port: u16) -> Probe {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .no_proxy()
        .build()
    {
        Ok(c) => c,
        Err(_) => return Probe::Free,
    };
    match client
        .get(format!("http://127.0.0.1:{port}/health"))
        .send()
        .await
    {
        Ok(resp) => {
            let is_hq = resp
                .json::<serde_json::Value>()
                .await
                .ok()
                .is_some_and(|v| v["service"] == "agent-hq");
            if is_hq { Probe::Hq } else { Probe::Other }
        }
        // Refused is the normal "free" case; a timeout means something is there.
        Err(e) if e.is_timeout() => Probe::Other,
        Err(_) => Probe::Free,
    }
}

fn is_loopback(bind: &str) -> bool {
    matches!(bind, "127.0.0.1" | "::1" | "localhost")
}

/// The token a server on `bind` needs: the configured one, else for a
/// non-loopback bind a generated one kept in `~/.hq/web.token` so the sign-in
/// link survives restarts. A loopback bind needs no token.
fn resolve_token(bind: &str, configured: Option<&str>) -> Result<Option<String>> {
    let configured = configured.map(str::trim).filter(|t| !t.is_empty());
    if let Some(t) = configured {
        return Ok(Some(t.to_string()));
    }
    if is_loopback(bind) {
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
        || std::env::var_os("DISPLAY").is_some()
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(not(target_os = "macos"))]
    let opener = "xdg-open";
    let _ = std::process::Command::new(opener)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_report_has_no_token_or_network_url() {
        let r = Report::new("127.0.0.1", 5678, None, Path::new("/ui"));
        assert_eq!(r.url, "http://localhost:5678");
        assert_eq!(r.login_url, r.url);
        assert!(r.network_url.is_none() && r.token.is_none());
    }

    #[test]
    fn token_goes_in_the_url_fragment_never_the_query() {
        let r = Report::new("0.0.0.0", 9000, Some("abc".into()), Path::new("/ui"));
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
    fn source_checkout_is_found_by_its_web_app_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("apps/hq-web")).unwrap();
        std::fs::write(tmp.path().join("apps/hq-web/package.json"), "{}").unwrap();
        std::fs::create_dir_all(tmp.path().join(".vault")).unwrap();
        assert_eq!(
            find_source_checkout(tmp.path()).as_deref(),
            Some(tmp.path())
        );
    }

    #[tokio::test]
    async fn a_closed_port_probes_as_free() {
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        assert!(matches!(probe(port).await, Probe::Free));
    }

    #[tokio::test]
    async fn a_non_hq_listener_is_not_mistaken_for_hq() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 512];
                let _ = sock.read(&mut buf).await;
                let body = r#"{"service":"other"}"#;
                let _ = sock
                    .write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes())
                    .await;
            }
        });
        assert!(matches!(probe(port).await, Probe::Other));
    }
}
