//! LSP protocol types — minimal subset for the operations we support.

use serde::{Deserialize, Serialize};

/// Position in a text document (0-based line and character).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

/// A range in a text document.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

/// A location in a file (file URI + range).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub uri: String,
    pub range: Range,
}

impl Location {
    /// Convert file:// URI to a plain filesystem path.
    pub fn file_path(&self) -> Option<String> {
        self.uri
            .strip_prefix("file://")
            .map(|p| urlencoding::decode(p).unwrap_or_default().into_owned())
    }

    /// Human-readable display: "path:line:col"
    pub fn display(&self) -> String {
        let path = self.file_path().unwrap_or_else(|| self.uri.clone());
        format!(
            "{}:{}:{}",
            path,
            self.range.start.line + 1,
            self.range.start.character + 1
        )
    }
}

/// Symbol kinds from LSP spec.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SymbolKind {
    File,
    Module,
    Namespace,
    Package,
    Class,
    Method,
    Property,
    Field,
    Constructor,
    Enum,
    Interface,
    Function,
    Variable,
    Constant,
    String,
    Number,
    Boolean,
    Array,
    Object,
    Key,
    Null,
    EnumMember,
    Struct,
    Event,
    Operator,
    TypeParameter,
    Unknown,
}

impl SymbolKind {
    pub fn from_lsp_number(n: u32) -> Self {
        match n {
            1 => Self::File,
            2 => Self::Module,
            3 => Self::Namespace,
            4 => Self::Package,
            5 => Self::Class,
            6 => Self::Method,
            7 => Self::Property,
            8 => Self::Field,
            9 => Self::Constructor,
            10 => Self::Enum,
            11 => Self::Interface,
            12 => Self::Function,
            13 => Self::Variable,
            14 => Self::Constant,
            15 => Self::String,
            16 => Self::Number,
            17 => Self::Boolean,
            18 => Self::Array,
            19 => Self::Object,
            20 => Self::Key,
            21 => Self::Null,
            22 => Self::EnumMember,
            23 => Self::Struct,
            24 => Self::Event,
            25 => Self::Operator,
            26 => Self::TypeParameter,
            _ => Self::Unknown,
        }
    }
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// Compact symbol info returned by document/workspace symbol requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: SymbolKind,
    pub location: Location,
    /// Container name (e.g., the struct that contains a method).
    pub container: Option<String>,
}

/// Hover content returned by textDocument/hover.
#[derive(Debug, Clone)]
pub struct HoverResult {
    pub contents: String,
}

/// JSON-RPC message types.
#[derive(Debug, Serialize)]
pub(crate) struct JsonRpcRequest {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JsonRpcResponse {
    pub id: Option<u64>,
    pub result: Option<serde_json::Value>,
    pub error: Option<JsonRpcError>,
    #[allow(dead_code)]
    pub method: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JsonRpcError {
    pub code: i64,
    pub message: String,
}
