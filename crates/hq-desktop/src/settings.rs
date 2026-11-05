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
    /// "Look for helper updates": off unless the person turns it on.
    pub look_for_helper_updates: bool,
    /// The one folder "Try it" runs a helper in. Unset: `~/.hq/desktop/try-it`.
    pub try_it_dir: Option<PathBuf>,
    /// How long HQ usually takes to start here, the median of the last starts. Kept by the app,
    /// never by the page: a saved copy from the page cannot change it.
    pub usual_start_secs: Option<u64>,
    /// The last few start times behind `usual_start_secs`, newest last. Kept by the app.
    pub recent_start_secs: Vec<u64>,
    /// The day (`YYYY-MM-DD`) the Connection sheet last opened by itself. Kept by the app.
    pub connection_sheet_day: Option<String>,
    /// Set once the Claude entries that pointed into the old HQ.app have been looked at. Kept by the app.
    pub bundle_rename_handled: bool,
    /// Lend this computer's model (Ollama) to the hubs it is paired with. Off unless turned on.
    pub share_model: bool,
    /// The model hubs should use. Unset: the first one Ollama lists.
    pub shared_model: Option<String>,
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
            look_for_helper_updates: false,
            try_it_dir: None,
            usual_start_secs: None,
            recent_start_secs: Vec::new(),
            connection_sheet_day: None,
            bundle_rename_handled: false,
            share_model: false,
            shared_model: None,
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
        if self.try_it_dir.as_deref().is_some_and(|d| !d.is_absolute()) {
            bail!("try_it_dir must be an absolute path");
        }
        Ok(())
    }

    /// These settings with the fields only the app writes taken from `kept`, so a stale copy
    /// sent back by the page cannot undo what the app learned in the meantime.
    pub fn keeping_app_fields(mut self, kept: &Settings) -> Self {
        self.usual_start_secs = kept.usual_start_secs;
        self.recent_start_secs = kept.recent_start_secs.clone();
        self.connection_sheet_day = kept.connection_sheet_day.clone();
        self.bundle_rename_handled = kept.bundle_rename_handled;
        self
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
        assert!(!s.share_model, "the model is not shared unless turned on");
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
    fn the_helper_settings_start_off_and_the_try_it_folder_is_absolute() {
        let s = Settings::default();
        assert!(!s.look_for_helper_updates);
        assert_eq!(s.try_it_dir, None);
        assert_eq!(s.usual_start_secs, None);
        let relative = Settings {
            try_it_dir: Some("try-it".into()),
            ..Settings::default()
        };
        assert!(relative.validate().is_err());
    }

    #[test]
    fn a_page_cannot_overwrite_what_the_app_learned() {
        let kept = Settings {
            usual_start_secs: Some(20),
            recent_start_secs: vec![19, 20, 21],
            connection_sheet_day: Some("2026-11-05".into()),
            bundle_rename_handled: true,
            ..Settings::default()
        };
        let from_page = Settings {
            device_name: "desk".into(),
            usual_start_secs: Some(1),
            ..Settings::default()
        };
        let merged = from_page.keeping_app_fields(&kept);
        assert_eq!(merged.device_name, "desk");
        assert_eq!(merged.usual_start_secs, Some(20));
        assert_eq!(merged.recent_start_secs, vec![19, 20, 21]);
        assert!(merged.bundle_rename_handled);
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
