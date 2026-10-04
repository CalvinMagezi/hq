//! Language server detection — auto-detect which server to use from project files.

use std::path::Path;
use tracing::debug;

/// Supported language server configurations.
#[derive(Debug, Clone)]
pub struct LanguageServer {
    /// Command to spawn the server.
    pub command: String,
    /// Arguments to pass.
    pub args: Vec<String>,
    /// Language ID for textDocument/didOpen.
    pub language_id: String,
    /// File extensions this server handles.
    pub extensions: Vec<String>,
}

/// Detect the appropriate language server for a project root.
///
/// Checks for project marker files (Cargo.toml, tsconfig.json, etc.)
/// and verifies the server binary is installed.
pub fn detect_language_server(project_root: &Path) -> Vec<LanguageServer> {
    let mut servers = Vec::new();

    // Rust: rust-analyzer
    if project_root.join("Cargo.toml").exists() && which::which("rust-analyzer").is_ok() {
        debug!("detected rust-analyzer for Cargo.toml project");
        servers.push(LanguageServer {
            command: "rust-analyzer".into(),
            args: vec![],
            language_id: "rust".into(),
            extensions: vec!["rs".into()],
        });
    }

    // TypeScript/JavaScript: typescript-language-server
    let has_ts = project_root.join("tsconfig.json").exists()
        || project_root.join("jsconfig.json").exists()
        || project_root.join("package.json").exists();
    if has_ts && which::which("typescript-language-server").is_ok() {
        debug!("detected typescript-language-server for TS/JS project");
        servers.push(LanguageServer {
            command: "typescript-language-server".into(),
            args: vec!["--stdio".into()],
            language_id: "typescript".into(),
            extensions: vec!["ts".into(), "tsx".into(), "js".into(), "jsx".into()],
        });
    }

    // Python: pyright
    let has_py = project_root.join("pyproject.toml").exists()
        || project_root.join("setup.py").exists()
        || project_root.join("requirements.txt").exists();
    if has_py {
        // Try pyright first, then pylsp
        if which::which("pyright-langserver").is_ok() {
            debug!("detected pyright for Python project");
            servers.push(LanguageServer {
                command: "pyright-langserver".into(),
                args: vec!["--stdio".into()],
                language_id: "python".into(),
                extensions: vec!["py".into()],
            });
        } else if which::which("pylsp").is_ok() {
            debug!("detected pylsp for Python project");
            servers.push(LanguageServer {
                command: "pylsp".into(),
                args: vec![],
                language_id: "python".into(),
                extensions: vec!["py".into()],
            });
        }
    }

    // Go: gopls
    if project_root.join("go.mod").exists() && which::which("gopls").is_ok() {
        debug!("detected gopls for Go project");
        servers.push(LanguageServer {
            command: "gopls".into(),
            args: vec![],
            language_id: "go".into(),
            extensions: vec!["go".into()],
        });
    }

    servers
}

/// Get the language ID for a file extension.
pub fn language_id_for_extension(ext: &str) -> &str {
    match ext {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "hpp" | "cc" | "cxx" => "cpp",
        "rb" => "ruby",
        _ => ext,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_id_for_extension() {
        assert_eq!(language_id_for_extension("rs"), "rust");
        assert_eq!(language_id_for_extension("ts"), "typescript");
        assert_eq!(language_id_for_extension("py"), "python");
        assert_eq!(language_id_for_extension("go"), "go");
        assert_eq!(language_id_for_extension("xyz"), "xyz");
    }
}
