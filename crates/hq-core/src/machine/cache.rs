use super::MachineProfile;
use super::probe::probe_machine;
use super::render::render_markdown;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn markdown_path(vault_path: &Path) -> PathBuf {
    vault_path.join("_system").join("MACHINE.md")
}

fn json_path(vault_path: &Path) -> PathBuf {
    vault_path.join("_system").join("machine.json")
}

/// Probe and write both cache files. Blocking.
pub fn refresh(vault_path: &Path) -> anyhow::Result<MachineProfile> {
    let profile = probe_machine(Some(vault_path));
    let system_dir = vault_path.join("_system");
    std::fs::create_dir_all(&system_dir)?;
    std::fs::write(markdown_path(vault_path), render_markdown(&profile))?;
    std::fs::write(
        json_path(vault_path),
        serde_json::to_string_pretty(&profile)?,
    )?;
    Ok(profile)
}

/// Read the cached structured profile, if the daemon has written one.
pub fn load_cached_profile(vault_path: &Path) -> Option<MachineProfile> {
    let raw = std::fs::read_to_string(json_path(vault_path)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Read the cached markdown block for prompt injection.
///
/// A stale profile is still returned, flagged inline — a prompt build must
/// never block on a subprocess fan-out. `None` only when nothing is cached,
/// which callers answer with [`super::probe_machine_fast`].
pub fn load_cached(vault_path: &Path, max_age: Duration) -> Option<String> {
    let path = markdown_path(vault_path);
    let content = std::fs::read_to_string(&path).ok()?;
    if content.trim().is_empty() {
        return None;
    }
    let age = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok());
    match age {
        Some(a) if a > max_age => {
            tracing::warn!(?path, age_secs = a.as_secs(), "machine profile is stale");
            Some(format!(
                "{content}\n_(profile is stale — call `system_info` to confirm before relying on it)_\n"
            ))
        }
        _ => Some(content),
    }
}
