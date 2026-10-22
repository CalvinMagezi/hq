//! `hq net tunnel`: this device as a userspace WireGuard peer of the plane `hq net` runs. The
//! work is in the `hq-tunnel` crate; this is argument handling and the few reads of the process
//! (stdin, a terminal check). Built only with the `tunnel` feature.
//!
//! The private key never goes through the command line or the environment: it is the first line
//! of stdin, or a file only the user can read. What an app sends after it is control lines.

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use hq_tunnel::join::extract_block;
use hq_tunnel::keyin::{read_key_file, read_key_from_fd};
use hq_tunnel::run::{Exit, RunOptions, run as run_peer};
use hq_tunnel::store::{EVENTS_FILE, Import, KEY_FILE, STATUS_FILE, TunnelDir, keygen_pair};
use std::io::{Read, Write};
use std::os::fd::{FromRawFd, RawFd};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// What `hq net peer add` prints around the join text (prose and a QR code) is a few kilobytes.
const IMPORT_READ_MAX: u64 = 64 * 1024;
/// Exit status of `import` when the hub has to be confirmed first, so an app can tell it from an
/// error without reading the message.
const EXIT_NEEDS_CONFIRMATION: i32 = 2;
const STDIN_FD: RawFd = 0;
const STDOUT_FD: RawFd = 1;
/// A file from before the status said how old it can be: the longest a busy peer goes unwritten.
const STATUS_STALE_FALLBACK_SECS: u64 = 15;
/// Slack on top of the peer's own promise of how often it rewrites the file.
const STATUS_STALE_MARGIN_SECS: u64 = 10;
const TUNNEL_SUBDIR: &str = "tunnel";

#[derive(Subcommand)]
pub enum TunnelAction {
    /// Make this device's private key and print the public key to give `hq net peer add`
    Keygen {
        /// Print the private key, then the public key, on stdout (for an app that seals the
        /// private one) instead of writing the key file. Refused when stdout is a terminal
        #[arg(long)]
        stdout: bool,
    },
    /// Take the join text `hq net peer add` printed, from FILE or stdin. A hub this device has not
    /// seen has to be confirmed with its key, and takes the serve and reach entries away
    Import {
        file: Option<PathBuf>,
        /// The hub's public key, copied from the hub's own `hq net` output
        #[arg(long, value_name = "KEY")]
        confirm_hub: Option<String>,
    },
    /// Check the join text, the serve and reach entries and the key file without connecting
    Check {
        /// Check this key file instead of the one in the directory
        #[arg(long)]
        key_file: Option<PathBuf>,
    },
    /// Run the peer. The key is the first line of stdin, or --key-file; the lines after it are
    /// `network-changed` and `quit`, and the end of stdin stops the peer
    Run {
        #[arg(long)]
        key_file: Option<PathBuf>,
        /// Do not read control lines from stdin (needs --key-file); stop with a signal
        #[arg(long)]
        no_control: bool,
    },
    /// Show what the running peer last reported
    Status {
        #[arg(long)]
        json: bool,
    },
}

/// Whether the action only reads, and so may run as any user.
pub fn reads_only(action: &TunnelAction) -> bool {
    matches!(
        action,
        TunnelAction::Check { .. } | TunnelAction::Status { .. }
    )
}

fn tunnel_dir(dir: Option<PathBuf>) -> TunnelDir {
    TunnelDir::new(dir.unwrap_or_else(|| hq_core::config::HqConfig::hq_dir().join(TUNNEL_SUBDIR)))
}

fn is_terminal(fd: RawFd) -> bool {
    // SAFETY: isatty only inspects the descriptor.
    unsafe { libc::isatty(fd) == 1 }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub fn run(dir: Option<PathBuf>, action: TunnelAction) -> Result<()> {
    let tunnel = tunnel_dir(dir);
    match action {
        TunnelAction::Keygen { stdout } => keygen(&tunnel, stdout),
        TunnelAction::Import { file, confirm_hub } => import(&tunnel, file, confirm_hub),
        TunnelAction::Check { key_file } => check(&tunnel, key_file),
        TunnelAction::Run {
            key_file,
            no_control,
        } => run_tunnel(&tunnel, key_file, no_control),
        TunnelAction::Status { json } => status(&tunnel, json),
    }
}

fn keygen(tunnel: &TunnelDir, to_stdout: bool) -> Result<()> {
    if !to_stdout {
        let public = tunnel.keygen_to_file()?;
        println!("Key written to {}", tunnel.path(KEY_FILE).display());
        println!("Public key: {public}");
        println!(
            "Give the public key to the hub: hq net peer add <label> --pubkey {public} --kind phone"
        );
        return Ok(());
    }
    if is_terminal(STDOUT_FD) {
        bail!("refusing to print a private key to a terminal; pipe it, or leave out --stdout");
    }
    let (private, public) = keygen_pair();
    // Room for everything up front: a string that grows leaves its old, unwiped buffer behind.
    let mut text = zeroize::Zeroizing::new(String::with_capacity(private.len() + public.len() + 2));
    text.push_str(&private);
    text.push('\n');
    text.push_str(&public);
    text.push('\n');
    // Written straight to the descriptor: the standard output buffer would keep a copy.
    // SAFETY: descriptor 1 is open for the life of the process; ManuallyDrop never closes it.
    let mut out = std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(STDOUT_FD) });
    out.write_all(text.as_bytes())?;
    Ok(())
}

fn import(tunnel: &TunnelDir, file: Option<PathBuf>, confirm_hub: Option<String>) -> Result<()> {
    let mut text = String::new();
    match file.as_deref() {
        Some(path) if path.as_os_str() != "-" => {
            std::fs::File::open(path)
                .with_context(|| format!("reading {}", path.display()))?
                .take(IMPORT_READ_MAX)
                .read_to_string(&mut text)
                .with_context(|| format!("reading {}", path.display()))?;
        }
        _ => {
            std::io::stdin()
                .take(IMPORT_READ_MAX)
                .read_to_string(&mut text)
                .context("reading stdin")?;
        }
    }
    // The output of `hq net peer add` can be given as it is; only the join text inside is kept.
    match tunnel.import(extract_block(&text), confirm_hub.as_deref())? {
        Import::NeedsConfirmation { hub_key, endpoint } => {
            eprintln!(
                "This device has not seen this hub before, so nothing was imported.\n  the text names hub key {hub_key} at {endpoint}\nCompare that key with the hub's own public key (`sudo wg show hq0 public-key` on the hub, or the \"Hub public key\" line `hq net init` printed). If they match, run this again with --confirm-hub and the key as you read it on the hub, not copied from here. Importing a new hub removes the serve and reach entries written for the old one."
            );
            std::process::exit(EXIT_NEEDS_CONFIRMATION);
        }
        Import::Saved {
            hub_key,
            endpoint,
            same_hub,
            maps_cleared,
        } => {
            println!("Saved the join text: hub key {hub_key}, endpoint {endpoint}");
            if !same_hub && maps_cleared {
                println!(
                    "Removed {}: it was written for another hub.",
                    tunnel.path(hq_tunnel::store::MAPS_FILE).display()
                );
            }
            Ok(())
        }
    }
}

fn check(tunnel: &TunnelDir, key_file: Option<PathBuf>) -> Result<()> {
    let join = tunnel.load_join()?;
    let config = tunnel.load_maps(&join)?;
    println!("Join text: ok");
    println!("  this device: {}", join.address);
    println!("  hub key:     {}", hq_net::key::encode(&join.hub_key));
    println!("  endpoint:    {}", join.endpoint);
    println!(
        "  keepalive:   {}",
        join.keepalive_secs
            .map_or("none".to_string(), |s| format!("{s} s"))
    );
    println!(
        "Maps: {} serve, {} reach ({})",
        config.serve.len(),
        config.reach.len(),
        tunnel.path(hq_tunnel::store::MAPS_FILE).display()
    );
    for s in &config.serve {
        println!("  serve :{} -> {} from {:?}", s.port, s.to, s.from);
    }
    for r in &config.reach {
        println!("  reach {} -> {}", r.listen, r.to);
    }
    let key_path = key_file.unwrap_or_else(|| tunnel.path(KEY_FILE));
    match read_key_file(&key_path) {
        Ok(_) => println!("Key file: ok ({})", key_path.display()),
        Err(e) => println!("Key file: not usable ({e}); an app passes the key on stdin instead"),
    }
    Ok(())
}

fn run_tunnel(tunnel: &TunnelDir, key_file: Option<PathBuf>, no_control: bool) -> Result<()> {
    if no_control && key_file.is_none() {
        bail!("--no-control needs --key-file: stdin is where the key comes from otherwise");
    }
    let join = tunnel.load_join()?;
    let config = tunnel.load_maps(&join)?;
    let private_key = match key_file {
        Some(path) => read_key_file(&path)?,
        None => read_key_from_fd(STDIN_FD)?,
    };
    tunnel.ensure()?;
    let opts = RunOptions {
        join,
        config,
        private_key,
        status_path: Some(tunnel.path(STATUS_FILE)),
        events_path: Some(tunnel.path(EVENTS_FILE)),
        control_fd: (!no_control).then_some(STDIN_FD),
        bind: std::net::SocketAddr::from(([0, 0, 0, 0], 0)),
        endpoint_override: None,
        on_ready: None,
    };
    let summary = run_peer(opts)?;
    if summary.exit == Exit::ControlClosed {
        eprintln!(
            "The input ended, so the peer stopped: it takes the end of its input to mean the app that started it is gone. To run it without a controlling app, use --key-file with --no-control."
        );
    }
    Ok(())
}

fn status(tunnel: &TunnelDir, json: bool) -> Result<()> {
    let value = tunnel.read_status()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    let text = |key: &str| {
        value[key]
            .as_str()
            .map_or_else(|| value[key].to_string(), str::to_string)
    };
    let updated = value["updated_unix"].as_u64().unwrap_or(0);
    let age = now_unix().saturating_sub(updated);
    let state = text("state");
    let allowed = value["max_age_s"]
        .as_u64()
        .map_or(STATUS_STALE_FALLBACK_SECS, |max| {
            max + STATUS_STALE_MARGIN_SECS
        });
    if state != "stopped" && age > allowed {
        println!("state: not running (the last report was {age} s ago, as \"{state}\")");
    } else {
        println!("state: {state}");
    }
    println!("address: {}", text("address"));
    println!("endpoint: {}", text("endpoint"));
    println!("last handshake age (s): {}", text("last_handshake_age_s"));
    println!("connections: {}", text("connections"));
    println!(
        "accepted: {}  refused: {}",
        text("accepted"),
        text("denied")
    );
    println!("wakeups: {}", text("wakeups"));
    if let Some(kb) = value["rss_kb"].as_u64() {
        println!("memory: {kb} KB, {} open descriptors", text("open_fds"));
    }
    if !value["error"].is_null() {
        println!("error: {}", text("error"));
    }
    Ok(())
}
