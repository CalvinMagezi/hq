//! `hq update`: thin wrapper over the `hq-update` crate.

use clap::{Args, Subcommand};
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
    #[command(subcommand)]
    sub: Option<UpdateSub>,
}

#[derive(Subcommand, Debug)]
pub enum UpdateSub {
    /// Internal: exit 0 only when <SIG> is a valid signature by the release key over <FILE>. The
    /// Android app checks its update manifest with it.
    #[command(name = "verify-file", hide = true)]
    VerifyFile {
        file: std::path::PathBuf,
        sig: std::path::PathBuf,
    },
}

/// Largest file `verify-file` reads; a release manifest is a few hundred bytes.
#[cfg(unix)]
const VERIFY_FILE_MAX_BYTES: u64 = 1 << 20;

/// Checks `sig_path` (a `.minisig`) over `file` against `key_text`, the trusted release key.
#[cfg(unix)]
fn verify_file(
    key_text: Option<&str>,
    file: &std::path::Path,
    sig_path: &std::path::Path,
) -> Result<(), String> {
    use std::io::Read;
    let key_text = key_text
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .ok_or("this build has no release key (build with HQ_UPDATE_PUBKEY)")?;
    let key = hq_update::verify::parse_public_key(key_text).map_err(|e| e.to_string())?;
    let read = |path: &std::path::Path| -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .and_then(|f| f.take(VERIFY_FILE_MAX_BYTES + 1).read_to_end(&mut bytes))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if bytes.len() as u64 > VERIFY_FILE_MAX_BYTES {
            return Err(format!("{} is too large to verify", path.display()));
        }
        Ok(bytes)
    };
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    hq_update::verify::verify(&key, &name, &read(file)?, &read(sig_path)?)
        .map_err(|e| e.to_string())
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
    if args.sub.is_some() {
        eprintln!("`hq update verify-file` is not available on Windows");
        return 1;
    }
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
                if let Some(rest) = name
                    .strip_prefix("hq-lite-")
                    .and_then(|n| n.strip_suffix("-windows-x86_64.zip"))
                {
                    let short = rest.rsplit('-').next().unwrap_or_default();
                    let has = |suffix: &str| {
                        r["assets"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .any(|b| b["name"].as_str() == Some(&format!("{name}{suffix}")))
                    };
                    // The installer refuses a build without a signed checksum, so never offer one.
                    if has(".sha256") && has(".sha256.minisig") && !short.is_empty() {
                        return Some((
                            r["tag_name"].as_str().unwrap_or_default().to_string(),
                            name.to_string(),
                            short.to_string(),
                        ));
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
        let client = match reqwest::Client::builder()
            .user_agent("hq-update")
            .timeout(std::time::Duration::from_secs(30))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("could not set up a connection: {e}");
                return 1;
            }
        };
        let listing: Value = match client
            .get(format!(
                "https://api.github.com/repos/{REPO}/releases?per_page=30"
            ))
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r.json().await.unwrap_or(Value::Null),
            Ok(r) => {
                eprintln!(
                    "GitHub answered {} (anonymous requests are limited per address; try again later)",
                    r.status()
                );
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
            println!(
                "{}",
                serde_json::json!({ "current": GIT_SHA, "newest": name, "tag": tag, "up_to_date": current })
            );
        } else if current {
            println!("HQ Lite is up to date ({name}).");
        } else {
            println!(
                "A different HQ Lite build is published: {name} ({tag}). You have commit {}.",
                &GIT_SHA[..GIT_SHA.len().min(7)]
            );
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
        let Ok(exe) = std::env::current_exe() else {
            return 1;
        };
        let dir = exe
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        println!(
            "Running the installer from https://agent-hq.online/install.ps1 into {dir} (it verifies the download's signature against HQ's release key, then its SHA-256) ..."
        );
        let script = format!(
            "& ([scriptblock]::Create((irm https://agent-hq.online/install.ps1))) -Edition lite -Yes -InstallDir '{}'",
            dir.replace('\'', "''")
        );
        match std::process::Command::new(
            std::env::var_os("SystemRoot")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| "C:\\Windows".into())
                .join(r"System32\WindowsPowerShell\v1.0\powershell.exe"),
        )
        .args(["-NoProfile", "-Command", &script])
        .status()
        {
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
        fn the_newest_lite_zip_needs_a_signed_checksum_and_skips_drafts() {
            let zip = "hq-lite-0.9.1-abc1234-windows-x86_64.zip";
            let all = |tag: &str, draft: bool| {
                json!({ "tag_name": tag, "draft": draft, "assets": [
                {"name": zip}, {"name": format!("{zip}.sha256")}, {"name": format!("{zip}.sha256.minisig")} ] })
            };
            let rels = json!([
                all("v4", true),
                { "tag_name": "v3", "draft": false, "assets": [ {"name": zip}, {"name": format!("{zip}.sha256")} ] },
                { "tag_name": "v2", "draft": false, "assets": [ {"name": zip} ] },
                all("v1", false),
            ]);
            assert_eq!(
                newest_lite(&rels),
                Some(("v1".into(), zip.into(), "abc1234".into()))
            );
            assert_eq!(newest_lite(&json!([])), None);
        }

        #[test]
        fn current_means_the_build_commit_starts_with_the_zip_s_short_sha() {
            assert!(is_current(
                "abc1234",
                "abc1234def5678900000000000000000000000000"
            ));
            assert!(!is_current(
                "abc1234",
                "fff1234def5678900000000000000000000000000"
            ));
            assert!(!is_current("", "abc"));
        }
    }
}

#[cfg(unix)]
pub async fn run(args: UpdateArgs) -> i32 {
    if let Some(UpdateSub::VerifyFile { file, sig }) = &args.sub {
        return match verify_file(EMBEDDED_PUBKEY, file, sig) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("{e}");
                1
            }
        };
    }
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

#[cfg(all(test, unix))]
mod verify_file_tests {
    use super::verify_file;

    // A throwaway key made with `minisign -G -W`; its secret half was discarded.
    const KEY: &str = "untrusted comment: minisign public key 42D1EE7A44CF7901\nRWQBec9Eeu7RQiMUD1cx2V5qHDo1rOjJxWe9Ekj5EOiaaRvN4eIL/uQJ\n";
    const DATA: &str = "hq android update fixture\n";
    const SIG: &str = "untrusted comment: signature from minisign secret key\nRUQBec9Eeu7RQjfpN0jJPgTNhf4IS8Z9rHnJhNF2sWnM0MD4Bi1cGvWuCSRHev2+BKJOqUoiHLVGuAWAZpTTzNP4Y2B9uCI6WQw=\ntrusted comment: hq release data.txt\nakFmS253BHLI4rdW4X7h/8HJxcGIwVSzilMAq+uRqns2Hpb04zv2QRzGYdBdgv7I3lzCS78//gtkYslPLXzpCA==\n";

    fn run(key: Option<&str>, data: &str, sig: &str) -> Result<(), String> {
        let dir = tempfile::tempdir().unwrap();
        let (file, sig_file) = (
            dir.path().join("data.txt"),
            dir.path().join("data.txt.minisig"),
        );
        std::fs::write(&file, data).unwrap();
        std::fs::write(&sig_file, sig).unwrap();
        verify_file(key, &file, &sig_file)
    }

    #[test]
    fn update_verify_file_accepts_a_real_minisign_signature() {
        assert_eq!(run(Some(KEY), DATA, SIG), Ok(()));
    }

    #[test]
    fn update_verify_file_refuses_a_changed_file_a_garbled_signature_and_a_missing_key() {
        assert!(run(Some(KEY), "tampered\n", SIG).is_err());
        assert!(run(Some(KEY), DATA, "not a signature").is_err());
        assert!(run(None, DATA, SIG).is_err());
        assert!(run(Some("  "), DATA, SIG).is_err());
    }

    #[test]
    fn update_verify_file_refuses_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("none");
        assert!(verify_file(Some(KEY), &missing, &missing).is_err());
    }
}
