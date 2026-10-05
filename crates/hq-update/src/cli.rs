//! `hq update` wiring: config, trusted key, real ports, exit codes.

use crate::config::{DEFAULT_CONF_PATH, DEFAULT_PUBKEY_PATH, UpdateConfig, resolve_public_key};
use crate::engine::{ApplyOptions, Engine, Outcome};
use crate::error::{Result, UpdateError};
use crate::manifest::parse_version;
use crate::ports::Restarter;
use crate::real::{HttpHealth, LaunchdRestarter, RealHost, ReqwestHttp, SystemdRestarter};
use crate::state::{Installed, Layout};
use crate::verify::parse_public_key;
use std::path::PathBuf;

pub const EXIT_UP_TO_DATE: i32 = 0;
pub const EXIT_ERROR: i32 = 1;
pub const EXIT_UPDATE_AVAILABLE: i32 = 10;
/// BSD `EX_TEMPFAIL`: another update holds the lock; the timer just retries.
pub const EXIT_LOCKED: i32 = 75;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Check,
    Apply,
    Rollback,
}

#[derive(Debug, Clone)]
pub struct CliArgs {
    pub mode: Mode,
    pub channel: Option<String>,
    pub pin: Option<String>,
    pub dry_run: bool,
    pub json: bool,
    pub force: bool,
    pub restore_db: bool,
    pub conf: Option<PathBuf>,
}

/// Identity of the running binary, supplied by the binary crate.
#[derive(Debug, Clone)]
pub struct BuildIdentity {
    pub version: String,
    pub git_sha: String,
    pub embedded_pubkey: Option<String>,
}

pub fn check_exit_code(report: &crate::engine::CheckReport) -> i32 {
    if report.update_available {
        EXIT_UPDATE_AVAILABLE
    } else {
        EXIT_UP_TO_DATE
    }
}

fn env_lookup(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

fn emit<T: serde::Serialize>(json: bool, value: &T, human: impl FnOnce() -> String) {
    if json {
        match serde_json::to_string(value) {
            Ok(text) => println!("{text}"),
            Err(e) => eprintln!("cannot encode result: {e}"),
        }
    } else {
        println!("{}", human());
    }
}

fn describe(outcome: &Outcome) -> String {
    match outcome {
        Outcome::UpToDate { current, note } => match note {
            Some(note) => format!("up to date at {current} ({note})"),
            None => format!("up to date at {current}"),
        },
        Outcome::Skipped { version, reason } => format!("skipped {version}: {reason}"),
        Outcome::DryRun { from, to } => format!("dry run: would update {from} -> {to}"),
        Outcome::Applied {
            from,
            to,
            upgrade_hook_ok,
            ..
        } => {
            let hook = if *upgrade_hook_ok {
                ""
            } else {
                " (hq install --upgrade failed, see logs)"
            };
            format!("updated {from} -> {to}{hook}")
        }
        Outcome::RolledBack {
            attempted,
            restored,
            db_restored,
            reason,
        } => format!(
            "update to {attempted} failed ({reason}); rolled back to {restored}{}",
            if *db_restored {
                " and restored the database snapshot"
            } else {
                ""
            }
        ),
        Outcome::ManualRollback {
            from,
            to,
            db_restored,
        } => format!(
            "rolled back {from} -> {to}{}",
            if *db_restored {
                " and restored the database snapshot"
            } else {
                ""
            }
        ),
    }
}

fn build_restarter(cfg: &UpdateConfig) -> Box<dyn Restarter> {
    if cfg.launchd_label.is_empty() {
        Box::new(SystemdRestarter {
            unit: cfg.service_unit.clone(),
        })
    } else {
        Box::new(LaunchdRestarter {
            label: cfg.launchd_label.clone(),
        })
    }
}

/// Bounds a whole run below the unit's start timeout. Cutting a run short is
/// safe: the in-progress journal lets the next run finish or undo it.
async fn within_deadline<T>(
    cfg: &UpdateConfig,
    fut: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    tokio::time::timeout(std::time::Duration::from_secs(cfg.deadline_secs), fut)
        .await
        .map_err(|_| {
            UpdateError::Other(anyhow::anyhow!(
                "update exceeded its {}s deadline; the next run will reconcile",
                cfg.deadline_secs
            ))
        })?
}

async fn run_inner(args: &CliArgs, ident: &BuildIdentity) -> Result<i32> {
    let conf_path = args
        .conf
        .clone()
        .or_else(|| env_lookup("HQ_UPDATE_CONF").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CONF_PATH));
    let mut cfg = UpdateConfig::load(&conf_path, &env_lookup)?;
    if let Some(channel) = &args.channel {
        cfg.channel = channel.clone();
        cfg.validate()?;
    }

    let key_text = resolve_public_key(
        &cfg,
        DEFAULT_PUBKEY_PATH.as_ref(),
        ident.embedded_pubkey.as_deref(),
    )?;
    let key = parse_public_key(&key_text)?;
    let version = parse_version("running version", &ident.version)?;

    let http = ReqwestHttp::new(crate::config::is_loopback_url(&cfg.base_url))?;
    let restarter = build_restarter(&cfg);
    let health = HttpHealth::new(&cfg.health_url)?;
    let host = RealHost::new(&cfg);
    let engine = Engine {
        cfg: &cfg,
        key,
        updater_version: version,
        current: Installed {
            version: ident.version.clone(),
            git_sha: ident.git_sha.clone(),
        },
        layout: Layout::from_config(&cfg),
        http: &http,
        restarter: restarter.as_ref(),
        health: &health,
        host: &host,
        platform: crate::manifest::host_platform(),
    };
    let pin = args.pin.as_deref();

    match args.mode {
        Mode::Check => {
            let report = engine.check(pin).await?;
            emit(args.json, &report, || {
                match (&report.available, report.update_available) {
                    (Some(v), true) => {
                        format!("{} -> {v} available on {}", report.current, report.channel)
                    }
                    _ => format!(
                        "{} is up to date on {}{}",
                        report.current,
                        report.channel,
                        report
                            .note
                            .as_deref()
                            .map(|n| format!(" ({n})"))
                            .unwrap_or_default()
                    ),
                }
            });
            Ok(check_exit_code(&report))
        }
        Mode::Apply => {
            let opts = ApplyOptions {
                pin: args.pin.clone(),
                dry_run: args.dry_run,
                force: args.force,
            };
            let outcome = within_deadline(&cfg, engine.apply(&opts)).await?;
            emit(args.json, &outcome, || describe(&outcome));
            Ok(if matches!(outcome, Outcome::RolledBack { .. }) {
                EXIT_ERROR
            } else {
                EXIT_UP_TO_DATE
            })
        }
        Mode::Rollback => {
            if args.dry_run {
                println!("dry run: would restore the previous binary and web files");
                return Ok(EXIT_UP_TO_DATE);
            }
            let outcome = within_deadline(&cfg, engine.rollback(args.restore_db)).await?;
            emit(args.json, &outcome, || describe(&outcome));
            Ok(EXIT_UP_TO_DATE)
        }
    }
}

/// Runs `hq update` and returns the process exit code.
pub async fn run(args: CliArgs, ident: BuildIdentity) -> i32 {
    match run_inner(&args, &ident).await {
        Ok(code) => code,
        Err(UpdateError::Locked) => {
            eprintln!("another update is already running");
            EXIT_LOCKED
        }
        Err(e) => {
            if args.json {
                println!(
                    "{}",
                    serde_json::json!({ "status": "error", "error": e.to_string() })
                );
            } else {
                eprintln!("hq update: {e}");
            }
            EXIT_ERROR
        }
    }
}
