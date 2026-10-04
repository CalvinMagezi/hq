//! Classifies filesystem paths as credential material, for the read tools'
//! argument check and for the text of bash commands.

use std::path::{Component, Path, PathBuf};

/// How sensitive a path is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretTier {
    /// Credential stores and HQ's own keys. Denied in every session.
    Always,
    /// Project-level secrets (`.env`, `*.pem`) that coding work sometimes
    /// needs. Denied only once untrusted content has entered the session.
    Tainted,
}

/// Home-relative credential stores. hq reaches what lives in them through
/// dedicated code paths (provider construction from config), never through a
/// generic read tool, so denying them costs no legitimate functionality.
const HOME_SECRET_PATHS: &[&str] = &[
    ".ssh",
    ".aws",
    ".gnupg",
    ".config/gcloud",
    ".config/gws",
    ".config/gh/hosts.yml",
    ".docker/config.json",
    ".kube/config",
    ".netrc",
    ".git-credentials",
    ".hq/config.yaml",
    ".hq/wallet.enc",
    ".hq/seed.enc",
    ".hq/secret.key",
    ".herenow/credentials",
];

/// OpenSSH private key basenames. A trailing `.pub` is public and allowed.
const PRIVATE_KEY_PREFIXES: &[&str] = &["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519"];

/// Extensions of files that usually carry keys or certificates.
const KEY_EXTENSIONS: &[&str] = &["pem", "key", "p12", "pfx", "keystore", "jks"];

/// `.env.<suffix>` templates that hold placeholders, not secrets.
const ENV_TEMPLATE_SUFFIXES: &[&str] = &["example", "sample", "template", "dist", "defaults"];

/// Basenames that are secret stores regardless of where they live.
const SECRET_BASENAMES: &[&str] = &["credentials.json", "secrets.json", "secrets.yaml"];

/// Classify one path. `path` should already be absolute where possible;
/// relative paths are still checked by basename.
pub fn classify_path(path: &Path) -> Option<SecretTier> {
    if is_proc_environ(path) || is_always_secret_location(path) {
        return Some(SecretTier::Always);
    }
    let name = path.file_name()?.to_str()?;
    if is_private_key_name(name) {
        return Some(SecretTier::Always);
    }
    if is_project_secret_name(name) {
        return Some(SecretTier::Tainted);
    }
    None
}

/// The strictest tier any path in `paths` falls into.
pub fn strictest_tier<'a>(paths: impl IntoIterator<Item = &'a Path>) -> Option<SecretTier> {
    let mut found = None;
    for path in paths {
        match classify_path(path) {
            Some(SecretTier::Always) => return Some(SecretTier::Always),
            Some(SecretTier::Tainted) => found = Some(SecretTier::Tainted),
            None => {}
        }
    }
    found
}

/// Where `gws` keeps its OAuth client secret and tokens when relocated.
const GWS_CONFIG_DIR_ENV: &str = "GOOGLE_WORKSPACE_CLI_CONFIG_DIR";

/// Absolute roots that are always denied: home credential stores, the gws
/// token store, and the active HQ config file. Also consumed by
/// `paths::sensitive_denied_paths`. The sandbox does not mask these, so the
/// `gws` and `gh` CLIs keep working from bash.
pub fn always_denied_roots() -> Vec<PathBuf> {
    let home = dirs::home_dir();
    let mut roots: Vec<PathBuf> = match &home {
        Some(home) => HOME_SECRET_PATHS.iter().map(|rel| home.join(rel)).collect(),
        None => Vec::new(),
    };
    let gws_dir = std::env::var_os(GWS_CONFIG_DIR_ENV)
        .filter(|d| !d.is_empty())
        .map(PathBuf::from);
    // A misconfigured value such as HOME itself would deny every file.
    if let Some(dir) = gws_dir.filter(|d| !home.as_ref().is_some_and(|h| h.starts_with(d))) {
        roots.push(dir);
    }
    roots.push(hq_core::config::HqConfig::config_read_path());
    roots.push(hq_core::config::HqConfig::config_file_path());
    roots.push(hq_core::approval_key::key_path());
    roots
}

fn is_always_secret_location(path: &Path) -> bool {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if always_denied_roots()
        .iter()
        .any(|root| resolved.starts_with(root) || path.starts_with(root))
    {
        return true;
    }
    is_hq_config_sibling(&resolved) || is_hq_config_sibling(path)
}

/// Backups of the HQ config (`config.yaml.bak-*`) and `*.env` files next to
/// it hold the same keys; on the VPS these are `/opt/hq/*.env`.
fn is_hq_config_sibling(path: &Path) -> bool {
    let config = hq_core::config::HqConfig::config_read_path();
    let (Some(config_dir), Some(config_name)) = (config.parent(), config.file_name()) else {
        return false;
    };
    if path.parent() != Some(config_dir) {
        return false;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let config_name = config_name.to_string_lossy();
    name.starts_with(config_name.as_ref()) || name.ends_with(".env")
}

fn is_proc_environ(path: &Path) -> bool {
    let parts: Vec<Component> = path.components().collect();
    let is_proc = parts
        .iter()
        .any(|c| matches!(c, Component::Normal(s) if *s == "proc"));
    is_proc && path.file_name().is_some_and(|n| n == "environ")
}

fn is_private_key_name(name: &str) -> bool {
    PRIVATE_KEY_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix) && !name.ends_with(".pub"))
}

fn is_project_secret_name(name: &str) -> bool {
    if SECRET_BASENAMES.contains(&name) {
        return true;
    }
    if name == ".env" || name.ends_with(".env") {
        return true;
    }
    if let Some(suffix) = name.strip_prefix(".env.") {
        return !ENV_TEMPLATE_SUFFIXES.contains(&suffix);
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => KEY_EXTENSIONS.contains(&ext),
        _ => false,
    }
}

/// Most matches a glob token may expand to before it counts as suspicious.
const GLOB_MAX_MATCHES: usize = 64;
/// Wall-clock budget for walking the filesystem for one glob token.
const GLOB_TIME_BUDGET: std::time::Duration = std::time::Duration::from_millis(250);

/// Path that always classifies as a secret (`/proc/*/environ`). A glob that
/// blows its budget is replaced by it, so the call is denied, not waved through.
const GLOB_DENY_SENTINEL: &str = "/proc/self/environ";

/// Name fragments that make an over-budget glob worth denying even when its
/// literal prefix is not near a known credential directory.
const SECRET_NAME_HINTS: &[&str] = &[
    "ssh", "ss?", "ss*", "id_", "env", "pem", "key", "secret", "credential", "aws", "gnupg",
    "netrc", "token", "hosts.yml", "config.yaml", "wallet", "seed",
];

/// Every token in a shell command that could name a file, resolved against
/// `home` (for `~` and `$HOME`) and `cwd` (for relative paths). Tokens are read
/// from the command as written and with quotes removed (so `~/.s''sh` is seen),
/// and glob tokens add the existing files they match. A glob that exceeds its
/// match or time budget and could reach credentials adds a denying sentinel.
pub fn path_tokens(command: &str, home: Option<&Path>, cwd: Option<&Path>) -> Vec<PathBuf> {
    let flat = crate::bash_policy::flatten_shell_text(command);
    let mut tokens: Vec<PathBuf> = [command, flat.as_str()]
        .iter()
        .flat_map(|text| text.split(is_shell_separator))
        .filter(|t| !t.is_empty() && !t.starts_with('-'))
        .filter_map(|t| resolve_token(t, home, cwd))
        .collect();
    let mut extra = Vec::new();
    for token in &tokens {
        match expand_glob(token) {
            GlobOutcome::Matches(found) => extra.extend(found),
            GlobOutcome::OverBudget(found) => {
                extra.extend(found);
                if glob_could_reach_secret(token) {
                    extra.push(PathBuf::from(GLOB_DENY_SENTINEL));
                }
            }
        }
    }
    tokens.extend(extra);
    tokens
}

#[derive(Debug)]
enum GlobOutcome {
    Matches(Vec<PathBuf>),
    /// Too many matches or too slow; carries what was found so far.
    OverBudget(Vec<PathBuf>),
}

/// Existing files a glob token matches. The walk runs on its own thread with a
/// time budget so a hostile pattern cannot stall the caller.
fn expand_glob(token: &Path) -> GlobOutcome {
    let text = token.to_string_lossy().into_owned();
    if !text.contains(['*', '?', '[']) {
        return GlobOutcome::Matches(Vec::new());
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let Ok(paths) = glob::glob(&text) else {
            let _ = tx.send((Vec::new(), false));
            return;
        };
        let found: Vec<PathBuf> = paths.flatten().take(GLOB_MAX_MATCHES + 1).collect();
        let over = found.len() > GLOB_MAX_MATCHES;
        let _ = tx.send((found.into_iter().take(GLOB_MAX_MATCHES).collect(), over));
    });
    match rx.recv_timeout(GLOB_TIME_BUDGET) {
        Ok((found, false)) => GlobOutcome::Matches(found),
        Ok((found, true)) => GlobOutcome::OverBudget(found),
        Err(_) => GlobOutcome::OverBudget(Vec::new()),
    }
}

/// True when the pattern's literal prefix is at, above or inside a credential
/// root, or its text names something secret-looking.
fn glob_could_reach_secret(token: &Path) -> bool {
    let text = token.to_string_lossy();
    let prefix: PathBuf = token
        .components()
        .take_while(|c| !c.as_os_str().to_string_lossy().contains(['*', '?', '[']))
        .collect();
    let near_root = always_denied_roots()
        .iter()
        .any(|root| root.starts_with(&prefix) || prefix.starts_with(root));
    let lowered = text.to_lowercase();
    near_root || SECRET_NAME_HINTS.iter().any(|hint| lowered.contains(hint))
}

fn is_shell_separator(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            ';' | '|' | '&' | '<' | '>' | '(' | ')' | '`' | '"' | '\'' | '=' | ','
        )
}

fn resolve_token(token: &str, home: Option<&Path>, cwd: Option<&Path>) -> Option<PathBuf> {
    let expanded = expand_home(token, home);
    let path = PathBuf::from(&expanded);
    if path.is_absolute() {
        return Some(path);
    }
    // A bare word such as `status` or `build` is never a secret basename, so
    // only tokens that look like files get resolved.
    let looks_like_file = expanded.contains('/') || expanded.contains('.');
    if !looks_like_file {
        return None;
    }
    Some(match cwd {
        Some(dir) if expanded.contains('/') => dir.join(&expanded),
        _ => path,
    })
}

fn expand_home(token: &str, home: Option<&Path>) -> String {
    let Some(home) = home else {
        return token.to_string();
    };
    let home = home.to_string_lossy();
    for prefix in ["${HOME}", "$HOME", "~"] {
        if let Some(rest) = token.strip_prefix(prefix)
            && (rest.is_empty() || rest.starts_with('/'))
        {
            return format!("{home}{rest}");
        }
    }
    token.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tier(p: &str) -> Option<SecretTier> {
        classify_path(Path::new(p))
    }

    #[test]
    fn private_keys_are_always_secret_but_public_halves_are_not() {
        assert_eq!(tier("/work/deploy/id_ed25519"), Some(SecretTier::Always));
        assert_eq!(tier("id_rsa"), Some(SecretTier::Always));
        assert_eq!(tier("/work/deploy/id_ed25519.pub"), None);
    }

    #[test]
    fn home_credential_stores_are_always_secret() {
        let home = dirs::home_dir().unwrap();
        for rel in [
            ".ssh/config",
            ".config/gws/token_cache.json",
            ".aws/credentials",
            ".hq/config.yaml",
            ".netrc",
        ] {
            assert_eq!(
                classify_path(&home.join(rel)),
                Some(SecretTier::Always),
                "{rel}"
            );
        }
    }

    #[test]
    fn proc_environ_is_always_secret() {
        assert_eq!(tier("/proc/1/environ"), Some(SecretTier::Always));
        assert_eq!(tier("/proc/self/environ"), Some(SecretTier::Always));
        assert_eq!(tier("/proc/cpuinfo"), None);
    }

    #[test]
    fn project_secrets_only_matter_once_tainted() {
        assert_eq!(tier("/repo/.env"), Some(SecretTier::Tainted));
        assert_eq!(tier("/repo/.env.production"), Some(SecretTier::Tainted));
        assert_eq!(tier("/repo/prod.env"), Some(SecretTier::Tainted));
        assert_eq!(tier("/repo/tls/server.pem"), Some(SecretTier::Tainted));
        assert_eq!(tier("/repo/.env.example"), None);
        assert_eq!(tier("/repo/src/main.rs"), None);
        assert_eq!(tier("/repo/Cargo.toml"), None);
    }

    #[test]
    fn command_tokens_expand_home_and_skip_flags() {
        let home = Path::new("/home/u");
        let cwd = Path::new("/repo");
        let tokens = path_tokens(
            "cat ~/.ssh/id_rsa && ls -la src/ $HOME/.aws",
            Some(home),
            Some(cwd),
        );
        assert!(tokens.contains(&PathBuf::from("/home/u/.ssh/id_rsa")));
        assert!(tokens.contains(&PathBuf::from("/home/u/.aws")));
        assert!(tokens.contains(&PathBuf::from("/repo/src/")));
        assert!(!tokens.iter().any(|t| t.to_string_lossy().contains("-la")));
    }

    #[test]
    fn quoted_and_redirected_paths_are_still_seen() {
        let tokens = path_tokens("base64 <'/x/.env' >\"out.txt\"", None, None);
        assert!(tokens.contains(&PathBuf::from("/x/.env")));
    }

    #[test]
    fn glob_tokens_expand_to_the_existing_files_they_match() {
        let dir = tempfile::tempdir().unwrap();
        let ssh = dir.path().join(".ssh");
        std::fs::create_dir(&ssh).unwrap();
        std::fs::write(ssh.join("id_ed25519"), "k").unwrap();
        let command = format!("cat {}/.ss?/id_*", dir.path().display());
        let tokens = path_tokens(&command, None, None);
        assert!(
            tokens
                .iter()
                .any(|t| classify_path(t) == Some(SecretTier::Always)),
            "{tokens:?}"
        );
    }

    fn key_dir(decoys: usize) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let ssh = dir.path().join(".ssh");
        std::fs::create_dir(&ssh).unwrap();
        for i in 0..decoys {
            std::fs::write(ssh.join(format!("a_decoy_{i:03}")), "x").unwrap();
        }
        std::fs::write(ssh.join("id_ed25519"), "k").unwrap();
        let pattern = format!("cat {}/.ss?/*", dir.path().display());
        (dir, pattern)
    }

    fn denied(command: &str) -> bool {
        let tokens = path_tokens(command, None, None);
        strictest_tier(tokens.iter().map(PathBuf::as_path)) == Some(SecretTier::Always)
    }

    #[test]
    fn a_decoy_flood_past_the_match_cap_still_denies_a_key_pattern() {
        let (_dir, pattern) = key_dir(GLOB_MAX_MATCHES + 40);
        assert!(denied(&pattern));
    }

    #[test]
    fn a_deep_wildcard_pattern_terminates_quickly() {
        let start = std::time::Instant::now();
        let _ = path_tokens("ls /*/*/*/*/*/*/*/*/*/*/x", None, None);
        let _ = path_tokens("ls /**/*.pem", None, None);
        assert!(start.elapsed() < std::time::Duration::from_secs(2), "{:?}", start.elapsed());
    }

    #[test]
    fn a_large_harmless_glob_is_not_denied() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..(GLOB_MAX_MATCHES + 10) {
            std::fs::write(dir.path().join(format!("f{i:03}.txt")), "x").unwrap();
        }
        let command = format!("wc -l {}/f*.txt", dir.path().display());
        assert!(!denied(&command));
    }
}
