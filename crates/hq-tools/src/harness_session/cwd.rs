use super::*;

/// Set by the MCP gateway (never by a caller) on calls that arrive on the
/// handoff-scoped key, so `herdr.handoff_cwd_allow` can bind them.
pub const HANDOFF_SCOPE_ARG: &str = "_hq_handoff_scope";

/// Characters a shell would expand inside the path the agent is started in.
pub(super) const CWD_FORBIDDEN: [char; 4] = ['~', '$', '`', '\0'];

/// A launch needs an absolute project directory. `/` and anything shaped like a user's home are
/// refused, whichever host the session runs on, because the agent stops at its folder-trust
/// dialog there and nobody is watching the pane. `..` components and shell-expanding characters
/// are refused so the string that was checked is the string herdr receives.
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
            "cwd '{cwd}' is refused by herdr.spawn_cwd_deny (matches '{entry}'); no session was started"
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
        "cwd '{}' is outside herdr.handoff_cwd_allow, which limits the handoff key; no session was started",
        cwd.display()
    )
}

/// `require_cwd` plus the deny list (and, for the handoff key, the allow list)
/// of an already loaded config.
pub fn require_cwd_in(
    cwd: Option<&str>,
    herdr: &hq_core::config::HerdrConfig,
    handoff_scope: bool,
) -> Result<PathBuf> {
    let path = require_cwd(cwd)?;
    check_cwd_allowed(&path.to_string_lossy(), &herdr.spawn_cwd_deny)?;
    if handoff_scope {
        check_handoff_cwd(&path, &herdr.handoff_cwd_allow)?;
    }
    Ok(path)
}

/// `require_cwd_in` with the config read from disk. A config that cannot be
/// read fails the check rather than skipping it.
pub fn require_allowed_cwd(cwd: Option<&str>) -> Result<PathBuf> {
    let cfg = hq_core::config::HqConfig::load()
        .map_err(|e| anyhow::anyhow!("cannot read the config to check herdr.spawn_cwd_deny: {e}"))?;
    require_cwd_in(cwd, &cfg.herdr, false)
}

/// Whether a tool call arrived on the handoff-scoped key.
pub fn is_handoff_scope(args: &Value) -> bool {
    args.get(HANDOFF_SCOPE_ARG).and_then(Value::as_bool) == Some(true)
}
