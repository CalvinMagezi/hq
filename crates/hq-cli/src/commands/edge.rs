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
    /// Show a QR code and link for one device to scan; type the six digits it shows and it is
    /// added to the list under its name (needs a build with the `bridge` feature)
    Invite {
        /// How long the invitation works, in seconds
        #[arg(long, default_value_t = 600, value_parser = clap::value_parser!(u64).range(10..=3600))]
        ttl: u64,
    },
}

#[cfg(all(unix, not(target_os = "android")))]
pub use supported::run;

#[cfg(not(all(unix, not(target_os = "android"))))]
pub async fn run(_action: EdgeAction) -> anyhow::Result<()> {
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
    use std::sync::Arc;

    /// Runs one `hq edge` subcommand. The `edge:` section comes from the config file every other
    /// `hq` command reads (`--config` or `HQ_CONFIG_PATH`, else `~/.hq/config.yaml`).
    pub async fn run(action: EdgeAction) -> Result<()> {
        let config = EdgeConfig::load(&HqConfig::config_read_path())?;
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
            EdgeAction::Invite { ttl } => invite(config, std::time::Duration::from_secs(ttl)).await,
        }
    }

    #[cfg(not(feature = "bridge"))]
    async fn invite(_config: EdgeConfig, _ttl: std::time::Duration) -> Result<()> {
        bail!(
            "this build of hq cannot invite devices by QR code: build with `--features bridge`, or add a device with `hq edge allow <id>`"
        )
    }

    /// Shows the invitation and waits for one device; the person types the digits the device shows.
    #[cfg(feature = "bridge")]
    async fn invite(config: EdgeConfig, ttl: std::time::Duration) -> Result<()> {
        use hq_bridge::{Tuning, code::printable, qr};
        const SHOWN_CHARS: usize = 120;
        let spec = hq_edge::default_invite_spec(
            &config,
            Tuning {
                ttl,
                ..Tuning::default()
            },
        )?;
        let mut open = hq_edge::open_invite(&config, spec).await?;
        let link = open.ticket.link();
        println!(
            "This edge: {} ({})",
            hq_edge::edge_id(&config.key_path())?,
            config.domain
        );
        println!(
            "\nScan this on the device that should be reached through this edge, or give it the text below ({} s, one use):\n",
            ttl.as_secs()
        );
        if let Some(art) = qr::render(&link) {
            println!("{art}");
        }
        println!("{link}\n");
        if config.invite_bind.port() == 0 {
            println!(
                "The invitation listens on a random UDP port. If the device cannot connect, set edge.invite_bind to a fixed port, open it in this machine's firewall, and (behind NAT) list the public address in edge.invite_advertise.\n"
            );
        } else {
            println!(
                "The device must be able to reach UDP {} on this machine while the invitation is open.\n",
                config.invite_bind
            );
        }
        let request = tokio::select! {
            request = open.requests.recv() => request,
            _ = tokio::signal::ctrl_c() => None,
        };
        let Some(request) = request else {
            println!("No device registered: the invitation ended.");
            return Ok(());
        };
        println!(
            "\"{}\" ({}) wants to be reached through this edge.",
            printable(&request.name, SHOWN_CHARS),
            request.joiner
        );
        let accept = crate::commands::bridge::typed_digits_match(&request.sas).await;
        let delivered = request.decide.send(accept).is_ok();
        if !delivered {
            println!("Too late: the device gave up waiting. Nothing was registered.");
        } else if !accept {
            println!("Declined {}.", request.joiner);
        }
        // The answer reaches the device first; the list is written after it. Wait for the
        // invitation to end, then say what the list says, not what was hoped.
        tokio::select! {
            _ = open.closed() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
        if delivered && accept {
            let listed = list_devices(&config.allowlist_path())
                .map(|devices| devices.iter().any(|d| d.id == request.joiner))
                .unwrap_or(false);
            if listed {
                println!(
                    "Registered {}. It is reached at {}.",
                    request.joiner,
                    device_name(&config, &request.joiner)
                );
            } else {
                println!(
                    "The device was accepted but is not on the list: see the warnings above, or add it with `hq edge allow {}`.",
                    request.joiner
                );
            }
        }
        Ok(())
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
