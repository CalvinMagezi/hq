use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A YAML-defined group of skills that appears as a virtual skill in the catalog.
///
/// Loading the bundle returns the concatenated contents of its member skills,
/// optionally preceded by bundle-level instructions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillBundle {
    pub name: String,
    pub description: String,
    pub skills: Vec<String>,
    pub instruction: Option<String>,
}

/// Return the default bundle file path for a given bundle name.
///
/// Bundles may also use the `.yml` extension; callers that need to discover
/// existing bundles should use [`load_skill_bundles`] or check both extensions.
pub fn bundle_path(bundles_dir: &Path, name: &str) -> PathBuf {
    bundles_dir.join(format!("{name}.yaml"))
}

/// Read all `.yaml` and `.yml` files in `bundles_dir` and deserialize them.
///
/// Files that fail to read or parse are silently skipped so that a single
/// malformed bundle does not break the catalog.
pub fn load_skill_bundles(bundles_dir: &Path) -> Vec<SkillBundle> {
    let Ok(entries) = std::fs::read_dir(bundles_dir) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_lowercase();
        if ext != "yaml" && ext != "yml" {
            continue;
        }

        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(_) => continue,
        };

        match serde_yaml::from_str::<SkillBundle>(&raw) {
            Ok(bundle) => out.push(bundle),
            Err(_) => continue,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn load_skill_bundles_reads_yaml_and_yml() {
        let dir = tempdir().unwrap();
        let bundles = dir.path();

        fs::write(
            bundles.join("one.yaml"),
            "name: one\ndescription: First bundle\nskills: [a, b]\n",
        )
        .unwrap();
        fs::write(
            bundles.join("two.yml"),
            "name: two\ndescription: Second bundle\nskills: [c]\n",
        )
        .unwrap();
        // non-yaml file ignored
        fs::write(bundles.join("three.txt"), "ignored").unwrap();

        let loaded = load_skill_bundles(bundles);
        assert_eq!(loaded.len(), 2);
        assert!(loaded.iter().any(|b| b.name == "one"));
        assert!(loaded.iter().any(|b| b.name == "two"));
    }

    #[test]
    fn load_skill_bundles_returns_empty_for_missing_dir() {
        let missing = Path::new("/does/not/exist");
        assert!(load_skill_bundles(missing).is_empty());
    }
}
