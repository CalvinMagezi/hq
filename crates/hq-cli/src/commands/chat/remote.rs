//! `hq chat --server`: drive the running daemon's backend over the web chat socket.
//!
//! The client speaks the same `/ws` protocol as the web UI: it sends a `chat`
//! message and renders the `text_delta`, `tool_*` and `turn_end` frames that
//! come back. The socket broadcasts every chat's events to every client, so
//! frames are filtered down to this client's own turn.

use crate::render::{self, Theme};
use anyhow::{Context, Result, bail};
use futures::{SinkExt, StreamExt};
use hq_core::config::HqConfig;
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
use std::net::IpAddr;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// Env var holding the daemon's web token, the same one the daemon reads.
pub const SERVER_TOKEN_ENV: &str = "HQ_WEB_AUTH_TOKEN";

const DETECT_TIMEOUT: Duration = Duration::from_millis(500);
/// How long to wait for the server to confirm a stop before giving up on the turn.
const STOP_GRACE: Duration = Duration::from_secs(10);
/// How long to wait for the server to acknowledge a turn (`turn_start`) before giving up.
const ACK_DEADLINE: Duration = Duration::from_secs(30);

/// What the command line asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerFlag {
    /// `--local`: always run in-process.
    Local,
    /// `HQ_CHAT_SERVER=auto`: use a daemon on loopback if one answers, else run in-process.
    Auto,
    /// `--server` with no URL: the configured loopback daemon, or an error.
    Daemon,
    /// `--server URL`.
    Url(String),
}

impl ServerFlag {
    /// Plain `hq chat` stays in-process; the daemon is opt-in, through
    /// `--server` or `HQ_CHAT_SERVER=auto` (`auto`).
    pub fn from_cli(local: bool, server: Option<String>, auto: bool) -> Self {
        match (local, server) {
            (true, _) => Self::Local,
            (false, None) if auto => Self::Auto,
            (false, None) => Self::Local,
            (false, Some(s)) if s.trim().is_empty() => Self::Daemon,
            (false, Some(s)) => Self::Url(s),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Selection {
    Local,
    DefaultDaemon,
    Url(String),
}

/// Pure mode choice. `daemon_up` says whether the configured loopback daemon answered.
pub fn select_mode(flag: &ServerFlag, daemon_up: bool) -> Result<Selection> {
    Ok(match flag {
        ServerFlag::Local => Selection::Local,
        ServerFlag::Auto if daemon_up => Selection::DefaultDaemon,
        ServerFlag::Auto => Selection::Local,
        ServerFlag::Daemon if daemon_up => Selection::DefaultDaemon,
        ServerFlag::Daemon => bail!(
            "no HQ daemon answered on the configured loopback address (is `hq start` running?). \
             Pass `--server URL` for a daemon elsewhere, or `--local`."
        ),
        ServerFlag::Url(u) => Selection::Url(u.clone()),
    })
}

/// Env var that opts a plain `hq chat` into using a loopback daemon when one answers.
pub const CHAT_SERVER_ENV: &str = "HQ_CHAT_SERVER";

/// Whether `HQ_CHAT_SERVER=auto` is set.
pub fn auto_from_env() -> bool {
    std::env::var(CHAT_SERVER_ENV).is_ok_and(|v| v.trim().eq_ignore_ascii_case("auto"))
}

/// A validated chat server. The token never appears in `Debug` output.
#[derive(PartialEq, Eq)]
pub struct ServerTarget {
    pub http_base: String,
    pub ws_url: String,
    token: Option<String>,
}

impl std::fmt::Debug for ServerTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerTarget")
            .field("http_base", &self.http_base)
            .field("ws_url", &self.ws_url)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

fn is_loopback_host(host: &str) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    bare.eq_ignore_ascii_case("localhost")
        || bare.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Validate a `--server` value. Loopback servers may use plain http/ws.
/// Anything else must be https/wss and come with a token, because the token
/// travels as a bearer header and must not cross the network in the clear.
pub fn parse_server_url(raw: &str, token: Option<String>) -> Result<ServerTarget> {
    let raw = raw.trim();
    let with_scheme = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("http://{raw}")
    };
    let url =
        reqwest::Url::parse(&with_scheme).with_context(|| format!("invalid server URL {raw:?}"))?;
    let secure = match url.scheme() {
        "http" | "ws" => false,
        "https" | "wss" => true,
        other => bail!("unsupported server URL scheme {other:?} (use http, https, ws or wss)"),
    };
    if !url.username().is_empty() || url.password().is_some() {
        bail!("put credentials in {SERVER_TOKEN_ENV}, not in the server URL");
    }
    let host = url.host_str().context("server URL has no host")?;
    let token = token.filter(|t| !t.trim().is_empty());
    if !is_loopback_host(host) {
        if token.is_none() {
            bail!(
                "refusing non-loopback server {host}: set {SERVER_TOKEN_ENV} to the daemon's web token"
            );
        }
        if !secure {
            bail!("refusing to send the web token to {host} over plain http/ws: use https or wss");
        }
    }
    let authority = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    let (http, ws) = if secure {
        ("https", "wss")
    } else {
        ("http", "ws")
    };
    Ok(ServerTarget {
        http_base: format!("{http}://{authority}"),
        ws_url: format!("{ws}://{authority}/ws"),
        token,
    })
}

/// `http://host:port` of the daemon this machine's config says is serving the
/// web UI, or `None` when it is bound to a non-loopback address (then only an
/// explicit `--server` URL, with a token and TLS, may reach it).
pub fn default_daemon_url(config: &HqConfig) -> Option<String> {
    let bind = config.web_bind.trim();
    let host = match bind {
        "" | "0.0.0.0" | "::" | "[::]" => "127.0.0.1".to_string(),
        b if is_loopback_host(b) && b.contains(':') && !b.starts_with('[') => format!("[{b}]"),
        b if is_loopback_host(b) => b.to_string(),
        _ => return None,
    };
    Some(format!("http://{host}:{}", config.ws_port))
}

/// Whether an HQ web server answers `/health` at `http_base`. The `service`
/// field is required so a bearer token is never sent to some other program that
/// happens to answer `{"status":"ok"}` on the port.
pub async fn detect_daemon(http_base: &str) -> bool {
    let mut builder = reqwest::Client::builder().timeout(DETECT_TIMEOUT);
    // A loopback probe must not go through an environment proxy.
    if reqwest::Url::parse(http_base)
        .ok()
        .and_then(|u| u.host_str().map(is_loopback_host))
        .unwrap_or(true)
    {
        builder = builder.no_proxy();
    }
    let Ok(client) = builder.build() else {
        return false;
    };
    let Ok(resp) = client.get(format!("{http_base}/health")).send().await else {
        return false;
    };
    let Ok(body) = resp.json::<Value>().await else {
        return false;
    };
    body["status"] == "ok"
        && body["service"] == hq_web::HEALTH_SERVICE
        && body["version"].is_string()
}

fn env_token() -> Option<String> {
    std::env::var(SERVER_TOKEN_ENV)
        .ok()
        .filter(|t| !t.trim().is_empty())
}

/// Decide the mode for this invocation. `None` means run in-process.
pub async fn resolve_target(config: &HqConfig, flag: &ServerFlag) -> Result<Option<ServerTarget>> {
    let default_url = default_daemon_url(config);
    let probe = matches!(flag, ServerFlag::Auto | ServerFlag::Daemon);
    let daemon_up = match (&default_url, probe) {
        (Some(url), true) => detect_daemon(url).await,
        _ => false,
    };
    match select_mode(flag, daemon_up)? {
        Selection::Local => Ok(None),
        // The local config's token is only ever sent to the daemon it configures.
        Selection::DefaultDaemon => {
            let url = default_url.context("no loopback daemon address")?;
            let token = env_token().or_else(|| config.web_auth_token.clone());
            parse_server_url(&url, token).map(Some)
        }
        Selection::Url(url) => {
            let target = parse_server_url(&url, env_token())?;
            if !detect_daemon(&target.http_base).await {
                bail!(
                    "{} did not answer /health as an HQ daemon; not sending the web token there",
                    target.http_base
                );
            }
            Ok(Some(target))
        }
    }
}

/// One thing this client's turn produced.
#[derive(Debug, PartialEq, Eq)]
pub enum Incoming {
    Started { thread_id: String },
    Text(String),
    Reasoning(String),
    ToolStart(String),
    ToolProgress(String),
    ToolEnd { tool: String, result: String },
    Error(String),
    Lag(u64),
    End { stopped: bool },
    Rejected(String),
}

/// Narrows the socket's broadcast to one turn: nothing counts until the server
/// acks this client's own `client_id`, then only that chat's frames do.
pub struct TurnFilter {
    client_id: String,
    thread_id: Option<String>,
    started: bool,
}

impl TurnFilter {
    pub fn new(client_id: String, thread_id: Option<String>) -> Self {
        Self {
            client_id,
            thread_id,
            started: false,
        }
    }

    pub fn thread_id(&self) -> Option<&str> {
        self.thread_id.as_deref()
    }

    /// Whether the server has acked this client's turn.
    pub fn started(&self) -> bool {
        self.started
    }

    pub fn accept(&mut self, frame: &str) -> Option<Incoming> {
        let v: Value = serde_json::from_str(frame).ok()?;
        let kind = v["type"].as_str()?;
        let ours = v["client_id"].as_str() == Some(self.client_id.as_str());
        let text = |key: &str| v[key].as_str().unwrap_or_default().to_string();
        match kind {
            "chat_rejected" if ours => return Some(Incoming::Rejected(text("reason"))),
            "turn_start" if !self.started && ours => {
                let thread_id = v["thread_id"].as_str()?.to_string();
                self.thread_id = Some(thread_id.clone());
                self.started = true;
                return Some(Incoming::Started { thread_id });
            }
            "stream_lag" => {
                return Some(Incoming::Lag(v["skipped"].as_u64().unwrap_or(0)));
            }
            _ => {}
        }
        if !self.started || v["thread_id"].as_str() != self.thread_id.as_deref() {
            return None;
        }
        Some(match kind {
            "text_delta" => Incoming::Text(text("content")),
            "reasoning_delta" => Incoming::Reasoning(text("content")),
            "tool_start" => Incoming::ToolStart(text("tool_name")),
            "tool_progress" => Incoming::ToolProgress(text("message")),
            "tool_end" => Incoming::ToolEnd {
                tool: text("tool_name"),
                result: text("result"),
            },
            "error" => Incoming::Error(text("content")),
            "turn_end" => Incoming::End {
                stopped: v["stopped"].as_bool().unwrap_or(false),
            },
            _ => return None,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Done {
        stopped: bool,
    },
    Rejected(String),
    /// Ctrl+C, and the server did not confirm the stop in time.
    Interrupted,
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct RemoteChat {
    ws: Socket,
    thread_id: Option<String>,
    ack_deadline: Duration,
}

impl RemoteChat {
    pub async fn connect(target: &ServerTarget) -> Result<Self> {
        let mut request = target
            .ws_url
            .as_str()
            .into_client_request()
            .context("building the websocket request")?;
        if let Some(token) = &target.token {
            let mut value = HeaderValue::from_str(&format!("Bearer {token}"))
                .context("web token is not a valid header value")?;
            value.set_sensitive(true);
            request.headers_mut().insert(AUTHORIZATION, value);
        }
        let (ws, _) = tokio_tungstenite::connect_async(request)
            .await
            .with_context(|| {
                format!(
                    "connecting to {} (wrong token, or is the daemon up?)",
                    target.ws_url
                )
            })?;
        Ok(Self {
            ws,
            thread_id: None,
            ack_deadline: ACK_DEADLINE,
        })
    }

    /// Start a fresh server-side chat on the next turn.
    pub fn new_thread(&mut self) {
        self.thread_id = None;
    }

    async fn send(&mut self, v: Value) -> Result<()> {
        self.ws
            .send(Message::text(v.to_string()))
            .await
            .context("sending to the server")
    }

    /// Ask the server to stop this client's turn; returns when to give up waiting for its `turn_end`.
    async fn send_stop(&mut self, filter: &TurnFilter) -> Result<Instant> {
        let thread_id = filter.thread_id().context("no chat thread to stop")?;
        self.send(json!({"type": "stop", "thread_id": thread_id}))
            .await?;
        Ok(Instant::now() + STOP_GRACE)
    }

    /// Send one user turn and feed the reply's events to `on_event` until it ends.
    pub async fn turn(
        &mut self,
        text: &str,
        mut on_event: impl FnMut(&Incoming),
    ) -> Result<Outcome> {
        let client_id = uuid::Uuid::new_v4().to_string();
        let mut filter = TurnFilter::new(client_id.clone(), self.thread_id.clone());
        self.send(json!({
            "type": "chat",
            "content": text,
            "thread_id": self.thread_id,
            "client_id": client_id,
        }))
        .await?;
        let ack_deadline = Instant::now() + self.ack_deadline;
        let mut interrupted = false;
        let mut stop_deadline: Option<Instant> = None;
        loop {
            // Waiting for the ack, or for the server to confirm a stop: both are bounded.
            let deadline = stop_deadline.or_else(|| (!filter.started()).then_some(ack_deadline));
            let frame = tokio::select! {
                f = self.ws.next() => f,
                _ = tokio::signal::ctrl_c(), if !interrupted => {
                    interrupted = true;
                    // Before the ack there is no thread to stop yet; the stop goes out on Started.
                    if filter.started() {
                        stop_deadline = Some(self.send_stop(&filter).await?);
                    }
                    continue;
                }
                _ = tokio::time::sleep_until(deadline.unwrap_or(ack_deadline)), if deadline.is_some() => {
                    if stop_deadline.is_some() {
                        return Ok(Outcome::Interrupted);
                    }
                    bail!("the server did not acknowledge the turn in time ({}s)", ACK_DEADLINE.as_secs());
                }
            };
            let text = match frame {
                Some(Ok(Message::Text(t))) => t,
                Some(Ok(Message::Close(_))) | None => bail!("the server closed the connection"),
                Some(Err(e)) => return Err(e).context("reading from the server"),
                Some(Ok(_)) => continue,
            };
            let Some(event) = filter.accept(text.as_str()) else {
                continue;
            };
            if let (Incoming::Lag(skipped), false) = (&event, filter.started()) {
                bail!(
                    "the stream lagged by {skipped} events before the server acknowledged the turn, so it may have been missed; retry"
                );
            }
            on_event(&event);
            match event {
                Incoming::Started { thread_id } => {
                    self.thread_id = Some(thread_id);
                    if interrupted && stop_deadline.is_none() {
                        stop_deadline = Some(self.send_stop(&filter).await?);
                    }
                }
                Incoming::End { stopped } => return Ok(Outcome::Done { stopped }),
                Incoming::Rejected(reason) => return Ok(Outcome::Rejected(reason)),
                _ => {}
            }
        }
    }
}

/// Renders a turn's events. `plain` keeps stdout to the reply text alone, with
/// tool noise on stderr, so `-p` output pipes cleanly.
struct Printer {
    theme: Theme,
    plain: bool,
    wrote_text: bool,
    saw_error: bool,
}

impl Printer {
    fn new(plain: bool) -> Self {
        Self {
            theme: Theme::dark(),
            plain,
            wrote_text: false,
            saw_error: false,
        }
    }

    fn show(&mut self, event: &Incoming) {
        let t = &self.theme;
        match event {
            Incoming::Text(s) => {
                if !self.wrote_text && !self.plain {
                    print!("\x1b[{}m hq>\x1b[0m ", render::fg(&t.secondary));
                }
                self.wrote_text = true;
                print!("{s}");
                let _ = io::stdout().flush();
            }
            Incoming::ToolStart(tool) if self.plain => eprintln!("[{tool}]"),
            Incoming::ToolStart(tool) => println!("\n{}", render::render_tool_start(tool, t)),
            Incoming::ToolProgress(msg) if !self.plain => {
                println!("{}", render::render_tool_progress("tool", msg, t));
            }
            Incoming::ToolEnd { tool, result } if !self.plain => {
                let shown = render::format_tool_result(tool, result);
                println!("\x1b[{}m{shown}\x1b[0m", render::fg(&t.text_dim));
            }
            Incoming::Error(msg) => {
                self.saw_error = true;
                if self.plain {
                    eprintln!("Error: {msg}");
                } else {
                    println!("{}", render::render_error(msg, t));
                }
            }
            Incoming::Lag(n) => {
                eprintln!("[stream lagged: {n} events missed, the reply may be incomplete]")
            }
            _ => {}
        }
    }

    fn finish(&self, outcome: &Outcome) {
        match outcome {
            Outcome::Done { stopped: true } | Outcome::Interrupted => {
                println!(
                    "\n\x1b[{}m  [stopped]\x1b[0m",
                    render::fg(&self.theme.text_muted)
                );
            }
            Outcome::Rejected(reason) => {
                eprintln!("\x1b[{}m  {reason}\x1b[0m", render::fg(&self.theme.warning));
            }
            Outcome::Done { stopped: false } => println!("\n"),
        }
    }
}

/// `hq chat --server -p "prompt"`: one turn, reply text on stdout.
pub async fn run_oneshot(target: &ServerTarget, prompt: &str) -> Result<()> {
    let mut chat = RemoteChat::connect(target).await?;
    let mut printer = Printer::new(true);
    let outcome = chat.turn(prompt, |e| printer.show(e)).await?;
    println!();
    match outcome {
        Outcome::Done { stopped: false } if printer.saw_error && !printer.wrote_text => {
            bail!("the server reported an error and produced no reply")
        }
        Outcome::Done { stopped: false } => Ok(()),
        Outcome::Done { stopped: true } | Outcome::Interrupted => {
            bail!("turn stopped before it finished")
        }
        Outcome::Rejected(reason) => bail!("{reason}"),
    }
}

/// Line-based REPL against the daemon. `/new` starts a fresh server chat.
pub async fn run_interactive(target: &ServerTarget) -> Result<()> {
    let mut chat = RemoteChat::connect(target).await?;
    let theme = Theme::dark();
    println!(
        "\x1b[{}m Type /new for a fresh chat, /quit to exit. Ctrl+C stops a running reply.\x1b[0m\n",
        render::fg(&theme.text_muted)
    );
    loop {
        print!("\x1b[{}myou> \x1b[0m", render::fg(&theme.primary));
        io::stdout().flush()?;
        // Blocking read on its own thread so Ctrl+C at an idle prompt still exits.
        let read = tokio::select! {
            r = tokio::task::spawn_blocking(|| {
                let mut buf = String::new();
                let n = io::stdin().lock().read_line(&mut buf)?;
                Ok::<_, io::Error>((n, buf))
            }) => r,
            _ = tokio::signal::ctrl_c() => {
                println!();
                std::process::exit(0);
            }
        };
        let (n, line) = read??;
        let input = line.trim();
        if n == 0 || matches!(input, "/quit" | "/exit") {
            println!();
            return Ok(());
        }
        match input {
            "" => continue,
            "/new" | "/reset" => {
                chat.new_thread();
                println!("  new chat");
                continue;
            }
            _ => {}
        }
        let mut printer = Printer::new(false);
        let outcome = chat.turn(input, |e| printer.show(e)).await?;
        printer.finish(&outcome);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::ws::{Message as AxMsg, WebSocket, WebSocketUpgrade};
    use axum::http::HeaderMap;
    use axum::routing::get;
    use std::sync::{Arc, Mutex};

    fn url(raw: &str, token: Option<&str>) -> Result<ServerTarget> {
        parse_server_url(raw, token.map(str::to_string))
    }

    #[test]
    fn loopback_urls_need_no_token_and_normalise() {
        let t = url("localhost:5678", None).unwrap();
        assert_eq!(
            (t.http_base.as_str(), t.ws_url.as_str()),
            ("http://localhost:5678", "ws://localhost:5678/ws")
        );
        assert!(url("http://127.0.0.1:9", None).is_ok());
        assert!(url("ws://[::1]:9/anything", None).is_ok());
    }

    #[test]
    fn non_loopback_needs_token_and_tls() {
        assert!(url("https://hq.example.com", None).is_err());
        assert!(url("http://hq.example.com", Some("tok")).is_err());
        assert!(url("ws://10.0.0.5:5678", Some("tok")).is_err());
        let ok = url("https://hq.example.com:8443/ignored/path", Some("tok")).unwrap();
        assert_eq!(ok.ws_url, "wss://hq.example.com:8443/ws");
    }

    #[test]
    fn rejects_bad_schemes_and_embedded_credentials() {
        assert!(url("ftp://localhost", None).is_err());
        assert!(url("http://user:pw@localhost:1", None).is_err());
        assert!(url("http://", None).is_err());
    }

    #[test]
    fn debug_output_redacts_the_token() {
        let t = url("https://hq.example.com", Some("super-secret")).unwrap();
        assert!(!format!("{t:?}").contains("super-secret"));
    }

    #[test]
    fn mode_selection() {
        use ServerFlag::*;
        assert_eq!(select_mode(&Auto, true).unwrap(), Selection::DefaultDaemon);
        assert_eq!(select_mode(&Auto, false).unwrap(), Selection::Local);
        assert_eq!(select_mode(&Local, true).unwrap(), Selection::Local);
        assert_eq!(
            select_mode(&Daemon, true).unwrap(),
            Selection::DefaultDaemon
        );
        assert!(
            select_mode(&Daemon, false).is_err(),
            "explicit --server must not fall back silently"
        );
        assert_eq!(
            select_mode(&Url("x:1".into()), false).unwrap(),
            Selection::Url("x:1".into())
        );
    }

    #[test]
    fn flag_from_cli() {
        assert_eq!(
            ServerFlag::from_cli(true, Some("u".into()), true),
            ServerFlag::Local
        );
        assert_eq!(ServerFlag::from_cli(false, None, false), ServerFlag::Local);
        assert_eq!(ServerFlag::from_cli(false, None, true), ServerFlag::Auto);
        assert_eq!(
            ServerFlag::from_cli(false, Some(String::new()), false),
            ServerFlag::Daemon
        );
        assert_eq!(
            ServerFlag::from_cli(false, Some("h:1".into()), false),
            ServerFlag::Url("h:1".into())
        );
    }

    #[test]
    fn default_daemon_url_follows_bind() {
        let mut c = HqConfig {
            ws_port: 5678,
            web_bind: "0.0.0.0".into(),
            ..HqConfig::default()
        };
        assert_eq!(
            default_daemon_url(&c).as_deref(),
            Some("http://127.0.0.1:5678")
        );
        c.web_bind = "::1".into();
        assert_eq!(default_daemon_url(&c).as_deref(), Some("http://[::1]:5678"));
        c.web_bind = "100.64.0.9".into();
        assert_eq!(default_daemon_url(&c), None);
    }

    fn frame(v: Value) -> String {
        v.to_string()
    }

    #[test]
    fn filter_ignores_other_chats_and_waits_for_own_ack() {
        let mut f = TurnFilter::new("me".into(), None);
        assert_eq!(
            f.accept(&frame(
                json!({"type":"text_delta","thread_id":"t0","content":"x"})
            )),
            None
        );
        assert_eq!(
            f.accept(&frame(
                json!({"type":"turn_start","thread_id":"t0","client_id":"other"})
            )),
            None
        );
        assert_eq!(
            f.accept(&frame(
                json!({"type":"turn_start","thread_id":"t1","client_id":"me"})
            )),
            Some(Incoming::Started {
                thread_id: "t1".into()
            })
        );
        assert_eq!(
            f.accept(&frame(
                json!({"type":"text_delta","thread_id":"t0","content":"x"})
            )),
            None
        );
        assert_eq!(
            f.accept(&frame(
                json!({"type":"text_delta","thread_id":"t1","content":"hi"})
            )),
            Some(Incoming::Text("hi".into()))
        );
        assert_eq!(
            f.accept(&frame(
                json!({"type":"turn_end","thread_id":"t1","stopped":true})
            )),
            Some(Incoming::End { stopped: true })
        );
        assert_eq!(f.accept("not json"), None);
    }

    #[test]
    fn filter_reports_rejection_for_own_client_id_only() {
        let mut f = TurnFilter::new("me".into(), Some("t1".into()));
        assert_eq!(
            f.accept(&frame(
                json!({"type":"chat_rejected","client_id":"you","reason":"busy"})
            )),
            None
        );
        assert_eq!(
            f.accept(&frame(
                json!({"type":"chat_rejected","client_id":"me","reason":"busy"})
            )),
            Some(Incoming::Rejected("busy".into()))
        );
    }

    /// A scripted stand-in for the daemon: records what the client sent
    /// (including the Authorization header) and replays a canned reply with
    /// unrelated-chat noise mixed in.
    async fn scripted_server() -> (String, Arc<Mutex<Vec<Value>>>, Arc<Mutex<Option<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let auth = Arc::new(Mutex::new(None));
        let (seen_h, auth_h) = (seen.clone(), auth.clone());
        let app = axum::Router::new().route(
            "/ws",
            get(move |headers: HeaderMap, ws: WebSocketUpgrade| {
                let (seen, auth) = (seen_h.clone(), auth_h.clone());
                *auth.lock().unwrap() = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                async move { ws.on_upgrade(move |s| script(s, seen)) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), seen, auth)
    }

    async fn script(mut socket: WebSocket, seen: Arc<Mutex<Vec<Value>>>) {
        while let Some(Ok(AxMsg::Text(t))) = socket.recv().await {
            let msg: Value = serde_json::from_str(t.as_str()).unwrap();
            seen.lock().unwrap().push(msg.clone());
            if msg["type"] != "chat" {
                continue;
            }
            let cid = msg["client_id"].clone();
            let replies = [
                json!({"type":"text_delta","thread_id":"other","content":"NOISE"}),
                json!({"type":"turn_start","thread_id":"thr-1","client_id":cid}),
                json!({"type":"tool_start","thread_id":"thr-1","tool_name":"grep"}),
                json!({"type":"tool_end","thread_id":"thr-1","tool_name":"grep","result":"3 hits"}),
                json!({"type":"text_delta","thread_id":"other","content":"NOISE"}),
                json!({"type":"text_delta","thread_id":"thr-1","content":"Hello, "}),
                json!({"type":"text_delta","thread_id":"thr-1","content":"world"}),
                json!({"type":"turn_end","thread_id":"thr-1","message_id":"m1"}),
            ];
            for r in replies {
                socket.send(AxMsg::text(r.to_string())).await.unwrap();
            }
        }
    }

    /// A daemon that answers every chat message with `frames` and nothing else.
    async fn canned_server(frames: Vec<Value>) -> String {
        let app = axum::Router::new().route(
            "/ws",
            get(move |ws: WebSocketUpgrade| {
                let frames = frames.clone();
                async move {
                    ws.on_upgrade(move |mut socket| async move {
                        while let Some(Ok(AxMsg::Text(_))) = socket.recv().await {
                            for f in &frames {
                                socket.send(AxMsg::text(f.to_string())).await.unwrap();
                            }
                        }
                    })
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn a_turn_the_server_never_acks_fails_instead_of_hanging() {
        let base = canned_server(vec![
            json!({"type":"text_delta","thread_id":"x","content":"noise"}),
        ])
        .await;
        let target = parse_server_url(&base, None).unwrap();
        let mut chat = RemoteChat::connect(&target).await.unwrap();
        chat.ack_deadline = Duration::from_millis(200);
        let err = chat.turn("hi", |_| {}).await.unwrap_err().to_string();
        assert!(err.contains("did not acknowledge"), "{err}");
    }

    #[tokio::test]
    async fn a_lag_before_the_ack_fails_the_turn_instead_of_waiting_for_a_missed_ack() {
        let base = canned_server(vec![json!({"type":"stream_lag","skipped":7})]).await;
        let target = parse_server_url(&base, None).unwrap();
        let mut chat = RemoteChat::connect(&target).await.unwrap();
        let err = chat.turn("hi", |_| {}).await.unwrap_err().to_string();
        assert!(err.contains("lagged by 7"), "{err}");
    }

    #[tokio::test]
    async fn turn_streams_own_reply_and_reuses_the_thread() {
        let (base, seen, auth) = scripted_server().await;
        let target = parse_server_url(&base, Some("tok".into())).unwrap();
        let mut chat = RemoteChat::connect(&target).await.unwrap();

        let mut events = Vec::new();
        let outcome = chat
            .turn("first", |e| events.push(format!("{e:?}")))
            .await
            .unwrap();
        assert_eq!(outcome, Outcome::Done { stopped: false });
        let text: String = events
            .iter()
            .filter_map(|e| {
                e.strip_prefix("Text(\"")
                    .and_then(|s| s.strip_suffix("\")"))
            })
            .collect();
        assert_eq!(
            text, "Hello, world",
            "noise from other chats must not leak: {events:?}"
        );
        assert!(events.iter().any(|e| e.contains("ToolStart(\"grep\")")));

        chat.turn("second", |_| {}).await.unwrap();
        let sent = seen.lock().unwrap().clone();
        assert_eq!(sent[0]["type"], "chat");
        assert_eq!(sent[0]["content"], "first");
        assert!(
            sent[0]["thread_id"].is_null(),
            "first turn opens a new chat"
        );
        assert_eq!(sent[1]["thread_id"], "thr-1", "later turns continue it");
        assert_eq!(auth.lock().unwrap().as_deref(), Some("Bearer tok"));
    }

    #[tokio::test]
    async fn detects_a_real_hq_web_server_by_health() {
        let vault = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(vault.path().join("_data")).unwrap();
        let state = hq_web::WsState::new(vault.path().to_path_buf(), None);
        let app = hq_web::create_router(Arc::new(state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        assert!(detect_daemon(&format!("http://{addr}")).await);
        // The real origin and auth middleware let a header-less CLI client open the chat socket.
        let target = parse_server_url(&format!("http://{addr}"), None).unwrap();
        assert!(RemoteChat::connect(&target).await.is_ok());
        // A port nothing listens on, and a listener that is not HQ, are both "no daemon".
        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed_addr = closed.local_addr().unwrap();
        drop(closed);
        assert!(!detect_daemon(&format!("http://{closed_addr}")).await);
        let other = axum::Router::new().route("/health", get(|| async { "up" }));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let other_addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, other).await.unwrap() });
        assert!(!detect_daemon(&format!("http://{other_addr}")).await);
    }

    #[tokio::test]
    async fn an_impostor_health_endpoint_is_not_a_daemon_and_gets_no_token() {
        let lookalike = axum::Router::new().route(
            "/health",
            get(|| async { axum::Json(json!({"status":"ok","version":"1.2.3"})) }),
        );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, lookalike).await.unwrap() });
        let base = format!("http://{addr}");
        assert!(
            !detect_daemon(&base).await,
            "status and version alone are not enough"
        );
        let err = resolve_target(&HqConfig::default(), &ServerFlag::Url(base))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("not sending the web token"), "{err}");
    }
}
