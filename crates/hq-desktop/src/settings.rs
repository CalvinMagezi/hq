//! The app's own settings: a small JSON file in the app folder, owner-only. Every launch starts
//! with nothing running; a setting is the only thing that may change that.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SETTINGS_FILE: &str = "settings.json";
/// The loopback origin the app's windows and the consent page use. Fixed, because the web app
/// keeps its sign-in per origin and a new port every launch would sign it out every launch.
pub const DEFAULT_PROXY_PORT: u16 = 47_474;
/// UDP port of the long-term pairing endpoint. Fixed, because a peer stores the address it paired
/// at and a relay-less restart on another port would leave that address stale.
pub const DEFAULT_BRIDGE_PORT: u16 = 47_475;
const MIN_PORT: u16 = 1_024;
pub const RELAY_NONE: &str = "none";
pub const RELAY_DEFAULT: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// Start the local `hq` host when the app opens. Off unless the person turns it on.
    pub start_host_on_launch: bool,
    pub proxy_port: u16,
    pub bridge_port: u16,
    /// How paired devices find this one besides a direct path: `none`, `default` (n0's public
    /// relays, for development and testing only) or a relay URL.
    pub relay: String,
    /// What this machine is called to the devices it pairs with. Empty: the host name.
    pub device_name: String,
    /// The `hq` program to run. Unset: next to the app, then `~/bin`, `/usr/local/bin`, `PATH`.
    pub hq_path: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            start_host_on_launch: false,
            proxy_port: DEFAULT_PROXY_PORT,
            bridge_port: DEFAULT_BRIDGE_PORT,
            relay: RELAY_NONE.into(),
            device_name: String::new(),
            hq_path: None,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        if self.proxy_port < MIN_PORT {
            bail!("proxy_port must be {MIN_PORT} or higher");
        }
        if self.bridge_port < MIN_PORT {
            bail!("bridge_port must be {MIN_PORT} or higher");
        }
        if self.proxy_port == self.bridge_port {
            // One is TCP and one UDP, but two identical numbers are always a mistake to debug.
            bail!("proxy_port and bridge_port must differ");
        }
        if self.relay != RELAY_NONE
            && self.relay != RELAY_DEFAULT
            && !self.relay.starts_with("https://")
        {
            bail!("relay must be none, default or an https:// relay URL");
        }
        Ok(())
    }

    /// The saved settings, or the defaults when there is no file yet. A file that cannot be read
    /// is an error, not a silent reset: it may hold the only copy of a choice.
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(SETTINGS_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let settings: Self = serde_json::from_str(&text)
            .with_context(|| format!("{} is not valid settings", path.display()))?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        self.validate()?;
        create_private_dir(dir)?;
        let path = dir.join(SETTINGS_FILE);
        let tmp = dir.join(format!("{SETTINGS_FILE}.tmp"));
        write_private(&tmp, serde_json::to_string_pretty(self)?.as_bytes())?;
        std::fs::rename(&tmp, &path).with_context(|| format!("saving {}", path.display()))
    }
}

pub fn create_private_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_starts_by_default() {
        let s = Settings::default();
        assert!(!s.start_host_on_launch);
        assert_eq!(s.relay, RELAY_NONE);
    }

    #[test]
    fn a_missing_file_gives_defaults_and_a_saved_one_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(dir.path()).unwrap(), Settings::default());
        let s = Settings {
            start_host_on_launch: true,
            device_name: "desk".into(),
            ..Settings::default()
        };
        s.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()).unwrap(), s);
    }

    #[test]
    fn a_broken_file_is_an_error_and_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        std::fs::write(&path, "{ not json").unwrap();
        assert!(Settings::load(dir.path()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    }

    #[test]
    fn unknown_keys_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(SETTINGS_FILE),
            r#"{"launch_at_login": true}"#,
        )
        .unwrap();
        assert!(Settings::load(dir.path()).is_err());
    }

    #[test]
    fn ports_and_relays_are_checked() {
        let bad = [
            Settings {
                proxy_port: 80,
                ..Settings::default()
            },
            Settings {
                bridge_port: 0,
                ..Settings::default()
            },
            Settings {
                bridge_port: DEFAULT_PROXY_PORT,
                ..Settings::default()
            },
            Settings {
                relay: "http://relay.example.com".into(),
                ..Settings::default()
            },
        ];
        for s in bad {
            assert!(s.validate().is_err(), "{s:?}");
        }
        let ok = Settings {
            relay: "https://relay.example.com".into(),
            ..Settings::default()
        };
        assert!(ok.validate().is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        Settings::default().save(dir.path()).unwrap();
        let mode = std::fs::metadata(dir.path().join(SETTINGS_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0);
    }
}
