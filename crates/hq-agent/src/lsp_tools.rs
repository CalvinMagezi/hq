//! LSP coding tools — semantic code intelligence via language servers.
//!
//! All tools are read-only and deferred (not included in every turn's prompt).
//! The agent discovers them via tool_search when it needs code intelligence.

use crate::lsp::LspManager;
use anyhow::Result;
use async_trait::async_trait;
use hq_core::types::ToolResult;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::debug;

use crate::coding::text_result;
use crate::tools::AgentTool;

/// Shared LSP manager instance across all LSP tools in a session.
pub type SharedLspManager = Arc<Mutex<LspManager>>;

/// Create a new shared LSP manager.
pub fn shared_lsp_manager() -> SharedLspManager {
    Arc::new(Mutex::new(LspManager::new()))
}

// ─── GotoDefinitionTool ────────────────────────────────────────

pub struct GotoDefinitionTool {
    pub manager: SharedLspManager,
}

#[async_trait]
impl AgentTool for GotoDefinitionTool {
    fn name(&self) -> &str {
        "goto_definition"
    }

    fn description(&self) -> &str {
        "Go to the definition of a symbol at a given file position. \
         Returns the target file path, line number, and surrounding context. \
         Position uses 0-based line and character numbers."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file_path", "line", "character"],
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file"
                },
                "line": {
                    "type": "integer",
                    "description": "0-based line number"
                },
                "character": {
                    "type": "integer",
                    "description": "0-based character offset"
                }
            }
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn should_defer(&self) -> bool {
        true
    }

    fn search_hint(&self) -> &str {
        "go to definition of a symbol, find where something is defined"
    }

    /// A hung language server would otherwise block indefinitely (and hold
    /// the shared `manager` mutex for every other LSP tool in the session);
    /// dropping this future on timeout releases that lock along with it.
    fn timeout_ms(&self) -> Option<u64> {
        Some(15_000)
    }

    async fn execute(&self, _id: &str, args: Value) -> Result<ToolResult> {
        let file_path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing file_path"))?;
        let line = args
            .get("line")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow::anyhow!("missing line"))? as u32;
        let character = args
            .get("character")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow::anyhow!("missing character"))? as u32;

        debug!(file = %file_path, line, character, "goto_definition");

        let manager = self.manager.lock().await;
        match manager
            .goto_definition(Path::new(file_path), line, character, 5)
            .await
        {
            Ok(result) => Ok(text_result(result)),
            Err(e) => Ok(text_result(format!("LSP error: {}", e))),
        }
    }
}

// ─── FindReferencesTool ────────────────────────────────────────

pub struct FindReferencesTool {
    pub manager: SharedLspManager,
}

#[async_trait]
impl AgentTool for FindReferencesTool {
    fn name(&self) -> &str {
        "find_references"
    }

    fn description(&self) -> &str {
        "Find all references to a symbol at a given position. \
         Returns locations grouped by file with one-line context, capped at 30 results."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file_path", "line", "character"],
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file"
                },
                "line": {
                    "type": "integer",
                    "description": "0-based line number"
                },
                "character": {
                    "type": "integer",
                    "description": "0-based character offset"
                }
            }
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn should_defer(&self) -> bool {
        true
    }

    fn search_hint(&self) -> &str {
        "find all references to a symbol, where is something used"
    }

    fn timeout_ms(&self) -> Option<u64> {
        Some(15_000)
    }

    async fn execute(&self, _id: &str, args: Value) -> Result<ToolResult> {
        let file_path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing file_path"))?;
        let line = args
            .get("line")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow::anyhow!("missing line"))? as u32;
        let character = args
            .get("character")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow::anyhow!("missing character"))? as u32;

        debug!(file = %file_path, line, character, "find_references");

        let manager = self.manager.lock().await;
        match manager
            .find_references(Path::new(file_path), line, character, 30)
            .await
        {
            Ok(result) => Ok(text_result(result)),
            Err(e) => Ok(text_result(format!("LSP error: {}", e))),
        }
    }
}

// ─── HoverTool ─────────────────────────────────────────────────

pub struct HoverTool {
    pub manager: SharedLspManager,
}

#[async_trait]
impl AgentTool for HoverTool {
    fn name(&self) -> &str {
        "hover"
    }

    fn description(&self) -> &str {
        "Get type information and documentation for a symbol at a given position. \
         Returns the type signature. Use to understand what a symbol is without reading its definition."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file_path", "line", "character"],
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file"
                },
                "line": {
                    "type": "integer",
                    "description": "0-based line number"
                },
                "character": {
                    "type": "integer",
                    "description": "0-based character offset"
                }
            }
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn should_defer(&self) -> bool {
        true
    }

    fn search_hint(&self) -> &str {
        "hover type info, what type is this symbol, type signature"
    }

    fn timeout_ms(&self) -> Option<u64> {
        Some(15_000)
    }

    async fn execute(&self, _id: &str, args: Value) -> Result<ToolResult> {
        let file_path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing file_path"))?;
        let line = args
            .get("line")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow::anyhow!("missing line"))? as u32;
        let character = args
            .get("character")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow::anyhow!("missing character"))? as u32;

        debug!(file = %file_path, line, character, "hover");

        let manager = self.manager.lock().await;
        match manager.hover(Path::new(file_path), line, character).await {
            Ok(result) => Ok(text_result(result)),
            Err(e) => Ok(text_result(format!("LSP error: {}", e))),
        }
    }
}

// ─── DocumentSymbolsTool ───────────────────────────────────────

pub struct DocumentSymbolsTool {
    pub manager: SharedLspManager,
}

#[async_trait]
impl AgentTool for DocumentSymbolsTool {
    fn name(&self) -> &str {
        "document_symbols"
    }

    fn description(&self) -> &str {
        "Get a structural outline of all symbols in a file (functions, types, methods, etc). \
         Returns name, kind, and line number for each symbol."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file_path"],
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file"
                }
            }
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn should_defer(&self) -> bool {
        true
    }

    fn search_hint(&self) -> &str {
        "list symbols in a file, document outline, what's in this file"
    }

    fn timeout_ms(&self) -> Option<u64> {
        Some(15_000)
    }

    async fn execute(&self, _id: &str, args: Value) -> Result<ToolResult> {
        let file_path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing file_path"))?;

        debug!(file = %file_path, "document_symbols");

        let manager = self.manager.lock().await;
        match manager.document_symbols(Path::new(file_path)).await {
            Ok(result) => Ok(text_result(result)),
            Err(e) => Ok(text_result(format!("LSP error: {}", e))),
        }
    }
}

// ─── WorkspaceSymbolsTool ──────────────────────────────────────

pub struct WorkspaceSymbolsTool {
    pub manager: SharedLspManager,
}

#[async_trait]
impl AgentTool for WorkspaceSymbolsTool {
    fn name(&self) -> &str {
        "workspace_symbols"
    }

    fn description(&self) -> &str {
        "Search for symbols across the entire workspace/project by name. \
         Returns matching symbols with their file paths and line numbers."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file_path", "query"],
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Any file in the project (used to determine project root)"
                },
                "query": {
                    "type": "string",
                    "description": "Symbol name to search for"
                }
            }
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn should_defer(&self) -> bool {
        true
    }

    fn search_hint(&self) -> &str {
        "search symbols across project, find function/type by name"
    }

    fn timeout_ms(&self) -> Option<u64> {
        Some(15_000)
    }

    async fn execute(&self, _id: &str, args: Value) -> Result<ToolResult> {
        let file_path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing file_path"))?;
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing query"))?;

        debug!(file = %file_path, query, "workspace_symbols");

        let manager = self.manager.lock().await;
        match manager.workspace_symbols(Path::new(file_path), query).await {
            Ok(result) => Ok(text_result(result)),
            Err(e) => Ok(text_result(format!("LSP error: {}", e))),
        }
    }
}

// ─── Registration ──────────────────────────────────────────────

/// Register all LSP tools with a shared manager.
pub fn register_lsp_tools(tools: &mut Vec<Box<dyn AgentTool>>, manager: SharedLspManager) {
    tools.push(Box::new(GotoDefinitionTool {
        manager: manager.clone(),
    }));
    tools.push(Box::new(FindReferencesTool {
        manager: manager.clone(),
    }));
    tools.push(Box::new(HoverTool {
        manager: manager.clone(),
    }));
    tools.push(Box::new(DocumentSymbolsTool {
        manager: manager.clone(),
    }));
    tools.push(Box::new(WorkspaceSymbolsTool { manager }));
}
