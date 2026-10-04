//! Checkpointed self-update lifecycle: HQ editing its own source with full
//! autonomy behind hard checkpoints.
//!
//! Flow: `self_update_begin` (branch + rollback binary snapshot) → HQ edits
//! code with its normal file tools → `self_update_check` (cargo check + test,
//! gate) → `self_update_install` (commit, release build, detached
//! `hq self-apply` swaps the binary and notifies via the value bus) →
//! `self_update_rollback` (restore snapshot) if anything looks wrong.
//!
//! The tools exist only when `self_update.enabled` is true, and
//! `self_update_install` never ships on the agent's say-so: it files an
//! approval request for the exact tree and proceeds only once the owner has
//! approved that request (chat button, web UI or `hq queue approve`).
//! Production deployments should update with `hq update` (signed releases).

use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_core::config::HqConfig;
use hq_db::Database;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::registry::HqTool;

const CHECK_TIMEOUT_SECS: u64 = 1200;
const BUILD_TIMEOUT_SECS: u64 = 1800;
const OUTPUT_TAIL_CHARS: usize = 4000;

fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let start = s.len() - max;
        let boundary = s
            .char_indices()
            .map(|(i, _)| i)
            .find(|i| *i >= start)
            .unwrap_or(start);
        s[boundary..].to_string()
    }
}

fn slugify(desc: &str) -> String {
    let slug: String = desc
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug.trim_matches('-').to_string();
    let mut compact = String::new();
    let mut last_dash = false;
    for c in slug.chars() {
        if c == '-' {
            if !last_dash {
                compact.push(c);
            }
            last_dash = true;
        } else {
            compact.push(c);
            last_dash = false;
        }
    }
    compact.chars().take(40).collect()
}

async fn run_cmd(
    program: &str,
    args: &[&str],
    cwd: &Path,
    timeout_secs: u64,
) -> Result<(bool, String)> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        tokio::process::Command::new(program)
            .args(args)
            .current_dir(cwd)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| {
        anyhow::anyhow!(
            "{program} {} timed out after {timeout_secs}s",
            args.join(" ")
        )
    })??;
    let mut combined = String::from_utf8_lossy(&output.stdout).to_string();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok((output.status.success(), combined))
}

async fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let (ok, out) = run_cmd("git", args, repo, 60).await?;
    if !ok {
        bail!("git {} failed: {}", args.join(" "), tail(&out, 500));
    }
    Ok(out.trim().to_string())
}

fn rollback_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".hq/rollback")
}

fn ensure_enabled(config: &HqConfig) -> Result<()> {
    if !config.self_update.enabled {
        bail!("self_update is disabled in config (self_update.enabled: false)");
    }
    Ok(())
}

/// Where an install request stands with the owner.
#[derive(Debug, PartialEq, Eq)]
enum ApprovalGate {
    /// No request has been filed for this tree yet.
    Request,
    /// Filed and not yet answered.
    Waiting,
    Declined,
    Approved,
}

/// An `Engaged` item counts only with a valid signature, so a database row
/// edited by hand is treated as still waiting.
fn approval_gate(state: Option<hq_core::types::ValueState>, signed: bool) -> ApprovalGate {
    use hq_core::types::ValueState::*;
    match state {
        None => ApprovalGate::Request,
        Some(Pending | Routed | Delivered) => ApprovalGate::Waiting,
        Some(Engaged) if signed => ApprovalGate::Approved,
        Some(Engaged) => ApprovalGate::Waiting,
        Some(Dismissed | Expired) => ApprovalGate::Declined,
    }
}

/// Stable per run, tree and built binary, so an approval cannot be reused for
/// different code or a different artifact.
fn approval_dedup_key(run_id: i64, tree: &str, binary_sha: &str) -> String {
    format!(
        "self-update-approval-{run_id}-{}-{}",
        &tree[..tree.len().min(12)],
        &binary_sha[..binary_sha.len().min(16)]
    )
}

/// Stages everything and returns the git tree hash of the working state.
async fn staged_tree(repo: &Path) -> Result<String> {
    git(repo, &["add", "-A"]).await?;
    git(repo, &["write-tree"]).await
}

fn approval_request(
    run: &hq_db::self_update_runs::SelfUpdateRun,
    tree: &str,
    binary_sha: &str,
    stat: &str,
    key: &str,
) -> hq_core::types::ValueItem {
    hq_core::types::ValueItem::new(
        "self-update",
        hq_core::types::ValueKind::ActionNeeded,
        format!("Approve install of self-update #{}: {}", run.id, run.description),
        format!(
            "HQ built its own binary from branch {} (tree {}, binary sha256 {}).\n\n{}\n\n\
             Approving swaps the installed binary and restarts the daemon. Review the branch first. \
             Dismiss to decline.",
            run.branch,
            &tree[..tree.len().min(12)],
            &binary_sha[..binary_sha.len().min(16)],
            tail(stat, 1500)
        ),
    )
    .with_dedup_key(key.to_string())
}

/// The currently installed binary to snapshot: first install path that exists.
fn current_binary(config: &HqConfig) -> Option<PathBuf> {
    config
        .self_update
        .install_paths
        .iter()
        .find(|p| p.is_file())
        .cloned()
}

pub struct SelfUpdateBeginTool {
    config: HqConfig,
    db: Arc<Database>,
}

#[async_trait]
impl HqTool for SelfUpdateBeginTool {
    fn name(&self) -> &str {
        "self_update_begin"
    }
    fn description(&self) -> &str {
        "Start a checkpointed self-update of HQ's own source: creates a self/<date>-<slug> git branch and snapshots the current binary for rollback. Refuses while another run is open. Edit code with normal file tools afterward, then self_update_check."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "description": { "type": "string", "description": "One-line description of the intended change" }
            },
            "required": ["description"]
        })
    }
    fn category(&self) -> &str {
        "self_update"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        ensure_enabled(&self.config)?;
        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if description.is_empty() {
            bail!("description is required");
        }
        let active = self.db.with_conn(hq_db::self_update_runs::get_active)?;
        if let Some(run) = active {
            bail!(
                "self-update run #{} ('{}', status {}) is already in flight on branch {}. Finish it with self_update_install or self_update_rollback first.",
                run.id,
                run.description,
                run.status,
                run.branch
            );
        }

        let repo = self
            .config
            .self_update
            .resolve_repo_path(&self.config.vault_path);
        let base_rev = git(&repo, &["rev-parse", "--short", "HEAD"]).await?;
        let date = chrono::Local::now().format("%Y%m%d");
        let branch = format!("self/{date}-{}", slugify(&description));
        git(&repo, &["checkout", "-b", &branch]).await?;

        let prev_binary = if let Some(bin) = current_binary(&self.config) {
            let dir = rollback_dir();
            std::fs::create_dir_all(&dir)?;
            let snapshot = dir.join(format!("hq-{base_rev}"));
            std::fs::copy(&bin, &snapshot)?;
            Some(snapshot.to_string_lossy().to_string())
        } else {
            None
        };

        let desc = description.clone();
        let branch_c = branch.clone();
        let base_rev_c = base_rev.clone();
        let prev_c = prev_binary.clone();
        let run_id = self.db.with_conn(move |c| {
            hq_db::self_update_runs::insert(c, &desc, &branch_c, &base_rev_c, prev_c.as_deref())
        })?;

        Ok(json!({
            "run_id": run_id,
            "branch": branch,
            "base_rev": base_rev,
            "rollback_binary": prev_binary,
            "repo": repo.to_string_lossy(),
            "next": "Edit code with your file tools, then call self_update_check."
        }))
    }
}

pub struct SelfUpdateCheckTool {
    config: HqConfig,
    db: Arc<Database>,
}

#[async_trait]
impl HqTool for SelfUpdateCheckTool {
    fn name(&self) -> &str {
        "self_update_check"
    }
    fn description(&self) -> &str {
        "Run cargo check and cargo test for an open self-update run. Passing marks the run 'checked' (the gate self_update_install requires). Optionally restrict tests to specific packages."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": { "type": "integer", "description": "The self-update run id from self_update_begin" },
                "packages": { "type": "array", "items": { "type": "string" }, "description": "Optional crate names to test (e.g. [\"hq-vault\"]). Omit for workspace-wide." }
            },
            "required": ["run_id"]
        })
    }
    fn category(&self) -> &str {
        "self_update"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        ensure_enabled(&self.config)?;
        let run_id = args.get("run_id").and_then(|v| v.as_i64()).unwrap_or(0);
        let packages: Vec<String> = args
            .get("packages")
            .cloned()
            .map(|v| serde_json::from_value(v).unwrap_or_default())
            .unwrap_or_default();
        let run = self
            .db
            .with_conn(move |c| hq_db::self_update_runs::get(c, run_id))?
            .ok_or_else(|| anyhow::anyhow!("no self-update run #{run_id}"))?;
        if run.status != hq_db::self_update_runs::STATUS_OPEN
            && run.status != hq_db::self_update_runs::STATUS_CHECKED
        {
            bail!("run #{run_id} is '{}', not open", run.status);
        }

        let repo = self
            .config
            .self_update
            .resolve_repo_path(&self.config.vault_path);

        let mut check_args = vec!["check"];
        if packages.is_empty() {
            check_args.push("--workspace");
        } else {
            for p in &packages {
                check_args.push("-p");
                check_args.push(p);
            }
        }
        let (check_ok, check_out) =
            run_cmd("cargo", &check_args, &repo, CHECK_TIMEOUT_SECS).await?;

        let (test_ok, test_out) = if check_ok {
            let mut test_args = vec!["test"];
            for p in &packages {
                test_args.push("-p");
                test_args.push(p);
            }
            run_cmd("cargo", &test_args, &repo, CHECK_TIMEOUT_SECS).await?
        } else {
            (false, String::from("(tests skipped: cargo check failed)"))
        };

        let passed = check_ok && test_ok;
        let combined_tail = tail(
            &format!("== cargo check ==\n{check_out}\n== cargo test ==\n{test_out}"),
            OUTPUT_TAIL_CHARS,
        );
        let tail_c = combined_tail.clone();
        self.db.with_conn(move |c| {
            hq_db::self_update_runs::set_check_output(c, run_id, &tail_c)?;
            if passed {
                hq_db::self_update_runs::set_status(
                    c,
                    run_id,
                    hq_db::self_update_runs::STATUS_CHECKED,
                )?;
            }
            Ok(())
        })?;

        Ok(json!({
            "run_id": run_id,
            "passed": passed,
            "check_ok": check_ok,
            "test_ok": test_ok,
            "output_tail": tail(&combined_tail, 1500),
            "next": if passed { "self_update_install" } else { "fix the failures, then re-run self_update_check" }
        }))
    }
}

pub struct SelfUpdateInstallTool {
    config: HqConfig,
    db: Arc<Database>,
    /// Program and arguments that produce `target/release/hq`; a seam for tests.
    build: (String, Vec<String>),
    /// Tests turn this off so no detached applier is started.
    spawn_apply: bool,
}

impl SelfUpdateInstallTool {
    fn new(config: HqConfig, db: Arc<Database>) -> Self {
        let args = ["build", "--release", "--locked", "-p", "hq-cli"].map(String::from).to_vec();
        Self { config, db, build: ("cargo".into(), args), spawn_apply: true }
    }
}

#[async_trait]
impl HqTool for SelfUpdateInstallTool {
    fn name(&self) -> &str {
        "self_update_install"
    }
    fn description(&self) -> &str {
        "Ship a checked self-update: commits the branch, builds the release binary, and spawns a detached `hq self-apply` that swaps the installed binary, restarts the daemon, health-probes, auto-rolls-back on failure, and notifies via Telegram/Discord. Refuses unless self_update_check passed."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": { "type": "integer", "description": "The self-update run id" },
                "commit_message": { "type": "string", "description": "Optional commit message; defaults to the run description" }
            },
            "required": ["run_id"]
        })
    }
    fn category(&self) -> &str {
        "self_update"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        ensure_enabled(&self.config)?;
        let run_id = args.get("run_id").and_then(|v| v.as_i64()).unwrap_or(0);
        let run = self
            .db
            .with_conn(move |c| hq_db::self_update_runs::get(c, run_id))?
            .ok_or_else(|| anyhow::anyhow!("no self-update run #{run_id}"))?;
        if run.status != hq_db::self_update_runs::STATUS_OPEN
            && run.status != hq_db::self_update_runs::STATUS_CHECKED
        {
            bail!("run #{run_id} is '{}', not open", run.status);
        }
        if self.config.self_update.require_tests
            && run.status != hq_db::self_update_runs::STATUS_CHECKED
        {
            bail!(
                "run #{run_id} is '{}': self_update_install requires a passing self_update_check first",
                run.status
            );
        }

        let repo = self
            .config
            .self_update
            .resolve_repo_path(&self.config.vault_path);
        let message = args
            .get("commit_message")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("self-update: {}", run.description));

        let tree = staged_tree(&repo).await?;
        // The artifact the owner approves is built first, so the approval names
        // the exact bytes that will be installed, not just the sources.
        let built = repo.join("target/release/hq");
        let build_args: Vec<&str> = self.build.1.iter().map(String::as_str).collect();
        let (build_ok, build_out) =
            run_cmd(&self.build.0, &build_args, &repo, BUILD_TIMEOUT_SECS).await?;
        if !build_ok {
            bail!("release build failed: {}", tail(&build_out, 1500));
        }
        if staged_tree(&repo).await? != tree {
            bail!("the working tree changed during the build; run self_update_install again");
        }
        let binary_sha = hq_core::approval_key::file_sha256(&built)?;
        let key = approval_dedup_key(run_id, &tree, &binary_sha);
        let state = hq_db::value_items::latest_state_by_dedup(&self.db, &key)?;
        let signed = hq_db::value_items::is_signed_approval(&self.db, &key)?;
        match approval_gate(state, signed) {
            ApprovalGate::Approved => {}
            ApprovalGate::Waiting => return Ok(awaiting_approval(run_id, &key)),
            ApprovalGate::Declined => bail!(
                "the owner declined (or let expire) the install request for this build. Do not retry; ask the owner what to change."
            ),
            ApprovalGate::Request => {
                let stat = git(&repo, &["diff", "--cached", "--stat", &run.base_rev, "--"])
                    .await
                    .unwrap_or_else(|e| format!("(diff unavailable: {e})"));
                let item = approval_request(&run, &tree, &binary_sha, &stat, &key);
                hq_db::value_items::emit(&self.db, &item)?;
                return Ok(awaiting_approval(run_id, &key));
            }
        }

        // An empty diff is fine (edits may already be committed on the branch).
        let _ = run_cmd("git", &["commit", "--no-verify", "-m", &message], &repo, 60).await?;
        let key_bytes = hq_core::approval_key::load_or_create_key()?;
        let mac = hq_core::approval_key::hmac_hex(
            &key_bytes,
            &hq_core::approval_key::self_update_message(run_id, &tree, &binary_sha),
        );
        self.db.with_conn(|c| {
            hq_db::self_update_runs::set_approved(c, run_id, &binary_sha, &mac)
        })?;

        // Detached applier: survives this process exiting or restarting.
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("hq"));
        let mut cmd = std::process::Command::new(exe);
        cmd.args(["self-apply", "--run-id", &run_id.to_string()])
            .current_dir(&repo)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        if self.spawn_apply {
            cmd.spawn()?;
        }

        Ok(json!({
            "run_id": run_id,
            "branch": run.branch,
            "status": "installing",
            "detail": "Release build succeeded. Detached self-apply will swap the binary, restart the daemon, health-probe, and notify via the value bus (auto-rollback on failure)."
        }))
    }
}

fn awaiting_approval(run_id: i64, key: &str) -> Value {
    json!({
        "run_id": run_id,
        "status": "awaiting_owner_approval",
        "approval_key": key,
        "detail": "An approval request was sent to the owner (chat, web UI, or `hq queue approve`). Nothing was built or installed. Tell the owner, then call self_update_install again after they approve."
    })
}

pub struct SelfUpdateRollbackTool {
    config: HqConfig,
    db: Arc<Database>,
}

#[async_trait]
impl HqTool for SelfUpdateRollbackTool {
    fn name(&self) -> &str {
        "self_update_rollback"
    }
    fn description(&self) -> &str {
        "Roll back a self-update: restores the snapshotted binary to all install paths, kickstarts the daemon, checks out the base branch, and marks the run rolled_back."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": { "type": "integer", "description": "The self-update run id" }
            },
            "required": ["run_id"]
        })
    }
    fn category(&self) -> &str {
        "self_update"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        ensure_enabled(&self.config)?;
        let run_id = args.get("run_id").and_then(|v| v.as_i64()).unwrap_or(0);
        let run = self
            .db
            .with_conn(move |c| hq_db::self_update_runs::get(c, run_id))?
            .ok_or_else(|| anyhow::anyhow!("no self-update run #{run_id}"))?;

        let mut restored = Vec::new();
        let mut failures = Vec::new();
        if let Some(ref snapshot) = run.prev_binary_path {
            let snapshot = PathBuf::from(snapshot);
            if snapshot.is_file() {
                for dest in &self.config.self_update.install_paths {
                    match std::fs::copy(&snapshot, dest) {
                        Ok(_) => restored.push(dest.to_string_lossy().to_string()),
                        Err(e) => failures.push(format!("{}: {e}", dest.display())),
                    }
                }
            } else {
                failures.push(format!("snapshot {} missing", snapshot.display()));
            }
        }

        let repo = self
            .config
            .self_update
            .resolve_repo_path(&self.config.vault_path);
        let _ = run_cmd("git", &["checkout", "main"], &repo, 60).await;

        let restart_outcome =
            hq_core::daemon_restart::restart_daemon(&self.config.self_update.launchd_label).await;
        let restart_note = match &restart_outcome {
            hq_core::daemon_restart::RestartOutcome::Restarted
            | hq_core::daemon_restart::RestartOutcome::SkippedNoLabel => String::new(),
            hq_core::daemon_restart::RestartOutcome::Unsupported { message }
            | hq_core::daemon_restart::RestartOutcome::Failed { message } => {
                format!(" Daemon NOT restarted: {message}.")
            }
        };

        self.db.with_conn(move |c| {
            hq_db::self_update_runs::set_status(
                c,
                run_id,
                hq_db::self_update_runs::STATUS_ROLLED_BACK,
            )
        })?;

        let item = hq_core::types::ValueItem::new(
            "self-update",
            hq_core::types::ValueKind::Fyi,
            format!("Self-update #{run_id} rolled back"),
            format!(
                "Branch {} rolled back to rev {}. Restored: {}. {}{}",
                run.branch,
                run.base_rev,
                restored.join(", "),
                if failures.is_empty() {
                    String::new()
                } else {
                    format!("Failures: {}", failures.join("; "))
                },
                restart_note
            ),
        )
        // FR-001 criterion 4: without a stable key, dismissing this from the
        // queue never stuck — a later daemon cycle for the same rollback
        // would insert a fresh, undismissable row.
        .with_dedup_key(format!("self-update-rollback-{run_id}"));
        let _ = hq_db::value_items::emit(&self.db, &item);

        Ok(json!({
            "run_id": run_id,
            "restored": restored,
            "failures": failures,
            "base_rev": run.base_rev
        }))
    }
}

pub struct SelfUpdateStatusTool {
    db: Arc<Database>,
}

#[async_trait]
impl HqTool for SelfUpdateStatusTool {
    fn name(&self) -> &str {
        "self_update_status"
    }
    fn description(&self) -> &str {
        "Show the active self-update run (if any) and recent history."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn category(&self) -> &str {
        "self_update"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, _args: Value) -> Result<Value> {
        let active = self.db.with_conn(hq_db::self_update_runs::get_active)?;
        let recent = self
            .db
            .with_conn(|c| hq_db::self_update_runs::list_recent(c, 10))?;
        Ok(json!({ "active": active, "recent": recent }))
    }
}

/// The self-update tools, or none at all unless `self_update.enabled` is true.
/// Every registry goes through here, so a disabled config never exposes them.
pub fn create_self_update_tools(config: &HqConfig, db: Arc<Database>) -> Vec<Box<dyn HqTool>> {
    if !config.self_update.enabled {
        return Vec::new();
    }
    vec![
        Box::new(SelfUpdateBeginTool {
            config: config.clone(),
            db: db.clone(),
        }),
        Box::new(SelfUpdateCheckTool {
            config: config.clone(),
            db: db.clone(),
        }),
        Box::new(SelfUpdateInstallTool::new(config.clone(), db.clone())),
        Box::new(SelfUpdateRollbackTool {
            config: config.clone(),
            db: db.clone(),
        }),
        Box::new(SelfUpdateStatusTool { db }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_compacts_and_truncates() {
        assert_eq!(slugify("Add a trivial comment!"), "add-a-trivial-comment");
        assert_eq!(slugify("  --weird--  input  "), "weird-input");
        assert!(slugify(&"x".repeat(100)).len() <= 40);
    }

    #[test]
    fn tail_respects_char_boundaries() {
        let s = "héllo wörld".repeat(100);
        let t = tail(&s, 50);
        assert!(t.len() <= 54);
        assert!(s.ends_with(&t));
    }

    use hq_core::types::ValueState;
    use hq_db::self_update_runs as runs;

    fn config_for(repo: &Path, enabled: bool) -> HqConfig {
        let mut config = HqConfig::default();
        config.self_update.enabled = enabled;
        config.self_update.repo_path = Some(repo.to_path_buf());
        config.self_update.install_paths = Vec::new();
        config
    }

    fn git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "user.name", "t"],
            vec!["commit", "-q", "--allow-empty", "-m", "base"],
        ] {
            let ok = std::process::Command::new("git")
                .args(&args)
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        }
        std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        dir
    }

    fn install_tool(config: HqConfig) -> (SelfUpdateInstallTool, Arc<Database>, i64) {
        let (mut tool, db, run_id) = install_tool_real(config);
        tool.spawn_apply = false;
        tool.build = (
            "sh".into(),
            vec!["-c".into(), "mkdir -p target/release && echo fake-binary > target/release/hq".into()],
        );
        (tool, db, run_id)
    }

    fn install_tool_real(config: HqConfig) -> (SelfUpdateInstallTool, Arc<Database>, i64) {
        let db = Arc::new(Database::open_memory().unwrap());
        let run_id = db
            .with_conn(|c| runs::insert(c, "tweak", "self/x", "HEAD", None))
            .unwrap();
        db.with_conn(|c| runs::set_status(c, run_id, runs::STATUS_CHECKED))
            .unwrap();
        (SelfUpdateInstallTool::new(config, db.clone()), db, run_id)
    }

    #[test]
    fn tools_are_not_registered_unless_enabled() {
        let db = Arc::new(Database::open_memory().unwrap());
        let off = HqConfig::default();
        assert!(!off.self_update.enabled, "default must be off");
        assert!(create_self_update_tools(&off, db.clone()).is_empty());
        let mut on = HqConfig::default();
        on.self_update.enabled = true;
        assert_eq!(create_self_update_tools(&on, db).len(), 5);
    }

    #[tokio::test]
    async fn every_lifecycle_tool_refuses_when_disabled() {
        let repo = git_repo();
        let (install, db, run_id) = install_tool(config_for(repo.path(), false));
        let config = config_for(repo.path(), false);
        let args = json!({ "run_id": run_id, "description": "x" });
        assert!(install.execute(args.clone()).await.is_err());
        assert!(
            SelfUpdateBeginTool {
                config: config.clone(),
                db: db.clone()
            }
            .execute(args.clone())
            .await
            .is_err()
        );
        assert!(
            SelfUpdateCheckTool {
                config: config.clone(),
                db: db.clone()
            }
            .execute(args.clone())
            .await
            .is_err()
        );
        assert!(
            SelfUpdateRollbackTool { config, db }
                .execute(args)
                .await
                .is_err()
        );
    }

    #[test]
    fn approval_gate_maps_every_state() {
        assert_eq!(approval_gate(None, false), ApprovalGate::Request);
        for waiting in [
            ValueState::Pending,
            ValueState::Routed,
            ValueState::Delivered,
        ] {
            assert_eq!(approval_gate(Some(waiting), false), ApprovalGate::Waiting);
        }
        assert_eq!(
            approval_gate(Some(ValueState::Engaged), true),
            ApprovalGate::Approved
        );
        assert_eq!(
            approval_gate(Some(ValueState::Engaged), false),
            ApprovalGate::Waiting,
            "an unsigned engaged row is not an approval"
        );
        assert_eq!(
            approval_gate(Some(ValueState::Dismissed), true),
            ApprovalGate::Declined
        );
        assert_eq!(
            approval_gate(Some(ValueState::Expired), true),
            ApprovalGate::Declined
        );
    }

    #[test]
    fn approval_key_differs_per_tree_and_per_run() {
        let a = approval_dedup_key(1, "aaaaaaaaaaaaaaaa", "1111111111111111");
        assert_ne!(a, approval_dedup_key(1, "bbbbbbbbbbbbbbbb", "1111111111111111"));
        assert_ne!(a, approval_dedup_key(2, "aaaaaaaaaaaaaaaa", "1111111111111111"));
        assert_ne!(a, approval_dedup_key(1, "aaaaaaaaaaaaaaaa", "2222222222222222"));
    }

    #[tokio::test]
    async fn install_files_a_request_and_builds_nothing_until_the_owner_approves() {
        let repo = git_repo();
        std::fs::write(repo.path().join("change.txt"), "x").unwrap();
        let (tool, db, run_id) = install_tool(config_for(repo.path(), true));

        let out = tool.execute(json!({ "run_id": run_id })).await.unwrap();
        assert_eq!(out["status"], "awaiting_owner_approval");
        let key = out["approval_key"].as_str().unwrap().to_string();
        let state = hq_db::value_items::latest_state_by_dedup(&db, &key).unwrap();
        assert_eq!(state, Some(ValueState::Pending));

        let again = tool.execute(json!({ "run_id": run_id })).await.unwrap();
        assert_eq!(
            again["status"], "awaiting_owner_approval",
            "repeat calls must not ship"
        );
        let status = db
            .with_conn(|c| runs::get(c, run_id))
            .unwrap()
            .unwrap()
            .status;
        assert_eq!(status, runs::STATUS_CHECKED);
    }

    #[tokio::test]
    async fn a_declined_request_is_final_and_a_changed_tree_needs_a_new_request() {
        let repo = git_repo();
        std::fs::write(repo.path().join("change.txt"), "x").unwrap();
        let (tool, db, run_id) = install_tool(config_for(repo.path(), true));
        let first = tool.execute(json!({ "run_id": run_id })).await.unwrap();
        let key = first["approval_key"].as_str().unwrap().to_string();
        let item_id = db
            .with_conn(|c| {
                c.query_row(
                    "SELECT id FROM value_items WHERE dedup_key = ?1",
                    [&key],
                    |r| r.get::<_, String>(0),
                )
                .map_err(Into::into)
            })
            .unwrap();

        assert!(hq_db::value_items::dismiss_by_id(&db, &item_id).unwrap());
        let err = tool.execute(json!({ "run_id": run_id })).await.unwrap_err();
        assert!(err.to_string().contains("declined"), "{err}");

        std::fs::write(repo.path().join("change.txt"), "y").unwrap();
        let fresh = tool.execute(json!({ "run_id": run_id })).await.unwrap();
        assert_eq!(fresh["status"], "awaiting_owner_approval");
        assert_ne!(fresh["approval_key"], first["approval_key"]);
    }

    #[tokio::test]
    async fn an_approved_request_proceeds_past_the_gate_to_the_build() {
        let repo = git_repo();
        std::fs::write(repo.path().join("change.txt"), "x").unwrap();
        let (tool, db, run_id) = install_tool(config_for(repo.path(), true));
        let first = tool.execute(json!({ "run_id": run_id })).await.unwrap();
        let key = first["approval_key"].as_str().unwrap().to_string();
        let item_id = db
            .with_conn(|c| {
                c.query_row(
                    "SELECT id FROM value_items WHERE dedup_key = ?1",
                    [&key],
                    |r| r.get::<_, String>(0),
                )
                .map_err(Into::into)
            })
            .unwrap();
        assert!(hq_db::value_items::approve_by_id(&db, &item_id).unwrap());

        let out = tool.execute(json!({ "run_id": run_id })).await.unwrap();
        assert_eq!(out["status"], "installing");
        let run = db.with_conn(|c| runs::get(c, run_id)).unwrap().unwrap();
        assert_eq!(run.status, runs::STATUS_APPROVED);
        let sha = run.approved_binary_sha256.unwrap();
        let key = hq_core::approval_key::load_or_create_key().unwrap();
        let tree = String::from_utf8(
            std::process::Command::new("git")
                .args(["rev-parse", "HEAD^{tree}"])
                .current_dir(repo.path())
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        let message = hq_core::approval_key::self_update_message(run_id, tree.trim(), &sha);
        assert!(hq_core::approval_key::verify_hex(&key, &message, &run.approval_mac.unwrap()));
    }

    #[tokio::test]
    async fn a_hand_edited_engaged_row_is_not_an_approval() {
        let repo = git_repo();
        std::fs::write(repo.path().join("change.txt"), "x").unwrap();
        let (tool, db, run_id) = install_tool(config_for(repo.path(), true));
        let first = tool.execute(json!({ "run_id": run_id })).await.unwrap();
        let key = first["approval_key"].as_str().unwrap().to_string();
        db.with_conn(|c| {
            c.execute(
                "UPDATE value_items SET state = 'engaged', engagement = 'approved' WHERE dedup_key = ?1",
                [&key],
            )
            .map_err(Into::into)
        })
        .unwrap();
        let again = tool.execute(json!({ "run_id": run_id })).await.unwrap();
        assert_eq!(again["status"], "awaiting_owner_approval");
        let status = db.with_conn(|c| runs::get(c, run_id)).unwrap().unwrap().status;
        assert_eq!(status, runs::STATUS_CHECKED);
    }

    #[tokio::test]
    async fn a_rebuilt_different_binary_needs_a_new_approval() {
        let repo = git_repo();
        std::fs::write(repo.path().join("change.txt"), "x").unwrap();
        let (mut tool, db, run_id) = install_tool(config_for(repo.path(), true));
        let first = tool.execute(json!({ "run_id": run_id })).await.unwrap();
        tool.build.1 = vec!["-c".into(), "mkdir -p target/release && echo other-binary > target/release/hq".into()];
        let second = tool.execute(json!({ "run_id": run_id })).await.unwrap();
        assert_ne!(first["approval_key"], second["approval_key"]);
        let _ = db;
    }
}
