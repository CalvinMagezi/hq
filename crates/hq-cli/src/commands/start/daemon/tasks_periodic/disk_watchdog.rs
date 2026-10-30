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
/// original script's hardcoded `~/Documents/GitHub/agent-hq`. A vault directly in
/// the home directory (`~/.vault`) has no repo around it, and walking home would
/// reach Music, Photos and Desktop, which macOS asks about, so there is no default then.
fn default_watch_root(vault_path: &Path) -> Option<PathBuf> {
    let parent = vault_path.parent()?;
    let is_home = dirs::home_dir().is_some_and(|home| home == parent);
    (!is_home && parent.parent().is_some()).then(|| parent.to_path_buf())
}

/// Inbox source of the breach report.
const DISK_WATCHDOG_SOURCE: &str = "disk-watchdog";

pub async fn run_disk_watchdog(
    vault_path: &Path,
    db: &hq_db::Database,
    config: &HqConfig,
) -> Result<()> {
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

    let item = hq_core::types::ValueItem::new(
        DISK_WATCHDOG_SOURCE,
        hq_core::types::ValueKind::ActionNeeded,
        "Disk watchdog: threshold(s) breached",
        report.join("\n"),
    )
    .with_dedup_key(format!(
        "disk-watchdog-{}",
        chrono::Utc::now().format("%Y-%m-%d")
    ));
    if let Err(e) = hq_db::value_items::emit(db, &item) {
        tracing::warn!(error = %e, "disk-watchdog: inbox item failed");
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
            Some(PathBuf::from("/home/alice/Documents/GitHub/agent-hq"))
        );
    }

    #[test]
    fn a_vault_in_the_home_directory_or_at_the_root_has_no_default_root() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(default_watch_root(&home.join(".vault")), None);
        assert_eq!(default_watch_root(Path::new("/.vault")), None);
    }

    #[tokio::test]
    async fn a_disabled_watchdog_is_a_silent_no_op() {
        let mut config = HqConfig::default();
        config.disk_watchdog.enabled = false;
        let vault = std::env::temp_dir();
        let db = hq_db::Database::open_memory().unwrap();
        assert!(run_disk_watchdog(&vault, &db, &config).await.is_ok());
    }
}
