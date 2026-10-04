//! `/etc/hq/update.conf` (TOML) plus environment overrides.

use crate::error::{Result, UpdateError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_CONF_PATH: &str = "/etc/hq/update.conf";
pub const DEFAULT_PUBKEY_PATH: &str = "/etc/hq/update.pub";
pub const DEFAULT_BASE_URL: &str = "https://github.com";
const MIB: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifyConfig {
    pub enabled: bool,
    /// Seconds between the "restarting" notice and the restart itself.
    pub grace_secs: u64,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            grace_secs: 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    pub max_manifest_bytes: u64,
    pub max_binary_bytes: u64,
    pub max_web_bytes: u64,
    /// Cap on unpacked web content, a guard against decompression bombs.
    pub max_web_unpacked_bytes: u64,
    pub max_web_files: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_manifest_bytes: MIB,
            max_binary_bytes: 300 * MIB,
            max_web_bytes: 100 * MIB,
            max_web_unpacked_bytes: 400 * MIB,
            max_web_files: 20_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateConfig {
    /// `<owner>/<repo>` on the release host. Required.
    pub repo: String,
    pub channel: String,
    /// Informational: the systemd timer owns the real schedule.
    pub interval: String,
    pub pin: Option<String>,
    pub base_url: String,
    pub pubkey_path: Option<PathBuf>,
    pub bin_path: PathBuf,
    pub web_dist: PathBuf,
    pub state_dir: PathBuf,
    /// Writable by the service user: snapshots are written and restored as that user.
    pub snapshots_dir: PathBuf,
    pub vault_db: PathBuf,
    pub vault_path: PathBuf,
    pub hq_config_path: PathBuf,
    pub service_unit: String,
    pub launchd_label: String,
    pub run_as_user: String,
    pub health_url: String,
    pub health_tries: u32,
    pub health_interval_secs: u64,
    pub keep_binaries: usize,
    pub snapshots_keep: usize,
    /// Optional: warn when the channel pointer was issued longer ago than this (freeze-attack hint).
    pub max_pointer_age_secs: Option<u64>,
    /// How long a failed release is skipped before the timer may retry it.
    pub blocked_ttl_secs: u64,
    /// Total time one `hq update --apply` may take; keep below the unit's TimeoutStartSec.
    pub deadline_secs: u64,
    pub notify: NotifyConfig,
    pub limits: Limits,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            repo: String::new(),
            channel: "stable".into(),
            interval: "10m".into(),
            pin: None,
            base_url: DEFAULT_BASE_URL.into(),
            pubkey_path: None,
            bin_path: "/usr/local/bin/hq".into(),
            web_dist: "/usr/local/share/hq/web/dist".into(),
            state_dir: "/var/lib/hq-update".into(),
            snapshots_dir: "/opt/hq/data/update-snapshots".into(),
            vault_db: "/opt/hq/.vault/_data/vault.db".into(),
            vault_path: "/opt/hq/.vault".into(),
            hq_config_path: "/opt/hq/config.yaml".into(),
            service_unit: "hq".into(),
            launchd_label: String::new(),
            run_as_user: "hq".into(),
            health_url: "http://127.0.0.1:5678/health".into(),
            health_tries: 6,
            health_interval_secs: 5,
            keep_binaries: 3,
            snapshots_keep: 5,
            max_pointer_age_secs: None,
            blocked_ttl_secs: 24 * 3600,
            deadline_secs: 15 * 60,
            notify: NotifyConfig::default(),
            limits: Limits::default(),
        }
    }
}

fn invalid(reason: impl Into<String>) -> UpdateError {
    UpdateError::Invalid {
        what: "update config",
        reason: reason.into(),
    }
}

/// `http://<loopback host>[:port][/...]`, parsed rather than pattern-matched
/// so `http://localhost:80@evil.test` and `http://127.0.0.1.evil.test` fail.
fn is_loopback_host(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') || authority.contains('\\') {
        return false;
    }
    let (host, port) = match authority.strip_prefix('[') {
        Some(v6) => match v6.split_once(']') {
            Some((h, tail)) => (format!("[{h}]"), tail.strip_prefix(':')),
            None => return false,
        },
        None => match authority.split_once(':') {
            Some((h, p)) => (h.to_string(), Some(p)),
            None => (authority.to_string(), None),
        },
    };
    let port_ok = port.is_none_or(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    port_ok && matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]")
}

impl UpdateConfig {
    pub fn parse(text: &str) -> Result<Self> {
        let cfg: UpdateConfig = toml::from_str(text).map_err(|e| invalid(format!("TOML: {e}")))?;
        Ok(cfg)
    }

    /// Loads the file (a missing file yields defaults, so env-only use works),
    /// then applies `HQ_UPDATE_*` overrides from `env`.
    pub fn load(path: &Path, env: &dyn Fn(&str) -> Option<String>) -> Result<Self> {
        let mut cfg = match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(invalid(format!("cannot read {}: {e}", path.display()))),
        };
        cfg.apply_env(env);
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn apply_env(&mut self, env: &dyn Fn(&str) -> Option<String>) {
        let get = |k: &str| env(k).filter(|v| !v.is_empty());
        if let Some(v) = get("HQ_UPDATE_REPO") {
            self.repo = v;
        }
        if let Some(v) = get("HQ_UPDATE_CHANNEL") {
            self.channel = v;
        }
        if let Some(v) = get("HQ_UPDATE_PIN") {
            self.pin = Some(v);
        }
        if let Some(v) = get("HQ_UPDATE_BASE_URL") {
            self.base_url = v;
        }
        if let Some(v) = get("HQ_UPDATE_STATE_DIR") {
            self.state_dir = v.into();
        }
        if let Some(v) = get("HQ_UPDATE_BIN_PATH") {
            self.bin_path = v.into();
        }
        if let Some(v) = get("HQ_UPDATE_WEB_DIST") {
            self.web_dist = v.into();
        }
        if let Some(n) = get("HQ_UPDATE_HEALTH_TRIES").and_then(|v| v.parse().ok()) {
            self.health_tries = n;
        }
        if let Some(n) = get("HQ_UPDATE_HEALTH_INTERVAL_SECS").and_then(|v| v.parse().ok()) {
            self.health_interval_secs = n;
        }
        if let Some(v) = get("HQ_UPDATE_HEALTH_URL") {
            self.health_url = v;
        }
    }

    pub fn validate(&self) -> Result<()> {
        let mut parts = self.repo.split('/');
        let (owner, name, extra) = (parts.next(), parts.next(), parts.next());
        let plain = |s: &str| {
            !s.is_empty()
                && s != "."
                && s != ".."
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        };
        match (owner, name, extra) {
            (Some(o), Some(n), None) if plain(o) && plain(n) => {}
            _ => return Err(invalid("`repo` must be <owner>/<repo>")),
        }
        if !crate::manifest::is_safe_name(&self.channel) {
            return Err(invalid(
                "`channel` must be a plain name such as main or stable",
            ));
        }
        if !(self.base_url.starts_with("https://") || is_loopback_host(&self.base_url)) {
            return Err(invalid(
                "`base_url` must be https (plain http only for loopback)",
            ));
        }
        if self.base_url.ends_with('/') {
            return Err(invalid("`base_url` must not end with a slash"));
        }
        if self.keep_binaries == 0 || self.keep_binaries > 9 {
            return Err(invalid("`keep_binaries` must be 1 to 9"));
        }
        if self.health_tries == 0 {
            return Err(invalid("`health_tries` must be at least 1"));
        }
        Ok(())
    }

    /// `{base}/{owner}/{repo}/releases/download`
    pub fn download_base(&self) -> String {
        format!("{}/{}/releases/download", self.base_url, self.repo)
    }

    pub fn channel_url(&self, channel: &str) -> String {
        format!(
            "{}/channel-{channel}/{}",
            self.download_base(),
            crate::manifest::channel_file_name(channel)
        )
    }

    pub fn release_manifest_url(&self, version: &str) -> String {
        format!(
            "{}/v{version}/{}",
            self.download_base(),
            crate::manifest::MANIFEST_NAME
        )
    }
}

/// True when `path` is owned by root (or by the current user when not root)
/// and neither group- nor world-writable.
pub fn is_loopback_url(url: &str) -> bool {
    is_loopback_host(url)
}

pub fn is_trusted_file(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    // SAFETY: geteuid has no preconditions.
    let euid = unsafe { libc::geteuid() };
    let owner_ok = meta.uid() == 0 || meta.uid() == euid;
    owner_ok && meta.mode() & 0o022 == 0 && meta.is_file()
}

/// Resolves the trusted public key text. Order: the configured
/// `pubkey_path`, then `default_path` when it exists, then the key compiled
/// into the binary. A key file that is not root-owned and read-only to
/// others is an error, never a silent fallback.
pub fn resolve_public_key(
    cfg: &UpdateConfig,
    default_path: &Path,
    embedded: Option<&str>,
) -> Result<String> {
    let candidate = cfg
        .pubkey_path
        .clone()
        .or_else(|| default_path.exists().then(|| default_path.to_path_buf()));
    if let Some(path) = candidate {
        if !is_trusted_file(&path) {
            return Err(UpdateError::NoPublicKey(format!(
                "{} must be a regular file owned by root and not writable by group or others",
                path.display()
            )));
        }
        return std::fs::read_to_string(&path)
            .map_err(|e| UpdateError::NoPublicKey(format!("{}: {e}", path.display())));
    }
    match embedded.map(str::trim).filter(|s| !s.is_empty()) {
        Some(key) => Ok(key.to_string()),
        None => Err(UpdateError::NoPublicKey(
            "no key at /etc/hq/update.pub and none compiled in (build with HQ_UPDATE_PUBKEY)"
                .into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn defaults_apply_to_a_minimal_file() {
        let cfg = UpdateConfig::parse("repo = \"acme/hq\"\n").unwrap();
        assert_eq!(cfg.channel, "stable");
        assert_eq!(cfg.keep_binaries, 3);
        assert_eq!(cfg.health_tries, 6);
        assert_eq!(cfg.health_interval_secs, 5);
        assert!(cfg.notify.enabled);
        cfg.validate().unwrap();
        assert_eq!(
            cfg.channel_url("stable"),
            "https://github.com/acme/hq/releases/download/channel-stable/channel-stable.json"
        );
    }

    #[test]
    fn file_values_and_env_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update.conf");
        std::fs::write(
            &path,
            "repo = \"acme/hq\"\nchannel = \"main\"\npin = \"0.9.0\"\n[notify]\nenabled = false\n[limits]\nmax_binary_bytes = 5\n",
        )
        .unwrap();
        let cfg = UpdateConfig::load(&path, &no_env).unwrap();
        assert_eq!(cfg.channel, "main");
        assert_eq!(cfg.pin.as_deref(), Some("0.9.0"));
        assert!(!cfg.notify.enabled);
        assert_eq!(cfg.limits.max_binary_bytes, 5);

        let env = |k: &str| match k {
            "HQ_UPDATE_CHANNEL" => Some("stable".to_string()),
            "HQ_UPDATE_REPO" => Some("other/repo".to_string()),
            _ => None,
        };
        let cfg = UpdateConfig::load(&path, &env).unwrap();
        assert_eq!(cfg.channel, "stable");
        let tries = |k: &str| (k == "HQ_UPDATE_HEALTH_TRIES").then(|| "24".to_string());
        assert_eq!(UpdateConfig::load(&path, &tries).unwrap().health_tries, 24);
        assert_eq!(cfg.repo, "other/repo");
    }

    #[test]
    fn missing_file_needs_env_repo() {
        let missing = Path::new("/definitely/not/here.conf");
        assert!(UpdateConfig::load(missing, &no_env).is_err());
        let env = |k: &str| (k == "HQ_UPDATE_REPO").then(|| "a/b".to_string());
        assert!(UpdateConfig::load(missing, &env).is_ok());
    }

    #[test]
    fn rejects_bad_values() {
        for text in [
            "repo = \"nope\"",
            "repo = \"a/b/c\"",
            "repo = \"a/..\"",
            "repo = \"a/b\"\nchannel = \"../x\"",
            "repo = \"a/b\"\nbase_url = \"http://example.com\"",
            "repo = \"a/b\"\nbase_url = \"https://example.com/\"",
            "repo = \"a/b\"\nkeep_binaries = 0",
            "repo = \"a/b\"\nunparseable = [",
        ] {
            let result = UpdateConfig::parse(text).and_then(|c| c.validate().map(|_| c));
            assert!(result.is_err(), "{text}");
        }
        let ok =
            UpdateConfig::parse("repo = \"a/b\"\nbase_url = \"http://127.0.0.1:8080\"").unwrap();
        ok.validate().unwrap();
    }

    #[test]
    fn loopback_host_is_parsed_exactly() {
        for ok in [
            "http://127.0.0.1",
            "http://localhost:8080",
            "http://127.0.0.1:1/x",
            "http://[::1]:9",
        ] {
            assert!(is_loopback_host(ok), "{ok}");
        }
        for bad in [
            "http://localhost:80@evil.test",
            "http://127.0.0.1@evil.test",
            "http://127.0.0.1.evil.test",
            "http://localhost.evil.test/",
            "http://evil.test/127.0.0.1",
            "http://localhost:80x",
            "http://localhost:",
            "https://localhost",
            "http://localhost\\@evil.test",
        ] {
            assert!(!is_loopback_host(bad), "{bad}");
        }
    }

    #[test]
    fn pubkey_resolution_prefers_trusted_file_over_embedded() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("update.pub");
        std::fs::write(&key, "FILEKEY\n").unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
        let cfg = UpdateConfig::default();
        let got = resolve_public_key(&cfg, &key, Some("EMBEDDED")).unwrap();
        assert_eq!(got.trim(), "FILEKEY");

        let missing = dir.path().join("none.pub");
        assert_eq!(
            resolve_public_key(&cfg, &missing, Some(" EMBEDDED ")).unwrap(),
            "EMBEDDED"
        );
        assert!(resolve_public_key(&cfg, &missing, None).is_err());
        assert!(resolve_public_key(&cfg, &missing, Some("  ")).is_err());
    }

    #[test]
    fn pubkey_file_must_not_be_group_or_world_writable() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("update.pub");
        std::fs::write(&key, "FILEKEY\n").unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o666)).unwrap();
        let cfg = UpdateConfig {
            pubkey_path: Some(key.clone()),
            ..UpdateConfig::default()
        };
        let err = resolve_public_key(&cfg, &key, Some("EMBEDDED")).unwrap_err();
        assert!(matches!(err, UpdateError::NoPublicKey(_)));
    }
}
