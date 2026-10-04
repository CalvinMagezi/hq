//! Production implementations of the ports: reqwest, systemd/launchd,
//! unprivileged child processes and the SQLite vault.

use crate::config::UpdateConfig;
use crate::error::{Result, UpdateError};
use crate::ports::{Health, HealthInfo, Host, Http, Restarter};
use async_trait::async_trait;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(900);
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const VERSION_TIMEOUT: Duration = Duration::from_secs(15);
const NOTIFY_TIMEOUT: Duration = Duration::from_secs(30);
const UPGRADE_TIMEOUT: Duration = Duration::from_secs(300);
const SYSTEMCTL_TIMEOUT: Duration = Duration::from_secs(120);
const DB_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_REDIRECTS: usize = 5;
const CHILD_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

fn download_err(e: impl std::fmt::Display) -> UpdateError {
    UpdateError::Download(e.to_string())
}

fn is_loopback(host: Option<&str>) -> bool {
    matches!(host, Some("127.0.0.1" | "localhost" | "[::1]" | "::1"))
}

pub struct ReqwestHttp {
    client: reqwest::Client,
}

impl ReqwestHttp {
    /// `allow_loopback` permits http redirects to loopback hosts, which only
    /// makes sense when the configured release host is itself loopback (tests).
    pub fn new(allow_loopback: bool) -> Result<Self> {
        let policy = reqwest::redirect::Policy::custom(move |attempt| {
            let ok_scheme = attempt.url().scheme() == "https"
                || (allow_loopback && is_loopback(attempt.url().host_str()));
            if attempt.previous().len() >= MAX_REDIRECTS || !ok_scheme {
                attempt.error("redirect refused (too many hops or not https)")
            } else {
                attempt.follow()
            }
        });
        let client = reqwest::Client::builder()
            .user_agent(concat!("hq-update/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(TRANSFER_TIMEOUT)
            .redirect(policy)
            .build()
            .map_err(download_err)?;
        Ok(Self { client })
    }

    async fn open(&self, url: &str, max_bytes: u64) -> Result<reqwest::Response> {
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| download_err(format!("{url}: {e}")))?;
        if !resp.status().is_success() {
            return Err(download_err(format!("{url}: HTTP {}", resp.status())));
        }
        if resp.content_length().is_some_and(|len| len > max_bytes) {
            return Err(UpdateError::SizeLimit {
                name: url.to_string(),
                limit: max_bytes,
            });
        }
        Ok(resp)
    }
}

#[async_trait]
impl Http for ReqwestHttp {
    async fn get(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>> {
        let mut resp = self.open(url, max_bytes).await?;
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(download_err)? {
            if body.len() as u64 + chunk.len() as u64 > max_bytes {
                return Err(UpdateError::SizeLimit {
                    name: url.to_string(),
                    limit: max_bytes,
                });
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    async fn download(&self, url: &str, dest: &Path, max_bytes: u64) -> Result<()> {
        let mut resp = self.open(url, max_bytes).await?;
        let std_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(dest)?;
        let mut file = tokio::fs::File::from_std(std_file);
        let mut written = 0u64;
        let result: Result<()> = async {
            while let Some(chunk) = resp.chunk().await.map_err(download_err)? {
                written += chunk.len() as u64;
                if written > max_bytes {
                    return Err(UpdateError::SizeLimit {
                        name: url.to_string(),
                        limit: max_bytes,
                    });
                }
                file.write_all(&chunk).await?;
            }
            file.flush().await?;
            Ok(())
        }
        .await;
        if result.is_err() {
            let _ = std::fs::remove_file(dest);
        }
        result
    }
}

async fn run_status(mut cmd: Command, what: &str, limit: Duration) -> Result<()> {
    cmd.stdin(Stdio::null()).kill_on_drop(true);
    let out = tokio::time::timeout(limit, cmd.output())
        .await
        .map_err(|_| anyhow::anyhow!("{what} timed out"))?
        .map_err(|e| anyhow::anyhow!("{what}: {e}"))?;
    if !out.status.success() {
        return Err(anyhow::anyhow!(
            "{what} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )
        .into());
    }
    Ok(())
}

pub struct SystemdRestarter {
    pub unit: String,
}

#[async_trait]
impl Restarter for SystemdRestarter {
    async fn restart(&self) -> Result<()> {
        let mut cmd = Command::new("systemctl");
        cmd.args(["restart", &self.unit]);
        run_status(cmd, "systemctl restart", SYSTEMCTL_TIMEOUT).await
    }

    async fn stop(&self) -> Result<()> {
        let mut cmd = Command::new("systemctl");
        cmd.args(["stop", &self.unit]);
        run_status(cmd, "systemctl stop", SYSTEMCTL_TIMEOUT).await
    }
}

/// macOS: reuses hq-core's launchd kickstart. There is no clean "stop"
/// there, so a database restore on macOS happens against a still-running
/// daemon until a launchd stop path exists (see TECHDEBT.md).
pub struct LaunchdRestarter {
    pub label: String,
}

#[async_trait]
impl Restarter for LaunchdRestarter {
    async fn restart(&self) -> Result<()> {
        use hq_core::daemon_restart::RestartOutcome;
        match hq_core::daemon_restart::restart_daemon(&self.label).await {
            RestartOutcome::Restarted => Ok(()),
            RestartOutcome::SkippedNoLabel => {
                Err(anyhow::anyhow!("no launchd label configured").into())
            }
            RestartOutcome::Unsupported { message } | RestartOutcome::Failed { message } => {
                Err(anyhow::anyhow!(message).into())
            }
        }
    }

    async fn stop(&self) -> Result<()> {
        tracing::warn!("launchd stop is not implemented; continuing with a live daemon");
        Ok(())
    }
}

pub struct HttpHealth {
    client: reqwest::Client,
    url: String,
}

impl HttpHealth {
    pub fn new(url: &str) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(PROBE_TIMEOUT)
            .build()
            .map_err(download_err)?;
        Ok(Self {
            client,
            url: url.to_string(),
        })
    }
}

#[async_trait]
impl Health for HttpHealth {
    async fn probe(&self) -> Option<HealthInfo> {
        let resp = self.client.get(&self.url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let body: serde_json::Value = resp.json().await.ok()?;
        if body.get("status").and_then(|s| s.as_str()) != Some("ok") {
            return None;
        }
        let text = |key: &str| body.get(key).and_then(|v| v.as_str()).map(str::to_string);
        Some(HealthInfo {
            git_sha: text("git_sha"),
            version: text("version"),
        })
    }
}

/// Which of the extra confinement flags this host's `setpriv` understands.
/// Missing ones are logged and skipped rather than failing the update.
pub fn hardening_flags_from_help(help: &str) -> (Vec<&'static str>, Vec<&'static str>) {
    let wanted: [(&str, &str); 4] = [
        ("--setsid", "--setsid"),
        ("--bounding-set", "--bounding-set=-all"),
        ("--inh-caps", "--inh-caps=-all"),
        ("--no-new-privs", "--no-new-privs"),
    ];
    let (mut have, mut missing) = (Vec::new(), Vec::new());
    for (probe, flag) in wanted {
        if help.contains(probe) {
            have.push(flag);
        } else {
            missing.push(flag);
        }
    }
    (have, missing)
}

fn setpriv_hardening_flags(setpriv: &Path) -> Vec<&'static str> {
    static FLAGS: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    FLAGS
        .get_or_init(|| {
            let help = std::process::Command::new(setpriv)
                .arg("--help")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default();
            let (have, missing) = hardening_flags_from_help(&help);
            if !missing.is_empty() {
                tracing::warn!(
                    ?missing,
                    "this setpriv lacks some confinement flags; running without them"
                );
            }
            have
        })
        .clone()
}

pub struct RealHost {
    cfg: UpdateConfig,
}

impl RealHost {
    pub fn new(cfg: &UpdateConfig) -> Self {
        Self { cfg: cfg.clone() }
    }

    /// Builds a command that runs as the service user. As root it needs
    /// `setpriv` or `runuser`; with neither it refuses rather than run an
    /// artifact with full privileges.
    fn unprivileged(&self, program: &Path, args: &[&str]) -> Result<Command> {
        self.unprivileged_with_env(program, args, &[])
    }

    fn unprivileged_with_env(
        &self,
        program: &Path,
        args: &[&str],
        extra_env: &[(&str, &str)],
    ) -> Result<Command> {
        // SAFETY: geteuid has no preconditions.
        let is_root = unsafe { libc::geteuid() } == 0;
        let user = &self.cfg.run_as_user;
        let mut cmd = if !is_root {
            Command::new(program)
        } else if let Ok(setpriv) = which::which("setpriv") {
            let mut c = Command::new(&setpriv);
            c.args(["--reuid", user, "--regid", user, "--init-groups"]);
            c.args(setpriv_hardening_flags(&setpriv));
            c.arg("--").arg(program);
            c
        } else if let Ok(runuser) = which::which("runuser") {
            let mut c = Command::new(runuser);
            c.args(["-u", user, "--"]).arg(program);
            c
        } else {
            return Err(anyhow::anyhow!(
                "running as root but neither setpriv nor runuser is available to drop privileges"
            )
            .into());
        };
        let home = self.cfg.vault_path.parent().unwrap_or(Path::new("/"));
        cmd.args(args)
            .env_clear()
            .env("PATH", CHILD_PATH)
            .env("HOME", home)
            .env("HQ_VAULT_PATH", &self.cfg.vault_path)
            .env("HQ_CONFIG_PATH", &self.cfg.hq_config_path)
            .envs(extra_env.iter().copied())
            .stdin(Stdio::null())
            .kill_on_drop(true);
        Ok(cmd)
    }
}

#[async_trait]
impl Host for RealHost {
    async fn staged_version(&self, bin: &Path) -> Result<String> {
        let mut cmd = self.unprivileged(bin, &["--version"])?;
        let out = tokio::time::timeout(VERSION_TIMEOUT, cmd.output())
            .await
            .map_err(|_| anyhow::anyhow!("`--version` timed out"))?
            .map_err(|e| anyhow::anyhow!("cannot run staged binary: {e}"))?;
        // A failure to run is not proof the release is bad (the service user can
        // break exec on purpose), so it never blocks the release; only a clean
        // run that reports the wrong build does, in the engine.
        if !out.status.success() {
            return Err(anyhow::anyhow!("`--version` exited with {}", out.status).into());
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    async fn post_upgrade(&self) -> Result<()> {
        // The marker keeps `hq install --upgrade` from regenerating files that
        // depend on the service's own environment (it runs with a clean one).
        let cmd = self.unprivileged_with_env(
            &self.cfg.bin_path,
            &["install", "--upgrade"],
            &[("HQ_UPDATE_HOOK", "1")],
        )?;
        run_status(cmd, "hq install --upgrade", UPGRADE_TIMEOUT).await
    }

    async fn notify(&self, phase: &str, sha: &str) {
        let Ok(cmd) = self.unprivileged(
            &self.cfg.bin_path,
            &["notify-restart", phase, "--reason", "update", "--sha", sha],
        ) else {
            return;
        };
        if let Err(e) = run_status(cmd, "hq notify-restart", NOTIFY_TIMEOUT).await {
            tracing::warn!(error = %e, "update notification failed");
        }
    }

    async fn alert(&self, message: &str) {
        tracing::error!(%message, "update alert");
        let Ok(cmd) = self.unprivileged(
            &self.cfg.bin_path,
            &["notify-restart", "alert", "--reason", message, "--sha", "-"],
        ) else {
            return;
        };
        if let Err(e) = run_status(cmd, "hq notify-restart", NOTIFY_TIMEOUT).await {
            tracing::warn!(error = %e, "update alert delivery failed");
        }
    }

    async fn db_prune(
        &self,
        dir: &Path,
        keep: usize,
        protected: &[std::path::PathBuf],
    ) -> Result<()> {
        let (dir, keep) = (path_arg(dir)?, keep.to_string());
        let mut args = vec!["prune", dir.as_str(), keep.as_str()];
        let protected: Vec<String> = protected
            .iter()
            .filter_map(|p| p.to_str().map(str::to_string))
            .collect();
        args.extend(protected.iter().map(String::as_str));
        self.db_child(&args).await.map(|_| ())
    }

    async fn db_snapshot(&self, dest: &Path) -> Result<bool> {
        let (src, dest) = (path_arg(&self.cfg.vault_db)?, path_arg(dest)?);
        let out = self.db_child(&["snapshot", &src, &dest]).await?;
        Ok(out.trim() == "created")
    }

    async fn db_restore(&self, snapshot: &Path) -> Result<()> {
        let (db, snapshot) = (path_arg(&self.cfg.vault_db)?, path_arg(snapshot)?);
        self.db_child(&["restore", &db, &snapshot])
            .await
            .map(|_| ())
    }

    async fn db_migrations(&self) -> Result<Option<u64>> {
        let db = path_arg(&self.cfg.vault_db)?;
        let out = self.db_child(&["count", &db]).await?;
        Ok(out.trim().parse().ok())
    }
}

fn path_arg(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("path {} is not UTF-8", path.display()).into())
}

impl RealHost {
    /// Runs `hq update-db <args>` as the service user and returns its stdout.
    async fn db_child(&self, args: &[&str]) -> Result<String> {
        let mut full = vec!["update-db"];
        full.extend_from_slice(args);
        let mut cmd = self.unprivileged(&self.cfg.bin_path, &full)?;
        let out = tokio::time::timeout(DB_TIMEOUT, cmd.output())
            .await
            .map_err(|_| anyhow::anyhow!("hq update-db timed out"))?
            .map_err(|e| anyhow::anyhow!("hq update-db: {e}"))?;
        if !out.status.success() {
            return Err(anyhow::anyhow!(
                "hq update-db {} failed: {}",
                args.first().copied().unwrap_or(""),
                String::from_utf8_lossy(&out.stderr).trim()
            )
            .into());
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardening_flags_degrade_to_what_setpriv_supports() {
        let full = "--setsid --bounding-set <caps> --inh-caps <caps> --no-new-privs";
        let (have, missing) = hardening_flags_from_help(full);
        assert_eq!(
            have,
            [
                "--setsid",
                "--bounding-set=-all",
                "--inh-caps=-all",
                "--no-new-privs"
            ]
        );
        assert!(missing.is_empty());
        let (have, missing) = hardening_flags_from_help("--no-new-privs only");
        assert_eq!(have, ["--no-new-privs"]);
        assert_eq!(missing.len(), 3);
        assert!(hardening_flags_from_help("").0.is_empty());
    }
}
