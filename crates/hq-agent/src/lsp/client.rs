//! LSP client — JSON-RPC over stdio communication with a language server process.

use super::detection::LanguageServer;
use super::protocol::*;
use anyhow::{Context, Result, bail};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{Mutex, oneshot};
use tracing::{debug, warn};

/// A single LSP client connected to a language server subprocess.
pub struct LspClient {
    /// Server config used to spawn this client.
    server: LanguageServer,
    /// Project root this server was initialized for.
    root: PathBuf,
    /// Next JSON-RPC request ID.
    next_id: AtomicU64,
    /// Child process stdin for sending requests.
    stdin: Arc<Mutex<ChildStdin>>,
    /// Pending response channels keyed by request ID.
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>,
    /// Child process handle (for cleanup).
    child: Arc<Mutex<Child>>,
    /// Whether the server has been initialized.
    initialized: Arc<Mutex<bool>>,
}

impl LspClient {
    /// Spawn a language server and perform the LSP initialize handshake.
    pub async fn start(server: LanguageServer, root: &Path) -> Result<Self> {
        debug!(
            command = %server.command,
            root = %root.display(),
            "starting LSP server"
        );

        let mut child = tokio::process::Command::new(&server.command)
            .args(&server.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .current_dir(root)
            .spawn()
            .with_context(|| format!("failed to spawn {}", server.command))?;

        let stdin = child.stdin.take().context("no stdin on child")?;
        let stdout = child.stdout.take().context("no stdout on child")?;

        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Spawn reader task to process responses.
        let pending_clone = pending.clone();
        tokio::spawn(async move {
            if let Err(e) = read_responses(stdout, pending_clone).await {
                warn!(error = %e, "LSP response reader exited");
            }
        });

        let client = Self {
            server,
            root: root.to_path_buf(),
            next_id: AtomicU64::new(1),
            stdin: Arc::new(Mutex::new(stdin)),
            pending,
            child: Arc::new(Mutex::new(child)),
            initialized: Arc::new(Mutex::new(false)),
        };

        // Send initialize request.
        client.initialize().await?;

        Ok(client)
    }

    /// Send the LSP initialize request.
    async fn initialize(&self) -> Result<()> {
        let root_uri = format!("file://{}", self.root.display());
        let params = json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "capabilities": {
                "textDocument": {
                    "definition": { "dynamicRegistration": false },
                    "references": { "dynamicRegistration": false },
                    "hover": {
                        "dynamicRegistration": false,
                        "contentFormat": ["plaintext"]
                    },
                    "documentSymbol": {
                        "dynamicRegistration": false,
                        "hierarchicalDocumentSymbolSupport": true
                    },
                    "synchronization": {
                        "didOpen": true,
                        "didChange": true,
                        "didSave": true
                    }
                },
                "workspace": {
                    "symbol": { "dynamicRegistration": false },
                    "workspaceFolders": false
                }
            }
        });

        let _resp = self.request("initialize", params).await?;

        // Send initialized notification.
        self.notify("initialized", json!({})).await?;

        *self.initialized.lock().await = true;
        debug!(root = %self.root.display(), "LSP server initialized");
        Ok(())
    }

    /// Send a textDocument/didOpen notification for a file.
    pub async fn did_open(&self, file_path: &Path, content: &str) -> Result<()> {
        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let language_id = super::detection::language_id_for_extension(ext);
        let uri = format!("file://{}", file_path.display());

        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": 1,
                    "text": content
                }
            }),
        )
        .await
    }

    /// Go to definition of the symbol at the given position.
    pub async fn goto_definition(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<Location>> {
        let uri = format!("file://{}", file_path.display());
        let params = json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        });

        let resp = self.request("textDocument/definition", params).await?;
        parse_locations(resp)
    }

    /// Find all references to the symbol at the given position.
    pub async fn find_references(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<Location>> {
        let uri = format!("file://{}", file_path.display());
        let params = json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
            "context": { "includeDeclaration": true }
        });

        let resp = self.request("textDocument/references", params).await?;
        parse_locations(resp)
    }

    /// Get hover info (type signature / documentation) at the given position.
    pub async fn hover(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Option<HoverResult>> {
        let uri = format!("file://{}", file_path.display());
        let params = json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        });

        let resp = self.request("textDocument/hover", params).await?;
        match resp {
            Some(val) => {
                let contents = extract_hover_contents(&val);
                if contents.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(HoverResult { contents }))
                }
            }
            None => Ok(None),
        }
    }

    /// Get all symbols in a document.
    pub async fn document_symbols(&self, file_path: &Path) -> Result<Vec<SymbolInfo>> {
        let uri = format!("file://{}", file_path.display());
        let params = json!({
            "textDocument": { "uri": uri }
        });

        let resp = self.request("textDocument/documentSymbol", params).await?;
        match resp {
            Some(val) => parse_symbols(&val, &uri),
            None => Ok(vec![]),
        }
    }

    /// Search for symbols across the workspace.
    pub async fn workspace_symbols(&self, query: &str) -> Result<Vec<SymbolInfo>> {
        let params = json!({ "query": query });
        let resp = self.request("workspace/symbol", params).await?;
        match resp {
            Some(val) => parse_symbols(&val, ""),
            None => Ok(vec![]),
        }
    }

    /// The language ID this client handles.
    pub fn language_id(&self) -> &str {
        &self.server.language_id
    }

    // ── Low-level JSON-RPC ──────────────────────────────────────────────────

    /// Send a JSON-RPC request and wait for the response.
    async fn request(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<Option<serde_json::Value>> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let body = serde_json::to_string(&request)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());

        let mut stdin = self.stdin.lock().await;
        stdin.write_all(header.as_bytes()).await?;
        stdin.write_all(body.as_bytes()).await?;
        stdin.flush().await?;

        // Wait for response with timeout.
        let resp = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .context("LSP request timed out")?
            .context("LSP response channel closed")?;

        if let Some(err) = resp.error {
            bail!("LSP error {}: {}", err.code, err.message);
        }

        Ok(resp.result)
    }

    /// Send a JSON-RPC notification (no response expected).
    async fn notify(&self, method: &str, params: serde_json::Value) -> Result<()> {
        let body = serde_json::to_string(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }))?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());

        let mut stdin = self.stdin.lock().await;
        stdin.write_all(header.as_bytes()).await?;
        stdin.write_all(body.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Gracefully shut down the server.
    pub async fn shutdown(&self) -> Result<()> {
        // Send shutdown request.
        let _ = self.request("shutdown", json!(null)).await;
        // Send exit notification.
        let _ = self.notify("exit", json!(null)).await;
        // Wait for process to exit.
        let mut child = self.child.lock().await;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await;
        Ok(())
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        // Best-effort kill on drop.
        if let Ok(mut child) = self.child.try_lock() {
            let _ = child.start_kill();
        }
    }
}

// ── Response reader task ────────────────────────────────────────────────────

async fn read_responses(
    stdout: ChildStdout,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>,
) -> Result<()> {
    let mut reader = BufReader::new(stdout);
    let mut header_buf = String::new();

    loop {
        // Read headers.
        header_buf.clear();
        let mut content_length: usize = 0;

        loop {
            header_buf.clear();
            let n = reader.read_line(&mut header_buf).await?;
            if n == 0 {
                return Ok(()); // EOF
            }
            let line = header_buf.trim();
            if line.is_empty() {
                break; // End of headers
            }
            if let Some(len_str) = line.strip_prefix("Content-Length: ") {
                content_length = len_str.parse().unwrap_or(0);
            }
        }

        if content_length == 0 {
            continue;
        }

        // Read body.
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body).await?;

        let response: JsonRpcResponse = match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(e) => {
                debug!(error = %e, "failed to parse LSP response, skipping");
                continue;
            }
        };

        // Route response to the waiting request.
        if let Some(id) = response.id {
            let mut map = pending.lock().await;
            if let Some(sender) = map.remove(&id) {
                let _ = sender.send(response);
            }
        }
        // Notifications (no id) are silently dropped.
    }
}

// ── Response parsing helpers ────────────────────────────────────────────────

fn parse_locations(result: Option<serde_json::Value>) -> Result<Vec<Location>> {
    match result {
        None => Ok(vec![]),
        Some(serde_json::Value::Null) => Ok(vec![]),
        Some(serde_json::Value::Array(arr)) => {
            let mut locations = Vec::new();
            for item in arr {
                if let Ok(loc) = serde_json::from_value::<Location>(item) {
                    locations.push(loc);
                }
            }
            Ok(locations)
        }
        Some(val) => {
            // Single location (not array).
            match serde_json::from_value::<Location>(val) {
                Ok(loc) => Ok(vec![loc]),
                Err(_) => Ok(vec![]),
            }
        }
    }
}

fn extract_hover_contents(val: &serde_json::Value) -> String {
    // Hover contents can be: string, MarkedString, MarkupContent, or array.
    if let Some(contents) = val.get("contents") {
        match contents {
            serde_json::Value::String(s) => return s.clone(),
            serde_json::Value::Object(obj) => {
                // MarkupContent { kind, value } or MarkedString { language, value }
                if let Some(value) = obj.get("value").and_then(|v| v.as_str()) {
                    return value.to_string();
                }
            }
            serde_json::Value::Array(arr) => {
                let parts: Vec<String> = arr
                    .iter()
                    .filter_map(|item| match item {
                        serde_json::Value::String(s) => Some(s.clone()),
                        serde_json::Value::Object(obj) => {
                            obj.get("value").and_then(|v| v.as_str()).map(String::from)
                        }
                        _ => None,
                    })
                    .collect();
                return parts.join("\n\n");
            }
            _ => {}
        }
    }
    String::new()
}

fn parse_symbols(val: &serde_json::Value, default_uri: &str) -> Result<Vec<SymbolInfo>> {
    let arr = match val.as_array() {
        Some(a) => a,
        None => return Ok(vec![]),
    };

    let mut symbols = Vec::new();
    for item in arr {
        // DocumentSymbol format (hierarchical).
        if item.get("range").is_some() && item.get("selectionRange").is_some() {
            parse_document_symbol(item, default_uri, None, &mut symbols);
            continue;
        }
        // SymbolInformation format (flat).
        if let Some(location) = item.get("location") {
            let name = item
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("<unknown>")
                .to_string();
            let kind_num = item.get("kind").and_then(|k| k.as_u64()).unwrap_or(0) as u32;
            let container = item
                .get("containerName")
                .and_then(|c| c.as_str())
                .map(String::from);

            if let Ok(loc) = serde_json::from_value::<Location>(location.clone()) {
                symbols.push(SymbolInfo {
                    name,
                    kind: SymbolKind::from_lsp_number(kind_num),
                    location: loc,
                    container,
                });
            }
        }
    }
    Ok(symbols)
}

fn parse_document_symbol(
    val: &serde_json::Value,
    uri: &str,
    container: Option<&str>,
    out: &mut Vec<SymbolInfo>,
) {
    let name = val
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("<unknown>")
        .to_string();
    let kind_num = val.get("kind").and_then(|k| k.as_u64()).unwrap_or(0) as u32;

    if let Some(range) = val.get("selectionRange")
        && let Ok(range) = serde_json::from_value::<Range>(range.clone())
    {
        out.push(SymbolInfo {
            name: name.clone(),
            kind: SymbolKind::from_lsp_number(kind_num),
            location: Location {
                uri: uri.to_string(),
                range,
            },
            container: container.map(String::from),
        });
    }

    // Recurse into children.
    if let Some(children) = val.get("children").and_then(|c| c.as_array()) {
        for child in children {
            parse_document_symbol(child, uri, Some(&name), out);
        }
    }
}
