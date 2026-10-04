//! LSP manager — connection pool with per-(language, root) server instances.

use super::client::LspClient;
use super::detection::{LanguageServer, detect_language_server};
use super::protocol::*;
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

/// Maximum number of concurrent LSP server instances.
const MAX_SERVERS: usize = 5;

/// Key for the server pool: (language_id, project_root).
type ServerKey = (String, PathBuf);

/// Manages LSP server lifecycle and provides high-level code intelligence operations.
pub struct LspManager {
    servers: Arc<Mutex<HashMap<ServerKey, Arc<LspClient>>>>,
    /// Tracks insertion order for LRU eviction.
    order: Arc<Mutex<Vec<ServerKey>>>,
    /// Files that have been opened with didOpen (to avoid duplicate opens).
    opened_files: Arc<Mutex<HashMap<PathBuf, String>>>,
}

impl LspManager {
    pub fn new() -> Self {
        Self {
            servers: Arc::new(Mutex::new(HashMap::new())),
            order: Arc::new(Mutex::new(Vec::new())),
            opened_files: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get or start an LSP client for the given file.
    ///
    /// Auto-detects the language server from the project root and file extension.
    /// Opens the file via didOpen if not already opened.
    pub async fn client_for_file(&self, file_path: &Path) -> Result<Arc<LspClient>> {
        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");

        // Find the project root by walking up to find marker files.
        let root = find_project_root(file_path)
            .unwrap_or_else(|| file_path.parent().unwrap_or(Path::new("/")).to_path_buf());

        // Find a matching server config.
        let servers = detect_language_server(&root);
        let server = servers
            .into_iter()
            .find(|s| s.extensions.iter().any(|e| e == ext))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no language server available for .{} files in {}",
                    ext,
                    root.display()
                )
            })?;

        let key = (server.language_id.clone(), root.clone());
        let client = self.get_or_start(key.clone(), server, &root).await?;

        // Ensure the file is opened.
        self.ensure_opened(file_path, &client).await?;

        Ok(client)
    }

    /// Ensure a file is opened in the LSP server.
    async fn ensure_opened(&self, file_path: &Path, client: &LspClient) -> Result<()> {
        let canonical = file_path.to_path_buf();
        let mut opened = self.opened_files.lock().await;
        if opened.contains_key(&canonical) {
            return Ok(());
        }

        let content = tokio::fs::read_to_string(file_path)
            .await
            .unwrap_or_default();
        client.did_open(file_path, &content).await?;
        opened.insert(canonical, client.language_id().to_string());
        Ok(())
    }

    /// Get an existing client or start a new one.
    async fn get_or_start(
        &self,
        key: ServerKey,
        server: LanguageServer,
        root: &Path,
    ) -> Result<Arc<LspClient>> {
        let mut map = self.servers.lock().await;

        if let Some(client) = map.get(&key) {
            // Move to end of LRU order.
            let mut order = self.order.lock().await;
            if let Some(pos) = order.iter().position(|k| k == &key) {
                order.remove(pos);
            }
            order.push(key);
            return Ok(client.clone());
        }

        // Evict oldest if at capacity.
        if map.len() >= MAX_SERVERS {
            let mut order = self.order.lock().await;
            if let Some(oldest_key) = order.first().cloned() {
                if let Some(old_client) = map.remove(&oldest_key) {
                    info!(
                        language = %oldest_key.0,
                        root = %oldest_key.1.display(),
                        "evicting LSP server (LRU)"
                    );
                    // Shutdown in background.
                    tokio::spawn(async move {
                        let _ = old_client.shutdown().await;
                    });
                }
                order.remove(0);
            }
        }

        // Start new server.
        let client = Arc::new(LspClient::start(server, root).await?);
        map.insert(key.clone(), client.clone());
        self.order.lock().await.push(key);
        Ok(client)
    }

    // ── High-level operations ───────────────────────────────────────────────

    /// Go to definition. Returns the target location(s) with context lines.
    pub async fn goto_definition(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
        context_lines: usize,
    ) -> Result<String> {
        let client = self.client_for_file(file_path).await?;
        let locations = client.goto_definition(file_path, line, character).await?;

        if locations.is_empty() {
            return Ok("No definition found.".into());
        }

        let mut output = String::new();
        for loc in locations.iter().take(5) {
            output.push_str(&format!("Definition at {}\n", loc.display()));
            if let Some(path) = loc.file_path() {
                let context = read_context_lines(Path::new(&path), &loc.range, context_lines).await;
                output.push_str(&context);
                output.push('\n');
            }
        }
        Ok(output)
    }

    /// Find all references. Returns grouped by file, capped at max_results.
    pub async fn find_references(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
        max_results: usize,
    ) -> Result<String> {
        let client = self.client_for_file(file_path).await?;
        let locations = client.find_references(file_path, line, character).await?;

        if locations.is_empty() {
            return Ok("No references found.".into());
        }

        let total = locations.len();

        // Group by file.
        let mut by_file: HashMap<String, Vec<&Location>> = HashMap::new();
        for loc in locations.iter().take(max_results) {
            let key = loc.file_path().unwrap_or_else(|| loc.uri.clone());
            by_file.entry(key).or_default().push(loc);
        }

        let mut output = format!("{} reference(s) found", total);
        if total > max_results {
            output.push_str(&format!(" (showing first {})", max_results));
        }
        output.push_str(":\n\n");

        for (file, locs) in &by_file {
            output.push_str(&format!("{}:\n", file));
            for loc in locs {
                let line_num = loc.range.start.line + 1;
                let context =
                    read_single_line(Path::new(file), loc.range.start.line as usize).await;
                output.push_str(&format!("  L{}: {}\n", line_num, context.trim()));
            }
            output.push('\n');
        }

        Ok(output)
    }

    /// Hover info at position. Returns type signature only (not full docs).
    pub async fn hover(&self, file_path: &Path, line: u32, character: u32) -> Result<String> {
        let client = self.client_for_file(file_path).await?;
        match client.hover(file_path, line, character).await? {
            Some(hover) => Ok(hover.contents),
            None => Ok("No hover info available.".into()),
        }
    }

    /// Document symbols — structural outline.
    pub async fn document_symbols(&self, file_path: &Path) -> Result<String> {
        let client = self.client_for_file(file_path).await?;
        let symbols = client.document_symbols(file_path).await?;

        if symbols.is_empty() {
            return Ok("No symbols found.".into());
        }

        let mut output = String::new();
        for sym in &symbols {
            let line = sym.location.range.start.line + 1;
            let container = sym
                .container
                .as_deref()
                .map(|c| format!(" (in {})", c))
                .unwrap_or_default();
            output.push_str(&format!(
                "  L{:>4} {:12} {}{}\n",
                line,
                sym.kind.to_string(),
                sym.name,
                container
            ));
        }
        Ok(output)
    }

    /// Workspace symbol search.
    pub async fn workspace_symbols(&self, file_path: &Path, query: &str) -> Result<String> {
        let client = self.client_for_file(file_path).await?;
        let symbols = client.workspace_symbols(query).await?;

        if symbols.is_empty() {
            return Ok(format!("No symbols matching '{}'.", query));
        }

        let mut output = format!("{} symbol(s) matching '{}':\n", symbols.len(), query);
        for sym in symbols.iter().take(30) {
            let file = sym.location.file_path().unwrap_or_default();
            let line = sym.location.range.start.line + 1;
            output.push_str(&format!(
                "  {} {:12} {}:{}\n",
                sym.name,
                sym.kind.to_string(),
                file,
                line
            ));
        }
        if symbols.len() > 30 {
            output.push_str(&format!("  ... and {} more\n", symbols.len() - 30));
        }
        Ok(output)
    }
}

impl Default for LspManager {
    fn default() -> Self {
        Self::new()
    }
}

// ── File helpers ────────────────────────────────────────────────────────────

/// Find the project root by walking up from a file path looking for markers.
fn find_project_root(file_path: &Path) -> Option<PathBuf> {
    let markers = [
        "Cargo.toml",
        "tsconfig.json",
        "package.json",
        "pyproject.toml",
        "go.mod",
        ".git",
    ];

    let mut dir = file_path.parent()?;
    loop {
        for marker in &markers {
            if dir.join(marker).exists() {
                return Some(dir.to_path_buf());
            }
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent,
            _ => return None,
        }
    }
}

/// Read context lines around a range in a file.
async fn read_context_lines(path: &Path, range: &Range, context: usize) -> String {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    let lines: Vec<&str> = content.lines().collect();
    let start = (range.start.line as usize).saturating_sub(context);
    let end = ((range.end.line as usize) + context + 1).min(lines.len());

    let mut output = String::new();
    for (i, line) in lines.iter().enumerate().take(end).skip(start) {
        let marker = if i == range.start.line as usize {
            ">"
        } else {
            " "
        };
        output.push_str(&format!("{} {:>4} | {}\n", marker, i + 1, line));
    }
    output
}

/// Read a single line from a file.
async fn read_single_line(path: &Path, line: usize) -> String {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    content.lines().nth(line).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_project_root() {
        // Should find the agent-hq root from any file in it.
        let root = find_project_root(Path::new(env!("CARGO_MANIFEST_DIR")));
        assert!(root.is_some());
        let root = root.unwrap();
        assert!(root.join("Cargo.toml").exists());
    }
}
