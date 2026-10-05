use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::registry::HqTool;

use super::{MAX_OUTPUT_BYTES, truncate_output};

pub struct GitCommitTool;

#[async_trait]
impl HqTool for GitCommitTool {
    fn name(&self) -> &str {
        "git_commit"
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Stage and review the diff before committing. Write why the change was made, not what changed — the diff already says what. Never commit unless you were asked to.",
        )
    }

    fn description(&self) -> &str {
        "Create a git commit. Optionally stage specific files first. \
         If no files specified, commits whatever is currently staged."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["message"],
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Commit message"
                },
                "files": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Files to stage before committing (optional)"
                },
                "all": {
                    "type": "boolean",
                    "description": "Stage all modified/deleted files (git add -A) before committing"
                }
            }
        })
    }

    fn category(&self) -> &str {
        "git"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let message = args["message"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing message"))?;
        let files = args
            .get("files")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>());
        let stage_all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);

        if stage_all {
            let out = tokio::process::Command::new("git")
                .args(["add", "-A"])
                .output()
                .await?;
            if !out.status.success() {
                return Ok(
                    json!({"error": format!("git add -A failed: {}", String::from_utf8_lossy(&out.stderr))}),
                );
            }
        } else if let Some(files) = &files {
            for file in files {
                let out = tokio::process::Command::new("git")
                    .args(["add", "--", file])
                    .output()
                    .await?;
                if !out.status.success() {
                    return Ok(
                        json!({"error": format!("git add {} failed: {}", file, String::from_utf8_lossy(&out.stderr))}),
                    );
                }
            }
        }

        let out = tokio::process::Command::new("git")
            .args(["commit", "-m", message])
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();

        if out.status.success() {
            Ok(json!({"success": true, "output": stdout}))
        } else {
            Ok(json!({"error": stderr, "output": stdout}))
        }
    }
}

pub struct GitDiffTool;

#[async_trait]
impl HqTool for GitDiffTool {
    fn name(&self) -> &str {
        "git_diff"
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Read this before any commit. Default shows unstaged changes — pass staged when you are about to commit.",
        )
    }

    fn description(&self) -> &str {
        "Show git diff. By default shows unstaged changes. Use staged=true for staged changes, \
         or provide a ref like 'HEAD~1' or 'main' to diff against."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "staged": {
                    "type": "boolean",
                    "description": "Show staged changes (--cached)"
                },
                "ref": {
                    "type": "string",
                    "description": "Diff against a ref (e.g., 'HEAD~1', 'main', commit SHA)"
                },
                "files": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Limit diff to specific files"
                },
                "stat": {
                    "type": "boolean",
                    "description": "Show diffstat summary instead of full diff"
                }
            }
        })
    }

    fn category(&self) -> &str {
        "git"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("diff").arg("--color=never");

        if args
            .get("staged")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            cmd.arg("--cached");
        }
        if args.get("stat").and_then(|v| v.as_bool()).unwrap_or(false) {
            cmd.arg("--stat");
        }
        if let Some(r) = args.get("ref").and_then(|v| v.as_str()) {
            // A leading dash would be parsed as an option (`--output=<path>` writes a file).
            if r.starts_with('-') {
                return Ok(json!({"error": "ref must not start with '-'"}));
            }
            cmd.arg(r);
        }
        if let Some(files) = args.get("files").and_then(|v| v.as_array()) {
            cmd.arg("--");
            for f in files {
                if let Some(s) = f.as_str() {
                    cmd.arg(s);
                }
            }
        }

        let out = cmd.output().await?;
        let diff = String::from_utf8_lossy(&out.stdout).to_string();

        Ok(json!({
            "diff": truncate_output(&diff, MAX_OUTPUT_BYTES),
            "lines": diff.lines().count()
        }))
    }
}

/// Collects a unified diff of every uncommitted change in `cwd`: staged and
/// unstaged edits to tracked files (`git diff HEAD`) plus untracked files
/// rendered as synthetic "new file" diffs, since `git diff HEAD` alone omits
/// them entirely. Used by `/codereview` to hand the adversarial critic the
/// same view of the working tree `git status` would show a human.
pub async fn uncommitted_diff(cwd: &std::path::Path) -> Result<String> {
    let mut out = String::new();

    let tracked = tokio::process::Command::new("git")
        .current_dir(cwd)
        .args(["diff", "HEAD", "--color=never"])
        .output()
        .await?;
    // A repo with no commits yet has no HEAD to diff against; that is not an
    // error, it just means every tracked change is picked up as untracked
    // below instead (git itself reports new files as untracked pre-commit).
    if tracked.status.success() {
        out.push_str(&String::from_utf8_lossy(&tracked.stdout));
    }

    let untracked = tokio::process::Command::new("git")
        .current_dir(cwd)
        .args(["ls-files", "--others", "--exclude-standard"])
        .output()
        .await?;
    if untracked.status.success() {
        for file in String::from_utf8_lossy(&untracked.stdout).lines() {
            let file = file.trim();
            if file.is_empty() {
                continue;
            }
            // `git diff --no-index` exits 1 when the two sides differ, which
            // is the expected outcome for every untracked file, so the status
            // is not checked here — only stdout matters.
            let diff = tokio::process::Command::new("git")
                .current_dir(cwd)
                .args(["diff", "--no-index", "--color=never", "/dev/null", file])
                .output()
                .await?;
            out.push_str(&String::from_utf8_lossy(&diff.stdout));
        }
    }

    Ok(out)
}

pub struct GitStatusTool;

#[async_trait]
impl HqTool for GitStatusTool {
    fn name(&self) -> &str {
        "git_status"
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Cheapest way to see what you have touched. Check it before committing and before switching branches; other agents may share this working tree.",
        )
    }

    fn description(&self) -> &str {
        "Show git working tree status: branch, staged/unstaged changes, untracked files."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    fn category(&self) -> &str {
        "git"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, _args: Value) -> Result<Value> {
        let branch = tokio::process::Command::new("git")
            .args(["branch", "--show-current"])
            .output()
            .await?;
        let status = tokio::process::Command::new("git")
            .args(["status", "--short"])
            .output()
            .await?;
        let log = tokio::process::Command::new("git")
            .args(["log", "--oneline", "-5"])
            .output()
            .await?;

        Ok(json!({
            "branch": String::from_utf8_lossy(&branch.stdout).trim().to_string(),
            "status": String::from_utf8_lossy(&status.stdout).to_string(),
            "recent_commits": String::from_utf8_lossy(&log.stdout).to_string()
        }))
    }
}

pub struct GitLogTool;

#[async_trait]
impl HqTool for GitLogTool {
    fn name(&self) -> &str {
        "git_log"
    }

    fn description(&self) -> &str {
        "Show git commit history. Returns recent commits with hash, author, date, and message."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "count": {
                    "type": "integer",
                    "description": "Number of commits to show (default: 10)"
                },
                "oneline": {
                    "type": "boolean",
                    "description": "One-line format (default: true)"
                },
                "file": {
                    "type": "string",
                    "description": "Show commits affecting a specific file"
                },
                "author": {
                    "type": "string",
                    "description": "Filter by author name/email"
                }
            }
        })
    }

    fn category(&self) -> &str {
        "git"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(10);
        let oneline = args
            .get("oneline")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("log").arg(format!("-{}", count));

        if oneline {
            cmd.arg("--oneline");
        } else {
            cmd.arg("--format=%H%n%an <%ae>%n%ai%n%s%n");
        }

        if let Some(author) = args.get("author").and_then(|v| v.as_str()) {
            cmd.arg(format!("--author={}", author));
        }
        if let Some(file) = args.get("file").and_then(|v| v.as_str()) {
            cmd.arg("--").arg(file);
        }

        let out = cmd.output().await?;
        let log = String::from_utf8_lossy(&out.stdout).to_string();

        Ok(json!({
            "log": log,
            "count": log.lines().filter(|l| !l.is_empty()).count()
        }))
    }
}

pub struct GitPrTool;

#[async_trait]
impl HqTool for GitPrTool {
    fn name(&self) -> &str {
        "git_pr"
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Shells out to the gh CLI. Confirm gh is authenticated first: the Machine Profile in your prompt reports it, or call system_info with check gh_auth. Pushes the current branch if needed; never open a PR from the default branch.",
        )
    }

    fn description(&self) -> &str {
        "Create a GitHub pull request using the gh CLI. Pushes current branch first if needed."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["title"],
            "properties": {
                "title": {
                    "type": "string",
                    "description": "PR title"
                },
                "body": {
                    "type": "string",
                    "description": "PR description/body"
                },
                "base": {
                    "type": "string",
                    "description": "Base branch (default: repo default branch)"
                },
                "draft": {
                    "type": "boolean",
                    "description": "Create as draft PR"
                },
                "push": {
                    "type": "boolean",
                    "description": "Push current branch before creating PR (default: true)"
                }
            }
        })
    }

    fn category(&self) -> &str {
        "git"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let title = args["title"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing title"))?;
        let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
        let draft = args.get("draft").and_then(|v| v.as_bool()).unwrap_or(false);
        let push = args.get("push").and_then(|v| v.as_bool()).unwrap_or(true);

        if push {
            let out = tokio::process::Command::new("git")
                .args(["push", "-u", "origin", "HEAD"])
                .output()
                .await?;
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                // Only error if it's not "everything up-to-date"
                if !stderr.contains("Everything up-to-date") && !stderr.contains("up to date") {
                    return Ok(json!({"error": format!("git push failed: {}", stderr)}));
                }
            }
        }

        let mut cmd = tokio::process::Command::new("gh");
        cmd.args(["pr", "create", "--title", title, "--body", body]);

        if draft {
            cmd.arg("--draft");
        }
        if let Some(base) = args.get("base").and_then(|v| v.as_str()) {
            cmd.args(["--base", base]);
        }

        let out = cmd.output().await?;
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();

        if out.status.success() {
            Ok(json!({"success": true, "url": stdout.trim()}))
        } else {
            Ok(json!({"error": stderr, "output": stdout}))
        }
    }
}

#[cfg(test)]
mod option_injection_tests {
    use super::*;

    #[tokio::test]
    async fn git_diff_refuses_refs_that_look_like_options() {
        let out = GitDiffTool
            .execute(json!({"ref": "--output=/tmp/hq-should-not-exist"}))
            .await
            .unwrap();
        assert!(out["error"].is_string(), "{out}");
        assert!(!std::path::Path::new("/tmp/hq-should-not-exist").exists());
    }
}
