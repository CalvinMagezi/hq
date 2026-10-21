//! `hq net`: set up and manage the WireGuard private plane. The logic lives in the `hq-net` crate;
//! this is argument handling and the few reads of the machine (routing table, `ip_forward`, `wg`).
//! Nothing here runs as root, and nothing here runs a command for the owner: `init` prints the
//! root steps to review.

use clap::{Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum NetAction {
    /// Make the hub's key, choose the subnet, and print the firewall, sysctl and systemd steps to review and run as root
    Init {
        /// Private range for the plane (default 10.77.0.0/24, or the next free /24 if this machine routes it)
        #[arg(long)]
        subnet: Option<String>,
        /// UDP port WireGuard listens on
        #[arg(long)]
        port: Option<u16>,
        /// This server's public address as devices see it, host:port (for example example.com:51820)
        #[arg(long)]
        endpoint: Option<String>,
    },
    /// Add, revoke or list devices
    Peer {
        #[command(subcommand)]
        action: PeerAction,
    },
    /// Let two devices reach each other through the hub; every other pair stays unable to
    Link {
        #[command(subcommand)]
        action: LinkAction,
    },
    /// Show each device with its last handshake (needs root to read WireGuard)
    Status {
        /// Read `wg show <interface> latest-handshakes` output from this file, or `-` for stdin
        #[arg(long)]
        handshakes: Option<String>,
    },
    /// Write the applier's input from peers.toml
    Apply,
}

#[derive(Subcommand)]
pub enum PeerAction {
    /// Admit a device by its public key and print the config it needs (never a private key)
    Add {
        /// A short name: lowercase letters, digits, - and _
        label: String,
        /// The device's WireGuard public key (44 characters); the device makes its own private key
        #[arg(long)]
        pubkey: String,
        #[arg(long, value_enum, default_value = "laptop")]
        kind: KindArg,
        /// Also make a preshared key for this device (shown once, in the config)
        #[arg(long)]
        psk: bool,
        /// Seconds between the keepalive packets the device sends; 0 for none, or 10 to 300.
        /// Default: 25 for a laptop, none for a phone. A phone that other devices must reach (HQ
        /// running on it) needs 25 or the carrier closes the path; it costs battery
        #[arg(long, value_name = "SECONDS")]
        keepalive: Option<u16>,
        /// Do not print the QR code
        #[arg(long)]
        no_qr: bool,
        /// The key has the shape of a private key but is a public one (about 1 in 16 public keys do)
        #[arg(long)]
        confirm_public: bool,
    },
    /// Remove a device; its address is retired and never reused
    Revoke { label: String },
    /// Retire an address no device owns and remove whatever the hub has on it (an entry nobody can name)
    Forget {
        /// A device address in the plane's subnet, for example 10.77.0.9
        address: String,
    },
    /// List devices
    List,
}

#[derive(Subcommand)]
pub enum LinkAction {
    /// Allow two devices to reach each other through the hub, in both directions
    Add {
        /// A device, as `hq net peer list` shows it
        a: String,
        /// The other device
        b: String,
        /// The first link only: the applier on the hub is built with links (a version 2 file is
        /// refused whole by one that is not, revocations included)
        #[arg(long)]
        applier_updated: bool,
    },
    /// Take a link away
    Remove { a: String, b: String },
    /// List the links
    List,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum KindArg {
    Laptop,
    Phone,
}

#[cfg(unix)]
pub use unix::run;

#[cfg(not(unix))]
pub fn run(
    _action: NetAction,
    _dir: Option<PathBuf>,
    _spool: Option<PathBuf>,
) -> anyhow::Result<()> {
    anyhow::bail!("hq net runs on Linux and macOS; the hub is a Linux server")
}

#[cfg(unix)]
mod unix {
    use super::{KindArg, LinkAction, NetAction, PathBuf, PeerAction};
    use anyhow::{Context, Result, bail};
    use hq_net::init::{InitContext, InitOptions, init};
    use hq_net::ops::{self, AddRequest};
    use hq_net::store::{Kind, SPOOL_DIR, Store};
    use hq_net::subnet::{Subnet, routed_subnets};
    use std::io::Read;
    use std::process::Command;

    const PROC_NET_ROUTE: &str = "/proc/net/route";
    const PROC_IP_FORWARD: &str = "/proc/sys/net/ipv4/ip_forward";

    fn kind_of(arg: KindArg) -> Kind {
        match arg {
            KindArg::Laptop => Kind::Laptop,
            KindArg::Phone => Kind::Phone,
        }
    }

    fn state_dir(dir: Option<PathBuf>) -> PathBuf {
        dir.unwrap_or_else(|| hq_core::config::HqConfig::hq_dir().join("net"))
    }

    fn now_unix() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// `wg show <interface> latest-handshakes`: public keys and times only. `None` when it cannot
    /// be run or read, which without root is the usual case.
    fn read_handshakes(interface: &str, from: Option<&str>) -> Result<Option<String>> {
        match from {
            Some("-") => {
                let mut text = String::new();
                std::io::stdin()
                    .read_to_string(&mut text)
                    .context("reading stdin")?;
                Ok(Some(text))
            }
            Some(path) => Ok(Some(
                std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?,
            )),
            None => {
                let output = Command::new("wg")
                    .args(["show", interface, "latest-handshakes"])
                    .output();
                Ok(output
                    .ok()
                    .filter(|o| o.status.success())
                    .and_then(|o| String::from_utf8(o.stdout).ok()))
            }
        }
    }

    /// The plane's files and the applier's input belong to the user that runs HQ. As root they
    /// would land in root's home and the applier would be told to trust root-owned input.
    fn refuse_root(action: &NetAction) -> Result<()> {
        let reads_only = matches!(action, NetAction::Status { .. });
        if !reads_only && hq_net::files::effective_uid() == 0 {
            bail!(
                "run hq net as the user that runs HQ, not as root; it prints the steps that need sudo"
            );
        }
        Ok(())
    }

    pub fn run(action: NetAction, dir: Option<PathBuf>, spool: Option<PathBuf>) -> Result<()> {
        refuse_root(&action)?;
        let store = Store::new(state_dir(dir));
        let spool = spool.unwrap_or_else(|| PathBuf::from(SPOOL_DIR));
        let mut out = std::io::stdout();
        match action {
            NetAction::Init {
                subnet,
                port,
                endpoint,
            } => {
                let subnet = subnet
                    .map(|text| Subnet::parse(&text))
                    .transpose()
                    .map_err(|e| anyhow::anyhow!("--subnet: {e}"))?;
                let routed = std::fs::read_to_string(PROC_NET_ROUTE)
                    .ok()
                    .map(|t| routed_subnets(&t));
                let forward = std::fs::read_to_string(PROC_IP_FORWARD).ok();
                let mut ctx = InitContext::with_defaults(routed, forward);
                ctx.spool_dir = spool;
                let text = init(
                    &store,
                    &InitOptions {
                        subnet,
                        port,
                        endpoint,
                    },
                    &ctx,
                )?;
                println!("{text}");
                Ok(())
            }
            NetAction::Peer {
                action:
                    PeerAction::Add {
                        label,
                        pubkey,
                        kind,
                        psk,
                        keepalive,
                        no_qr,
                        confirm_public,
                    },
            } => {
                let request = AddRequest {
                    label: &label,
                    public_key: &pubkey,
                    kind: kind_of(kind),
                    preshared: psk,
                    keepalive,
                    with_qr: !no_qr,
                    confirm_public,
                };
                Ok(ops::peer_add(&store, &spool, &request, &mut out)?)
            }
            NetAction::Peer {
                action: PeerAction::Revoke { label },
            } => Ok(ops::peer_revoke(&store, &spool, &label, &mut out)?),
            NetAction::Peer {
                action: PeerAction::Forget { address },
            } => Ok(ops::peer_forget(&store, &spool, &address, &mut out)?),
            NetAction::Peer {
                action: PeerAction::List,
            } => Ok(ops::peer_list(&store, &mut out)?),
            NetAction::Link {
                action:
                    LinkAction::Add {
                        a,
                        b,
                        applier_updated,
                    },
            } => Ok(ops::link_add(
                &store,
                &spool,
                &a,
                &b,
                applier_updated,
                &mut out,
            )?),
            NetAction::Link {
                action: LinkAction::Remove { a, b },
            } => Ok(ops::link_remove(&store, &spool, &a, &b, &mut out)?),
            NetAction::Link {
                action: LinkAction::List,
            } => Ok(ops::link_list(&store, &mut out)?),
            NetAction::Apply => Ok(ops::apply(&store, &spool, &mut out)?),
            NetAction::Status { handshakes } => {
                let net = store.require_net()?;
                let text = read_handshakes(&net.interface, handshakes.as_deref())?;
                if text.is_none() && handshakes.is_some() {
                    bail!("could not read the handshakes");
                }
                Ok(ops::peer_status(
                    &store,
                    text.as_deref(),
                    now_unix(),
                    &mut out,
                )?)
            }
        }
    }
}
