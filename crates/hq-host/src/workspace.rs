//! The folder agents started from the web run in by default, and the browsing and
//! creating of folders inside it. Everything is resolved on the machine that runs
//! the agents, so the HQ server never judges a path by its own operating system.

use nix::unistd::{User, geteuid};
use serde_json::{Value, json};
use std::fs;
use std::io::{Error, ErrorKind};
use std::path::{Component, Path, PathBuf};

/// Under the user's home, on every platform HQ hosts run on. A headless Ubuntu
/// server has no `Documents` until something creates it.
const WORKSPACE_PARTS: [&str; 2] = ["Documents", "HQ"];
const MAX_LISTED_DIRS: usize = 500;
const MAX_NAME_CHARS: usize = 100;
/// Most file systems cap a name at 255 bytes, and a name of 100 wide characters can pass that.
const MAX_NAME_BYTES: usize = 200;
/// Entries read from one folder before listing stops, so a huge folder cannot tie the host up.
const MAX_SCANNED_ENTRIES: usize = MAX_LISTED_DIRS * 10;
/// What a spawn's folder check refuses in a path, so a home containing one could never be used.
const UNUSABLE_PATH_CHARS: [char; 4] = ['~', '$', '`', '\0'];

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidInput, message.into())
}

fn home_dir() -> Option<PathBuf> {
    let from_env = std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute());
    from_env.or_else(|| User::from_uid(geteuid()).ok().flatten().map(|u| u.dir))
}

fn root_in(home: &Path) -> PathBuf {
    WORKSPACE_PARTS.iter().fold(home.to_path_buf(), |p, part| p.join(part))
}

/// `<home>/Documents/HQ`.
pub fn workspace_root() -> std::io::Result<PathBuf> {
    let home = home_dir().ok_or_else(|| Error::new(ErrorKind::NotFound, "no home directory"))?;
    Ok(root_in(&home))
}

fn ensure_at(root: &Path) -> std::io::Result<PathBuf> {
    fs::create_dir_all(root)?;
    root.canonicalize()
}

/// Creates the workspace folder if it is missing and returns it, resolved.
pub fn ensure_workspace() -> std::io::Result<PathBuf> {
    let root = workspace_root()?;
    if root.to_string_lossy().contains(UNUSABLE_PATH_CHARS) {
        return Err(invalid(format!(
            "{} has a '~', '$' or backtick in it, which agents cannot be started in",
            root.display()
        )));
    }
    ensure_at(&root)
}

/// Running inside WSL2, where the user reaches Linux files from Windows through `\\wsl$`.
fn wsl_distro() -> Option<String> {
    std::env::var("WSL_DISTRO_NAME").ok().filter(|d| !d.is_empty())
}

/// What the web needs to offer the folder: where it is, which OS, and how a
/// Windows user would open it in Explorer.
pub fn describe(root: &Path) -> Value {
    describe_for(root, wsl_distro())
}

fn describe_for(root: &Path, distro: Option<String>) -> Value {
    let explorer = distro.map(|distro| {
        let tail = root.to_string_lossy().replace('/', "\\");
        format!("\\\\wsl$\\{distro}{tail}")
    });
    json!({
        "root": root.to_string_lossy(),
        "os": std::env::consts::OS,
        "wsl": explorer.is_some(),
        "explorer_path": explorer,
    })
}

/// A folder under the workspace. A relative `path` is taken from the root; an
/// absolute one must already be inside it. Symlinks are resolved first, so a
/// link out of the workspace is refused instead of followed.
pub fn resolve_inside(root: &Path, path: Option<&str>) -> std::io::Result<PathBuf> {
    let root = root.canonicalize()?;
    let wanted = match path.map(str::trim).filter(|p| !p.is_empty()) {
        None => root.clone(),
        Some(p) if Path::new(p).is_absolute() => PathBuf::from(p),
        Some(p) => root.join(p),
    };
    if wanted.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(invalid("a folder path cannot contain '..'"));
    }
    // Before the disk is touched, so a path elsewhere cannot be probed for existence.
    if !wanted.starts_with(&root) {
        return Err(invalid("that folder is outside the HQ folder"));
    }
    let real = wanted.canonicalize()?;
    if !real.starts_with(&root) {
        return Err(invalid("that folder is outside the HQ folder"));
    }
    if real.to_str().is_none() {
        return Err(invalid("that folder's name is not plain text"));
    }
    if !real.is_dir() {
        return Err(invalid("that path is not a folder"));
    }
    Ok(real)
}

/// The sub-folders of `path` (hidden ones left out), alphabetical, capped.
pub fn list_dirs(root: &Path, path: Option<&str>) -> std::io::Result<Value> {
    let root = root.canonicalize()?;
    let dir = resolve_inside(&root, path)?;
    let mut names: Vec<String> = fs::read_dir(&dir)?
        .take(MAX_SCANNED_ENTRIES + 1)
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    let truncated = names.len() > MAX_LISTED_DIRS || names.len() > MAX_SCANNED_ENTRIES;
    names.truncate(MAX_LISTED_DIRS);
    let parent = (dir != root).then(|| dir.parent().map(Path::to_path_buf)).flatten();
    let dirs: Vec<Value> = names
        .iter()
        .map(|n| json!({ "name": n, "path": dir.join(n) }))
        .collect();
    Ok(json!({ "path": dir, "parent": parent, "dirs": dirs, "truncated": truncated }))
}

/// A single folder name: no separators, not `.` or `..`, no control characters.
pub fn check_folder_name(name: &str) -> std::io::Result<&str> {
    let name = name.trim();
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.chars().count() > MAX_NAME_CHARS
        || name.len() > MAX_NAME_BYTES
        || name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '~' | '$' | '`'));
    if bad {
        return Err(invalid(
            "a folder name is 1 to 100 characters with no slashes, colons, '~', '$' or backticks",
        ));
    }
    Ok(name)
}

/// Creates `name` inside `parent` (inside the workspace). An existing folder is
/// returned as is, so a retry changes nothing.
pub fn make_dir(root: &Path, parent: Option<&str>, name: &str) -> std::io::Result<PathBuf> {
    let name = check_folder_name(name)?;
    let parent = resolve_inside(root, parent)?;
    let target = parent.join(name);
    match fs::create_dir(&target) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::AlreadyExists && target.is_dir() => {}
        Err(e) => return Err(e),
    }
    let target = target.to_str().ok_or_else(|| invalid("that folder's name is not plain text"))?;
    resolve_inside(root, Some(target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn root() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("HQ");
        fs::create_dir_all(root.join("alpha")).unwrap();
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join("file.txt"), "x").unwrap();
        (dir, root)
    }

    #[test]
    fn listing_shows_visible_folders_only() {
        let (_d, root) = root();
        let v = list_dirs(&root, None).unwrap();
        let names: Vec<&str> = v["dirs"].as_array().unwrap().iter().map(|d| d["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["alpha"]);
        assert!(v["parent"].is_null());
    }

    #[test]
    fn a_parent_reference_or_outside_path_is_refused() {
        let (d, root) = root();
        assert!(resolve_inside(&root, Some("../x")).is_err());
        assert!(resolve_inside(&root, Some(d.path().to_str().unwrap())).is_err());
        assert!(resolve_inside(&root, Some("/etc")).is_err());
    }

    #[test]
    fn a_symlink_out_of_the_workspace_is_refused() {
        let (d, root) = root();
        symlink(d.path(), root.join("escape")).unwrap();
        assert!(resolve_inside(&root, Some("escape")).is_err());
    }

    #[test]
    fn a_file_is_not_a_folder() {
        let (_d, root) = root();
        assert!(resolve_inside(&root, Some("file.txt")).is_err());
    }

    #[test]
    fn make_dir_is_idempotent_and_validates_the_name() {
        let (_d, root) = root();
        let first = make_dir(&root, None, "My Project").unwrap();
        assert_eq!(make_dir(&root, None, "My Project").unwrap(), first);
        for bad in ["", "..", "a/b", "a\\b", "C:x", "~x", "a\nb"] {
            assert!(make_dir(&root, None, bad).is_err(), "{bad:?}");
        }
        let nested = make_dir(&root, first.to_str(), "inner").unwrap();
        assert!(nested.starts_with(&first));
    }

    #[test]
    fn a_path_elsewhere_is_refused_without_touching_the_disk() {
        let (_d, root) = root();
        let missing = resolve_inside(&root, Some("/definitely/not/here")).unwrap_err();
        let present = resolve_inside(&root, Some("/etc")).unwrap_err();
        assert_eq!(missing.to_string(), present.to_string());
    }

    #[test]
    fn on_wsl_the_explorer_path_points_into_the_distro_and_elsewhere_there_is_none() {
        let root = Path::new("/home/user/Documents/HQ");
        let wsl = describe_for(root, Some("Ubuntu".into()));
        assert_eq!(wsl["wsl"], true);
        assert_eq!(wsl["explorer_path"], "\\\\wsl$\\Ubuntu\\home\\user\\Documents\\HQ");
        let plain = describe_for(root, None);
        assert_eq!(plain["wsl"], false);
        assert!(plain["explorer_path"].is_null());
    }

    #[test]
    fn a_name_too_long_in_bytes_is_refused() {
        assert!(check_folder_name(&"\u{4e2d}".repeat(80)).is_err());
    }

    #[test]
    fn ensure_creates_the_folder_under_home() {
        let home = tempfile::tempdir().unwrap();
        let made = ensure_at(&root_in(home.path())).unwrap();
        assert!(made.ends_with("Documents/HQ"));
        assert_eq!(ensure_at(&root_in(home.path())).unwrap(), made);
    }
}
