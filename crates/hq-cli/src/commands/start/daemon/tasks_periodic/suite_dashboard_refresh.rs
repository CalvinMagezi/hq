//! `suite-dashboard-refresh`: a scheduled PDF and PNG snapshot of each dashboard note listed in
//! `suite.dashboard_refresh.notes`, written to `Exports/<yyyy-mm>/` with the date in the name.
//!
//! Off by default (`suite.dashboard_refresh.enabled`). It sends nothing anywhere: the files only
//! land in the vault, and handing one to a chat stays an explicit act (`file_deliver`). One line
//! goes to the daemon log per run. The daemon wakes this task every few minutes; it acts when
//! `every_minutes` have passed since the last run, which is kept in `_data/suite/` in the vault.

use anyhow::Result;
use hq_core::config::HqConfig;
use std::path::Path;
#[cfg(feature = "suite")]
use tracing::{info, warn};

#[cfg(feature = "suite")]
use hq_core::config::{DASHBOARD_REFRESH_MAX_NOTES, DASHBOARD_REFRESH_MIN_MINUTES};

/// Where the time of the last run is kept, relative to the vault.
#[cfg(feature = "suite")]
const LAST_RUN_REL: &str = "_data/suite/dashboard-refresh.last";

#[cfg(feature = "suite")]
pub async fn run_suite_dashboard_refresh(vault_path: &Path, config: &HqConfig) -> Result<()> {
    let c = &config.suite.dashboard_refresh;
    if !c.enabled || c.notes.is_empty() {
        return Ok(());
    }
    let now = chrono::Local::now();
    let every = i64::from(c.every_minutes.max(DASHBOARD_REFRESH_MIN_MINUTES));
    if !is_due(vault_path, now.timestamp(), every * 60) {
        return Ok(());
    }
    let notes: Vec<String> = c
        .notes
        .iter()
        .take(DASHBOARD_REFRESH_MAX_NOTES)
        .cloned()
        .collect();
    // A day-long cadence names files by date; a shorter one adds the time so runs do not collide.
    let stamp = if every >= 1440 {
        now.format("%Y-%m-%d").to_string()
    } else {
        now.format("%Y-%m-%d-%H%M").to_string()
    };
    let line = refresh_notes(vault_path, &notes, &stamp).await;
    // Recorded even when a note failed, so a broken note is not retried every few minutes.
    mark_run(vault_path, now.timestamp());
    if line.failed > 0 {
        warn!("suite-dashboard-refresh: {}", line.text);
    } else {
        info!("suite-dashboard-refresh: {}", line.text);
    }
    Ok(())
}

#[cfg(not(feature = "suite"))]
pub async fn run_suite_dashboard_refresh(_vault_path: &Path, _config: &HqConfig) -> Result<()> {
    Ok(())
}

#[cfg(feature = "suite")]
fn is_due(vault: &Path, now: i64, every_secs: i64) -> bool {
    std::fs::read_to_string(vault.join(LAST_RUN_REL))
        .ok()
        .and_then(|t| t.trim().parse::<i64>().ok())
        .is_none_or(|last| now - last >= every_secs)
}

#[cfg(feature = "suite")]
fn mark_run(vault: &Path, now: i64) {
    let file = vault.join(LAST_RUN_REL);
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&file, now.to_string()) {
        warn!("suite-dashboard-refresh: cannot record the run time: {e}");
    }
}

#[cfg(feature = "suite")]
struct RefreshLine {
    text: String,
    failed: usize,
}

/// Snapshots every note as PDF and PNG and says what happened in one line.
#[cfg(feature = "suite")]
async fn refresh_notes(vault: &Path, notes: &[String], stamp: &str) -> RefreshLine {
    let (mut files, mut failed) = (0usize, 0usize);
    let mut problems: Vec<String> = Vec::new();
    for note in notes {
        match snapshot_note(vault, note, stamp).await {
            Ok(n) => files += n,
            Err(e) => {
                failed += 1;
                problems.push(format!("{note}: {e:#}"));
            }
        }
    }
    let mut text = format!(
        "{} of {} notes snapshotted ({files} files in Exports)",
        notes.len() - failed,
        notes.len()
    );
    if !problems.is_empty() {
        text.push_str(&format!("; failed: {}", problems.join("; ")));
    }
    RefreshLine { text, failed }
}

/// One note as `Exports/<yyyy-mm>/<name>-<stamp>.pdf` and `.png`. Returns how many files were made.
#[cfg(feature = "suite")]
async fn snapshot_note(vault: &Path, note_rel: &str, stamp: &str) -> Result<usize> {
    use anyhow::Context;
    let resolved = hq_convert::note_pdf::resolve_note(vault, note_rel)
        .with_context(|| "the note is not in the vault".to_string())?;
    let (v, rel) = (vault.to_path_buf(), note_rel.to_string());
    let prepared =
        tokio::task::spawn_blocking(move || hq_suite_adapter::snapshot::prepare(&v, &rel))
            .await??;
    let dir = vault.join(hq_tool_api::files::exports_month_dir());
    std::fs::create_dir_all(&dir)?;
    let stem = Path::new(note_rel)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("dashboard");
    let stem = format!("{}-{stamp}", hq_tool_api::files::clean_stem(stem));
    let mut made = 0;
    for (format, ext) in [
        (hq_export::Format::Pdf, "pdf"),
        (hq_export::Format::Png, "png"),
    ] {
        let (v, note, text) = (vault.to_path_buf(), resolved.clone(), prepared.text.clone());
        let done = tokio::task::spawn_blocking(move || {
            hq_export::export_note_text(&v, &note, &text, format, None, &[])
        })
        .await?
        .map_err(|e| anyhow::anyhow!("{ext} snapshot failed: {e}"))?;
        std::fs::write(
            hq_tool_api::files::unique_path(&dir, &stem, ext),
            &done.output.bytes,
        )?;
        made += 1;
    }
    Ok(made)
}

#[cfg(all(test, feature = "suite"))]
mod tests {
    use super::*;

    const DASH: &str = "---\nsuite_kind: dashboard\n---\n# Sales\n\n```sheet name=Stock types=\"text,int\"\n| item | qty |\n|---|---|\n| Sugar | 40 |\n| Salt | 25 |\n```\n\n```tile from=\"B2:B3\" fn=sum label=Units\n```\n";

    fn vault() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("Notebooks")).unwrap();
        std::fs::write(d.path().join("Notebooks/Sales.md"), DASH).unwrap();
        d
    }

    #[test]
    fn it_acts_only_when_the_cadence_has_passed() {
        let d = tempfile::tempdir().unwrap();
        assert!(is_due(d.path(), 1_000_000, 600), "never run: due");
        mark_run(d.path(), 1_000_000);
        assert!(!is_due(d.path(), 1_000_599, 600));
        assert!(is_due(d.path(), 1_000_600, 600));
    }

    #[tokio::test]
    async fn a_listed_note_is_snapshotted_as_pdf_and_png_and_a_bad_one_is_named() {
        let d = vault();
        let notes = vec!["Notebooks/Sales.md".to_string(), "Notebooks/Gone.md".into()];
        let line = refresh_notes(d.path(), &notes, "2026-11-20").await;
        assert_eq!(line.failed, 1, "{}", line.text);
        assert!(
            line.text.starts_with("1 of 2 notes snapshotted (2 files")
                && line.text.contains("Notebooks/Gone.md"),
            "{}",
            line.text
        );
        let dir = d.path().join(hq_tool_api::files::exports_month_dir());
        for name in ["Sales-2026-11-20.pdf", "Sales-2026-11-20.png"] {
            let len = std::fs::metadata(dir.join(name)).unwrap().len();
            assert!(len > 100, "{name} is {len} bytes");
        }
        // A second run the same day keeps the first files.
        refresh_notes(d.path(), &notes[..1], "2026-11-20").await;
        assert!(dir.join("Sales-2026-11-20-2.pdf").exists());
    }

    #[tokio::test]
    async fn the_task_does_nothing_while_it_is_off_or_has_no_notes() {
        let d = vault();
        let mut config = HqConfig::default();
        run_suite_dashboard_refresh(d.path(), &config)
            .await
            .unwrap();
        config.suite.dashboard_refresh.enabled = true;
        run_suite_dashboard_refresh(d.path(), &config)
            .await
            .unwrap();
        assert!(!d.path().join("Exports").exists());
        config.suite.dashboard_refresh.notes = vec!["Notebooks/Sales.md".into()];
        run_suite_dashboard_refresh(d.path(), &config)
            .await
            .unwrap();
        assert!(d.path().join(LAST_RUN_REL).exists());
        assert!(d.path().join("Exports").exists());
        // Just ran: the next wake finds it not due.
        let before = std::fs::read_to_string(d.path().join(LAST_RUN_REL)).unwrap();
        run_suite_dashboard_refresh(d.path(), &config)
            .await
            .unwrap();
        assert_eq!(
            before,
            std::fs::read_to_string(d.path().join(LAST_RUN_REL)).unwrap()
        );
    }
}
