//! Shared binary/config-path resolution helpers.

use std::path::PathBuf;
use std::sync::LazyLock;

/// The per-OS Claude Desktop config file path, or `None` on a platform with
/// no known location. `hq mcp install` writes it; other call sites only check it.
pub fn claude_desktop_config_path() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        // Windows keeps this under AppData, not the home directory.
        return dirs::config_dir().map(|d| d.join("Claude/claude_desktop_config.json"));
    }
    let home = dirs::home_dir()?;
    if cfg!(target_os = "macos") {
        Some(home.join("Library/Application Support/Claude/claude_desktop_config.json"))
    } else if cfg!(target_os = "linux") {
        Some(home.join(".config/claude/claude_desktop_config.json"))
    } else {
        None
    }
}

/// Resolve the `gws` (Google Workspace CLI) binary path.
///
/// Checks the Homebrew install locations first (matching how most macOS
/// installs put it there), then falls back to a bare `$PATH` lookup — which
/// is the only thing that works on a Linux VPS (no Homebrew) or any install
/// that put `gws` somewhere else entirely.
pub fn resolve_gws_binary() -> &'static str {
    static BIN: LazyLock<&'static str> = LazyLock::new(|| {
        if std::path::Path::new("/opt/homebrew/bin/gws").exists() {
            "/opt/homebrew/bin/gws"
        } else if std::path::Path::new("/usr/local/bin/gws").exists() {
            "/usr/local/bin/gws"
        } else {
            "gws"
        }
    });
    *BIN
}
