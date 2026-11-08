//! A server installed before the env files moved to `/etc/hq` still has an `hq.service` that
//! loads `/opt/hq/*.env`. The service user owns `/opt/hq`, so anything HQ runs could create one of
//! those files and have systemd load it, keys and paths included, at the next restart (an update
//! restarts it). The updater does not rewrite units, so the daemon checks its own unit at start and
//! asks the owner to re-run `deploy/install.sh`, which moves the files and fixes the unit.

use std::path::{Path, PathBuf};

const UNIT: &str = "/etc/systemd/system/hq.service";
const DROPINS: &str = "/etc/systemd/system/hq.service.d";
const SOURCE: &str = "server-setup";
const DEDUP_KEY: &str = "unit-env-files-writable";

/// The env files a unit and its drop-ins (in the order systemd applies them) end up loading.
/// An empty `EnvironmentFile=` resets the list, as it does in systemd.
pub(super) fn env_files<'a>(texts: impl IntoIterator<Item = &'a str>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for text in texts {
        for line in text.lines() {
            let Some(value) = line.trim().strip_prefix("EnvironmentFile=") else {
                continue;
            };
            let value = value.trim();
            if value.is_empty() {
                files.clear();
                continue;
            }
            files.push(PathBuf::from(value.strip_prefix('-').unwrap_or(value)));
        }
    }
    files
}

/// Whether this user could put a file at `path` or change the one there: it owns the file, or
/// the folder that holds it.
pub(super) fn writable_here(path: &Path) -> bool {
    let owned = |p: &Path| hq_core::fs_private::owned_by_current_user(p).unwrap_or(false);
    owned(path) || path.parent().is_some_and(owned)
}

/// The unit's env files this user can write. Empty when there is no unit to read.
fn writable_env_files() -> Vec<PathBuf> {
    let Ok(unit) = std::fs::read_to_string(UNIT) else {
        return Vec::new();
    };
    let mut dropins: Vec<PathBuf> = std::fs::read_dir(DROPINS)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "conf"))
                .collect()
        })
        .unwrap_or_default();
    dropins.sort();
    let texts: Vec<String> = std::iter::once(unit)
        .chain(
            dropins
                .iter()
                .filter_map(|p| std::fs::read_to_string(p).ok()),
        )
        .collect();
    env_files(texts.iter().map(String::as_str))
        .into_iter()
        .filter(|p| writable_here(p))
        .collect()
}

/// Puts one item in the owner's inbox (once, until they dismiss it) when the unit loads an env
/// file this service could write.
pub(super) fn check(db: &hq_db::Database) {
    // Root can write anywhere, so the question means nothing there (`/` belongs to root).
    if hq_core::fs_private::owned_by_current_user(Path::new("/")).unwrap_or(false) {
        return;
    }
    let found = writable_env_files();
    if found.is_empty() {
        return;
    }
    let list = found
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    tracing::warn!(files = %list, "hq.service loads env files the service user can write");
    let item = hq_core::types::ValueItem::new(
        SOURCE,
        hq_core::types::ValueKind::ActionNeeded,
        "Move HQ's env files out of its own folder",
        format!(
            "hq.service loads {list} from a folder HQ itself can write, so anything HQ runs could \
             add keys or settings there for the next restart to load. On the server, as root, \
             re-run deploy/install.sh from a checkout of the repository: it moves the files to \
             /etc/hq and fixes the unit. See deploy/README.md."
        ),
    )
    .with_dedup_key(DEDUP_KEY);
    if let Err(e) = hq_db::value_items::emit(db, &item) {
        tracing::warn!("could not record the env file notice: {e:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_list_systemd_would_load() {
        let unit = "[Service]\nEnvironmentFile=-/opt/hq/gh.env\n EnvironmentFile=/opt/hq/gws.env\n";
        assert_eq!(
            env_files([unit]),
            vec![PathBuf::from("/opt/hq/gh.env"), "/opt/hq/gws.env".into()]
        );
        let reset = "[Service]\nEnvironmentFile=\nEnvironmentFile=-/etc/hq/gh.env\n";
        let mcp = "[Service]\nEnvironmentFile=/etc/hq/mcp.env\n";
        assert_eq!(
            env_files([unit, reset, mcp]),
            vec![PathBuf::from("/etc/hq/gh.env"), "/etc/hq/mcp.env".into()]
        );
    }

    #[test]
    fn a_file_in_a_folder_this_user_owns_counts_even_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(writable_here(&dir.path().join("gh.env")));
        assert!(!writable_here(Path::new("/nonexistent-hq-dir/gh.env")));
    }
}
