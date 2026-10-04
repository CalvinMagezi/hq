//! OS-aware daemon restart helper, shared by self-update's install and
//! rollback paths (`hq-tools::self_update`, `hq-cli::self_apply`).

/// Outcome of attempting to restart the running HQ daemon process after a
/// binary swap.
#[derive(Debug, Clone)]
pub enum RestartOutcome {
    /// Restart command ran and reported success.
    Restarted,
    /// No restart was attempted — the configured label was empty.
    SkippedNoLabel,
    /// This platform has no restart mechanism wired up here; the caller
    /// must restart the service manually.
    Unsupported { message: String },
    /// A restart mechanism exists but the command failed.
    Failed { message: String },
}

/// Restart the daemon after a self-update binary swap.
///
/// macOS uses launchd (`label` is the launchd job label passed to
/// `launchctl kickstart`). Linux has no equivalent config today: a
/// `hq.service`-style system unit usually isn't restartable by the
/// unprivileged user self-update runs as (no sudo, no systemd `--user`
/// mapping to a root-owned unit), so guessing a unit name and failing
/// silently would be worse than just reporting `Unsupported` and telling
/// the operator to restart it themselves.
pub async fn restart_daemon(label: &str) -> RestartOutcome {
    if label.is_empty() {
        return RestartOutcome::SkippedNoLabel;
    }

    #[cfg(target_os = "macos")]
    {
        let uid = match tokio::process::Command::new("id").arg("-u").output().await {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            Ok(o) => {
                return RestartOutcome::Failed {
                    message: format!(
                        "could not resolve uid: {}",
                        String::from_utf8_lossy(&o.stderr)
                    ),
                };
            }
            Err(e) => return RestartOutcome::Failed {
                message: format!("could not resolve uid for launchctl kickstart: {e}"),
            },
        };
        let target = format!("gui/{uid}/{label}");
        match tokio::process::Command::new("launchctl")
            .args(["kickstart", "-k", &target])
            .output()
            .await
        {
            Ok(o) if o.status.success() => RestartOutcome::Restarted,
            Ok(o) => RestartOutcome::Failed {
                message: format!(
                    "launchctl kickstart {target} failed: {}",
                    String::from_utf8_lossy(&o.stderr)
                ),
            },
            Err(e) => RestartOutcome::Failed {
                message: format!("launchctl kickstart {target}: {e}"),
            },
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        RestartOutcome::Unsupported {
            message: format!(
                "no restart mechanism configured for this platform (label {label:?}) — \
                 restart the service manually, e.g. `systemctl restart hq` (system unit) or \
                 `systemctl --user restart <unit>` (see `hq service install`)"
            ),
        }
    }
}
