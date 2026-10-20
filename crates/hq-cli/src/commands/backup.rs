//! `hq backup` and `hq restore`: argument handling and what is printed. The format, the
//! encryption and every check live in the `hq-backup` crate; see `docs/BACKUP.md`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use hq_backup::{Argon2Cost, BackupOptions, RestoreOptions, Secret};
use hq_core::config::HqConfig;

const PASSPHRASE_ENV: &str = "HQ_BACKUP_PASSPHRASE";
const LIST_LIMIT: usize = 10;
const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;
const SECS_PER_HOUR: u64 = 3600;

/// How to open or seal a backup. Exactly one of the three, or none when `backup.key_file` is set.
#[derive(Args, Clone)]
pub struct SecretArgs {
    /// A key file made by `hq backup keygen` (recommended: nothing to forget or guess)
    #[arg(long, value_name = "FILE")]
    pub key_file: Option<PathBuf>,
    /// A file whose first line is the passphrase (a file, not the passphrase itself: arguments show in process lists)
    #[arg(long, value_name = "FILE")]
    pub passphrase_file: Option<PathBuf>,
    /// Read the passphrase from the environment variable HQ_BACKUP_PASSPHRASE
    #[arg(long)]
    pub passphrase_env: bool,
}

#[derive(Args)]
pub struct RestoreArgs {
    /// The backup file
    pub archive: PathBuf,
    /// A folder that does not exist yet, or an empty one, to restore the vault into
    #[arg(long)]
    pub into: PathBuf,
    #[command(flatten)]
    pub secret: SecretArgs,
    /// Where to put the config file if the backup holds one (otherwise it is not restored)
    #[arg(long, value_name = "FILE")]
    pub config_out: Option<PathBuf>,
    /// Do not rebuild the search index after restoring
    #[arg(long)]
    pub no_reindex: bool,
    /// Refuse a backup that unpacks to more than this many GiB (default 64)
    #[arg(long, value_name = "GIB")]
    pub max_size_gib: Option<u64>,
}

#[derive(Subcommand)]
pub enum BackupCommand {
    /// Write an encrypted backup of the vault and its database
    Create {
        /// A file to create, or a folder to put a timestamped file in (default: backup.dir)
        #[arg(long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        secret: SecretArgs,
        /// Also back up config.yaml, which holds API keys and tokens
        #[arg(long)]
        include_config: bool,
        /// Skip reading the finished backup back to check it
        #[arg(long)]
        no_verify: bool,
    },
    /// Open a backup and check all of it, without restoring anything
    Verify {
        archive: PathBuf,
        #[command(flatten)]
        secret: SecretArgs,
        /// Refuse a backup that unpacks to more than this many GiB (default 64)
        #[arg(long, value_name = "GIB")]
        max_size_gib: Option<u64>,
    },
    /// Show whether scheduled backups are on and how the last one went
    Status,
    /// Restore a backup into a new or empty folder (the same as `hq restore`)
    Restore(RestoreArgs),
    /// Make a random key file for backups
    Keygen {
        /// Where to write it; an existing file is never replaced
        path: PathBuf,
    },
    /// Delete all but the newest backups in a folder
    Prune {
        dir: PathBuf,
        /// How many to keep
        #[arg(long, default_value_t = 7)]
        keep: usize,
    },
}

pub fn run(config: &HqConfig, command: BackupCommand) -> Result<()> {
    match command {
        BackupCommand::Create {
            out,
            secret,
            include_config,
            no_verify,
        } => create(config, out, &secret, include_config, !no_verify),
        BackupCommand::Verify {
            archive,
            secret,
            max_size_gib,
        } => verify(config, &archive, &secret, max_size_gib),
        BackupCommand::Status => status(config),
        BackupCommand::Restore(args) => restore(config, &args),
        BackupCommand::Keygen { path } => keygen(&path),
        BackupCommand::Prune { dir, keep } => prune(&dir, keep),
    }
}

/// The secret the arguments name, else the key file from `backup.key_file`.
fn secret(args: &SecretArgs, config: &HqConfig) -> Result<Secret> {
    let named = usize::from(args.key_file.is_some())
        + usize::from(args.passphrase_file.is_some())
        + usize::from(args.passphrase_env);
    if named > 1 {
        bail!("name one way to open the backup: --key-file, --passphrase-file or --passphrase-env");
    }
    if let Some(path) = &args.key_file {
        return Ok(Secret::key_file(path)?);
    }
    if let Some(path) = &args.passphrase_file {
        let text = hq_backup::read_private_file(path)?;
        return Ok(Secret::passphrase(text.lines().next().unwrap_or(""))?);
    }
    if args.passphrase_env {
        return Ok(Secret::passphrase_from_env(PASSPHRASE_ENV)?);
    }
    match &config.backup.key_file {
        Some(path) => Ok(Secret::key_file(path)?),
        None => bail!(
            "say how the backup is sealed: --key-file <file> (make one with `hq backup keygen \
             <file>`), --passphrase-file <file>, or --passphrase-env"
        ),
    }
}

fn gib_limit(gib: Option<u64>) -> Option<u64> {
    gib.map(|g| g.saturating_mul(BYTES_PER_GIB))
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

fn print_skipped(skipped: &[String]) {
    if skipped.is_empty() {
        return;
    }
    println!("Not backed up ({}):", skipped.len());
    // What could not be read first: it is what makes a backup incomplete, and a vault full of
    // links must not push it below the cut.
    let (unreadable, other): (Vec<&String>, Vec<&String>) = skipped
        .iter()
        .partition(|line| line.contains("not readable by this user"));
    for line in unreadable.into_iter().chain(other).take(LIST_LIMIT) {
        println!("  {line}");
    }
    if skipped.len() > LIST_LIMIT {
        println!("  and {} more", skipped.len() - LIST_LIMIT);
    }
}

fn create(
    config: &HqConfig,
    out: Option<PathBuf>,
    args: &SecretArgs,
    include_config: bool,
    verify: bool,
) -> Result<()> {
    let out = out
        .or_else(|| config.backup.dir.clone())
        .context("say where to write it: --out <file or folder>, or set backup.dir")?;
    let secret = secret(args, config)?;
    let config_file = include_config.then(HqConfig::config_read_path);
    let db = config.db_path();
    let report = hq_backup::create(&BackupOptions {
        vault: &config.vault_path,
        db: &db,
        config_file: config_file.as_deref(),
        out: &out,
        secret: &secret,
        hq_version: env!("CARGO_PKG_VERSION"),
        cost: Argon2Cost::default(),
        verify,
    })?;
    println!("Backup written: {}", report.path.display());
    println!(
        "  {} files, {} before compression, {} on disk{}",
        report.files,
        megabytes(report.bytes),
        megabytes(report.archive_bytes),
        if report.verified {
            ", read back and checked"
        } else {
            ", NOT read back (--no-verify)"
        }
    );
    print_skipped(&report.skipped);
    if include_config {
        println!("It includes config.yaml, with your API keys and tokens.");
    }
    println!(
        "Keep the key or passphrase somewhere other than this machine: without it no backup can be opened."
    );
    if report.unreadable > 0 {
        bail!(
            "{} file(s) or folder(s) could not be read by this user, so this backup is incomplete (it was written; the list is above)",
            report.unreadable
        );
    }
    Ok(())
}

fn verify(
    config: &HqConfig,
    archive: &Path,
    args: &SecretArgs,
    max_size_gib: Option<u64>,
) -> Result<()> {
    let manifest =
        hq_backup::verify_limited(archive, &secret(args, config)?, gib_limit(max_size_gib))?;
    println!("{}: all of it checks out.", archive.display());
    println!(
        "  made {} by HQ {}: {} files, {}{}",
        manifest.created,
        manifest.hq_version,
        manifest.file_count(),
        megabytes(manifest.total_bytes()),
        if manifest.config.is_some() {
            ", with config.yaml"
        } else {
            ""
        }
    );
    print_skipped(&manifest.skipped);
    Ok(())
}

pub fn restore(config: &HqConfig, args: &RestoreArgs) -> Result<()> {
    let secret = secret(&args.secret, config)?;
    let report = hq_backup::restore(&RestoreOptions {
        archive: &args.archive,
        into: &args.into,
        secret: &secret,
        config_out: args.config_out.as_deref(),
        reindex: !args.no_reindex,
        max_bytes: gib_limit(args.max_size_gib),
    })?;
    println!(
        "Restored {} files ({}) into {}",
        report.manifest.file_count(),
        megabytes(report.manifest.total_bytes()),
        report.vault.display()
    );
    println!(
        "  made {} by HQ {}",
        report.manifest.created, report.manifest.hq_version
    );
    if let Some(path) = &report.config_restored {
        println!("  config written to {}", path.display());
    }
    print_skipped(&report.manifest.skipped);
    for warning in &report.warnings {
        println!("Note: {warning}");
    }
    println!(
        "To use it: HQ_VAULT_PATH={} hq status",
        report.vault.display()
    );
    Ok(())
}

fn keygen(path: &Path) -> Result<()> {
    Secret::generate_key_file(path)?;
    println!(
        "Wrote a new backup key to {} (readable only by you).",
        path.display()
    );
    println!(
        "It is not shown here. Keep a copy somewhere other than this machine, such as a password manager:"
    );
    println!(
        "a backup cannot be opened without it, and anyone who has it and a backup can read the backup."
    );
    Ok(())
}

fn status(config: &HqConfig) -> Result<()> {
    let cfg = &config.backup;
    println!(
        "Scheduled backups: {}",
        if cfg.enabled {
            "on"
        } else {
            "off (backup.enabled)"
        }
    );
    let Some(dir) = cfg.dir.as_deref() else {
        println!("No backup.dir is set.");
        return Ok(());
    };
    println!(
        "  every {} h, keeping the newest {}, in {}",
        cfg.every_hours,
        cfg.keep,
        dir.display()
    );
    match hq_backup::newest(dir) {
        Ok(Some((path, written))) => {
            let age = written.elapsed().map_or(0, |a| a.as_secs() / SECS_PER_HOUR);
            println!("Newest backup: {} ({age} h ago)", path.display());
        }
        Ok(None) => println!("Newest backup: none yet"),
        Err(e) => println!("Newest backup: cannot look ({e})"),
    }
    match hq_backup::status::read(dir) {
        Some(last) => {
            println!(
                "Last scheduled run: {} at {}\n  {}",
                if last.ok { "ok" } else { "FAILED" },
                last.finished,
                last.message
            );
            if last.left_out > 0 {
                println!(
                    "  {} file(s) or folder(s) were left out (links, files that kept changing, \
                     anything unreadable); `hq backup verify <file>` lists them",
                    last.left_out
                );
            }
        }
        None => println!("Last scheduled run: none recorded"),
    }
    Ok(())
}

fn prune(dir: &Path, keep: usize) -> Result<()> {
    let removed = hq_backup::prune(dir, keep, None)?;
    println!("Kept the newest {keep}; removed {}.", removed.len());
    for path in removed.iter().take(LIST_LIMIT) {
        println!("  {}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> SecretArgs {
        SecretArgs {
            key_file: None,
            passphrase_file: None,
            passphrase_env: false,
        }
    }

    #[test]
    fn naming_no_secret_and_having_no_default_key_says_how() {
        let err = secret(&none(), &HqConfig::default())
            .err()
            .unwrap()
            .to_string();
        assert!(
            err.contains("--key-file") && err.contains("keygen"),
            "{err}"
        );
    }

    #[test]
    fn more_than_one_way_to_open_a_backup_is_refused() {
        let args = SecretArgs {
            key_file: Some(PathBuf::from("/nonexistent/key")),
            passphrase_env: true,
            ..none()
        };
        let err = secret(&args, &HqConfig::default())
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("one way"), "{err}");
    }

    #[test]
    fn the_configured_key_file_is_the_default_and_must_still_be_private() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("k.key");
        Secret::generate_key_file(&key).unwrap();
        let mut config = HqConfig::default();
        config.backup.key_file = Some(key.clone());
        assert!(secret(&none(), &config).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(secret(&none(), &config).is_err());
        }
    }

    #[test]
    fn a_passphrase_file_uses_its_first_line_only() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("p.txt");
        std::fs::write(&file, "a long enough passphrase\nsecond line\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let args = SecretArgs {
            passphrase_file: Some(file),
            ..none()
        };
        assert!(secret(&args, &HqConfig::default()).is_ok());
    }
}
