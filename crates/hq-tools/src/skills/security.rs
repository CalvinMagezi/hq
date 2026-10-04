//! Security flags for skill content and which categories block adoption.

use regex::Regex;
use std::sync::LazyLock;

/// A base64-looking run longer than a SHA-512 hex digest is treated as an opaque payload.
const MAX_OPAQUE_TOKEN_CHARS: usize = 160;
/// A one-line shell command longer than this cannot be reviewed at a glance.
pub(super) const MAX_SHELL_LINE_CHARS: usize = 300;
/// How much of a match to quote back in a flag.
const FLAG_EXCERPT_CHARS: usize = 32;
const SHELL_OPERATORS: &[&str] = &["|", "&&", ";", "$("];
const CODE_FENCE: &str = "```";
const SHELL_PROMPT: &str = "$ ";
const LOCAL_HOSTS: &[&str] = &["localhost", "127.0.0.1", "[::1]", "0.0.0.0"];
pub(super) const NETWORK_EGRESS: &str = "network egress";
pub(super) const OPAQUE_PAYLOAD: &str = "opaque payload";
const INSTRUCTION_OVERRIDE: &str = "instruction override";
/// Flag categories no machine-written text may carry into a live skill.
const BLOCKING_CATEGORIES: &[&str] = &[INSTRUCTION_OVERRIDE, OPAQUE_PAYLOAD];

static SECURITY_PATTERNS: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    [
        (NETWORK_EGRESS, r"(?i)\b(curl|wget)\b"),
        (
            "credential access",
            r"(?i)\b(keyring|keychain|api[ _-]?keys?|secrets?|passwords?|passwd|tokens?|credentials?)\b|\[REDACTED\]",
        ),
        (
            "destructive operation",
            r"(?i)\brm\s+-[a-z]*(rf|fr)|--force\b|\bgit\s+push\b|\bdelete\b|\brotate\b|\bdrop\s+table\b",
        ),
        (
            INSTRUCTION_OVERRIDE,
            r"(?i)\bignore\s+((all|any|the|previous|prior|earlier|above)\s+)+instructions\b|\bdisregard\b|\byou\s+are\s+now\b",
        ),
    ]
    .into_iter()
    .map(|(category, p)| (category, Regex::new(p).expect("static security pattern must compile")))
    .collect()
});

static URL_HOST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\bhttps?://([^/\s:?#)"'>]+)"#).expect("static url pattern must compile")
});

static OPAQUE_BLOB: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        "[A-Za-z0-9+/=_-]{{{},}}",
        MAX_OPAQUE_TOKEN_CHARS + 1
    ))
    .expect("static blob pattern must compile")
});

fn push_flag(flags: &mut Vec<String>, category: &str, hit: &str) {
    if flags.iter().any(|f| f.starts_with(category)) {
        return;
    }
    let excerpt: String = hit.chars().take(FLAG_EXCERPT_CHARS).collect();
    flags.push(format!("{category}: {excerpt}"));
}

/// Lines a reader would run rather than read: inside a fenced code block, or
/// typed after a `$ ` prompt. A long prose bullet with a semicolon is not one.
fn shell_lines(content: &str) -> impl Iterator<Item = &str> {
    let mut in_fence = false;
    content.lines().filter(move |line| {
        let trimmed = line.trim_start();
        if trimmed.starts_with(CODE_FENCE) {
            in_fence = !in_fence;
            return false;
        }
        in_fence || trimmed.starts_with(SHELL_PROMPT)
    })
}

/// What in a machine-written skill is worth recording. Categories in
/// [`BLOCKING_CATEGORIES`] keep the change from going live; the rest are
/// audited only.
pub fn security_flags(content: &str) -> Vec<String> {
    let mut flags = Vec::new();
    for (category, pattern) in SECURITY_PATTERNS.iter() {
        if let Some(m) = pattern.find(content) {
            push_flag(&mut flags, category, m.as_str());
        }
    }
    let external_host = URL_HOST
        .captures_iter(content)
        .filter_map(|c| c.get(1))
        .map(|m| m.as_str().to_lowercase())
        .find(|host| !LOCAL_HOSTS.contains(&host.as_str()));
    if let Some(host) = external_host {
        push_flag(&mut flags, NETWORK_EGRESS, &host);
    }
    if let Some(m) = OPAQUE_BLOB.find(content) {
        // The length, not the blob, so a payload is never copied into notices or the audit log.
        push_flag(
            &mut flags,
            OPAQUE_PAYLOAD,
            &format!("{}-char encoded run", m.len()),
        );
    }
    let long_shell = shell_lines(content).find(|l| {
        l.len() > MAX_SHELL_LINE_CHARS && SHELL_OPERATORS.iter().any(|op| l.contains(op))
    });
    if let Some(line) = long_shell {
        push_flag(&mut flags, OPAQUE_PAYLOAD, line);
    }
    flags
}

/// Whether any flag from [`security_flags`] is in a category that must never go live.
pub fn blocks_adoption(flags: &[String]) -> bool {
    flags
        .iter()
        .any(|f| BLOCKING_CATEGORIES.iter().any(|c| f.starts_with(c)))
}
