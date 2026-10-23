//! The app as a whole: settings, the host, pairing, the proxy and Connect Claude, wired from the
//! HQ config. The Tauri shell holds one of these and calls into it.

use crate::connect::ConnectClient;
use crate::host::{self, HostController, HostSpec};
use crate::pairing::{PairingConfig, PairingController};
use crate::proxy::{self, ProxyConfig, ProxyHandle};
use crate::settings::{Settings, create_private_dir};
use anyhow::{Context, Result, bail};
use hq_bridge::{RelayMode, Tuning};
use hq_core::config::HqConfig;
use hq_db::Database;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokio::sync::Mutex as AsyncMutex;

const HOST_LOG: &str = "host.log";
/// The longest closing may take in all: the host's grace period, a kill and a little slack.
const SHUTDOWN_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);
/// How long closing the pairing endpoints may take before the app stops waiting for them.
const SHUTDOWN_STEP: std::time::Duration = std::time::Duration::from_secs(5);
const DEFAULT_DEVICE_NAME: &str = "hq-desktop";

pub struct DesktopInit {
    /// The app's own folder (`~/.hq/desktop`): settings and the host's log.
    pub dir: PathBuf,
    /// The long-term pairing key. The same file `hq bridge` uses, so both agree on who this is.
    pub bridge_key: PathBuf,
    pub config: HqConfig,
    /// Where the app's own program lives, to find a bundled `hq` next to it.
    pub app_dir: Option<PathBuf>,
    pub home: Option<PathBuf>,
    pub path_var: Option<std::ffi::OsString>,
}

impl DesktopInit {
    /// The usual places on this machine.
    pub fn from_environment(config: HqConfig) -> Result<Self> {
        let home = dirs::home_dir().context("no home directory")?;
        Ok(Self {
            dir: home.join(".hq").join("desktop"),
            bridge_key: home.join(".hq").join("bridge").join("bridge.key"),
            app_dir: std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(Path::to_path_buf)),
            home: Some(home),
            path_var: std::env::var_os("PATH"),
            config,
        })
    }
}

pub struct Desktop {
    dir: PathBuf,
    settings: Mutex<Settings>,
    config: HqConfig,
    pub host: HostController,
    pub pairing: PairingController,
    proxy: AsyncMutex<Option<ProxyHandle>>,
    upstream: SocketAddr,
}

/// Where HQ's web server is reachable from this machine. The app talks to it only over loopback,
/// so a host that listens on one non-loopback address cannot be used.
pub fn upstream_addr(config: &HqConfig) -> Result<SocketAddr> {
    let bind: IpAddr = config
        .web_bind
        .parse()
        .with_context(|| format!("web_bind {:?} is not an address", config.web_bind))?;
    // The app dials 127.0.0.1, so only a host that answers there is usable: not `::1` alone.
    if bind != IpAddr::V4(Ipv4Addr::LOCALHOST) && !bind.is_unspecified() {
        bail!(
            "HQ listens on {bind} only, which this app does not use; set web_bind to 127.0.0.1 or 0.0.0.0"
        );
    }
    Ok(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), config.ws_port))
}

fn relay_mode(text: &str) -> Result<RelayMode> {
    match text {
        crate::settings::RELAY_NONE => Ok(RelayMode::Disabled),
        crate::settings::RELAY_DEFAULT => Ok(RelayMode::Default),
        url => Ok(RelayMode::custom([url.parse().context("relay URL")?])),
    }
}

impl Desktop {
    pub fn open(init: DesktopInit) -> Result<Self> {
        create_private_dir(&init.dir)?;
        let settings = Settings::load(&init.dir)?;
        let upstream = upstream_addr(&init.config)?;
        let candidates = host::search_path(
            init.app_dir.as_deref(),
            init.home.as_deref(),
            init.path_var.as_deref(),
        );
        // A missing program is reported when the host is started, not when the app opens.
        let program = host::locate_hq(settings.hq_path.as_deref(), &candidates)
            .unwrap_or_else(|_| PathBuf::from("hq"));
        let host = HostController::new(HostSpec::for_hq(
            program,
            init.dir.join(HOST_LOG),
            init.config.ws_port,
        ));
        let name = if settings.device_name.trim().is_empty() {
            hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| DEFAULT_DEVICE_NAME.into())
        } else {
            settings.device_name.clone()
        };
        let db_path = init.config.db_path();
        let pairing = PairingController::new(
            PairingConfig {
                key: init.bridge_key,
                bind: vec![SocketAddr::new(
                    Ipv4Addr::UNSPECIFIED.into(),
                    settings.bridge_port,
                )],
                relay: relay_mode(&settings.relay)?,
                name,
                with_addrs: true,
                tuning: Tuning::default(),
            },
            move || Database::open(&db_path),
            None,
        );
        Ok(Self {
            dir: init.dir,
            settings: Mutex::new(settings),
            config: init.config,
            host,
            pairing,
            proxy: AsyncMutex::new(None),
            upstream,
        })
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    /// Saves settings. Ports, relay and name take effect the next time the app opens.
    pub fn save_settings(&self, new: Settings) -> Result<()> {
        new.save(&self.dir)?;
        *self.settings.lock().unwrap() = new;
        Ok(())
    }

    /// Starts the loopback proxy if it is not running and returns its origin.
    pub async fn proxy_origin(&self) -> Result<String> {
        let mut slot = self.proxy.lock().await;
        if slot.is_none() {
            let port = self.settings().proxy_port;
            *slot = Some(
                proxy::start(ProxyConfig {
                    listen: SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port),
                    upstream: self.upstream,
                })
                .await?,
            );
        }
        Ok(slot.as_ref().expect("just set").origin())
    }

    /// The address of the main window: the proxy, with the web token in the fragment so HQ's web
    /// app signs itself in. The fragment never goes to the proxy or HQ; it stays in the window.
    pub async fn main_window_url(&self) -> Result<String> {
        let origin = self.proxy_origin().await?;
        Ok(
            match self
                .config
                .web_auth_token
                .as_deref()
                .filter(|t| !t.trim().is_empty())
            {
                Some(token) => format!("{origin}/#token={token}"),
                None => format!("{origin}/"),
            },
        )
    }

    pub async fn connect(&self) -> Result<ConnectClient> {
        let local = format!("http://127.0.0.1:{}", self.settings().proxy_port);
        ConnectClient::new(
            format!("http://{}", self.upstream),
            self.config
                .web_auth_token
                .clone()
                .filter(|t| !t.trim().is_empty()),
            local,
        )
    }

    /// The link that signs a browser in to HQ with a one-time code, on the app's own address.
    pub async fn sign_in_link(&self) -> Result<String> {
        let code = self.connect().await?.sign_in_code().await?;
        Ok(format!("{}/#code={code}", self.proxy_origin().await?))
    }

    /// Runs once when the app opens: starts the host only if the person's setting says so.
    pub async fn on_launch(&self) {
        if self.settings().start_host_on_launch
            && let Err(e) = self.host.start().await
        {
            tracing::warn!("could not start the host at launch: {e}");
        }
    }

    /// Runs when the app quits: closes what it opened, including the host it started. The three
    /// closes run side by side and each is bounded, so an open event stream or a slow relay
    /// cannot keep the host from being stopped.
    pub async fn shutdown(&self) {
        let _ = tokio::time::timeout(SHUTDOWN_LIMIT, self.close_everything()).await;
    }

    async fn close_everything(&self) {
        let host = async {
            let _ = self.host.stop_for_good().await;
        };
        let pairing = async {
            let _ = tokio::time::timeout(SHUTDOWN_STEP, self.pairing.stop()).await;
        };
        let proxy = async {
            let open = self.proxy.lock().await.take();
            if let Some(proxy) = open {
                proxy.shutdown().await;
            }
        };
        tokio::join!(host, pairing, proxy);
    }
}

/// Resolves when this process is asked to end (SIGTERM or SIGINT), so the app can close what it
/// opened instead of leaving its host behind.
#[cfg(unix)]
pub async fn wait_for_termination() {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut term), Ok(mut int)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) else {
        return std::future::pending().await;
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
}

#[cfg(not(unix))]
pub async fn wait_for_termination() {
    std::future::pending::<()>().await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(bind: &str) -> HqConfig {
        HqConfig {
            web_bind: bind.into(),
            ws_port: 6001,
            ..HqConfig::default()
        }
    }

    #[test]
    fn the_app_reaches_hq_over_loopback_or_not_at_all() {
        for ok in ["127.0.0.1", "0.0.0.0", "::"] {
            assert_eq!(
                upstream_addr(&config(ok)).unwrap(),
                "127.0.0.1:6001".parse().unwrap(),
                "{ok}"
            );
        }
        for bad in [
            "192.168.1.9",
            "100.64.0.5",
            "hq.example.com",
            "::1",
            "127.0.0.2",
        ] {
            assert!(upstream_addr(&config(bad)).is_err(), "{bad}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_termination_signal_ends_the_wait() {
        let waiting = tokio::spawn(wait_for_termination());
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        // SAFETY: raises a signal the wait above has just installed a handler for.
        unsafe {
            libc::raise(libc::SIGTERM);
        }
        tokio::time::timeout(std::time::Duration::from_secs(3), waiting)
            .await
            .expect("the wait did not end on SIGTERM")
            .unwrap();
    }

    #[tokio::test]
    async fn opening_the_app_starts_nothing_and_saves_nothing_it_was_not_told_to() {
        let dir = tempfile::tempdir().unwrap();
        let desktop = Desktop::open(DesktopInit {
            dir: dir.path().join("desktop"),
            bridge_key: dir.path().join("bridge").join("bridge.key"),
            config: HqConfig {
                ws_port: crate::host::tests_support_unused_port(),
                ..HqConfig::default()
            },
            app_dir: None,
            home: None,
            path_var: None,
        })
        .unwrap();
        assert!(!desktop.settings().start_host_on_launch);
        desktop.on_launch().await;
        assert_eq!(
            desktop.host.status().await,
            crate::host::HostStatus::Stopped
        );
        assert!(!desktop.pairing.snapshot().await.running);
        assert!(desktop.proxy.lock().await.is_none());
        assert!(!dir.path().join("bridge").exists());
        assert!(!dir.path().join("desktop").join("settings.json").exists());
    }
}
