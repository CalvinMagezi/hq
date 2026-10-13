//! Discord owner resolution (shared by the relay and the web settings).

use std::path::{Path, PathBuf};

/// Where a redeemed `!pair` code records the owner's user id.
pub fn owner_file(vault_path: &Path) -> PathBuf {
    vault_path.join("_system").join(".discord-auth-user")
}

/// The owner a pairing recorded, if any.
pub fn paired_owner(vault_path: &Path) -> Option<u64> {
    std::fs::read_to_string(owner_file(vault_path))
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_recorded_owner_and_ignores_junk() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(paired_owner(dir.path()), None);
        std::fs::create_dir_all(dir.path().join("_system")).unwrap();
        std::fs::write(owner_file(dir.path()), "42\n").unwrap();
        assert_eq!(paired_owner(dir.path()), Some(42));
        std::fs::write(owner_file(dir.path()), "not a number").unwrap();
        assert_eq!(paired_owner(dir.path()), None);
    }
}
