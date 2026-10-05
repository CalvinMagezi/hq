//! The update flow: resolve and verify a release, stage it, swap it in,
//! restart, confirm health, and roll back on failure.

use crate::archive::{self, TreeLimits};
use crate::config::UpdateConfig;
use crate::error::{Result, UpdateError};
use crate::lock::UpdateLock;
use crate::manifest::{self, ChannelPointer, Manifest, SIGNATURE_SUFFIX};
use crate::ports::{Health, Host, Http, Restarter};
use crate::state::{InProgress, Installed, Layout, Slot, State};
use crate::{swap, verify};
use minisign_verify::PublicKey;
use semver::Version;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Files a web build must contain, mirroring the web deploy health check.
pub const WEB_REQUIRED_FILES: [&str; 3] = ["index.html", "sw.js", "manifest.json"];
const BINARY_FILE_NAME: &str = "hq";
const VERSION_SHA_PREFIX: usize = 7;
const SNAPSHOT_PREFIX: &str = "vault-";

#[derive(Debug, Clone, Default)]
pub struct ApplyOptions {
    pub pin: Option<String>,
    pub dry_run: bool,
    /// Reinstall the channel head even when it equals the running version.
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckReport {
    pub current: String,
    pub current_git_sha: String,
    pub channel: String,
    pub available: Option<String>,
    pub available_git_sha: Option<String>,
    pub update_available: bool,
    pub blocked: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    UpToDate {
        current: String,
        note: Option<String>,
    },
    Skipped {
        version: String,
        reason: String,
    },
    DryRun {
        from: String,
        to: String,
    },
    Applied {
        from: String,
        to: String,
        git_sha: String,
        upgrade_hook_ok: bool,
        warnings: Vec<String>,
    },
    RolledBack {
        attempted: String,
        restored: String,
        db_restored: bool,
        reason: String,
    },
    ManualRollback {
        from: String,
        to: String,
        db_restored: bool,
    },
}

pub struct Engine<'a> {
    pub cfg: &'a UpdateConfig,
    pub key: PublicKey,
    pub updater_version: Version,
    pub current: Installed,
    pub layout: Layout,
    pub http: &'a dyn Http,
    pub restarter: &'a dyn Restarter,
    pub health: &'a dyn Health,
    pub host: &'a dyn Host,
    /// `<os>-<arch>` of the running binary, see `manifest::host_platform`.
    pub platform: String,
}

struct Resolved {
    manifest: Manifest,
    base_url: String,
    /// `(seq, manifest_sha256)` of the accepted channel pointer, when it carried a seq.
    pointer_seq: Option<(u64, String)>,
    warnings: Vec<String>,
}

fn sha256_file(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn is_commit_sha(s: &str) -> bool {
    (VERSION_SHA_PREFIX..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Prefix comparison that tolerates short and long forms of one commit.
/// Never slices by byte offset: the health body is not trusted to be ASCII.
fn sha_matches(reported: &str, expected: &str) -> bool {
    if !is_commit_sha(reported) || !is_commit_sha(expected) {
        return false;
    }
    let n = reported.len().min(expected.len());
    match (reported.get(..n), expected.get(..n)) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => false,
    }
}

#[cfg(test)]
pub(crate) fn sha_matches_for_tests(a: &str, b: &str) -> bool {
    sha_matches(a, b)
}

/// `hq <version> (<sha> <time>)` from the first line of `--version`.
fn parse_version_line(out: &str) -> Option<Installed> {
    let mut tokens = out.lines().next()?.split_whitespace();
    if tokens.next()? != "hq" {
        return None;
    }
    let version = tokens.next()?.to_string();
    let git_sha = tokens
        .next()?
        .strip_prefix('(')?
        .trim_end_matches(')')
        .to_string();
    Some(Installed { version, git_sha })
}

struct WorkDir(PathBuf);
impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Engine<'_> {
    fn current_semver(&self) -> Result<Version> {
        manifest::parse_version("installed version", &self.current.version)
    }

    async fn fetch_verified(&self, url: &str, what: &str) -> Result<Vec<u8>> {
        let limit = self.cfg.limits.max_manifest_bytes;
        let body = self.http.get(url, limit).await?;
        let sig = self
            .http
            .get(&format!("{url}{SIGNATURE_SUFFIX}"), limit)
            .await
            .map_err(|e| UpdateError::Signature {
                what: what.to_string(),
                reason: format!("signature file unavailable: {e}"),
            })?;
        verify::verify(&self.key, what, &body, &sig)?;
        Ok(body)
    }

    async fn resolve(&self, pin: Option<&str>) -> Result<Resolved> {
        let mut warnings = Vec::new();
        let mut pointer_seq = None;
        let (manifest_url, expect_sha, expect_version) = match pin {
            Some(pin) => {
                tracing::warn!(
                    "--pin or a configured pin bypasses the channel pointer and its replay protection"
                );
                let version = manifest::parse_version("pin", pin)?.to_string();
                (self.cfg.release_manifest_url(&version), None, version)
            }
            None => {
                let url = self.cfg.channel_url(&self.cfg.channel);
                let bytes = self.fetch_verified(&url, "channel pointer").await?;
                let pointer = ChannelPointer::parse(&bytes)?;
                if pointer.channel != self.cfg.channel {
                    return Err(UpdateError::Invalid {
                        what: "channel pointer",
                        reason: format!(
                            "signed for channel `{}`, this host follows `{}`",
                            pointer.channel, self.cfg.channel
                        ),
                    });
                }
                let prefix = format!("{}/", self.cfg.download_base());
                if !pointer.manifest_url.starts_with(&prefix) {
                    return Err(UpdateError::Invalid {
                        what: "channel pointer",
                        reason: format!("manifest_url must live under {prefix}"),
                    });
                }
                if !manifest::is_clean_url_tail(&pointer.manifest_url[prefix.len()..]) {
                    return Err(UpdateError::Invalid {
                        what: "channel pointer",
                        reason:
                            "manifest_url has traversal, empty segments or other odd characters"
                                .into(),
                    });
                }
                self.check_pointer_freshness(&pointer, &mut warnings)?;
                pointer_seq = pointer.seq.map(|s| (s, pointer.manifest_sha256.clone()));
                let version =
                    manifest::parse_version("channel pointer", &pointer.version)?.to_string();
                (pointer.manifest_url, Some(pointer.manifest_sha256), version)
            }
        };
        let bytes = self.fetch_verified(&manifest_url, "manifest").await?;
        if let Some(expected) = expect_sha {
            let actual = sha256_bytes(&bytes);
            if actual != expected {
                return Err(UpdateError::ChecksumMismatch {
                    name: "manifest.json".into(),
                    expected,
                    actual,
                });
            }
        }
        let manifest = Manifest::parse(&bytes)?;
        if manifest.version != expect_version {
            return Err(UpdateError::Invalid {
                what: "manifest",
                reason: format!(
                    "version {} does not match the requested {expect_version}",
                    manifest.version
                ),
            });
        }
        let required = manifest::parse_version("manifest", &manifest.min_updater_version)?;
        if required > self.updater_version {
            return Err(UpdateError::UpdaterTooOld {
                version: manifest.version.clone(),
                required: manifest.min_updater_version.clone(),
                have: self.updater_version.to_string(),
            });
        }
        let base_url = manifest_url
            .rsplit_once('/')
            .map(|(base, _)| base.to_string())
            .unwrap_or_default();
        Ok(Resolved {
            manifest,
            base_url,
            pointer_seq,
            warnings,
        })
    }

    /// Refuses a replayed (lower `seq`, or same `seq` with different content)
    /// pointer and flags a stale one. A lost or corrupt `state.json` forgets
    /// the high-water mark, which is the same as a first run.
    fn check_pointer_freshness(
        &self,
        pointer: &ChannelPointer,
        warnings: &mut Vec<String>,
    ) -> Result<()> {
        let state = State::load(&self.layout)?;
        match (pointer.seq, state.seen_pointer(&pointer.channel)) {
            (Some(seq), Some(seen)) if seq < seen.seq => {
                return Err(UpdateError::Invalid {
                    what: "channel pointer",
                    reason: format!(
                        "seq {seq} is older than the {} already accepted for `{}`; refusing a replayed pointer",
                        seen.seq, pointer.channel
                    ),
                });
            }
            (Some(seq), Some(seen))
                if seq == seen.seq && seen.manifest_sha256 != pointer.manifest_sha256 =>
            {
                return Err(UpdateError::Invalid {
                    what: "channel pointer",
                    reason: format!("seq {seq} was already accepted with different content"),
                });
            }
            (None, Some(_)) => tracing::warn!(
                "channel pointer has no seq, so it cannot be ordered against a replay"
            ),
            _ => {}
        }
        if let Some(max_age) = self.cfg.max_pointer_age_secs {
            let age = pointer
                .issued_at
                .as_deref()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map(|t| chrono::Utc::now().signed_duration_since(t).num_seconds());
            match age {
                Some(age) if age > max_age as i64 => {
                    warnings.push(format!(
                        "the channel pointer was issued {age}s ago (limit {max_age}s); the channel may be stale or frozen"
                    ));
                }
                None => warnings
                    .push("the channel pointer carries no issued_at to check its age".into()),
                _ => {}
            }
        }
        Ok(())
    }

    fn remember_pointer(&self, state: &mut State, resolved: &Resolved) {
        if let Some((seq, sha)) = &resolved.pointer_seq
            && state.record_pointer(&self.cfg.channel, *seq, sha)
        {
            let _ = state.save(&self.layout);
        }
    }

    pub async fn check(&self, pin: Option<&str>) -> Result<CheckReport> {
        let pin = pin.or(self.cfg.pin.as_deref());
        let resolved = self.resolve(pin).await?;
        let state = State::load(&self.layout)?;
        let target = resolved.manifest.semver();
        let current = self.current_semver()?;
        let blocked = pin.is_none() && state.is_blocked(&resolved.manifest.version);
        let (update_available, note) = match (pin, target.cmp(&current)) {
            (Some(_), std::cmp::Ordering::Equal) => (false, None),
            (Some(_), _) => (true, None),
            (None, std::cmp::Ordering::Greater) if blocked => (
                false,
                Some("the newest release previously failed on this host".to_string()),
            ),
            (None, std::cmp::Ordering::Greater) => (true, None),
            (None, std::cmp::Ordering::Less) => (
                false,
                Some(format!(
                    "channel head {target} is older than installed {current}; downgrades need --pin"
                )),
            ),
            (None, std::cmp::Ordering::Equal) => (false, None),
        };
        let note = match (note, resolved.warnings.is_empty()) {
            (n, true) => n,
            (n, false) => Some(
                n.into_iter()
                    .chain(resolved.warnings.iter().cloned())
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
        };
        Ok(CheckReport {
            current: self.current.version.clone(),
            current_git_sha: self.current.git_sha.clone(),
            channel: self.cfg.channel.clone(),
            available: Some(resolved.manifest.version.clone()),
            available_git_sha: Some(resolved.manifest.git_sha.clone()),
            update_available,
            blocked,
            note,
        })
    }

    /// The macOS restarter cannot stop the service, so a swap or database
    /// restore there would happen under a live daemon.
    fn refuse_unmanaged_darwin(&self) -> Result<()> {
        if manifest::is_darwin(&self.platform) && !self.cfg.allow_darwin {
            return Err(UpdateError::DarwinNotAllowed);
        }
        Ok(())
    }

    pub async fn apply(&self, opts: &ApplyOptions) -> Result<Outcome> {
        if !opts.dry_run {
            self.refuse_unmanaged_darwin()?;
        }
        let explicit_pin = opts.pin.as_deref();
        let pin = explicit_pin.or(self.cfg.pin.as_deref());

        // The lock comes first so a process that resolved a stale view of the
        // install can never act on it (dry runs only read).
        let _lock = if opts.dry_run {
            None
        } else {
            Some(UpdateLock::acquire(&self.layout.lock_file())?)
        };
        let mut state = State::load(&self.layout)?;
        if !opts.dry_run {
            self.reconcile(&mut state).await?;
            if let Some(installed) = self.installed_identity().await
                && installed.version != self.current.version
            {
                return Ok(Outcome::Skipped {
                    version: installed.version.clone(),
                    reason: format!(
                        "the installed build is {} but this run started as {}; retrying next run",
                        installed.version, self.current.version
                    ),
                });
            }
        }

        let resolved = self.resolve(pin).await?;
        let m = &resolved.manifest;
        let (target, current) = (m.semver(), self.current_semver()?);

        if pin.is_none() && target < current {
            if !opts.dry_run {
                self.remember_pointer(&mut state, &resolved);
            }
            return Ok(Outcome::UpToDate {
                current: self.current.version.clone(),
                note: Some(
                    UpdateError::Downgrade {
                        current: self.current.version.clone(),
                        target: m.version.clone(),
                    }
                    .to_string(),
                ),
            });
        }
        // Only a pin typed on the command line overrides an earlier failure;
        // a configured pin would otherwise retry a bad build every tick.
        if explicit_pin.is_none() && state.is_blocked(&m.version) {
            return Ok(Outcome::Skipped {
                version: m.version.clone(),
                reason: UpdateError::Blocked(m.version.clone()).to_string(),
            });
        }
        if target == current && !opts.force {
            if !opts.dry_run {
                self.remember_pointer(&mut state, &resolved);
            }
            return Ok(Outcome::UpToDate {
                current: self.current.version.clone(),
                note: (!resolved.warnings.is_empty()).then(|| resolved.warnings.join("; ")),
            });
        }
        if opts.dry_run {
            return Ok(Outcome::DryRun {
                from: self.current.version.clone(),
                to: m.version.clone(),
            });
        }

        if explicit_pin.is_some() {
            state.unblock(&m.version);
        }
        let result = self.apply_locked(&resolved, &mut state).await;
        if matches!(result, Ok(Outcome::Applied { .. })) {
            self.remember_pointer(&mut state, &resolved);
        }
        if let Err(e) = &result
            && matches!(
                e,
                UpdateError::StagedBinary(_)
                    | UpdateError::UnsafeArchive(_)
                    | UpdateError::Invalid { .. }
            )
        {
            state.block(&m.version, self.cfg.blocked_ttl_secs);
            let _ = state.save(&self.layout);
        }
        result
    }

    /// The build currently installed at `bin_path`, from its own `--version`.
    async fn installed_identity(&self) -> Option<Installed> {
        let out = self.host.staged_version(&self.cfg.bin_path).await.ok()?;
        parse_version_line(&out)
    }

    /// Finishes or undoes an update that a dead run left behind: repairs the
    /// web tree, then either confirms the new build is healthy or rolls back.
    async fn reconcile(&self, state: &mut State) -> Result<()> {
        let repaired_web = self.repair_web_tree(state, None);
        let Some(marker) = state.in_progress.clone() else {
            return Ok(());
        };
        let installed = self.installed_identity().await;
        let target_live = installed.as_ref().is_some_and(|i| {
            i.version == marker.target.version
                && (!is_commit_sha(&marker.target.git_sha)
                    || sha_matches(&i.git_sha, &marker.target.git_sha))
        });
        if !target_live {
            // The old build is still (or again) installed: nothing half-done in the binary.
            self.repair_web_tree(state, Some(false));
            state.in_progress = None;
            return state.save(&self.layout);
        }
        self.alert(&format!(
            "an earlier update to {} was interrupted; finishing it now{}",
            marker.target.version,
            if repaired_web {
                " (web files repaired)"
            } else {
                ""
            }
        ))
        .await;
        if state.history.first().map(|s| &s.installed) != Some(&marker.from)
            && self.layout.backup(1).is_file()
        {
            state.history.insert(
                0,
                Slot {
                    installed: marker.from.clone(),
                    db_snapshot: marker.db_snapshot.clone(),
                    has_web: false,
                },
            );
            state.history.truncate(self.cfg.keep_binaries);
        }
        self.repair_web_tree(state, Some(true));
        state.save(&self.layout)?;

        let restarted = self.restarter.restart().await;
        if restarted.is_ok() && self.wait_healthy(&marker.target.git_sha).await {
            state.in_progress = None;
            state.save(&self.layout)?;
            let _ = self.host.post_upgrade().await;
            self.notify("post-ok", &marker.target.git_sha).await;
            return Ok(());
        }
        let after = self.host.db_migrations().await.unwrap_or(None);
        let migrated = matches!((marker.migrations_before, after), (Some(b), Some(a)) if a != b);
        state.block(&marker.target.version, self.cfg.blocked_ttl_secs);
        let rolled = self.rollback_inner(state, migrated, false).await;
        state.in_progress = None;
        state.save(&self.layout)?;
        self.alert(&format!(
            "the interrupted update to {} did not come up healthy and was rolled back",
            marker.target.version
        ))
        .await;
        rolled.map(|_| ())
    }

    /// Puts the web tree in a consistent state after a crash. `binary_new` is
    /// `Some(true)` when the new binary is live, `Some(false)` when the old one
    /// is, `None` when only a missing live tree should be repaired.
    fn repair_web_tree(&self, state: &mut State, binary_new: Option<bool>) -> bool {
        let (live, staged, prev) = (
            &self.layout.web_dist,
            self.layout.staged_web(),
            self.layout.prev_web(),
        );
        let mut repaired = false;
        match binary_new {
            Some(true) if staged.exists() => {
                if swap::install_web(&self.layout).unwrap_or(false)
                    && let Some(first) = state.history.first_mut()
                {
                    first.has_web = true;
                }
                repaired = true;
            }
            Some(false) => {
                if !live.exists() && prev.exists() {
                    repaired = std::fs::rename(&prev, live).is_ok();
                }
                let _ = std::fs::remove_dir_all(&staged);
            }
            _ => {}
        }
        if !live.exists() {
            let source = if staged.exists() { &staged } else { &prev };
            if source.exists() {
                repaired = std::fs::rename(source, live).is_ok() || repaired;
            }
        }
        repaired
    }

    async fn download_artifact(
        &self,
        resolved: &Resolved,
        name: &str,
        limit: u64,
        dir: &Path,
    ) -> Result<PathBuf> {
        let artifact = resolved
            .manifest
            .artifact(name)
            .ok_or_else(|| UpdateError::Invalid {
                what: "manifest",
                reason: format!("artifact {name} is not listed"),
            })?;
        if artifact.size > limit {
            return Err(UpdateError::SizeLimit {
                name: name.to_string(),
                limit,
            });
        }
        let dest = dir.join(name);
        self.http
            .download(
                &format!("{}/{name}", resolved.base_url),
                &dest,
                artifact.size,
            )
            .await?;
        let actual_size = std::fs::metadata(&dest)?.len();
        if actual_size != artifact.size {
            return Err(UpdateError::ChecksumMismatch {
                name: name.to_string(),
                expected: format!("{} bytes", artifact.size),
                actual: format!("{actual_size} bytes"),
            });
        }
        let actual = sha256_file(&dest)?;
        if actual != artifact.sha256 {
            return Err(UpdateError::ChecksumMismatch {
                name: name.to_string(),
                expected: artifact.sha256.clone(),
                actual,
            });
        }
        Ok(dest)
    }

    async fn stage(&self, resolved: &Resolved) -> Result<bool> {
        let m = &resolved.manifest;
        let work = WorkDir(self.layout.work_dir().join(std::process::id().to_string()));
        swap::make_private_dir(&work.0)?;
        let limits = &self.cfg.limits;

        // A missing platform is not a bad build, so it must not block the version.
        let bin_name = manifest::binary_artifact_name_for(&m.version, &self.platform);
        if m.artifact(&bin_name).is_none() {
            return Err(UpdateError::NoPlatformArtifact {
                version: m.version.clone(),
                platform: self.platform.clone(),
            });
        }
        let bin_tgz = self
            .download_artifact(resolved, &bin_name, limits.max_binary_bytes, &work.0)
            .await?;
        let web_name = manifest::web_artifact_name(&m.version);
        let web_tgz = if m.artifact(&web_name).is_some() {
            Some(
                self.download_artifact(resolved, &web_name, limits.max_web_bytes, &work.0)
                    .await?,
            )
        } else {
            None
        };

        let staged_bin = self.layout.staged_binary();
        let _ = std::fs::remove_file(&staged_bin);
        archive::extract_single_file(
            &bin_tgz,
            BINARY_FILE_NAME,
            &staged_bin,
            limits.max_binary_bytes,
        )?;
        if let Some(tgz) = &web_tgz {
            let dir = self.layout.staged_web();
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir)?;
            let tree = TreeLimits {
                max_total_bytes: limits.max_web_unpacked_bytes,
                max_files: limits.max_web_files,
            };
            archive::extract_tree(tgz, &dir, &tree)?;
            for required in WEB_REQUIRED_FILES {
                if !dir.join(required).is_file() {
                    return Err(UpdateError::Invalid {
                        what: "web archive",
                        reason: format!("missing {required}, refusing to install a partial build"),
                    });
                }
            }
        }

        // Exec failures pass through as plain errors; only a clean run that
        // reports the wrong build counts against the release.
        let reported = self.host.staged_version(&staged_bin).await?;
        let sha_prefix = &m.git_sha[..m.git_sha.len().min(VERSION_SHA_PREFIX)];
        let matches = parse_version_line(&reported).is_some_and(|i| {
            i.version == m.version
                && i.git_sha
                    .get(..sha_prefix.len())
                    .is_some_and(|s| s.eq_ignore_ascii_case(sha_prefix))
        });
        if !matches {
            return Err(UpdateError::StagedBinary(format!(
                "`--version` printed `{}`, expected `hq {} ({sha_prefix}...`",
                reported.lines().next().unwrap_or("").trim(),
                m.version
            )));
        }
        Ok(web_tgz.is_some())
    }

    fn cleanup_staging(&self) {
        let _ = std::fs::remove_file(self.layout.staged_binary());
        let _ = std::fs::remove_dir_all(self.layout.staged_web());
    }

    async fn notify(&self, phase: &str, sha: &str) {
        if self.cfg.notify.enabled {
            self.host.notify(phase, sha).await;
        }
    }

    /// Polls `/health` until it reports `expected_sha`. A build whose own
    /// commit is unknown (no git at build time) can only be checked for "ok".
    async fn wait_healthy(&self, expected_sha: &str) -> bool {
        let check_sha = is_commit_sha(expected_sha);
        for _ in 0..self.cfg.health_tries {
            tokio::time::sleep(Duration::from_secs(self.cfg.health_interval_secs)).await;
            let Some(info) = self.health.probe().await else {
                continue;
            };
            let matches = !check_sha
                || info
                    .git_sha
                    .as_deref()
                    .is_some_and(|s| sha_matches(s, expected_sha));
            if matches {
                return true;
            }
        }
        false
    }

    async fn snapshot_db(&self, state: &State) -> Result<Option<PathBuf>> {
        let dir = self.layout.snapshots_dir();
        let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
        let dest = dir.join(format!(
            "{SNAPSHOT_PREFIX}{stamp}-{}.db",
            self.current.version
        ));
        if !self.host.db_snapshot(&dest).await? {
            return Ok(None);
        }
        let mut protected: Vec<PathBuf> = state
            .history
            .iter()
            .filter_map(|s| s.db_snapshot.clone())
            .collect();
        protected.push(dest.clone());
        if let Err(e) = self
            .host
            .db_prune(&dir, self.cfg.snapshots_keep, &protected)
            .await
        {
            tracing::warn!(error = %e, "could not prune old snapshots");
        }
        Ok(Some(dest))
    }

    async fn alert(&self, message: &str) {
        if self.cfg.notify.enabled {
            self.host.alert(message).await;
        }
    }

    /// Puts the previous binary and web files back after a failed swap,
    /// without a restart (the old process is still the one running).
    fn undo_swap(&self, state: &mut State, web_swapped: bool) {
        if let Err(e) = swap::restore_binary(&self.layout, state, self.cfg.keep_binaries) {
            tracing::error!(error = %e, "could not restore the previous binary after a failed swap");
        }
        if web_swapped && let Err(e) = swap::restore_web(&self.layout) {
            tracing::error!(error = %e, "could not restore the previous web files after a failed swap");
        }
        state.in_progress = None;
        let _ = state.save(&self.layout);
    }

    async fn apply_locked(&self, resolved: &Resolved, state: &mut State) -> Result<Outcome> {
        let m = &resolved.manifest;
        // A killed run can leave a work dir behind; sweep it while holding the lock.
        let _ = std::fs::remove_dir_all(self.layout.work_dir());
        let has_web = match self.stage(resolved).await {
            Ok(w) => w,
            Err(e) => {
                self.cleanup_staging();
                return Err(e);
            }
        };

        self.notify("pre", &m.git_sha).await;
        tokio::time::sleep(Duration::from_secs(self.cfg.notify.grace_secs)).await;

        // Taken after the grace period so the restore point is as fresh as possible.
        // The service user can make this fail, so it only stops the update when
        // the release says it cannot run without a restore point.
        let mut warnings = resolved.warnings.clone();
        let snapshot = match self.snapshot_db(state).await {
            Ok(s) => s,
            Err(e) if !m.requires_db_snapshot => {
                let warning =
                    format!("database snapshot skipped, updating without a restore point: {e}");
                tracing::warn!("{warning}");
                self.alert(&warning).await;
                warnings.push(warning);
                None
            }
            Err(e) => {
                self.cleanup_staging();
                return Err(e);
            }
        };
        let migrations_before = self.host.db_migrations().await.unwrap_or(None);

        // Journal first: if this run dies from here on, the next one finishes or undoes it.
        state.in_progress = Some(InProgress {
            target: Installed {
                version: m.version.clone(),
                git_sha: m.git_sha.clone(),
            },
            from: self.current.clone(),
            db_snapshot: snapshot.clone(),
            migrations_before,
            phase: "swapping".into(),
        });
        if let Err(e) = state.save(&self.layout) {
            self.cleanup_staging();
            return Err(e);
        }

        let slot = Slot {
            installed: self.current.clone(),
            db_snapshot: snapshot,
            has_web: false,
        };
        if let Err(e) = swap::install_binary(
            &self.layout,
            state,
            &self.layout.staged_binary(),
            self.cfg.keep_binaries,
            slot,
        ) {
            state.in_progress = None;
            let _ = state.save(&self.layout);
            self.cleanup_staging();
            return Err(e);
        }
        // Persist the new backup slot before anything else can fail.
        if let Err(e) = state.save(&self.layout) {
            self.undo_swap(state, false);
            self.cleanup_staging();
            return Err(e);
        }
        let web_swapped = if has_web {
            match swap::install_web(&self.layout) {
                Ok(w) => w,
                Err(e) => {
                    self.undo_swap(state, false);
                    self.cleanup_staging();
                    return Err(e);
                }
            }
        } else {
            false
        };
        if let Some(first) = state.history.first_mut() {
            first.has_web = web_swapped;
        }
        if let Err(e) = state.save(&self.layout) {
            self.undo_swap(state, web_swapped);
            return Err(e);
        }

        let restarted = self.restarter.restart().await;
        let healthy = restarted.is_ok() && self.wait_healthy(&m.git_sha).await;
        if healthy {
            state.in_progress = None;
            state.save(&self.layout)?;
            let hook = self.host.post_upgrade().await;
            if let Err(e) = &hook {
                tracing::warn!(error = %e, "hq install --upgrade failed after a healthy update");
            }
            self.notify("post-ok", &m.git_sha).await;
            return Ok(Outcome::Applied {
                from: self.current.version.clone(),
                to: m.version.clone(),
                git_sha: m.git_sha.clone(),
                upgrade_hook_ok: hook.is_ok(),
                warnings,
            });
        }

        let reason = match restarted {
            Err(e) => format!("restart failed: {e}"),
            Ok(()) => format!(
                "no healthy {} after {} tries",
                &m.git_sha[..m.git_sha.len().min(VERSION_SHA_PREFIX)],
                self.cfg.health_tries
            ),
        };
        let migrations_after = self.host.db_migrations().await.unwrap_or(None);
        // An unreadable count is not evidence that migrations ran.
        let migrations_ran =
            matches!((migrations_before, migrations_after), (Some(b), Some(a)) if a != b);
        let restore_db = migrations_ran || m.requires_db_snapshot;
        state.block(&m.version, self.cfg.blocked_ttl_secs);
        let _ = state.save(&self.layout);
        let rolled = self.rollback_inner(state, restore_db, false).await;
        state.in_progress = None;
        let _ = state.save(&self.layout);
        match rolled {
            Ok(db_restored) => {
                self.notify("post-rolled-back", &m.git_sha).await;
                self.alert(&format!(
                    "update to {} failed ({reason}); rolled back to {}{}",
                    m.version,
                    self.current.version,
                    if db_restored {
                        " and restored the database snapshot"
                    } else {
                        ""
                    }
                ))
                .await;
                Ok(Outcome::RolledBack {
                    attempted: m.version.clone(),
                    restored: self.current.version.clone(),
                    db_restored,
                    reason,
                })
            }
            Err(e) => {
                self.alert(&format!(
                    "update to {} failed ({reason}) and the rollback also failed: {e}. Manual intervention needed.",
                    m.version
                ))
                .await;
                Err(e)
            }
        }
    }

    /// Restores the newest kept build (binary, web, optionally the DB) and
    /// restarts. `db_required` makes a missing snapshot an error instead of a
    /// logged skip. Returns whether the DB snapshot was restored.
    async fn rollback_inner(
        &self,
        state: &mut State,
        restore_db: bool,
        db_required: bool,
    ) -> Result<bool> {
        let slot = state
            .history
            .first()
            .cloned()
            .ok_or(UpdateError::NoRollbackTarget)?;
        // Everything that can fail cheaply is checked before the service is stopped.
        if !self.layout.backup(1).is_file() {
            return Err(UpdateError::NoRollbackTarget);
        }
        let snapshot = slot.db_snapshot.clone().filter(|p| p.is_file());
        if restore_db && snapshot.is_none() {
            if db_required {
                return Err(UpdateError::Other(anyhow::anyhow!(
                    "the database snapshot for {} is missing; nothing was changed",
                    slot.installed.version
                )));
            }
            tracing::error!("database restore wanted but no snapshot is available");
        }

        // A failed stop must not strand the broken build: restore and restart anyway.
        if let Err(e) = self.restarter.stop().await {
            tracing::warn!(error = %e, "could not stop the service before rollback");
        }
        let restored = self
            .restore_files_and_db(state, &slot, restore_db.then_some(snapshot).flatten())
            .await;
        // Whatever happened, try to bring a service back up.
        let restarted = self.restarter.restart().await;
        let db_restored = restored?;
        restarted?;
        if !self.wait_healthy(&slot.installed.git_sha).await {
            return Err(UpdateError::Other(anyhow::anyhow!(
                "rolled back to {} but it did not become healthy; manual intervention needed",
                slot.installed.version
            )));
        }
        Ok(db_restored)
    }

    /// Each step is recorded in `state.json` as soon as it happens, and a
    /// failing web or database step does not stop the later ones, so the
    /// state always matches the backup files that were actually shifted.
    async fn restore_files_and_db(
        &self,
        state: &mut State,
        slot: &Slot,
        snapshot: Option<PathBuf>,
    ) -> Result<bool> {
        swap::restore_binary(&self.layout, state, self.cfg.keep_binaries)?;
        state.save(&self.layout)?;
        let mut failures = Vec::new();
        if slot.has_web {
            match swap::restore_web(&self.layout) {
                Ok(()) => {
                    // Only one previous web tree is kept, and it is now live.
                    for older in &mut state.history {
                        older.has_web = false;
                    }
                    state.save(&self.layout)?;
                }
                Err(e) => failures.push(format!("web files: {e}")),
            }
        }
        let mut db_restored = false;
        if let Some(path) = snapshot {
            match self.host.db_restore(&path).await {
                Ok(()) => db_restored = true,
                Err(e) => failures.push(format!("database: {e}")),
            }
        }
        if failures.is_empty() {
            Ok(db_restored)
        } else {
            Err(UpdateError::Other(anyhow::anyhow!(
                "rollback restored the binary but not everything: {}",
                failures.join("; ")
            )))
        }
    }

    /// `hq update --rollback`.
    pub async fn rollback(&self, restore_db: bool) -> Result<Outcome> {
        self.refuse_unmanaged_darwin()?;
        let _lock = UpdateLock::acquire(&self.layout.lock_file())?;
        let mut state = State::load(&self.layout)?;
        self.reconcile(&mut state).await?;
        let target = state
            .history
            .first()
            .cloned()
            .ok_or(UpdateError::NoRollbackTarget)?;
        self.notify("pre", &target.installed.git_sha).await;
        state.block(&self.current.version, self.cfg.blocked_ttl_secs);
        let db_restored = self
            .rollback_inner(&mut state, restore_db, restore_db)
            .await?;
        self.notify("post-ok", &target.installed.git_sha).await;
        Ok(Outcome::ManualRollback {
            from: self.current.version.clone(),
            to: target.installed.version,
            db_restored,
        })
    }
}
