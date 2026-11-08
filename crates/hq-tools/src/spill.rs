//! Spill: the full text of a tool output that was cut to fit the context, or of messages a
//! long turn compacted away, kept on disk so the agent can read it back (`tool_output_read`).
//!
//! Files live at `<vault>/_data/spill/<session>/<name>.txt`, are redacted of known secret shapes
//! before they are written, and are pruned after [`SPILL_RETENTION_DAYS`] by the daemon's cleanup
//! task. A ref is `spill:<session>/<name>`; both parts are checked so a ref can never name a
//! path outside the spill directory.

use anyhow::{Result, bail};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::registry::HqTool;

/// Spilled text older than this is deleted by [`prune`].
pub const SPILL_RETENTION_DAYS: u64 = 7;

/// Characters one `tool_output_read` call returns at most.
const MAX_READ_CHARS: usize = 20_000;

/// One spilled file is cut to this many bytes, so a runaway output cannot fill the disk.
const MAX_SPILL_BYTES: usize = 2_000_000;

fn spill_root(vault: &Path) -> PathBuf {
    vault.join("_data").join("spill")
}

fn safe_part(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn clean_part(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    if out.is_empty() { "x".to_string() } else { out }
}

/// Writes `text` (redacted) for `session` under a name built from `label`, and returns its ref.
pub fn write(vault: &Path, session: &str, label: &str, text: &str) -> Result<String> {
    let session = clean_part(session);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = format!("{}-{nanos}", clean_part(label));
    let dir = spill_root(vault).join(&session);
    std::fs::create_dir_all(&dir)?;
    let mut body = hq_core::redact::redact_secrets(text);
    if body.len() > MAX_SPILL_BYTES {
        let cut = hq_core::tokens::snap_to_char_boundary(&body, MAX_SPILL_BYTES);
        body.truncate(cut);
        body.push_str("\n[spill cut here: the output was larger than 2 MB]");
    }
    std::fs::write(dir.join(format!("{name}.txt")), body)?;
    Ok(format!("spill:{session}/{name}"))
}

/// Reads `limit` characters of a spill from character `offset`. Returns the slice and the total
/// length in characters.
pub fn read(vault: &Path, reference: &str, offset: usize, limit: usize) -> Result<(String, usize)> {
    let Some((session, name)) = reference
        .trim()
        .strip_prefix("spill:")
        .and_then(|r| r.split_once('/'))
    else {
        bail!("not a spill ref: expected spill:<session>/<name>");
    };
    if !safe_part(session) || !safe_part(name) {
        bail!("not a spill ref: {reference}");
    }
    let path = spill_root(vault).join(session).join(format!("{name}.txt"));
    let text = std::fs::read_to_string(&path).map_err(|_| {
        anyhow::anyhow!("{reference} is gone (spills are kept for {SPILL_RETENTION_DAYS} days)")
    })?;
    let total = text.chars().count();
    let slice: String = text.chars().skip(offset).take(limit).collect();
    Ok((slice, total))
}

/// Deletes spill files older than `max_age` and the session folders left empty. Returns how many
/// files went.
pub fn prune(vault: &Path, max_age: Duration) -> usize {
    let root = spill_root(vault);
    let Ok(sessions) = std::fs::read_dir(&root) else {
        return 0;
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for session in sessions.flatten() {
        let dir = session.path();
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        for file in files.flatten() {
            let old = file
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| now.duration_since(t).ok())
                .is_some_and(|age| age > max_age);
            if old && std::fs::remove_file(file.path()).is_ok() {
                removed += 1;
            }
        }
        let _ = std::fs::remove_dir(&dir);
    }
    removed
}

/// `tool_output_read`: reads back a spilled tool output or compacted transcript.
pub struct ToolOutputReadTool {
    vault: PathBuf,
}

impl ToolOutputReadTool {
    pub fn new(vault: PathBuf) -> Self {
        Self { vault }
    }
}

#[async_trait]
impl HqTool for ToolOutputReadTool {
    fn name(&self) -> &str {
        "tool_output_read"
    }

    fn description(&self) -> &str {
        "Read the full text of a tool output that was cut to fit, or of earlier messages this \
         turn compacted away. Pass the spill:... ref from the note that says the full text was \
         kept, and an offset to read further."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "ref": {"type": "string", "description": "The spill:<session>/<name> ref."},
                "offset": {"type": "integer", "default": 0, "description": "Character to start from."},
                "limit": {"type": "integer", "default": 8000, "maximum": 20000, "description": "Characters to return."}
            },
            "required": ["ref"]
        })
    }

    fn category(&self) -> &str {
        "memory"
    }

    fn search_hint(&self) -> Option<&str> {
        Some("read the full text of a truncated tool output")
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let Some(reference) = args.get("ref").and_then(|v| v.as_str()) else {
            return Ok(json!({"error": "missing required field: ref"}));
        };
        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(8_000)
            .clamp(1, MAX_READ_CHARS as u64) as usize;
        match read(&self.vault, reference, offset, limit) {
            Ok((text, total)) => {
                let end = (offset + text.chars().count()).min(total);
                let mut out = json!({"ref": reference, "offset": offset, "end": end, "total": total, "text": text});
                if end < total {
                    out["next_offset"] = json!(end);
                }
                Ok(out)
            }
            Err(e) => Ok(json!({"error": e.to_string()})),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spill_reads_back_in_pieces_and_refuses_paths_outside() {
        let dir = tempfile::tempdir().unwrap();
        let body = "line\n".repeat(10);
        let r = write(dir.path(), "sess-1", "bash", &body).unwrap();
        assert!(r.starts_with("spill:sess-1/bash-"));
        let (first, total) = read(dir.path(), &r, 0, 10).unwrap();
        assert_eq!(total, 50);
        assert_eq!(first, "line\nline\n");
        let (rest, _) = read(dir.path(), &r, 45, 100).unwrap();
        assert_eq!(rest, "line\n");
        for bad in [
            "spill:../x/y",
            "spill:sess-1/../../etc",
            "spill:sess-1",
            "nope",
            "spill:a/b.txt",
        ] {
            assert!(read(dir.path(), bad, 0, 10).is_err(), "{bad}");
        }
    }

    #[test]
    fn secrets_are_redacted_before_they_reach_the_disk() {
        let dir = tempfile::tempdir().unwrap();
        let key = format!("sk-{}", "abcdefghijklmnopqrstuvwx1234");
        let r = write(dir.path(), "s", "out", &format!("token {key}")).unwrap();
        let (text, _) = read(dir.path(), &r, 0, 1_000).unwrap();
        assert!(!text.contains(&key), "{text}");
    }

    #[test]
    fn old_spills_are_pruned_and_new_ones_kept() {
        let dir = tempfile::tempdir().unwrap();
        let r = write(dir.path(), "s", "out", "kept").unwrap();
        assert_eq!(prune(dir.path(), Duration::from_secs(3_600)), 0);
        assert!(read(dir.path(), &r, 0, 10).is_ok());
        assert_eq!(prune(dir.path(), Duration::ZERO), 1);
        assert!(read(dir.path(), &r, 0, 10).is_err());
    }

    #[tokio::test]
    async fn the_tool_pages_through_a_spill() {
        let dir = tempfile::tempdir().unwrap();
        let r = write(dir.path(), "s", "out", &"a".repeat(30)).unwrap();
        let tool = ToolOutputReadTool::new(dir.path().to_path_buf());
        let out = tool.execute(json!({"ref": r, "limit": 20})).await.unwrap();
        assert_eq!(out["next_offset"], 20);
        let out = tool
            .execute(json!({"ref": "spill:../../x", "limit": 20}))
            .await
            .unwrap();
        assert!(out.get("error").is_some());
    }
}
