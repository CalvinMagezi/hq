//! `hq edge`: run the public forwarder, and manage the list of devices it forwards for. The logic
//! is in the `hq-edge` crate; this is argument handling and the process lifecycle. The forwarder
//! is not built for Android or Windows, where `hq edge` explains that and exits.

use clap::Subcommand;

#[derive(Subcommand)]
pub enum EdgeAction {
    /// Run the forwarder in this terminal (what the systemd unit and the Docker image run)
    Run,
    /// Print the edge's endpoint id, which every device pins as the only peer allowed to open edge streams
    Id,
    /// Allow a device by its endpoint id (64 hex characters); an optional relay URL names the device's home relay when it is not this edge's
    Allow {
        id: String,
        #[arg(value_name = "RELAY_URL")]
        relay: Option<String>,
    },
    /// Remove a device. Its open connections are cut within a few seconds
    Revoke { id: String },
    /// List the allowed devices and the name each is reached at
    List,
}

#[cfg(all(unix, not(target_os = "android")))]
pub use supported::run;

#[cfg(not(all(unix, not(target_os = "android"))))]
pub async fn run(_action: EdgeAction, _config: Option<std::path::PathBuf>) -> anyhow::Result<()> {
    anyhow::bail!(
        "hq edge runs on Linux and macOS servers, not on Android or Windows. Run it on a public machine you control."
    )
}

#[cfg(all(unix, not(target_os = "android")))]
mod supported {
    use super::EdgeAction;
    use anyhow::{Context, Result, bail};
    use hq_core::config::HqConfig;
    use hq_edge::{
        AllowlistFile, Edge, EdgeConfig, Registered, allow_device, device_label, list_devices,
        revoke_device,
    };
    use std::{path::PathBuf, sync::Arc};

    /// Runs one `hq edge` subcommand. `config` is the config file to read the `edge:` section
    /// from (default: the one the rest of `hq` reads).
    pub async fn run(action: EdgeAction, config: Option<PathBuf>) -> Result<()> {
        let path = config.unwrap_or_else(HqConfig::config_read_path);
        let config = EdgeConfig::load(&path)?;
        match action {
            EdgeAction::Run => serve(config).await,
            EdgeAction::Id => {
                println!("{}", hq_edge::edge_id(&config.key_path())?);
                Ok(())
            }
            EdgeAction::Allow { id, relay } => {
                let id = id
                    .parse()
                    .context("not an endpoint id (64 hex characters)")?;
                let relay = relay
                    .map(|url| url.parse())
                    .transpose()
                    .context("not a relay URL")?;
                allow_device(&config.allowlist_path(), Registered { id, relay })?;
                println!("allowed {}", device_name(&config, &id));
                Ok(())
            }
            EdgeAction::Revoke { id } => {
                let id = id
                    .parse()
                    .context("not an endpoint id (64 hex characters)")?;
                if !revoke_device(&config.allowlist_path(), &id)? {
                    bail!("that device is not on the list");
                }
                println!(
                    "revoked {}; its open connections end within a few seconds",
                    device_name(&config, &id)
                );
                Ok(())
            }
            EdgeAction::List => {
                for device in list_devices(&config.allowlist_path())? {
                    println!("{}  {}", device.id, device_name(&config, &device.id));
                }
                Ok(())
            }
        }
    }

    fn device_name(config: &EdgeConfig, id: &iroh::EndpointId) -> String {
        match config.domain.is_empty() {
            true => device_label(id),
            false => format!("{}.{}", device_label(id), config.domain),
        }
    }

    async fn serve(config: EdgeConfig) -> Result<()> {
        config.validate()?;
        let registry = Arc::new(AllowlistFile::open(config.allowlist_path())?);
        let edge = Edge::start(config.clone(), registry).await?;
        println!(
            "hq edge: forwarding {} on {} (edge id {})",
            config.domain,
            edge.local_addr(),
            edge.id()
        );
        wait_for_stop().await;
        let stats = edge.stats();
        edge.shutdown().await;
        println!(
            "hq edge: stopped after {} connections ({} forwarded, {} refused)",
            stats.accepted,
            stats.forwarded + stats.relay_forwarded,
            stats.unknown_name + stats.hello_bad + stats.hello_timeout + stats.refused_busy
        );
        Ok(())
    }

    async fn wait_for_stop() {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = signal(SignalKind::terminate()).ok();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = async {
                match terminate.as_mut() {
                    Some(s) => { s.recv().await; }
                    None => std::future::pending::<()>().await,
                }
            } => {}
        }
    }
}
