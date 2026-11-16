//! Mechanical checks shared by everything that publishes on an agent's behalf: the `hq/` branch
//! tool for a tarball checkout (`github_write`) and the host's push for a helper's own worktree.
//! One copy, so the two paths cannot drift on what counts as a branch name, a credential or a
//! path that is never pushed.

use regex::Regex;
use std::sync::LazyLock;

pub const MAX_BRANCH_CHARS: usize = 100;
/// GitHub's own limits, checked before anything is pushed so a refusal never comes halfway.
pub const MAX_TITLE_CHARS: usize = 256;
pub const MAX_BODY_CHARS: usize = 65_536;

/// Branches HQ never pushes from a helper, whatever the remote calls its default.
const PROTECTED_BRANCHES: [&str; 3] = ["main", "master", "develop"];
const PROTECTED_PREFIXES: [&str; 2] = ["release/", "releases/"];

/// Directories whose contents run with the repository's secrets or change how CI runs.
const CI_DIRS: [&str; 4] = [".github", ".circleci", ".buildkite", ".gitlab"];
/// Single files that configure CI or a git hook runner.
const CI_FILES: [&str; 7] = [
    ".gitlab-ci.yml",
    "azure-pipelines.yml",
    "Jenkinsfile",
    "bitbucket-pipelines.yml",
    ".travis.yml",
    "lefthook.yml",
    ".pre-commit-config.yaml",
];
const HOOK_DIRS: [&str; 2] = [".husky", ".githooks"];

static BRANCH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]*(/[A-Za-z0-9][A-Za-z0-9._-]*)*$").unwrap()
});

/// Plain names joined by `/`, each one a name git accepts for a branch component.
pub fn check_branch(name: &str) -> Result<(), &'static str> {
    let parts_ok = name
        .split('/')
        .all(|part| !part.ends_with(".lock") && !part.ends_with('.'));
    let ok =
        BRANCH.is_match(name) && name.len() <= MAX_BRANCH_CHARS && !name.contains("..") && parts_ok;
    if ok {
        Ok(())
    } else {
        Err("`branch` must be plain names joined by /, such as fix-plant-log")
    }
}

/// A branch a helper may never publish: the usual defaults, release lines, and `default`
/// (what the remote's HEAD names, when the caller knows it).
pub fn is_protected_branch(name: &str, default: Option<&str>) -> bool {
    PROTECTED_BRANCHES.contains(&name)
        || PROTECTED_PREFIXES.iter().any(|p| name.starts_with(p))
        || default.is_some_and(|d| d == name)
}

/// Why a changed path is never pushed from a helper, or `None` when it may be. Matches on
/// every component so a nested `.github` is caught as well as the root one.
pub fn refused_path(path: &str) -> Option<&'static str> {
    let parts: Vec<&str> = path.split('/').collect();
    let last = parts.last().copied().unwrap_or_default();
    if parts.iter().any(|p| CI_DIRS.contains(p)) {
        return Some("CI configuration runs with the repository's secrets");
    }
    if parts.iter().any(|p| HOOK_DIRS.contains(p)) {
        return Some("hook configuration runs code on every contributor's machine");
    }
    if CI_FILES.contains(&last) {
        return Some("CI or hook configuration runs code outside review");
    }
    if parts.contains(&".git") || last.starts_with(".git") {
        return Some("git control files change how the repository behaves");
    }
    None
}

/// `owner/name` from a remote URL (`https://host/owner/name.git`, `git@host:owner/name.git`,
/// `ssh://git@host/owner/name`, a local path ending in `owner/name.git`): its last two path
/// parts. `None` for anything with fewer, or with a part that is not a plain repository name.
pub fn repo_slug_of_url(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let path = url.rsplit_once(':').map_or(url, |(_, rest)| rest);
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.rsplit('/');
    let (name, owner) = (parts.next()?, parts.next()?);
    let plain = |p: &str| {
        !p.is_empty()
            && p.len() <= 100
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    };
    (plain(owner) && plain(name)).then(|| format!("{owner}/{name}"))
}

/// Values of this process's secret-named variables, refused verbatim wherever they appear,
/// whatever their shape. Short values are left out: they would match ordinary text.
pub fn env_secrets() -> Vec<String> {
    const SECRET_NAME_ENDINGS: [&str; 5] = ["_KEY", "_TOKEN", "_SECRET", "_PASSWORD", "_PAT"];
    const MIN_SECRET_CHARS: usize = 12;
    std::env::vars()
        .filter(|(name, value)| {
            let name = name.to_ascii_uppercase();
            SECRET_NAME_ENDINGS.iter().any(|e| name.ends_with(e)) && value.len() >= MIN_SECRET_CHARS
        })
        .map(|(_, value)| value)
        .collect()
}

/// Whether `value` is shaped like a credential or holds one of the live `secrets` verbatim.
pub fn holds_secret(value: &str, secrets: &[String]) -> bool {
    let leaked = secrets
        .iter()
        .any(|s| !s.is_empty() && value.contains(s.as_str()));
    leaked || crate::redact::looks_like_credential(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_names_are_plain_components() {
        assert!(check_branch("fix-plant-log").is_ok());
        assert!(check_branch("feat/a.b").is_ok());
        for bad in ["", "-x", "a..b", "a/", "x.lock", "a b", "a//b"] {
            assert!(check_branch(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn protected_branches_include_the_remotes_default() {
        assert!(is_protected_branch("main", None));
        assert!(is_protected_branch("release/1.2", None));
        assert!(is_protected_branch("trunk", Some("trunk")));
        assert!(!is_protected_branch("feat/x", Some("trunk")));
    }

    #[test]
    fn ci_hook_and_git_paths_are_refused_anywhere_in_the_tree() {
        for bad in [
            ".github/workflows/ci.yml",
            "sub/.github/x.yml",
            ".gitattributes",
            "pkg/.gitmodules",
            ".gitlab-ci.yml",
            ".husky/pre-commit",
            "Jenkinsfile",
            "a/.git/config",
        ] {
            assert!(refused_path(bad).is_some(), "{bad}");
        }
        for ok in [
            "src/main.rs",
            "docs/github.md",
            "README.md",
            "gitignore.txt",
        ] {
            assert!(refused_path(ok).is_none(), "{ok}");
        }
    }

    #[test]
    fn slugs_come_from_the_last_two_parts_of_a_remote() {
        for url in [
            "https://example.com/acme/widgets.git",
            "git@example.com:acme/widgets.git",
            "ssh://git@example.com/acme/widgets",
            "/srv/git/acme/widgets.git",
        ] {
            assert_eq!(
                repo_slug_of_url(url).as_deref(),
                Some("acme/widgets"),
                "{url}"
            );
        }
        assert!(repo_slug_of_url("widgets.git").is_none());
        assert!(repo_slug_of_url("https://example.com/a b/c").is_none());
    }

    #[test]
    fn secrets_are_caught_by_shape_and_by_value() {
        let secrets = vec!["supersecretvalue1".to_string()];
        assert!(holds_secret("see supersecretvalue1 here", &secrets));
        assert!(!holds_secret("plain words", &secrets));
        assert!(holds_secret(
            "key sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789",
            &[]
        ));
    }
}
