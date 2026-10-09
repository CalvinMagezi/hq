//! `hq update`: thin wrapper over the `hq-update` crate.

use clap::Args;
#[cfg(unix)]
use hq_update::cli::{BuildIdentity, CliArgs, Mode};

pub const GIT_SHA: &str = env!("HQ_GIT_SHA");
pub const BUILD_TIME: &str = env!("HQ_BUILD_TIME");
pub const VERSION: &str = env!("HQ_VERSION");
pub const LONG_VERSION: &str = concat!(
    env!("HQ_VERSION"),
    " (",
    env!("HQ_GIT_SHA"),
    " ",
    env!("HQ_BUILD_TIME"),
    ")"
);
/// Public key for verifying releases; set `HQ_UPDATE_PUBKEY` at build time
/// (forks) or ship `/etc/hq/update.pub`.
#[cfg(unix)]
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
        version: VERSION.to_string(),
        git_sha: GIT_SHA.to_string(),
        build_time: BUILD_TIME.to_string(),
    });
}

#[cfg(not(unix))]
pub async fn run(args: UpdateArgs) -> i32 {
    windows::run(args.apply, args.json).await
}

/// HQ Lite on Windows: look for a newer Lite zip among the GitHub releases, and with `--apply`
/// run the installer, which does the checksum check and the swap. No background updater, no
/// service: you ask, it looks.
#[cfg(not(unix))]
mod windows {
    use super::GIT_SHA;
    use serde_json::Value;

    const REPO: &str = "CalvinMagezi/hq";

    /// The newest Lite zip in a GitHub releases listing: (tag, zip name, short commit).
    pub(super) fn newest_lite(releases: &Value) -> Option<(String, String, String)> {
        for r in releases.as_array()? {
            if r["draft"].as_bool().unwrap_or(false) {
                continue;
            }
            for a in r["assets"].as_array().into_iter().flatten() {
                let name = a["name"].as_str().unwrap_or_default();
                // hq-lite-<version>-<short sha>-windows-x86_64.zip
                if let Some(rest) = name.strip_prefix("hq-lite-").and_then(|n| n.strip_suffix("-windows-x86_64.zip")) {
                    let short = rest.rsplit('-').next().unwrap_or_default();
                    let has_sum = r["assets"].as_array().into_iter().flatten().any(|b| b["name"].as_str() == Some(&format!("{name}.sha256")));
                    if has_sum && !short.is_empty() {
                        return Some((r["tag_name"].as_str().unwrap_or_default().to_string(), name.to_string(), short.to_string()));
                    }
                }
            }
        }
        None
    }

    pub(super) fn is_current(short: &str, git_sha: &str) -> bool {
        !short.is_empty() && git_sha.starts_with(short)
    }

    pub(super) async fn run(apply: bool, json: bool) -> i32 {
        let client = match reqwest::Client::builder().user_agent("hq-update").timeout(std::time::Duration::from_secs(30)).build() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("could not set up a connection: {e}");
                return 1;
            }
        };
        let listing: Value = match client.get(format!("https://api.github.com/repos/{REPO}/releases?per_page=30")).send().await {
            Ok(r) if r.status().is_success() => r.json().await.unwrap_or(Value::Null),
            Ok(r) => {
                eprintln!("GitHub answered {} (anonymous requests are limited per address; try again later)", r.status());
                return 1;
            }
            Err(e) => {
                eprintln!("could not reach GitHub: {e}");
                return 1;
            }
        };
        let Some((tag, name, short)) = newest_lite(&listing) else {
            eprintln!("No published HQ Lite build was found.");
            return 1;
        };
        let current = is_current(&short, GIT_SHA);
        if json {
            println!("{}", serde_json::json!({ "current": GIT_SHA, "newest": name, "tag": tag, "up_to_date": current }));
        } else if current {
            println!("HQ Lite is up to date ({name}).");
        } else {
            println!("A different HQ Lite build is published: {name} ({tag}). You have commit {}.", &GIT_SHA[..GIT_SHA.len().min(7)]);
        }
        if current {
            return 0;
        }
        if !apply {
            if !json {
                println!("Run `hq update --apply` to install it. Your notes and tasks are kept.");
            }
            return 10;
        }
        let Ok(exe) = std::env::current_exe() else { return 1 };
        let dir = exe.parent().map(|p| p.display().to_string()).unwrap_or_default();
        println!("Running the installer from https://agent-hq.online/install.ps1 into {dir} (it checks the download's SHA-256 first; the check shows integrity, not authorship) ...");
        let script = format!(
            "& ([scriptblock]::Create((irm https://agent-hq.online/install.ps1))) -Edition lite -Yes -InstallDir '{}'",
            dir.replace('\'', "''")
        );
        match std::process::Command::new(std::env::var_os("SystemRoot").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into()).join(r"System32\WindowsPowerShell\v1.0\powershell.exe")).args(["-NoProfile", "-Command", &script]).status() {
            Ok(s) if s.success() => 0,
            Ok(s) => s.code().unwrap_or(1),
            Err(e) => {
                eprintln!("could not start PowerShell: {e}");
                1
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::json;

        #[test]
        fn the_newest_lite_zip_needs_a_checksum_and_skips_drafts() {
            let zip = "hq-lite-0.9.1-abc1234-windows-x86_64.zip";
            let rels = json!([
                { "tag_name": "v3", "draft": true, "assets": [ {"name": zip}, {"name": format!("{zip}.sha256")} ] },
                { "tag_name": "v2", "draft": false, "assets": [ {"name": zip} ] },
                { "tag_name": "v1", "draft": false, "assets": [ {"name": zip}, {"name": format!("{zip}.sha256")} ] },
            ]);
            assert_eq!(newest_lite(&rels), Some(("v1".into(), zip.into(), "abc1234".into())));
            assert_eq!(newest_lite(&json!([])), None);
        }

        #[test]
        fn current_means_the_build_commit_starts_with_the_zip_s_short_sha() {
            assert!(is_current("abc1234", "abc1234def5678900000000000000000000000000"));
            assert!(!is_current("abc1234", "fff1234def5678900000000000000000000000000"));
            assert!(!is_current("", "abc"));
        }
    }
}

#[cfg(unix)]
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
        version: VERSION.to_string(),
        git_sha: GIT_SHA.to_string(),
        embedded_pubkey: EMBEDDED_PUBKEY.map(str::to_string),
    };
    hq_update::cli::run(cli, ident).await
}

/// `hq update-db ...`: prints the result on stdout, errors on stderr.
#[cfg(not(unix))]
pub fn run_db(_args: &[String]) -> i32 {
    eprintln!("`hq update-db` is not available on Windows");
    1
}

#[cfg(unix)]
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
