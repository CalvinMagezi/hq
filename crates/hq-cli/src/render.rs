//! Themed ANSI output for the inline chat REPL and the status/health commands.

use std::path::{Path, PathBuf};

pub type Rgb = (u8, u8, u8);

#[derive(Clone)]
pub struct Theme {
    pub primary: Rgb,
    pub secondary: Rgb,
    pub accent: Rgb,
    pub success: Rgb,
    pub error: Rgb,
    pub warning: Rgb,
    pub info: Rgb,
    pub text_dim: Rgb,
    pub text_muted: Rgb,
    pub border: Rgb,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            primary: (130, 170, 255),
            secondary: (180, 140, 255),
            accent: (255, 180, 100),
            success: (100, 220, 100),
            error: (255, 100, 100),
            warning: (255, 200, 60),
            info: (100, 180, 255),
            text_dim: (140, 140, 160),
            text_muted: (90, 90, 110),
            border: (60, 60, 80),
        }
    }
}

/// ANSI SGR foreground parameter for a truecolor RGB value.
pub fn fg(&(r, g, b): &Rgb) -> String {
    format!("38;2;{r};{g};{b}")
}

pub fn colored(text: &str, color: &Rgb) -> String {
    format!("\x1b[{}m{}\x1b[0m", fg(color), text)
}

pub fn bold(text: &str, color: &Rgb) -> String {
    format!("\x1b[1;{}m{}\x1b[0m", fg(color), text)
}

pub fn dim(text: &str, color: &Rgb) -> String {
    format!("\x1b[2;{}m{}\x1b[0m", fg(color), text)
}

pub fn status_ok(msg: &str, color: &Rgb) -> String {
    format!("  \x1b[1;{}m[OK]\x1b[0m   {}", fg(color), msg)
}

pub fn status_fail(msg: &str, color: &Rgb) -> String {
    format!("  \x1b[1;{}m[FAIL]\x1b[0m {}", fg(color), msg)
}

pub fn status_warn(msg: &str, color: &Rgb) -> String {
    format!("  \x1b[1;{}m[WARN]\x1b[0m {}", fg(color), msg)
}

pub fn status_dim(msg: &str, color: &Rgb) -> String {
    format!("  \x1b[{}m[----]\x1b[0m {}", fg(color), msg)
}

fn tool_icon(tool_name: &str) -> &'static str {
    match tool_name {
        "read_file" | "view_file" | "vault_read_note" | "read_resource" => "📄 Read",
        "edit_file"
        | "write_file"
        | "replace_file_content"
        | "multi_replace_file_content"
        | "write_to_file"
        | "vault_write_note" => "✏️ Edit",
        "run_command" | "bash" | "shell" => "⚡ Bash",
        "grep" | "glob" | "vault_search" => "🔍 Search",
        "spawn_subagents" => "🤖 Agent",
        _ => "⚙️ Tool",
    }
}

pub fn render_tool_start(tool_name: &str, theme: &Theme) -> String {
    format!(
        "  {} {}",
        bold(tool_icon(tool_name), &theme.info),
        colored(&format!(": {tool_name}"), &theme.accent),
    )
}

pub fn render_error(msg: &str, theme: &Theme) -> String {
    format!("  {} {}", bold("✗", &theme.error), colored(msg, &theme.error))
}

pub fn render_compaction(old_count: usize, new_count: usize, theme: &Theme) -> String {
    format!(
        "  {} {}",
        colored("↺", &theme.info),
        dim(
            &format!("Context compacted: {old_count} → {new_count} messages"),
            &theme.text_dim,
        ),
    )
}

pub fn render_tool_progress(tool_name: &str, message: &str, theme: &Theme) -> String {
    format!(
        "  {} {}",
        dim(&format!("  ... [{tool_name}]"), &theme.text_muted),
        dim(message, &theme.text_dim),
    )
}

const SEARCH_RESULT_CHARS: usize = 1000;
const DEFAULT_RESULT_CHARS: usize = 500;

/// Reads show only a size (the model has the content, the human needs
/// confirmation), writes their first line, searches and shell output more
/// room than other tools.
pub fn format_tool_result(tool_name: &str, result: &str) -> String {
    match tool_name {
        "read_file" | "view_file" | "vault_read_note" | "read_resource" => {
            format!("({} lines, {} bytes)", result.lines().count(), result.len())
        }
        "write_file" | "edit" | "patch" | "vault_write_note" => {
            result.lines().next().unwrap_or(result).to_string()
        }
        "grep" | "glob" | "vault_search" | "bash" | "shell" | "run_command" => {
            truncated(result, SEARCH_RESULT_CHARS)
        }
        _ => truncated(result, DEFAULT_RESULT_CHARS),
    }
}

fn truncated(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let end = s.floor_char_boundary(max_bytes);
    format!("{}... ({} bytes)", &s[..end], s.len())
}

/// A chat session saved by `/save`, reloaded by `/resume`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SavedSession {
    pub id: String,
    pub model: String,
    pub title: String,
    pub messages: Vec<SavedMessage>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SavedMessage {
    pub role: String,
    pub content: String,
    pub ttft_ms: Option<u64>,
}

impl SavedSession {
    pub fn save(&self, dir: &Path) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("{}.json", self.id));
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(&path, json)?;
        Ok(path)
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        serde_json::from_str(&json).map_err(std::io::Error::other)
    }

    /// Saved sessions in `dir` as (path, id), newest first.
    pub fn list_sessions(dir: &Path) -> std::io::Result<Vec<(PathBuf, String)>> {
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut entries: Vec<_> = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|e| Some((e.path(), e.metadata().ok()?.modified().ok()?)))
            .collect();
        entries.sort_by_key(|a| std::cmp::Reverse(a.1));
        Ok(entries
            .into_iter()
            .filter_map(|(path, _)| {
                let name = path.file_stem()?.to_string_lossy().to_string();
                Some((path, name))
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_result_does_not_echo_file_contents() {
        let marker = "fn super_secret_implementation_detail() {}";
        let formatted = format_tool_result("read_file", &format!("one\ntwo\n{marker}\n"));
        assert!(!formatted.contains(marker), "{formatted}");
        assert!(formatted.contains("3 lines"));
    }

    #[test]
    fn long_results_truncate_on_a_char_boundary() {
        let formatted = format_tool_result("other", &"é".repeat(DEFAULT_RESULT_CHARS));
        assert!(formatted.ends_with(&format!("... ({} bytes)", DEFAULT_RESULT_CHARS * 2)));
    }

    #[test]
    fn saved_session_round_trips_and_lists() {
        let dir = tempfile::TempDir::new().unwrap();
        let saved = SavedSession {
            id: "s1".into(),
            model: "m".into(),
            title: "t".into(),
            messages: vec![SavedMessage { role: "user".into(), content: "hi".into(), ttft_ms: None }],
            tokens_in: 1,
            tokens_out: 2,
            cost_usd: 0.0,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let path = saved.save(dir.path()).unwrap();
        assert_eq!(SavedSession::load(&path).unwrap().messages[0].content, "hi");
        assert_eq!(SavedSession::list_sessions(dir.path()).unwrap()[0].1, "s1");
    }
}
