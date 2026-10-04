//! `hq update`: thin wrapper over the `hq-update` crate.

use clap::Args;
use hq_update::cli::{BuildIdentity, CliArgs, Mode};

pub const GIT_SHA: &str = env!("HQ_GIT_SHA");
pub const BUILD_TIME: &str = env!("HQ_BUILD_TIME");
pub const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("HQ_GIT_SHA"),
    " ",
    env!("HQ_BUILD_TIME"),
    ")"
);
/// Public key for verifying releases; set `HQ_UPDATE_PUBKEY` at build time
/// (forks) or ship `/etc/hq/update.pub`.
const EMBEDDED_PUBKEY: Option<&str> = option_env!("HQ_UPDATE_PUBKEY");

#[derive(Args, Debug)]
#[command(group(clap::ArgGroup::new("mode").args(["check", "apply", "rollback"])))]
pub struct UpdateArgs {
    /// Print current vs available; exit 0 when up to date, 10 when an update is available (default)
    #[arg(long)]
    check: bool,
    /// Download, verify and install the newest release, then restart
    #[arg(long)]
    apply: bool,
    /// Restore the previous binary and web files
    #[arg(long)]
    rollback: bool,
    /// With --rollback: also restore the database snapshot taken before the update
    #[arg(long, requires = "rollback")]
    restore_db: bool,
    /// Follow this channel for this run (main or stable)
    #[arg(long)]
    channel: Option<String>,
    /// Install exactly this version, even if older
    #[arg(long, value_name = "VERSION")]
    pin: Option<String>,
    /// Verify and report what would happen without changing anything
    #[arg(long)]
    dry_run: bool,
    /// With --apply: reinstall even if already on the newest version
    #[arg(long, requires = "apply")]
    force: bool,
    /// Machine-readable output
    #[arg(long)]
    json: bool,
    /// Update config file (default /etc/hq/update.conf)
    #[arg(long, value_name = "PATH")]
    conf: Option<std::path::PathBuf>,
}

pub fn register_build_info() {
    hq_core::build_info::set(hq_core::build_info::BuildInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        git_sha: GIT_SHA.to_string(),
        build_time: BUILD_TIME.to_string(),
    });
}

pub async fn run(args: UpdateArgs) -> i32 {
    let mode = if args.apply {
        Mode::Apply
    } else if args.rollback {
        Mode::Rollback
    } else {
        Mode::Check
    };
    let cli = CliArgs {
        mode,
        channel: args.channel,
        pin: args.pin,
        dry_run: args.dry_run,
        json: args.json,
        force: args.force,
        restore_db: args.restore_db,
        conf: args.conf,
    };
    let ident = BuildIdentity {
        version: env!("CARGO_PKG_VERSION").to_string(),
        git_sha: GIT_SHA.to_string(),
        embedded_pubkey: EMBEDDED_PUBKEY.map(str::to_string),
    };
    hq_update::cli::run(cli, ident).await
}

/// `hq update-db ...`: prints the result on stdout, errors on stderr.
pub fn run_db(args: &[String]) -> i32 {
    match hq_update::dbops::run_command(args) {
        Ok(out) => {
            println!("{out}");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}
