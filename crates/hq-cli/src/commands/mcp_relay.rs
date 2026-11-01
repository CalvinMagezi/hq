//! `hq mcp-serve --remote <name>`: a stdio MCP server that relays every message to the HTTP MCP
//! endpoint of a `remote_mcp` entry, for clients that only start stdio servers (Claude Desktop).
//! The remote HQ answers; nothing runs against this machine's vault.

use anyhow::{Context, Result, bail};
use hq_core::config::{HqConfig, RemoteMcpServer};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const SESSION_HEADER: &str = "mcp-session-id";
/// Longer than any tool call a client waits for, so the client's own timeout decides.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);
/// JSON-RPC "internal error", for a request the relay could not deliver.
const RELAY_ERROR: i64 = -32603;

pub async fn run(config: &HqConfig, name: &str) -> Result<()> {
    let server = config
        .remote_mcp
        .iter()
        .find(|s| s.name == name)
        .with_context(|| format!("no remote_mcp entry is named `{name}`"))?
        .clone();
    let mut relay = Relay::new(server)?;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut out = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        for reply in relay.forward(&line).await {
            out.write_all(reply.as_bytes()).await?;
            out.write_all(b"\n").await?;
            out.flush().await?;
        }
    }
    Ok(())
}

pub(crate) struct Relay {
    http: reqwest::Client,
    server: RemoteMcpServer,
    session: Option<String>,
}

impl Relay {
    pub(crate) fn new(server: RemoteMcpServer) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()?;
        Ok(Self {
            http,
            server,
            session: None,
        })
    }

    /// The lines to write back for one message from the client. A notification gets none; a
    /// request that could not be delivered gets a JSON-RPC error with its id.
    pub(crate) async fn forward(&mut self, line: &str) -> Vec<String> {
        let id = serde_json::from_str::<Value>(line)
            .ok()
            .and_then(|v| v.get("id").cloned());
        match self.post(line).await {
            Ok(replies) => replies,
            Err(e) => id
                .map(|id| {
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": RELAY_ERROR, "message": format!("{}: {e:#}", self.server.name) },
                    })
                    .to_string()
                })
                .into_iter()
                .collect(),
        }
    }

    async fn post(&mut self, line: &str) -> Result<Vec<String>> {
        let mut req = self
            .http
            .post(&self.server.url)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(line.to_string());
        if let Some(key) = &self.server.api_key {
            req = req.bearer_auth(key);
        }
        if let Some(session) = &self.session {
            req = req.header(SESSION_HEADER, session);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if let Some(session) = resp.headers().get(SESSION_HEADER) {
            self.session = session.to_str().ok().map(str::to_string);
        }
        let event_stream = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        let body = resp.text().await?;
        if !status.is_success() {
            bail!("HTTP {status}: {}", body.trim());
        }
        Ok(replies(&body, event_stream))
    }
}

/// The JSON-RPC messages in a response body: each `data:` line of an event stream, or the body.
fn replies(body: &str, event_stream: bool) -> Vec<String> {
    if !event_stream {
        let body = body.trim();
        return (!body.is_empty())
            .then(|| body.to_string())
            .into_iter()
            .collect();
    }
    body.lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, http::HeaderMap, routing::post};

    const KEY: &str = "relay-test-key";

    async fn fake_hq() -> String {
        let app = Router::new().route(
            "/mcp",
            post(|headers: HeaderMap, body: String| async move {
                let authorized = headers
                    .get("authorization")
                    .is_some_and(|v| v == &format!("Bearer {KEY}")[..]);
                if !authorized {
                    return (
                        axum::http::StatusCode::UNAUTHORIZED,
                        HeaderMap::new(),
                        "no".into(),
                    );
                }
                let msg: Value = serde_json::from_str(&body).unwrap();
                let mut out = HeaderMap::new();
                out.insert(SESSION_HEADER, "s-1".parse().unwrap());
                let Some(id) = msg.get("id") else {
                    return (axum::http::StatusCode::ACCEPTED, out, String::new());
                };
                let seen = headers.get(SESSION_HEADER).is_some();
                let reply = json!({"jsonrpc":"2.0","id":id,"result":{"session_seen":seen}});
                (axum::http::StatusCode::OK, out, reply.to_string())
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/mcp")
    }

    fn server(url: String, key: &str) -> RemoteMcpServer {
        RemoteMcpServer {
            name: "phone".into(),
            url,
            api_key: Some(key.into()),
            live_user_turn_only: true,
        }
    }

    #[tokio::test]
    async fn relays_requests_keeps_the_session_and_drops_notification_replies() {
        let mut relay = Relay::new(server(fake_hq().await, KEY)).unwrap();
        let first = relay
            .forward(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#)
            .await;
        assert_eq!(first.len(), 1);
        assert!(first[0].contains(r#""session_seen":false"#), "{first:?}");
        let note = relay
            .forward(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .await;
        assert!(note.is_empty());
        let second = relay
            .forward(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
            .await;
        assert!(second[0].contains(r#""session_seen":true"#), "{second:?}");
    }

    #[tokio::test]
    async fn a_refused_request_comes_back_as_an_error_with_its_id() {
        let mut relay = Relay::new(server(fake_hq().await, "wrong")).unwrap();
        let out = relay
            .forward(r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#)
            .await;
        let err: Value = serde_json::from_str(&out[0]).unwrap();
        assert_eq!(err["id"], 7);
        assert!(err["error"]["message"].as_str().unwrap().contains("401"));
        let note = relay
            .forward(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .await;
        assert!(note.is_empty(), "a notification never gets a reply");
    }

    #[test]
    fn reads_every_data_line_of_an_event_stream() {
        let body = "event: message\ndata: {\"id\":1}\n\ndata: {\"id\":2}\n\n";
        assert_eq!(replies(body, true), vec![r#"{"id":1}"#, r#"{"id":2}"#]);
        assert_eq!(replies(" {\"id\":3}\n", false), vec![r#"{"id":3}"#]);
        assert!(replies("", false).is_empty());
    }
}
