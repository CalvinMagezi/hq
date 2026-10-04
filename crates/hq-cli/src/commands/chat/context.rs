use std::path::Path;

use hq_agent::session::AgentSession;

/// Append project-specific context (git info, CLAUDE.md, etc.) to the system prompt.
pub(super) fn inject_project_context(session: &mut AgentSession, cwd: &Path) {
    let git_branch = detect_git_branch(cwd);
    let git_status = detect_git_status(cwd);
    let project_ctx = read_project_context(cwd);

    let base = session.system_prompt().unwrap_or("").to_string();
    let enriched = format!(
        "{base}\n\n\
         # Project Context\n\n\
         - Working directory: {cwd}\n\
         - Git branch: {branch}\n\
         - Git status: {status}\n\
         {project}",
        cwd = cwd.display(),
        branch = git_branch,
        status = git_status,
        project = if project_ctx.is_empty() {
            String::new()
        } else {
            format!("\n# Project Instructions\n\n{project_ctx}")
        },
    );
    session.set_system_prompt(enriched);
}

// ── Image attachment helpers ──────────────────────────────────────
//
// `ChatMessage` (hq-core) is text-only, so the CLI has no way to hand raw
// image bytes to the model. The Telegram relay hit the same wall and settled
// on on-device OCR (`telegram/media.rs::handle_media`, macOS Vision) rather
// than requiring a vision-capable model — this mirrors that approach so a
// pasted image path works the same way from either interface.

/// Extensions recognized as images when scanning user input for attachments.
/// Kept in sync with `telegram/media.rs`'s photo detection, plus a few extra
/// formats (bmp/tiff/heic) a CLI user is more likely to reference directly
/// from disk (e.g. a screenshot util's default format).
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "tiff", "heic"];

fn is_image_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| IMAGE_EXTENSIONS.contains(&e.to_lowercase().as_str()))
            .unwrap_or(false)
}

/// True when the first whitespace-delimited token of `input` is an existing
/// path on disk — used to tell a pasted file path (which also starts with
/// `/`) apart from an actual slash command before treating it as "unknown".
pub(super) fn looks_like_existing_path(input: &str) -> bool {
    input
        .split_whitespace()
        .next()
        .map(|first| Path::new(first).exists())
        .unwrap_or(false)
}

/// Scan `input` for image file paths, run on-device OCR against each, and
/// append the extracted text so the model can "see" the image's content even
/// though the chat message itself stays plain text. Best-effort: OCR
/// failures (e.g. non-macOS) are reported inline rather than dropped, and
/// never block sending the rest of the message.
pub(super) async fn augment_with_image_attachments(input: &str) -> String {
    let mut augmented = input.to_string();
    let mut seen = std::collections::HashSet::new();

    for token in input.split_whitespace() {
        let trimmed =
            token.trim_matches(|c: char| matches!(c, '"' | '\'' | ',' | ';' | '(' | ')' | '`'));
        if trimmed.is_empty() || !seen.insert(trimmed.to_string()) {
            continue;
        }
        let path = Path::new(trimmed);
        if !is_image_file(path) {
            continue;
        }

        augmented.push_str(&format!("\n[Image attached: {trimmed}]"));
        match hq_convert::OcrEngine::extract_text(path).await {
            Ok(text) if !text.trim().is_empty() => {
                augmented.push_str(&format!("\n[OCR text from image]:\n{text}"));
            }
            Ok(_) => augmented.push_str("\n[No text detected in image]"),
            Err(e) => augmented.push_str(&format!("\n[OCR unavailable: {e}]")),
        }
    }

    augmented
}

// ── Project context helpers ──────────────────────────────────────

pub(super) fn detect_git_branch(cwd: &Path) -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(cwd)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "n/a".into())
}

fn detect_git_status(cwd: &Path) -> String {
    std::process::Command::new("git")
        .args(["status", "--short"])
        .current_dir(cwd)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                let text = String::from_utf8_lossy(&o.stdout).to_string();
                if text.trim().is_empty() {
                    Some("clean".into())
                } else {
                    let lines: Vec<&str> = text.lines().collect();
                    Some(format!("{} changed files", lines.len()))
                }
            } else {
                None
            }
        })
        .unwrap_or_else(|| "not a git repo".into())
}

/// Read project-level config files (CLAUDE.md, AGENTS.md, .hq/context.md).
/// Walks from cwd up to git root looking for these files.
fn read_project_context(cwd: &Path) -> String {
    let candidates = [
        "CLAUDE.md",
        "AGENTS.md",
        ".hq/context.md",
        ".claude/CLAUDE.md",
    ];

    let mut parts = Vec::new();

    let git_root = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(std::path::PathBuf::from(
                    String::from_utf8_lossy(&o.stdout).trim().to_string(),
                ))
            } else {
                None
            }
        });

    let mut dir = cwd.to_path_buf();
    let stop_at = git_root.unwrap_or_else(|| cwd.to_path_buf());

    loop {
        for candidate in &candidates {
            let path = dir.join(candidate);
            if path.exists()
                && let Ok(content) = std::fs::read_to_string(&path)
            {
                let truncated = truncate_utf8(&content, 8000);
                let suffix = if content.len() > 8000 {
                    "\n...(truncated)"
                } else {
                    ""
                };
                parts.push(format!("## {}\n\n{}{}", path.display(), truncated, suffix));
            }
        }

        if dir == stop_at || !dir.pop() {
            break;
        }
    }

    parts.join("\n\n---\n\n")
}

/// Truncate a string to at most `max_bytes`, respecting UTF-8 char boundaries.
fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    &s[..s.floor_char_boundary(max_bytes)]
}

#[cfg(test)]
mod image_attachment_tests {
    use super::*;

    #[test]
    pub(super) fn looks_like_existing_path_true_for_a_real_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        assert!(looks_like_existing_path(path));
        assert!(looks_like_existing_path(&format!(
            "{path} are you able to see this image?"
        )));
    }

    #[test]
    pub(super) fn looks_like_existing_path_false_for_a_real_slash_command() {
        // Regression check: these used to be indistinguishable from an
        // absolute file path since both start with `/`.
        assert!(!looks_like_existing_path("/help"));
        assert!(!looks_like_existing_path("/model sonnet"));
        assert!(!looks_like_existing_path(""));
    }

    #[test]
    fn is_image_file_requires_a_known_extension_and_existence() {
        let tmp = tempfile::tempdir().unwrap();
        let png_path = tmp.path().join("shot.png");
        std::fs::write(
            &png_path,
            b"not real png bytes, extension is what matters here",
        )
        .unwrap();
        assert!(is_image_file(&png_path));

        let txt_path = tmp.path().join("notes.txt");
        std::fs::write(&txt_path, b"hello").unwrap();
        assert!(!is_image_file(&txt_path));

        let missing = tmp.path().join("missing.png");
        assert!(!is_image_file(&missing));
    }

    #[tokio::test]
    pub(super) async fn augment_with_image_attachments_is_a_noop_without_an_image_path() {
        // No image path present, so this must never shell out to the OCR
        // binary (external state — see hq-convert's own OCR tests).
        let text = "just a normal message, no paths here";
        let augmented = augment_with_image_attachments(text).await;
        assert_eq!(augmented, text);
    }
}
