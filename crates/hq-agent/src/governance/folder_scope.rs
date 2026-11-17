//! Folder allow: which folder an approval request could be allowed for, whether a folder may be
//! allowed at all, and whether a call stays inside one. The owner's answer writes the grant
//! (`hq_db::owner_folder_grants`); this module only decides what a grant may name and what it
//! covers. Nothing here reads a model's words about a folder: the folder comes from the call's
//! own path arguments or, for `bash`, from the `cd` the command starts with, and the owner sees it
//! spelled out on the card before they allow it. Policy: docs/security/OWNER_APPROVAL.md.

use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use super::paths::{WRITE_TOOLS, path_values, resolve_for_check};

pub const BASH_TOOL: &str = "bash";

/// The reserved argument the runtime sets on a covered `bash` call: the folders its sandbox may
/// write to. It only ever narrows what the shell can change, so a caller who sends one gains
/// nothing; the model's own `_hq_*` keys are stripped before a call gets here regardless.
pub const CONFINE_ARG: &str = "_hq_confine_roots";

/// Top-level folders a grant never names: the system, and the scratch roots every shell can
/// already write to.
const SYSTEM_TOP_FOLDERS: &[&str] = &[
    "etc",
    "usr",
    "bin",
    "sbin",
    "lib",
    "lib32",
    "lib64",
    "boot",
    "dev",
    "proc",
    "sys",
    "var",
    "run",
    "tmp",
    "System",
    "Library",
    "Applications",
    "Volumes",
    "private",
    "cores",
];

/// Whether a call of `tool` can be covered by a folder allow at all.
pub fn is_coverable(tool: &str) -> bool {
    tool == BASH_TOOL || WRITE_TOOLS.contains(&tool)
}

/// Whether `folder` may be allowed. Returns it canonical, or the reason the owner is shown.
/// `vault` is the session's vault, which is HQ's own and never a project folder.
pub fn grantable(folder: &Path, vault: Option<&Path>) -> Result<PathBuf, String> {
    let canonical = folder
        .canonicalize()
        .map_err(|_| format!("{} is not a folder that exists", folder.display()))?;
    if !canonical.is_dir() {
        return Err(format!("{} is not a folder", canonical.display()));
    }
    let depth = canonical
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .count();
    if depth < 2 {
        return Err("a folder this high up covers too much of the machine".into());
    }
    if let Some(Component::Normal(top)) = canonical.components().nth(1)
        && SYSTEM_TOP_FOLDERS.iter().any(|t| top == *t)
    {
        return Err("system and scratch folders cannot be allowed".into());
    }
    if let Some(home) = dirs::home_dir().and_then(|h| h.canonicalize().ok()) {
        if home.starts_with(&canonical) {
            return Err("your home folder, or one that contains it, covers too much".into());
        }
        if let Ok(inside) = canonical.strip_prefix(&home)
            && let Some(Component::Normal(first)) = inside.components().next()
            && (first.to_string_lossy().starts_with('.') || first == "Library")
        {
            return Err("settings and credential folders cannot be allowed".into());
        }
    }
    if let Some(dir) = hq_core::config::HqConfig::config_file_path()
        .parent()
        .and_then(|p| p.canonicalize().ok())
        && (canonical.starts_with(&dir) || dir.starts_with(&canonical))
    {
        return Err("HQ's own configuration folder cannot be allowed".into());
    }
    if let Some(vault) = vault.and_then(|v| v.canonicalize().ok())
        && canonical.starts_with(&vault)
    {
        return Err("the vault is HQ's own, and its notes have their own tools".into());
    }
    Ok(canonical)
}

/// The folder a request could be allowed for, from what the call itself says: the git
/// repository (or else the folder) of the file an edit names, or exactly the directory a `bash`
/// command starts by changing into. `None` when the call names no folder, so the card offers none.
pub fn candidate_folder(tool: &str, args: &Value, vault: Option<&Path>) -> Option<PathBuf> {
    let folder = if tool == BASH_TOOL {
        let command = args.get("command").and_then(Value::as_str)?;
        let dir = leading_cd(command)?;
        // Exactly the folder the command works in, not the repository around it.
        resolve_for_check(Path::new(&dir))
    } else if WRITE_TOOLS.contains(&tool) {
        let paths = path_values(tool, args);
        let first = resolve_for_check(Path::new(paths.first()?.as_ref()));
        // Every path of a batch has to sit in the one folder the owner is shown.
        let folder = repo_root_of(&first).unwrap_or_else(|| nearest_dir(&first));
        if !paths
            .iter()
            .all(|p| resolve_for_check(Path::new(p.as_ref())).starts_with(&folder))
        {
            return None;
        }
        folder
    } else {
        return None;
    };
    grantable(&folder, vault).ok()
}

/// Whether the call `tool` with `args` stays inside one of `roots`, and which. Only the file
/// tools are decided here; a `bash` command is covered by confining its sandbox (see
/// [`CONFINE_ARG`]), not by reading it.
pub fn file_call_root(tool: &str, args: &Value, roots: &[PathBuf]) -> Option<PathBuf> {
    if !WRITE_TOOLS.contains(&tool) {
        return None;
    }
    let paths = path_values(tool, args);
    if paths.is_empty() {
        return None;
    }
    let resolved: Vec<PathBuf> = paths
        .iter()
        .map(|p| resolve_for_check(Path::new(p.as_ref())))
        .collect();
    roots
        .iter()
        .find(|root| resolved.iter().all(|p| p.starts_with(root)))
        .cloned()
}

/// The directory a command starts by changing into (`cd /some/dir && ...`), when it is spelled
/// plainly: an absolute or `~/` path with no variable, substitution or glob in it.
pub fn leading_cd(command: &str) -> Option<String> {
    let rest = command.trim_start().strip_prefix("cd")?;
    let rest = rest.strip_prefix(char::is_whitespace)?.trim_start();
    let (word, after) = match rest.chars().next()? {
        q @ ('"' | '\'') => {
            let body = &rest[1..];
            let end = body.find(q)?;
            (&body[..end], &body[end + 1..])
        }
        _ => {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            (&rest[..end], &rest[end..])
        }
    };
    let after = after.trim_start();
    if !(after.is_empty() || after.starts_with("&&") || after.starts_with(';')) {
        return None;
    }
    if word.is_empty() || word.contains(['$', '`', '(', ')', '*', '?', '\\', '\n']) {
        return None;
    }
    if let Some(tail) = word.strip_prefix("~/") {
        return dirs::home_dir().map(|h| h.join(tail).to_string_lossy().into_owned());
    }
    word.starts_with('/').then(|| word.to_string())
}

/// The nearest ancestor of `path` (or itself) that is a directory that exists.
fn nearest_dir(path: &Path) -> PathBuf {
    let mut dir = path.to_path_buf();
    loop {
        if dir.is_dir() {
            return dir;
        }
        // A file's own folder, or the nearest folder that exists for one not created yet.
        if !dir.pop() {
            return PathBuf::from("/");
        }
    }
}

/// The top of the git repository `path` is in, when it is in one below the home folder.
fn repo_root_of(path: &Path) -> Option<PathBuf> {
    let home = dirs::home_dir().and_then(|h| h.canonicalize().ok());
    let mut dir = nearest_dir(path);
    loop {
        if home.as_deref() == Some(dir.as_path()) {
            return None;
        }
        if dir.join(".git").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn work() -> tempfile::TempDir {
        // A project folder under the user's real home would be a dotfolder risk; the system
        // temp dir is a scratch root on some platforms, so make one under the cwd's target.
        tempfile::Builder::new()
            .prefix("hq-folder-scope")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap()
    }

    #[test]
    fn a_plain_cd_names_its_folder() {
        assert_eq!(
            leading_cd("cd /work/app && make").as_deref(),
            Some("/work/app")
        );
        assert_eq!(
            leading_cd("  cd '/work/my app'; ls").as_deref(),
            Some("/work/my app")
        );
        assert_eq!(leading_cd("cd /work/app").as_deref(), Some("/work/app"));
    }

    #[test]
    fn a_cd_that_hides_where_it_goes_names_nothing() {
        for command in [
            "cd $HOME && ls",
            "cd /work/$(whoami) && ls",
            "cd /work/* && ls",
            "cd relative/dir && ls",
            "cd /work/app | sh",
            "echo hi && cd /work/app",
            "cd",
            "cd /work/app || rm -rf x",
        ] {
            assert_eq!(leading_cd(command), None, "{command}");
        }
    }

    #[test]
    fn folders_that_cover_too_much_are_refused() {
        assert!(grantable(Path::new("/"), None).is_err());
        assert!(grantable(Path::new("/tmp"), None).is_err());
        assert!(grantable(Path::new("/usr/local"), None).is_err());
        assert!(grantable(Path::new("/etc"), None).is_err());
        if let Some(home) = dirs::home_dir() {
            assert!(grantable(&home, None).is_err());
            assert!(grantable(home.parent().unwrap(), None).is_err());
            assert!(grantable(&home.join(".ssh"), None).is_err());
        }
        assert!(grantable(Path::new("/definitely/not/here"), None).is_err());
    }

    #[test]
    fn a_project_folder_is_grantable_and_the_vault_is_not() {
        let dir = work();
        let project = dir.path().join("app");
        std::fs::create_dir_all(&project).unwrap();
        let ok = grantable(&project, None).unwrap();
        assert_eq!(ok, project.canonicalize().unwrap());
        assert!(grantable(&project, Some(dir.path())).is_err());
    }

    #[test]
    fn an_edit_is_covered_only_when_every_path_is_inside_a_root() {
        let dir = work();
        let root = dir.path().canonicalize().unwrap().join("app");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let inside = root.join("src/new.rs");
        let roots = vec![root.clone()];
        let write = json!({"file_path": inside, "content": "x"});
        assert_eq!(
            file_call_root("write_file", &write, &roots),
            Some(root.clone())
        );

        let escape = json!({"file_path": root.join("src/../../other/x"), "content": "x"});
        assert_eq!(file_call_root("write_file", &escape, &roots), None);

        let batch = json!({"edits": [
            {"file_path": root.join("a.rs")},
            {"file_path": dir.path().join("elsewhere.rs")},
        ]});
        assert_eq!(file_call_root("file_edit_batch", &batch, &roots), None);
        assert_eq!(
            file_call_root("bash", &json!({"command": "ls"}), &roots),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_folder_does_not_stay_inside_it() {
        let dir = work();
        let base = dir.path().canonicalize().unwrap();
        let root = base.join("app");
        let outside = base.join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        let args = json!({"file_path": root.join("link/stolen.txt"), "content": "x"});
        assert_eq!(file_call_root("write_file", &args, &[root]), None);
    }

    #[test]
    fn a_bash_command_names_its_folder_through_its_leading_cd() {
        let dir = work();
        let project = dir.path().canonicalize().unwrap().join("app");
        std::fs::create_dir_all(&project).unwrap();
        let args = json!({"command": format!("cd {} && cp a b", project.display())});
        assert_eq!(candidate_folder("bash", &args, None), Some(project));
        assert_eq!(
            candidate_folder("bash", &json!({"command": "cp a b"}), None),
            None
        );
    }

    #[test]
    fn an_edit_offers_the_git_repository_it_is_in() {
        let dir = work();
        let repo = dir.path().canonicalize().unwrap().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("src/deep")).unwrap();
        let args = json!({"file_path": repo.join("src/deep/a.rs"), "content": "x"});
        assert_eq!(candidate_folder("write_file", &args, None), Some(repo));
    }
}
