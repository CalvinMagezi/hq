//! The app was called HQ and is now AgentHQ, so the `hq` program moved from `HQ.app` to
//! `AgentHQ.app`. Claude Desktop entries this app wrote still name the old path. This rewrites
//! exactly those and nothing else, keeps a dated backup, and says which entries it changed.

use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const OLD_BUNDLE: &str = "HQ.app";
const NEW_BUNDLE: &str = "AgentHQ.app";

/// Where `program` was before the rename, when it sits inside `AgentHQ.app`.
pub fn old_program(program: &Path) -> Option<PathBuf> {
    let mut found = false;
    let old: PathBuf = program
        .components()
        .map(|c| {
            if c.as_os_str() == NEW_BUNDLE {
                found = true;
                OLD_BUNDLE.into()
            } else {
                c.as_os_str().to_owned()
            }
        })
        .collect();
    found.then_some(old)
}

/// Whether an entry is one this app wrote for a hub: named `hq` or `hq-<hub>`, started with
/// exactly `mcp-serve --remote <hub>` by the old program.
fn is_ours(name: &str, entry: &Value, old: &Path, hubs: &[String]) -> bool {
    let args: Vec<&str> = entry["args"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let [verb, flag, hub] = args[..] else {
        return false;
    };
    verb == "mcp-serve"
        && flag == "--remote"
        && hubs.iter().any(|h| h == hub)
        && (name == "hq" || name == format!("hq-{hub}"))
        && entry["command"].as_str() == old.to_str()
}

/// Rewrites Claude Desktop's config at `config` so its entries for these hubs start `program`
/// instead of the old path. Returns the entry names it changed, or none when nothing matched.
pub fn rewrite_claude_desktop(
    config: &Path,
    program: &Path,
    hubs: &[String],
    today: &str,
) -> Result<Vec<String>> {
    let Some(old) = old_program(program) else {
        return Ok(Vec::new());
    };
    let Ok(text) = std::fs::read_to_string(config) else {
        return Ok(Vec::new());
    };
    let mut doc: Value =
        serde_json::from_str(&text).context("Claude Desktop's config is not JSON")?;
    let mut changed = Vec::new();
    if let Some(servers) = doc.get_mut("mcpServers").and_then(Value::as_object_mut) {
        for (name, entry) in servers.iter_mut() {
            if is_ours(name, entry, &old, hubs) {
                entry["command"] = json!(program.to_string_lossy());
                changed.push(name.clone());
            }
        }
    }
    if changed.is_empty() {
        return Ok(changed);
    }
    let mut backup = config.as_os_str().to_owned();
    backup.push(format!(".bak-{today}"));
    std::fs::write(&backup, &text).context("could not keep a backup of Claude Desktop's config")?;
    let mut temp = config.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, serde_json::to_string_pretty(&doc)?)?;
    std::fs::rename(&temp, config)?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW: &str = "/Applications/AgentHQ.app/Contents/MacOS/hq";
    const OLD: &str = "/Applications/HQ.app/Contents/MacOS/hq";

    fn hubs() -> Vec<String> {
        vec!["desk".into()]
    }

    fn config(dir: &Path) -> PathBuf {
        let path = dir.join("claude_desktop_config.json");
        let doc = json!({"mcpServers": {
            "hq-desk": {"command": OLD, "args": ["mcp-serve", "--remote", "desk"]},
            "hq": {"command": OLD, "args": ["mcp-serve", "--remote", "other"]},
            "mine": {"command": OLD, "args": ["mcp-serve", "--remote", "desk"]},
            "hq-x": {"command": "/usr/local/bin/hq", "args": ["mcp-serve", "--remote", "desk"]}
        }, "theme": "dark"});
        std::fs::write(&path, doc.to_string()).unwrap();
        path
    }

    #[test]
    fn only_the_hubs_entries_that_point_into_the_old_bundle_are_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = config(dir.path());
        let changed = rewrite_claude_desktop(&path, Path::new(NEW), &hubs(), "2026-11-05").unwrap();
        assert_eq!(changed, ["hq-desk"]);
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(doc["mcpServers"]["hq-desk"]["command"], NEW);
        assert_eq!(doc["mcpServers"]["hq"]["command"], OLD);
        assert_eq!(doc["mcpServers"]["mine"]["command"], OLD);
        assert_eq!(doc["mcpServers"]["hq-x"]["command"], "/usr/local/bin/hq");
        assert_eq!(doc["theme"], "dark");
        assert!(
            dir.path()
                .join("claude_desktop_config.json.bak-2026-11-05")
                .exists()
        );
    }

    #[test]
    fn a_program_outside_the_new_bundle_or_a_missing_file_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = config(dir.path());
        let before = std::fs::read_to_string(&path).unwrap();
        assert!(
            rewrite_claude_desktop(&path, Path::new("/usr/local/bin/hq"), &hubs(), "d")
                .unwrap()
                .is_empty()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        let none = dir.path().join("absent.json");
        assert!(
            rewrite_claude_desktop(&none, Path::new(NEW), &hubs(), "d")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn the_old_path_swaps_only_the_bundle_name() {
        assert_eq!(old_program(Path::new(NEW)).unwrap(), Path::new(OLD));
        assert_eq!(old_program(Path::new("/opt/hq")), None);
    }
}
