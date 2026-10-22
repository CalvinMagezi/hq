//! Collectors: read a fact from outside the task and record it as an observation.
//!
//! Each one records what it found, including that it found nothing readable. None of them
//! decides anything; the verifier reads the log.

use std::collections::HashMap;
use std::path::Path;

use serde_json::{Value, json};

use super::observation::ObservationLog;
use super::remote::GitHubRead;
use super::tamper::{TamperReport, scan_diff};

/// Most names listed in a fact before it says "and more".
const LISTED: usize = 20;
/// Most changed paths kept on the observation.
const MAX_PATHS: usize = 100;

/// One URL path segment: everything but letters, digits and `._-` is percent-encoded, so a
/// branch called `feature/x` or `release+1` stays one segment.
pub(crate) fn encode_segment(s: &str) -> String {
    encode_with(s, false)
}

/// A query-string value, where `/` may stay as it is.
pub(crate) fn encode_query_value(s: &str) -> String {
    encode_with(s, true)
}

fn encode_with(s: &str, keep_slash: bool) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-' => out.push(b as char),
            b'/' if keep_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn names(list: &[String]) -> String {
    let shown: Vec<&str> = list.iter().take(5).map(String::as_str).collect();
    let more = list.len().saturating_sub(shown.len());
    if more > 0 {
        format!("{} and {more} more", shown.join(", "))
    } else {
        shown.join(", ")
    }
}

/// What a pull request looks like.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrFacts {
    pub repo: String,
    pub number: u64,
    pub head_sha: Option<String>,
    pub base_ref: Option<String>,
    pub merged: bool,
    pub open: bool,
    pub changed_files: u64,
}

/// Reads a pull request's state. Merged supports completion; closed without merging contradicts
/// it; open is neither.
pub async fn collect_pr(
    gh: &dyn GitHubRead,
    log: &mut ObservationLog,
    repo: &str,
    number: u64,
) -> Option<PrFacts> {
    let path = format!("repos/{repo}/pulls/{number}");
    let v = match gh.api_json(&path).await {
        Ok(v) => v,
        Err(e) => {
            log.unavailable(
                "github",
                "pr_state",
                format!("{repo}#{number} could not be read ({e})"),
            );
            return None;
        }
    };
    let text = |p: &[&str]| -> Option<String> {
        let mut cur = &v;
        for k in p {
            cur = cur.get(*k)?;
        }
        cur.as_str().map(str::to_string)
    };
    let merged = v.get("merged").and_then(Value::as_bool).unwrap_or(false);
    let open = text(&["state"]).as_deref() == Some("open");
    let facts = PrFacts {
        repo: repo.to_string(),
        number,
        head_sha: text(&["head", "sha"]),
        base_ref: text(&["base", "ref"]),
        merged,
        open,
        changed_files: v.get("changed_files").and_then(Value::as_u64).unwrap_or(0),
    };
    let (ok, fact) = if merged {
        (
            Some(true),
            format!(
                "{repo}#{number} is merged into {}",
                facts.base_ref.as_deref().unwrap_or("its base")
            ),
        )
    } else if open {
        (None, format!("{repo}#{number} is open and not merged"))
    } else {
        (
            Some(false),
            format!("{repo}#{number} was closed without merging"),
        )
    };
    log.record(
        "github",
        "pr_state",
        fact,
        ok,
        json!({
            "repo": repo,
            "number": number,
            "merged": merged,
            "state": text(&["state"]),
            "head_sha": facts.head_sha,
            "base_ref": facts.base_ref,
            "merged_at": text(&["merged_at"]),
            "author": text(&["user", "login"]),
            "changed_files": facts.changed_files,
            "additions": v.get("additions").and_then(Value::as_u64),
            "deletions": v.get("deletions").and_then(Value::as_u64),
        }),
    );
    Some(facts)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckState {
    Success,
    Failure,
    Pending,
}

fn run_state(status: &str, conclusion: Option<&str>) -> CheckState {
    if status != "completed" {
        return CheckState::Pending;
    }
    match conclusion {
        Some("success" | "neutral" | "skipped") => CheckState::Success,
        Some(_) => CheckState::Failure,
        None => CheckState::Pending,
    }
}

fn status_state(state: &str) -> CheckState {
    match state {
        "success" => CheckState::Success,
        "failure" | "error" => CheckState::Failure,
        _ => CheckState::Pending,
    }
}

/// Reads the checks reported on `sha` (check runs and commit statuses), and, when `base_ref` is
/// given, which of them the base branch requires. A failing check contradicts completion; one
/// still running or one never reported is neither; required checks that all succeeded support it.
pub async fn collect_checks(
    gh: &dyn GitHubRead,
    log: &mut ObservationLog,
    repo: &str,
    sha: &str,
    base_ref: Option<&str>,
) {
    let mut states: HashMap<String, CheckState> = HashMap::new();
    let runs = gh
        .api_json(&format!(
            "repos/{repo}/commits/{sha}/check-runs?per_page=100"
        ))
        .await;
    match &runs {
        Ok(v) => {
            for run in v
                .get("check_runs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let (Some(name), Some(status)) = (
                    run.get("name").and_then(Value::as_str),
                    run.get("status").and_then(Value::as_str),
                ) else {
                    continue;
                };
                let state = run_state(status, run.get("conclusion").and_then(Value::as_str));
                // The worse of two runs with one name decides.
                let entry = states.entry(name.to_string()).or_insert(state);
                if state == CheckState::Failure
                    || (state == CheckState::Pending && *entry == CheckState::Success)
                {
                    *entry = state;
                }
            }
        }
        Err(e) => {
            log.unavailable(
                "github",
                "check_runs",
                format!("checks on {sha} could not be read ({e})"),
            );
        }
    }
    // Some CI reports through the older commit status API instead.
    if let Ok(v) = gh
        .api_json(&format!("repos/{repo}/commits/{sha}/status"))
        .await
    {
        for s in v
            .get("statuses")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let (Some(name), Some(state)) = (
                s.get("context").and_then(Value::as_str),
                s.get("state").and_then(Value::as_str),
            ) {
                states
                    .entry(name.to_string())
                    .or_insert_with(|| status_state(state));
            }
        }
    }

    if runs.is_ok() {
        let failed: Vec<String> = sorted(
            states
                .iter()
                .filter(|(_, s)| **s == CheckState::Failure)
                .map(|(n, _)| n.clone()),
        );
        let pending: Vec<String> = sorted(
            states
                .iter()
                .filter(|(_, s)| **s == CheckState::Pending)
                .map(|(n, _)| n.clone()),
        );
        let total = states.len();
        let (ok, fact) = if total == 0 {
            (None, format!("no checks are reported on {sha}"))
        } else if !failed.is_empty() {
            (
                Some(false),
                format!(
                    "{} of {total} checks failed on {sha}: {}",
                    failed.len(),
                    names(&failed)
                ),
            )
        } else if !pending.is_empty() {
            (
                None,
                format!(
                    "{} of {total} checks are still running on {sha}: {}",
                    pending.len(),
                    names(&pending)
                ),
            )
        } else {
            (Some(true), format!("all {total} checks succeeded on {sha}"))
        };
        log.record(
            "github",
            "check_runs",
            fact,
            ok,
            json!({
                "sha": sha,
                "total": total,
                "failed": failed.iter().take(LISTED).collect::<Vec<_>>(),
                "pending": pending.iter().take(LISTED).collect::<Vec<_>>(),
            }),
        );
    }

    let Some(base) = base_ref else { return };
    let protection = gh
        .api_json(&format!(
            "repos/{repo}/branches/{}/protection/required_status_checks",
            encode_segment(base)
        ))
        .await;
    let v = match protection {
        Ok(v) => v,
        Err(e) => {
            log.unavailable(
                "github",
                "required_checks",
                format!("branch protection on {base} could not be read ({e}); it may need admin access, or none is set"),
            );
            return;
        }
    };
    let mut required: Vec<String> = v
        .get("contexts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| c.as_str().map(str::to_string))
        .chain(
            v.get("checks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|c| c.get("context").and_then(Value::as_str).map(str::to_string)),
        )
        .collect();
    required.sort();
    required.dedup();
    if required.is_empty() {
        log.record(
            "github",
            "required_checks",
            format!("{base} requires no status checks, so passing checks are not a gate"),
            None,
            json!({ "base": base, "required": [] }),
        );
        return;
    }
    let state_of = |name: &String| states.get(name).copied();
    let failed: Vec<String> = required
        .iter()
        .filter(|n| state_of(n) == Some(CheckState::Failure))
        .cloned()
        .collect();
    let waiting: Vec<String> = required
        .iter()
        .filter(|n| matches!(state_of(n), None | Some(CheckState::Pending)))
        .cloned()
        .collect();
    let (ok, fact) = if !failed.is_empty() {
        (
            Some(false),
            format!("required checks failed on {sha}: {}", names(&failed)),
        )
    } else if !waiting.is_empty() {
        (
            None,
            format!(
                "required checks not yet passing on {sha}: {}",
                names(&waiting)
            ),
        )
    } else {
        (
            Some(true),
            format!(
                "all {} required checks on {base} succeeded on {sha}",
                required.len()
            ),
        )
    };
    log.record(
        "github",
        "required_checks",
        fact,
        ok,
        json!({ "base": base, "sha": sha, "required": required, "failed": failed, "waiting": waiting }),
    );
}

fn sorted(items: impl Iterator<Item = String>) -> Vec<String> {
    let mut v: Vec<String> = items.collect();
    v.sort();
    v
}

/// What a pull request's change touches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFacts {
    pub files: Vec<String>,
    /// The change touches a path the owner marked as high blast radius.
    pub tier2: bool,
    /// The scan raised at least one flag, or could not cover the whole change.
    pub flagged: bool,
    pub report: TamperReport,
}

/// Whether `path` is under one of the configured high-risk prefixes. `deploy/` matches
/// `deploy/x.yml` and `infra/deploy/x.yml`.
pub fn touches_tier2_path(path: &str, prefixes: &[String]) -> bool {
    let p = path.trim_start_matches("./").trim_start_matches('/');
    prefixes.iter().any(|raw| {
        let prefix = raw.trim().trim_start_matches("./").trim_start_matches('/');
        !prefix.is_empty() && (p.starts_with(prefix) || p.contains(&format!("/{prefix}")))
    })
}

/// The paths a unified diff changes, in order.
fn changed_paths(diff: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let path = rest
                .rsplit_once(" b/")
                .map(|(_, new)| new.to_string())
                .unwrap_or_else(|| rest.to_string());
            if !out.contains(&path) {
                out.push(path);
            }
        }
    }
    out
}

/// Reads a pull request's diff, records what it changes and scans it for edits that weaken the
/// checks themselves.
pub async fn collect_diff(
    gh: &dyn GitHubRead,
    log: &mut ObservationLog,
    repo: &str,
    number: u64,
    tier2_paths: &[String],
) -> Option<DiffFacts> {
    let diff = match gh.pr_diff(repo, number).await {
        Ok(d) => d,
        Err(e) => {
            log.unavailable(
                "github",
                "diff_scan",
                format!("the diff of {repo}#{number} could not be read ({e})"),
            );
            return None;
        }
    };
    let files = changed_paths(&diff.text);
    let hits: Vec<String> = files
        .iter()
        .filter(|f| touches_tier2_path(f, tier2_paths))
        .cloned()
        .collect();
    log.record(
        "github",
        "changed_paths",
        if hits.is_empty() {
            format!(
                "{repo}#{number} changes {} files, none in a high-risk path",
                files.len()
            )
        } else {
            format!(
                "{repo}#{number} changes {} files, including high-risk paths: {}",
                files.len(),
                names(&hits)
            )
        },
        None,
        json!({
            "count": files.len(),
            "files": files.iter().take(MAX_PATHS).collect::<Vec<_>>(),
            "tier2": !hits.is_empty(),
            "tier2_paths": hits.iter().take(LISTED).collect::<Vec<_>>(),
        }),
    );
    let report = scan_diff(&diff.text, diff.truncated);
    let flagged = !report.is_clean();
    if flagged {
        let kinds: Vec<String> = {
            let mut k: Vec<String> = report.flags.iter().map(|f| f.kind.clone()).collect();
            k.sort();
            k.dedup();
            k
        };
        log.record(
            "diff_scan",
            "diff_scan",
            format!("the change needs a closer look: {}", names(&kinds)),
            None,
            json!({ "flagged": true, "flags": report.flags, "more_flags": report.more_flags, "files_scanned": report.files_scanned }),
        );
    } else {
        log.record(
            "diff_scan",
            "diff_scan",
            format!(
                "scanned {} files; nothing weakens the tests, lints or CI",
                report.files_scanned
            ),
            Some(true),
            json!({ "flagged": false, "files_scanned": report.files_scanned }),
        );
    }
    Some(DiffFacts {
        tier2: !hits.is_empty(),
        files,
        flagged,
        report,
    })
}

/// Records that a note exists and what its content hashes to. The path is relative to the vault
/// and may not leave it.
pub fn collect_note(vault_root: &Path, log: &mut ObservationLog, rel_path: &str) -> Option<String> {
    let rel = rel_path.trim();
    let escapes = rel.is_empty()
        || rel.starts_with('/')
        || rel.contains('\\')
        || rel.split('/').any(|part| part == ".." || part.is_empty());
    if escapes {
        log.unavailable(
            "vault",
            "note_hash",
            format!("{rel_path} is not a path inside the vault"),
        );
        return None;
    }
    let full = vault_root.join(rel);
    match std::fs::read_to_string(&full) {
        Ok(raw) => {
            let hash = hq_db::search::content_hash(&raw);
            log.record(
                "vault",
                "note_hash",
                format!("{rel} exists ({} bytes)", raw.len()),
                Some(true),
                json!({ "path": rel, "content_hash": hash, "bytes": raw.len() }),
            );
            Some(hash)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log.record(
                "vault",
                "note_hash",
                format!("{rel} does not exist"),
                Some(false),
                json!({ "path": rel }),
            );
            None
        }
        Err(e) => {
            log.unavailable(
                "vault",
                "note_hash",
                format!("{rel} could not be read ({e})"),
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::remote::fake::FakeGitHub;
    use super::*;

    fn pr_json(merged: bool, state: &str) -> Value {
        json!({
            "state": state, "merged": merged, "merged_at": merged.then_some("2026-10-20T10:00:00Z"),
            "head": {"sha": "abc123"}, "base": {"ref": "main"}, "user": {"login": "dev"},
            "changed_files": 3, "additions": 10, "deletions": 2,
        })
    }

    #[tokio::test]
    async fn a_merged_pr_supports_a_closed_one_contradicts_and_an_open_one_is_neither() {
        for (merged, state, ok) in [
            (true, "closed", Some(true)),
            (false, "closed", Some(false)),
            (false, "open", None),
        ] {
            let gh = FakeGitHub::default().with("repos/o/r/pulls/7", pr_json(merged, state));
            let mut log = ObservationLog::new();
            let facts = collect_pr(&gh, &mut log, "o/r", 7).await.unwrap();
            assert_eq!(log.items()[0].ok, ok, "{merged} {state}");
            assert_eq!(facts.head_sha.as_deref(), Some("abc123"));
            assert_eq!(facts.base_ref.as_deref(), Some("main"));
            assert_eq!(facts.merged, merged);
        }
    }

    #[tokio::test]
    async fn an_unreadable_pr_is_unavailable_never_a_pass() {
        let mut log = ObservationLog::new();
        assert!(
            collect_pr(&FakeGitHub::default(), &mut log, "o/r", 7)
                .await
                .is_none()
        );
        assert!(log.items()[0].unavailable);
        assert!(!log.supports("pr_state"));
    }

    fn checks(runs: Value) -> FakeGitHub {
        FakeGitHub::default().with(
            "repos/o/r/commits/abc123/check-runs?per_page=100",
            json!({ "check_runs": runs }),
        )
    }

    #[tokio::test]
    async fn checks_all_green_support_any_failure_contradicts_and_running_is_neither() {
        let green = checks(json!([
            {"name": "test", "status": "completed", "conclusion": "success"},
            {"name": "lint", "status": "completed", "conclusion": "skipped"},
        ]));
        let mut log = ObservationLog::new();
        collect_checks(&green, &mut log, "o/r", "abc123", None).await;
        assert_eq!(log.of_kind("check_runs").next().unwrap().ok, Some(true));

        let red = checks(json!([
            {"name": "test", "status": "completed", "conclusion": "failure"},
            {"name": "lint", "status": "completed", "conclusion": "success"},
        ]));
        let mut log = ObservationLog::new();
        collect_checks(&red, &mut log, "o/r", "abc123", None).await;
        let o = log.of_kind("check_runs").next().unwrap();
        assert_eq!(o.ok, Some(false));
        assert!(o.fact.contains("test"));

        let running =
            checks(json!([{"name": "test", "status": "in_progress", "conclusion": null}]));
        let mut log = ObservationLog::new();
        collect_checks(&running, &mut log, "o/r", "abc123", None).await;
        assert_eq!(log.of_kind("check_runs").next().unwrap().ok, None);

        let none = checks(json!([]));
        let mut log = ObservationLog::new();
        collect_checks(&none, &mut log, "o/r", "abc123", None).await;
        let o = log.of_kind("check_runs").next().unwrap();
        assert_eq!(o.ok, None, "no checks is not a pass");
    }

    #[tokio::test]
    async fn the_worse_of_two_runs_with_one_name_decides() {
        let gh = checks(json!([
            {"name": "test", "status": "completed", "conclusion": "success"},
            {"name": "test", "status": "completed", "conclusion": "failure"},
        ]));
        let mut log = ObservationLog::new();
        collect_checks(&gh, &mut log, "o/r", "abc123", None).await;
        assert_eq!(log.of_kind("check_runs").next().unwrap().ok, Some(false));
    }

    #[tokio::test]
    async fn required_checks_must_each_have_passed() {
        let base = "repos/o/r/branches/main/protection/required_status_checks";
        let gh = checks(json!([
            {"name": "test", "status": "completed", "conclusion": "success"},
            {"name": "lint", "status": "completed", "conclusion": "success"},
        ]))
        .with(base, json!({ "contexts": ["test", "lint"] }));
        let mut log = ObservationLog::new();
        collect_checks(&gh, &mut log, "o/r", "abc123", Some("main")).await;
        assert!(log.supports("required_checks"));

        // A required check that never reported is not passing.
        let gh = checks(json!([{"name": "test", "status": "completed", "conclusion": "success"}]))
            .with(base, json!({ "contexts": ["test", "deploy-preview"] }));
        let mut log = ObservationLog::new();
        collect_checks(&gh, &mut log, "o/r", "abc123", Some("main")).await;
        let o = log.of_kind("required_checks").next().unwrap();
        assert_eq!(o.ok, None);
        assert!(o.fact.contains("deploy-preview"));

        // A required check that failed contradicts, even when others passed.
        let gh = checks(json!([
            {"name": "test", "status": "completed", "conclusion": "failure"},
            {"name": "lint", "status": "completed", "conclusion": "success"},
        ]))
        .with(
            base,
            json!({ "checks": [{"context": "test"}, {"context": "lint"}] }),
        );
        let mut log = ObservationLog::new();
        collect_checks(&gh, &mut log, "o/r", "abc123", Some("main")).await;
        assert!(log.contradicts("required_checks"));
    }

    #[tokio::test]
    async fn no_required_checks_and_unreadable_protection_are_each_said_plainly() {
        let gh = checks(json!([{"name": "test", "status": "completed", "conclusion": "success"}]))
            .with(
                "repos/o/r/branches/main/protection/required_status_checks",
                json!({ "contexts": [] }),
            );
        let mut log = ObservationLog::new();
        collect_checks(&gh, &mut log, "o/r", "abc123", Some("main")).await;
        let o = log.of_kind("required_checks").next().unwrap();
        assert!(o.fact.contains("not a gate") && o.ok.is_none() && !o.unavailable);

        let gh = checks(json!([{"name": "test", "status": "completed", "conclusion": "success"}]));
        let mut log = ObservationLog::new();
        collect_checks(&gh, &mut log, "o/r", "abc123", Some("main")).await;
        assert!(log.of_kind("required_checks").next().unwrap().unavailable);
        assert!(!log.supports("required_checks"));
    }

    #[tokio::test]
    async fn commit_statuses_count_like_check_runs() {
        let gh = checks(json!([])).with(
            "repos/o/r/commits/abc123/status",
            json!({ "statuses": [{"context": "ci/build", "state": "failure"}] }),
        );
        let mut log = ObservationLog::new();
        collect_checks(&gh, &mut log, "o/r", "abc123", None).await;
        assert!(log.contradicts("check_runs"));
    }

    #[tokio::test]
    async fn a_branch_name_is_encoded_in_the_protection_path() {
        let gh = checks(json!([])).with(
            "repos/o/r/branches/release%2B1/protection/required_status_checks",
            json!({ "contexts": ["x"] }),
        );
        let mut log = ObservationLog::new();
        collect_checks(&gh, &mut log, "o/r", "abc123", Some("release+1")).await;
        assert!(log.of_kind("required_checks").next().is_some());
        assert!(!log.of_kind("required_checks").next().unwrap().unavailable);
    }

    #[tokio::test]
    async fn the_diff_records_the_paths_and_a_clean_scan() {
        let diff = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-x\n+y\n";
        let gh = FakeGitHub::default().with_diff("o/r", 7, diff, false);
        let mut log = ObservationLog::new();
        let facts = collect_diff(&gh, &mut log, "o/r", 7, &["deploy/".into()])
            .await
            .unwrap();
        assert_eq!(facts.files, ["src/a.rs"]);
        assert!(!facts.tier2 && !facts.flagged);
        assert!(log.supports("diff_scan"));
    }

    #[tokio::test]
    async fn a_flagged_diff_is_neither_a_pass_nor_a_failure_and_a_tier_two_path_is_noticed() {
        let diff = "diff --git a/deploy/app.yml b/deploy/app.yml\n--- a/deploy/app.yml\n+++ b/deploy/app.yml\n@@ -1 +1 @@\n-x\n+y\ndiff --git a/src/a_test.go b/src/a_test.go\n--- a/src/a_test.go\n+++ b/src/a_test.go\n@@ -1 +1 @@\n+\tt.Skip(\"later\")\n";
        let gh = FakeGitHub::default().with_diff("o/r", 7, diff, false);
        let mut log = ObservationLog::new();
        let facts = collect_diff(&gh, &mut log, "o/r", 7, &["deploy/".into()])
            .await
            .unwrap();
        assert!(facts.tier2 && facts.flagged);
        let scan = log.of_kind("diff_scan").next().unwrap();
        assert_eq!(
            scan.ok, None,
            "a flag asks for scrutiny, it does not fail the task"
        );
        assert_eq!(scan.data["flagged"], true);
        assert!(!log.supports("diff_scan"));
        assert_eq!(
            log.of_kind("changed_paths").next().unwrap().data["tier2"],
            true
        );
    }

    #[tokio::test]
    async fn an_unreadable_diff_is_unavailable() {
        let mut log = ObservationLog::new();
        assert!(
            collect_diff(&FakeGitHub::default(), &mut log, "o/r", 7, &[])
                .await
                .is_none()
        );
        assert!(log.items()[0].unavailable && !log.supports("diff_scan"));
    }

    #[test]
    fn tier_two_paths_match_at_the_root_or_below_any_directory() {
        let p = vec![
            "deploy/".to_string(),
            ".github/workflows/".to_string(),
            "migrations/".to_string(),
        ];
        for path in [
            "deploy/a.yml",
            "infra/deploy/a.yml",
            ".github/workflows/ci.yml",
            "db/migrations/001.sql",
            "./deploy/x",
        ] {
            assert!(touches_tier2_path(path, &p), "{path}");
        }
        for path in ["src/deployment.rs", "docs/redeploy.md", "src/a.rs"] {
            assert!(!touches_tier2_path(path, &p), "{path}");
        }
        assert!(
            !touches_tier2_path("anything", &["".to_string()]),
            "an empty prefix matches nothing"
        );
    }

    #[test]
    fn a_note_is_recorded_with_its_hash_and_a_missing_one_contradicts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Notebooks")).unwrap();
        std::fs::write(dir.path().join("Notebooks/a.md"), "# hello").unwrap();
        let mut log = ObservationLog::new();
        let hash = collect_note(dir.path(), &mut log, "Notebooks/a.md").unwrap();
        assert_eq!(hash, hq_db::search::content_hash("# hello"));
        assert!(log.supports("note_hash"));

        let mut log = ObservationLog::new();
        assert!(collect_note(dir.path(), &mut log, "Notebooks/gone.md").is_none());
        assert!(log.contradicts("note_hash"));
    }

    #[test]
    fn a_note_path_cannot_leave_the_vault() {
        let dir = tempfile::tempdir().unwrap();
        for bad in [
            "../secret.md",
            "/etc/passwd",
            "a/../../b.md",
            "a\\b.md",
            "",
            "a//b.md",
        ] {
            let mut log = ObservationLog::new();
            assert!(collect_note(dir.path(), &mut log, bad).is_none(), "{bad}");
            assert!(log.items()[0].unavailable, "{bad}");
        }
    }
}
