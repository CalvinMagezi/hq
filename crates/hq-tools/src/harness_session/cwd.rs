use super::*;

/// Set by the MCP gateway (never by a caller) on calls that arrive on the
/// handoff-scoped key, so `agent_host.handoff_cwd_allow` can bind them.
pub const HANDOFF_SCOPE_ARG: &str = "_hq_handoff_scope";
/// Set by the gateway, never by a caller, on a call that arrived on the tasks scope.
pub const TASKS_SCOPE_ARG: &str = "_hq_tasks_scope";
/// Who a tasks-scope write is attributed to, whatever name the caller supplies.
pub const TASKS_SCOPE_ACTOR: &str = "mcp:tasks";

/// Characters a shell would expand inside the path the agent is started in.
pub(super) const CWD_FORBIDDEN: [char; 4] = ['~', '$', '`', '\0'];

/// A launch needs an absolute project directory. `/` and anything shaped like a user's home are
/// refused, whichever host the session runs on, because the agent stops at its folder-trust
/// dialog there and nobody is watching the pane. `..` components and shell-expanding characters
/// are refused so the string that was checked is the string the host receives.
pub fn require_cwd(cwd: Option<&str>) -> Result<PathBuf> {
    let cwd = cwd.map(str::trim).unwrap_or_default();
    if cwd.is_empty() {
        bail!("cwd is required: name the project directory to run the session in");
    }
    if cwd.contains(CWD_FORBIDDEN) {
        bail!("cwd '{cwd}' contains '~', '$', a backtick or a NUL; name the directory literally");
    }
    let path = PathBuf::from(cwd);
    if !path.is_absolute() {
        bail!("cwd '{cwd}' is not an absolute path");
    }
    if path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        bail!("cwd '{cwd}' contains '..'; name the directory without parent references");
    }
    let is_own_home = dirs::home_dir().is_some_and(|home| home == path);
    if is_home_shaped(&path) || is_own_home {
        bail!("cwd '{cwd}' is a home or root directory; name the project directory instead");
    }
    Ok(path)
}

/// Home directories of other machines HQ can start sessions on: `/Users/<name>` (macOS),
/// `/home/<name>` and `/root`, `/var/root`, and the HQ service user's `/opt/hq`. Also the
/// directories that only hold homes, and `/`. Components, not strings, so a trailing or doubled
/// slash cannot get past it, and case-insensitive because macOS volumes are.
pub(super) fn is_home_shaped(path: &Path) -> bool {
    let parts: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(p) => p.to_str().map(str::to_ascii_lowercase),
            _ => None,
        })
        .collect();
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    matches!(
        parts.as_slice(),
        [] | ["users" | "home" | "root"] | ["users" | "home", _] | ["var", "root"] | ["opt", "hq"]
    )
}

/// Lowercase, collapse repeated slashes and drop trailing ones so `/Clients//Acme/` and
/// `/clients/acme` compare equal.
pub(super) fn normalize_fragment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.trim().to_lowercase().chars() {
        if !(c == '/' && out.ends_with('/')) {
            out.push(c);
        }
    }
    out.trim_end_matches('/').to_string()
}

/// Refuse a working directory that contains any `deny` entry, compared
/// case-insensitively after `.` and `..` are resolved (a case-insensitive
/// filesystem would otherwise let `/Clients/X` past `/clients/x`). Blank
/// entries are ignored and entries are normalized (trailing and doubled
/// slashes). The path is checked as written, so a symlink into a denied
/// directory is not caught.
pub fn check_cwd_allowed(cwd: &str, deny: &[String]) -> Result<()> {
    let normalized = normalize_fragment(
        &crate::util::lexically_normalize(Path::new(cwd.trim())).to_string_lossy(),
    );
    let hit = deny
        .iter()
        .map(|d| (d.trim(), normalize_fragment(d)))
        .filter(|(_, n)| !n.is_empty())
        .find(|(_, n)| normalized.contains(n.as_str()));
    if let Some((entry, _)) = hit {
        bail!(
            "cwd '{cwd}' is refused by agent_host.spawn_cwd_deny (matches '{entry}'); no session was started"
        );
    }
    Ok(())
}

/// With `allow` non-empty, the handoff key may only start sessions in or under one of its entries.
pub fn check_handoff_cwd(cwd: &Path, allow: &[String]) -> Result<()> {
    let entries: Vec<&str> = allow.iter().map(|a| a.trim()).filter(|a| !a.is_empty()).collect();
    if entries.is_empty() || entries.iter().any(|a| cwd.starts_with(a)) {
        return Ok(());
    }
    bail!(
        "cwd '{}' is outside agent_host.handoff_cwd_allow, which limits the handoff key; no session was started",
        cwd.display()
    )
}

/// `require_cwd` plus the deny list (and, for the handoff key, the allow list)
/// of an already loaded config.
pub fn require_cwd_in(
    cwd: Option<&str>,
    agent_host: &hq_core::config::AgentHostConfig,
    handoff_scope: bool,
) -> Result<PathBuf> {
    let path = require_cwd(cwd)?;
    check_cwd_allowed(&path.to_string_lossy(), &agent_host.spawn_cwd_deny)?;
    if handoff_scope {
        check_handoff_cwd(&path, &agent_host.handoff_cwd_allow)?;
    }
    Ok(path)
}

/// `require_cwd_in` with the config read from disk. A config that cannot be
/// read fails the check rather than skipping it.
pub fn require_allowed_cwd(cwd: Option<&str>) -> Result<PathBuf> {
    let cfg = hq_core::config::HqConfig::load()
        .map_err(|e| anyhow::anyhow!("cannot read the config to check agent_host.spawn_cwd_deny: {e}"))?;
    require_cwd_in(cwd, &cfg.agent_host, false)
}

/// Whether a tool call arrived on the tasks scope. That scope may file and edit tasks but
/// must not reach anything else, so task tools that would otherwise write to an agent's
/// mailbox, attribute a write to a name the caller picked, or set routing tags skip it.
pub fn is_tasks_scope(args: &Value) -> bool {
    args.get(TASKS_SCOPE_ARG).and_then(Value::as_bool) == Some(true)
}

/// Whether a tool call arrived on the handoff-scoped key.
pub fn is_handoff_scope(args: &Value) -> bool {
    args.get(HANDOFF_SCOPE_ARG).and_then(Value::as_bool) == Some(true)
}

/// Vault folders that hold the owner's identity, threads and databases.
const VAULT_PRIVATE_DIRS: [&str; 3] = ["_system", "_threads", "_data"];

fn absolute_normalized(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    crate::util::lexically_normalize(&absolute)
}

fn lowercase_parts(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(p) => Some(p.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

/// Case-insensitive `starts_with`, because macOS volumes are.
fn is_inside(path: &Path, root: &Path) -> bool {
    let (path, root) = (lowercase_parts(path), lowercase_parts(root));
    path.len() >= root.len() && path[..root.len()] == root[..]
}

/// A session on the machine that holds the vault may not start in the vault, in HQ's config
/// directory, or in a private vault folder: the agent could rewrite identity files or the config.
/// A folder that merely contains the vault is refused too, unless it is a git checkout (a repo
/// with its vault in `.vault/` is the normal development layout). Sessions on other hosts are
/// unaffected, since the vault path means nothing on another machine. Paths are compared as
/// written, so a symlink into the vault is not caught.
pub fn check_cwd_outside_vault(
    cwd: &Path,
    host: &str,
    default_host: &str,
    vault: &Path,
) -> Result<()> {
    let host = if host.is_empty() { default_host } else { host };
    if host != hq_core::config::NATIVE_HOST && host != hq_core::config::LOCAL_HOST {
        return Ok(());
    }
    let cwd = absolute_normalized(cwd);
    let vault = absolute_normalized(vault);
    let hq_dir = absolute_normalized(&hq_core::config::HqConfig::hq_dir());
    let private = VAULT_PRIVATE_DIRS.iter().any(|d| is_inside(&cwd, &vault.join(d)));
    let is_vault = lowercase_parts(&cwd) == lowercase_parts(&vault);
    let holds_vault = is_inside(&vault, &cwd) && !cwd.join(".git").is_dir();
    if private || is_vault || holds_vault || is_inside(&cwd, &hq_dir) {
        bail!(
            "cwd '{}' is the HQ vault, HQ's config directory, a folder that holds the vault, or one of its private folders; name the project directory instead. No session was started.",
            cwd.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod vault_guard_tests {
    use super::*;

    const VAULT: &str = "/srv/hq/.vault";

    fn check(cwd: &str, host: &str) -> Result<()> {
        check_cwd_outside_vault(Path::new(cwd), host, "native", Path::new(VAULT))
    }

    #[test]
    fn the_vault_its_ancestors_and_its_private_folders_are_refused_on_the_vault_host() {
        for cwd in [
            VAULT,
            "/srv/hq",
            "/srv",
            "/srv/hq/.vault/_system",
            "/srv/hq/.vault/_threads/x",
            "/srv/hq/.vault/../.vault",
            "/SRV/HQ/.Vault/_System",
        ] {
            assert!(check(cwd, "").is_err(), "{cwd}");
            assert!(check(cwd, "local").is_err(), "{cwd}");
        }
        let hq_dir = hq_core::config::HqConfig::hq_dir();
        assert!(check(&hq_dir.display().to_string(), "").is_err(), "HQ's config directory");
    }

    #[test]
    fn project_directories_and_other_hosts_are_unaffected() {
        assert!(check("/srv/hq/.vault/Notebooks/Projects/site", "").is_ok());
        assert!(check("/srv/projects/app", "").is_ok());
        assert!(check("/srv/hq-oss", "native").is_ok());
        assert!(check(VAULT, "laptop").is_ok(), "a remote host's paths are its own");
        assert!(check("/workspace/projects/app", "laptop").is_ok());
    }

    #[test]
    fn a_git_checkout_that_contains_the_vault_is_allowed_but_its_vault_is_not() {
        let repo = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(repo.path().join(".git")).unwrap();
        let vault = repo.path().join(".vault");
        std::fs::create_dir(&vault).unwrap();
        assert!(check_cwd_outside_vault(repo.path(), "", "native", &vault).is_ok());
        assert!(check_cwd_outside_vault(&vault, "", "native", &vault).is_err());
        assert!(check_cwd_outside_vault(&vault.join("_system"), "", "native", &vault).is_err());
        let plain = tempfile::TempDir::new().unwrap();
        let inner = plain.path().join("vault");
        assert!(check_cwd_outside_vault(plain.path(), "", "native", &inner).is_err());
    }
}
