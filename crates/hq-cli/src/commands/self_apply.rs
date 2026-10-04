//! Detached applier for `self_update_install`.
//!
//! A running binary cannot safely replace itself, so the install tool spawns
//! `hq self-apply --run-id N` in its own process group. This process waits for
//! its parent to exit, swaps the freshly built binary into every install path,
//! kickstarts the daemon, health-probes the result, restores the rollback
//! snapshot on a broken binary, and reports the outcome through the value bus
//! (which the relay delivers to Telegram/Discord).

use anyhow::{Context, Result};
use hq_core::config::HqConfig;
use hq_core::types::{ValueItem, ValueKind};
use std::path::{Path, PathBuf};
use std::time::Duration;

const PARENT_WAIT_SECS: u64 = 30;
const DAEMON_PROBE_SECS: u64 = 15;

fn parent_exited() -> bool {
    std::os::unix::process::parent_id() == 1
}

async fn wait_for_parent_exit() {
    for _ in 0..(PARENT_WAIT_SECS * 2) {
        if parent_exited() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Tree hash of the committed branch, which equals the tree the owner approved.
fn git_tree(repo: &Path) -> Result<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD^{tree}"])
        .current_dir(repo)
        .output()
        .context("run git")?;
    anyhow::ensure!(out.status.success(), "git rev-parse failed");
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn copy_to_install_paths(src: &Path, dests: &[PathBuf]) -> (Vec<String>, Vec<String>) {
    let mut ok = Vec::new();
    let mut failed = Vec::new();
    for dest in dests {
        match std::fs::copy(src, dest) {
            Ok(_) => ok.push(dest.to_string_lossy().to_string()),
            Err(e) => failed.push(format!("{}: {e}", dest.display())),
        }
    }
    (ok, failed)
}

fn binary_version(bin: &Path) -> Option<String> {
    let out = std::process::Command::new(bin)
        .arg("--version")
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

async fn daemon_reachable(port: u16) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(DAEMON_PROBE_SECS);
    let addr = format!("127.0.0.1:{port}");
    while std::time::Instant::now() < deadline {
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

// FR-001 criterion 4: `dedup_key` must be stable per logical event, not
// embed anything that changes across a retry of the same run_id, or
// dismissing this from the queue never sticks.
fn notify(config: &HqConfig, kind: ValueKind, title: &str, body: &str, dedup_key: &str) {
    let item = ValueItem::new("self-update", kind, title, body).with_dedup_key(dedup_key);
    if let Err(e) = hq_db::value_items::emit_at(&config.vault_path, &item) {
        eprintln!("self-apply: failed to emit value item: {e}");
    }
}

/// A binary swap that doesn't also restart the process leaves the old
/// binary running — surface that instead of swallowing it, so an operator
/// finds out from a notification rather than by noticing the version never
/// changed.
fn warn_on_restart_failure(
    config: &HqConfig,
    run_id: i64,
    outcome: &hq_core::daemon_restart::RestartOutcome,
) {
    use hq_core::daemon_restart::RestartOutcome;
    let message = match outcome {
        RestartOutcome::Restarted | RestartOutcome::SkippedNoLabel => return,
        RestartOutcome::Unsupported { message } | RestartOutcome::Failed { message } => message,
    };
    notify(
        config,
        ValueKind::ActionNeeded,
        &format!("Self-update #{run_id} binary swapped, daemon not restarted"),
        message,
        &format!("self-apply-{run_id}-restart-failed"),
    );
}

/// Refuse anything but a run the owner approved, so running this command by
/// hand (or from an agent's shell) cannot swap the binary.
fn ensure_applicable(config: &HqConfig, run: &hq_db::self_update_runs::SelfUpdateRun) -> Result<()> {
    if !config.self_update.enabled {
        anyhow::bail!("self_update is disabled in config");
    }
    if run.status != hq_db::self_update_runs::STATUS_APPROVED {
        anyhow::bail!(
            "run #{} is '{}', not '{}': self-apply only installs a run the owner approved",
            run.id,
            run.status,
            hq_db::self_update_runs::STATUS_APPROVED
        );
    }
    Ok(())
}

/// The approval must carry a valid signature from the install key (a bare
/// database write cannot make one) and name the exact binary about to be
/// installed. Checked immediately before the swap.
fn verify_approval(
    run: &hq_db::self_update_runs::SelfUpdateRun,
    built: &Path,
    key: &[u8],
    tree: &str,
) -> Result<()> {
    let (Some(approved_sha), Some(mac)) = (&run.approved_binary_sha256, &run.approval_mac) else {
        anyhow::bail!("run #{} has no signed approval", run.id);
    };
    let message = hq_core::approval_key::self_update_message(run.id, tree, approved_sha);
    if !hq_core::approval_key::verify_hex(key, &message, mac) {
        anyhow::bail!("run #{}: approval signature is invalid", run.id);
    }
    let actual = hq_core::approval_key::file_sha256(built)?;
    if &actual != approved_sha {
        anyhow::bail!(
            "run #{}: {} changed after approval (sha256 {actual}, approved {approved_sha})",
            run.id,
            built.display()
        );
    }
    Ok(())
}

pub async fn run(config: &HqConfig, run_id: i64) -> Result<()> {
    let db = hq_db::Database::open(&config.db_path()).context("open vault db")?;
    let run = db
        .with_conn(move |c| hq_db::self_update_runs::get(c, run_id))?
        .ok_or_else(|| anyhow::anyhow!("no self-update run #{run_id}"))?;
    ensure_applicable(config, &run)?;

    let repo = config.self_update.resolve_repo_path(&config.vault_path);
    let built = repo.join("target/release/hq");
    if !built.is_file() {
        anyhow::bail!("release binary {} not found", built.display());
    }

    let tree = git_tree(&repo)?;
    let key = hq_core::approval_key::load_or_create_key()?;
    if let Err(e) = verify_approval(&run, &built, &key, &tree) {
        notify(
            config,
            ValueKind::ActionNeeded,
            &format!("Self-update #{run_id} refused"),
            &e.to_string(),
            &format!("self-apply-{run_id}-refused"),
        );
        return Err(e);
    }

    wait_for_parent_exit().await;

    let install_paths = &config.self_update.install_paths;
    let (installed, failed) = copy_to_install_paths(&built, install_paths);
    if installed.is_empty() {
        db.with_conn(move |c| {
            hq_db::self_update_runs::set_status(c, run_id, hq_db::self_update_runs::STATUS_FAILED)
        })?;
        notify(
            config,
            ValueKind::ActionNeeded,
            &format!("Self-update #{run_id} install failed"),
            &format!(
                "Could not write any install path: {}. The old binary is untouched.",
                failed.join("; ")
            ),
            &format!("self-apply-{run_id}-install-failed"),
        );
        return Ok(());
    }

    let first_restart = hq_core::daemon_restart::restart_daemon(&config.self_update.launchd_label).await;
    warn_on_restart_failure(config, run_id, &first_restart);

    // A binary that cannot report its version is broken: restore the snapshot.
    let primary = PathBuf::from(&installed[0]);
    let version = binary_version(&primary);
    if version.is_none() {
        let mut restored = Vec::new();
        if let Some(ref snapshot) = run.prev_binary_path {
            let snapshot = PathBuf::from(snapshot);
            if snapshot.is_file() {
                let (ok, _) = copy_to_install_paths(&snapshot, install_paths);
                restored = ok;
            }
        }
        let rollback_restart =
            hq_core::daemon_restart::restart_daemon(&config.self_update.launchd_label).await;
        warn_on_restart_failure(config, run_id, &rollback_restart);
        db.with_conn(move |c| {
            hq_db::self_update_runs::set_status(
                c,
                run_id,
                hq_db::self_update_runs::STATUS_ROLLED_BACK,
            )
        })?;
        notify(
            config,
            ValueKind::ActionNeeded,
            &format!("Self-update #{run_id} auto-rolled back"),
            &format!(
                "New binary failed its version probe. Restored snapshot to: {}. Branch {} is preserved for inspection.",
                restored.join(", "),
                run.branch
            ),
            &format!("self-apply-{run_id}-rolled-back"),
        );
        return Ok(());
    }

    let daemon_ok = daemon_reachable(config.ws_port).await;
    db.with_conn(move |c| {
        hq_db::self_update_runs::set_status(c, run_id, hq_db::self_update_runs::STATUS_INSTALLED)
    })?;
    notify(
        config,
        ValueKind::Fyi,
        &format!("Self-update #{run_id} installed"),
        &format!(
            "{} now running on branch {} ({}). Installed to: {}.{}{}",
            version.unwrap_or_default(),
            run.branch,
            run.description,
            installed.join(", "),
            if failed.is_empty() {
                String::new()
            } else {
                format!(" Skipped (permission?): {}.", failed.join("; "))
            },
            if daemon_ok {
                " Daemon healthy."
            } else {
                " Daemon port not reachable; check `hq status` if it should be running."
            }
        ),
        &format!("self-apply-{run_id}-installed"),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::self_update_runs::*;

    fn run_with(status: &str) -> SelfUpdateRun {
        SelfUpdateRun {
            id: 1,
            description: "d".into(),
            branch: "self/x".into(),
            base_rev: "abc".into(),
            prev_binary_path: None,
            status: status.into(),
            test_output_tail: None,
            created_at: String::new(),
            installed_at: None,
            approved_binary_sha256: None,
            approval_mac: None,
        }
    }

    fn enabled() -> HqConfig {
        let mut config = HqConfig::default();
        config.self_update.enabled = true;
        config
    }

    #[test]
    fn only_an_approved_run_is_applied() {
        assert!(ensure_applicable(&enabled(), &run_with(STATUS_APPROVED)).is_ok());
        for status in [STATUS_OPEN, STATUS_CHECKED, STATUS_INSTALLED, STATUS_ROLLED_BACK, STATUS_FAILED] {
            assert!(ensure_applicable(&enabled(), &run_with(status)).is_err(), "{status}");
        }
    }

    #[test]
    fn nothing_is_applied_while_self_update_is_disabled() {
        assert!(ensure_applicable(&HqConfig::default(), &run_with(STATUS_APPROVED)).is_err());
    }

    fn approved_run(key: &[u8], bin: &Path) -> SelfUpdateRun {
        let sha = hq_core::approval_key::file_sha256(bin).unwrap();
        let mac = hq_core::approval_key::hmac_hex(
            key,
            &hq_core::approval_key::self_update_message(1, "tree", &sha),
        );
        SelfUpdateRun {
            approved_binary_sha256: Some(sha),
            approval_mac: Some(mac),
            ..run_with(STATUS_APPROVED)
        }
    }

    #[test]
    fn a_signed_approval_for_the_same_binary_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("hq");
        std::fs::write(&bin, "binary").unwrap();
        let run = approved_run(b"key", &bin);
        assert!(verify_approval(&run, &bin, b"key", "tree").is_ok());
    }

    #[test]
    fn a_forged_row_without_the_install_key_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("hq");
        std::fs::write(&bin, "binary").unwrap();
        let forged = approved_run(b"attacker-guess", &bin);
        assert!(verify_approval(&forged, &bin, b"real-key", "tree").is_err());
        assert!(verify_approval(&run_with(STATUS_APPROVED), &bin, b"real-key", "tree").is_err());
    }

    #[test]
    fn a_binary_swapped_after_approval_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("hq");
        std::fs::write(&bin, "binary").unwrap();
        let run = approved_run(b"key", &bin);
        std::fs::write(&bin, "swapped").unwrap();
        let err = verify_approval(&run, &bin, b"key", "tree").unwrap_err();
        assert!(err.to_string().contains("changed after approval"), "{err}");
    }

    #[test]
    fn a_different_tree_invalidates_the_signature() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("hq");
        std::fs::write(&bin, "binary").unwrap();
        let run = approved_run(b"key", &bin);
        assert!(verify_approval(&run, &bin, b"key", "other-tree").is_err());
    }
}
