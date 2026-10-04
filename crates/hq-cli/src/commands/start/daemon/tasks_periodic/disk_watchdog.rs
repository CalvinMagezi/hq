//! Disk/build-artifact threshold watchdog — replaces OpenClaw's
//! `disk-watchdog.sh` cron job (retired 2026-08-10).
//!
//! The original ran the same four checks inside an OpenClaw `agentTurn`
//! wrapper (LLM decides whether to relay the script's exit code to
//! Telegram), which is where its observed `status=error` runs came from.
//! This is pure deterministic Rust: run the checks, and if any threshold is
//! breached, send the report straight to Telegram — no LLM turn in the
//! path. Detect-and-report only, same as the original; `cargo-gc.sh`
//! already handles `target/` cleanup on its own schedule.

use anyhow::Result;
use hq_core::config::HqConfig;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

const FIND_MAX_DEPTH_NODE_MODULES: &str = "4";
const FIND_MAX_DEPTH_TARGET: &str = "2";
const DU_TIMEOUT: Duration = Duration::from_secs(30);

fn human_mb(mb: u64) -> String {
    if mb >= 1024 {
        format!("{:.1}GB", mb as f64 / 1024.0)
    } else {
        format!("{mb}MB")
    }
}

/// (used_pct, available_gb), preferring the macOS data volume over the
/// system volume — that's where user data (and this repo) actually lives.
/// No equivalent split exists on Linux, so that platform goes straight to `/`.
async fn overall_disk_usage() -> Option<(u8, f64)> {
    #[cfg(target_os = "macos")]
    let out = match Command::new("df")
        .args(["-k", "/System/Volumes/Data"])
        .output()
        .await
    {
        Ok(o) if o.status.success() => o,
        _ => Command::new("df").args(["-k", "/"]).output().await.ok()?,
    };
    #[cfg(not(target_os = "macos"))]
    let out = Command::new("df").args(["-k", "/"]).output().await.ok()?;

    let text = String::from_utf8_lossy(&out.stdout);
    let last = text.lines().last()?;
    let fields: Vec<&str> = last.split_whitespace().collect();
    let avail_kb: f64 = fields.get(3)?.parse().ok()?;
    let used_pct: u8 = fields.get(4)?.trim_end_matches('%').parse().ok()?;
    Some((used_pct, avail_kb / 1_048_576.0))
}

async fn dir_size_mb(path: &Path) -> Option<u64> {
    let run = Command::new("du")
        .args(["-sm", &path.display().to_string()])
        .output();
    let out = tokio::time::timeout(DU_TIMEOUT, run).await.ok()?.ok()?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

async fn find_dirs(root: &Path, max_depth: &str, name: &str) -> Vec<PathBuf> {
    let run = Command::new("find")
        .arg(root)
        .args(["-maxdepth", max_depth, "-type", "d", "-name", name])
        .output();
    let Ok(Ok(out)) = tokio::time::timeout(DU_TIMEOUT, run).await else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(PathBuf::from)
        .collect()
}

async fn check_bloat_dirs(
    root: &Path,
    max_depth: &str,
    name: &str,
    threshold_mb: u64,
    suffix: &str,
    report: &mut Vec<String>,
) {
    for dir in find_dirs(root, max_depth, name).await {
        let Some(mb) = dir_size_mb(&dir).await else {
            continue;
        };
        if mb >= threshold_mb {
            report.push(format!(
                "{} is {} (threshold {}){suffix}",
                dir.display(),
                human_mb(mb),
                human_mb(threshold_mb)
            ));
        }
    }
}

/// The repo agent-hq's own vault lives directly under, used as the default
/// watch root when the config doesn't name any — more portable than the
/// original script's hardcoded `~/Documents/GitHub/agent-hq`.
fn default_watch_root(vault_path: &Path) -> Option<PathBuf> {
    vault_path.parent().map(PathBuf::from)
}

pub async fn run_disk_watchdog(vault_path: &Path, config: &HqConfig) -> Result<()> {
    let cfg = &config.disk_watchdog;
    if !cfg.enabled {
        return Ok(());
    }

    let roots: Vec<PathBuf> = if cfg.watch_roots.is_empty() {
        default_watch_root(vault_path).into_iter().collect()
    } else {
        cfg.watch_roots.iter().map(PathBuf::from).collect()
    };

    let mut report = Vec::new();

    let disk = overall_disk_usage().await;
    if let Some((used_pct, avail_gb)) = disk
        && used_pct >= cfg.disk_pct_threshold
    {
        report.push(format!(
            "Disk {used_pct}% used (threshold {}%), only {avail_gb:.1}GB free",
            cfg.disk_pct_threshold
        ));
    }

    let bun_cache = dirs::home_dir().map(|h| h.join(".bun/install/cache"));
    if let Some(bun_cache) = bun_cache.filter(|p| p.exists())
        && let Some(mb) = dir_size_mb(&bun_cache).await
        && mb >= cfg.bun_cache_threshold_mb
    {
        report.push(format!(
            "~/.bun/install/cache is {} (threshold {})",
            human_mb(mb),
            human_mb(cfg.bun_cache_threshold_mb)
        ));
    }

    for root in &roots {
        if !root.exists() {
            continue;
        }
        check_bloat_dirs(
            root,
            FIND_MAX_DEPTH_NODE_MODULES,
            "node_modules",
            cfg.node_modules_threshold_mb,
            "",
            &mut report,
        )
        .await;
        check_bloat_dirs(
            root,
            FIND_MAX_DEPTH_TARGET,
            "target",
            cfg.cargo_target_threshold_mb,
            " — cargo-gc.sh may need a manual run",
            &mut report,
        )
        .await;
    }

    if report.is_empty() {
        tracing::info!("disk-watchdog: all clear");
        return Ok(());
    }

    tracing::warn!(
        breaches = report.len(),
        "disk-watchdog: threshold(s) breached"
    );

    let Some(token) = config
        .relay
        .notifications_token
        .as_deref()
        .or(config.relay.telegram_token.as_deref())
    else {
        return Ok(());
    };
    let Some(chat) = super::super::helpers::telegram_notification_chat_id(vault_path) else {
        return Ok(());
    };
    let text = format!(
        "Disk watchdog: threshold(s) breached\n\n{}",
        report.join("\n")
    );
    #[cfg(feature = "telegram")]
    if let Err(e) = hq_relay::telegram::send_message(token, chat, &text).await {
        tracing::warn!(error = %e, "disk-watchdog: telegram delivery failed");
    }
    #[cfg(not(feature = "telegram"))]
    {
        let _ = (token, chat, text);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_mb_switches_units_at_1024() {
        assert_eq!(human_mb(512), "512MB");
        assert_eq!(human_mb(1024), "1.0GB");
        assert_eq!(human_mb(10240), "10.0GB");
    }

    #[test]
    fn default_watch_root_is_the_vault_parent() {
        let vault = Path::new("/home/alice/Documents/GitHub/agent-hq/.vault");
        assert_eq!(
            default_watch_root(vault),
            Some(PathBuf::from(
                "/home/alice/Documents/GitHub/agent-hq"
            ))
        );
    }

    #[tokio::test]
    async fn a_disabled_watchdog_is_a_silent_no_op() {
        let mut config = HqConfig::default();
        config.disk_watchdog.enabled = false;
        let vault = std::env::temp_dir();
        assert!(run_disk_watchdog(&vault, &config).await.is_ok());
    }
}
