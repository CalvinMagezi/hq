//! `harness_session_request_merge`: a helper whose pull request is green asks HQ to merge it.
//! HQ never merges on its own say. It reads the pull request through the host, files one inbox
//! approval naming the repository, title, number, commit and passing checks, and returns
//! "waiting for the owner". The approval is bound to those facts with `args_digest`, so a new
//! commit is a different approval. On the owner's yes the helper calls again and the host
//! re-reads the pull request, requires the approved commit to still be the head and the checks
//! still green, then squash-merges once with HQ's own login. The helper names only the pull
//! request; it reads no approval from its arguments and nothing it can call signs one.

use super::*;
use crate::agent_host::{AgentHostError, MergeAsk, MergeOutcome};
use crate::registry::HqTool;
use crate::util::arg_str;
use async_trait::async_trait;
use hq_core::approval_key::args_digest;
use hq_core::config::PublishConfig;
use hq_core::merge_check::{NEVER_MERGE_REASON, parse_pr_ref};
use hq_core::types::{ValueItem, ValueKind};
use hq_db::value_items::{self, ApprovalGate, RequestState};

const SOURCE: &str = "session-merge";
const KEY_PREFIX: &str = "merge-approval:";
const EVENT_KIND: &str = "merge";
const COMMENT_AUTHOR: &str = "HQ";
/// How long the owner has to answer one request, and how long a no is remembered.
const REQUEST_TTL_HOURS: i64 = 24;
/// Requests waiting at once, so a hijacked helper cannot flood the owner's inbox.
const MAX_WAITING: i64 = 10;
const MAX_TITLE_CHARS: usize = 200;

/// The key one approval is filed and spent under: the repository and number to read, and the
/// digest of exactly what the owner is shown, including the commit.
fn approval_key(ready: &MergeOutcome) -> String {
    let bound = json!({
        "repo": ready.repo.to_ascii_lowercase(),
        "pr": ready.number,
        "head_sha": ready.head_sha,
    });
    format!(
        "{KEY_PREFIX}{}#{}:{}",
        ready.repo.to_ascii_lowercase(),
        ready.number,
        args_digest(&bound)
    )
}

/// A pull request title is the helper's text, shown to the owner: one line, no control or
/// invisible characters, bounded.
fn shown_title(title: &str) -> String {
    let clean: String = title
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .filter(|c| !hq_db::tasks::has_hidden_chars(&c.to_string()))
        .take(MAX_TITLE_CHARS)
        .collect();
    clean.trim().to_string()
}

fn ask_item(ready: &MergeOutcome, key: String) -> ValueItem {
    let mut item = ValueItem::new(
        SOURCE,
        ValueKind::ActionNeeded,
        format!(
            "Merge {}#{}: {}?",
            ready.repo,
            ready.number,
            shown_title(&ready.title)
        ),
        format!(
            "A coding helper asked HQ to squash-merge pull request {url} in {repo} at commit {sha}. \
             Every check passed: {checks}. Approving merges exactly this commit once, with this \
             machine's GitHub login; a new commit or a failing check cancels it and asks you again. \
             Dismiss to refuse.",
            url = ready.url,
            repo = ready.repo,
            sha = ready.head_sha,
            checks = ready.checks.join(", "),
        ),
    )
    .with_dedup_key(key);
    item.expires_at = Some(chrono::Utc::now() + chrono::Duration::hours(REQUEST_TTL_HOURS));
    item
}

/// Spends the owner's approval for this exact request, or files one and says to wait.
fn gate(db: &Database, ready: &MergeOutcome) -> Result<Gate> {
    let key = approval_key(ready);
    match value_items::approval_gate(db, &key)? {
        ApprovalGate::Run => Ok(Gate::Merge),
        ApprovalGate::Pending(RequestState::Declined) => Ok(Gate::Declined),
        ApprovalGate::Pending(RequestState::Wait) => Ok(Gate::Wait),
        ApprovalGate::Pending(RequestState::Ask) => {
            if value_items::count_waiting_with_prefix(db, KEY_PREFIX)? >= MAX_WAITING {
                bail!("too many merge requests are waiting on the owner; wait for them to answer");
            }
            value_items::insert(db, &ask_item(ready, key))?;
            Ok(Gate::Wait)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Gate {
    Merge,
    Wait,
    Declined,
}

fn host_refusal(e: AgentHostError) -> anyhow::Error {
    match e {
        AgentHostError::Api { code, message } => {
            anyhow::anyhow!("merge refused ({code}): {message}")
        }
        other => anyhow::anyhow!("merge failed: {other}"),
    }
}

async fn call(host: &Host, ask: MergeAsk) -> Result<MergeOutcome> {
    let host = host.clone();
    agent_host::blocking(move || host.pr_merge(&ask).map_err(host_refusal)).await?
}

struct Caller {
    host: String,
    task: Option<String>,
}

fn caller_of(db: &Database, session_id: &str) -> Result<Caller> {
    let Some(row) = db.with_conn(|c| registry::get(c, session_id))? else {
        bail!("session {session_id} is not known to HQ");
    };
    Ok(Caller {
        host: row.host,
        task: row.mission_id,
    })
}

/// The refusals that need no host: off, an unreadable pull request, a repository the rules
/// never allow.
fn refuse_early(cfg: &PublishConfig, pr: &str) -> Result<(String, u64)> {
    if !cfg.merge.enabled {
        bail!("merging for helpers is turned off (agent_host.publish.merge.enabled)");
    }
    let Some((repo, number)) = parse_pr_ref(pr) else {
        bail!("`pr` must be a pull request address or <owner>/<repo>#<number>");
    };
    if cfg.never_merges(&repo) {
        bail!("HQ never merges in {repo}: {NEVER_MERGE_REASON}; the owner or the team merges it");
    }
    if !cfg.merge_lists(&repo) {
        bail!(
            "{repo} is not in agent_host.publish.merge.repos (or publish.repos); HQ does not merge there"
        );
    }
    Ok((repo, number))
}

/// The whole flow, with the host passed in so a test can script it.
pub(super) async fn request_merge_for(
    db: &Arc<Database>,
    cfg: &PublishConfig,
    session_id: &str,
    pr: &str,
    host: impl FnOnce(&str) -> Result<Host>,
) -> Result<Value> {
    let (repo, number) = refuse_early(cfg, pr)?;
    let caller = caller_of(db, session_id)?;
    let host = host(&caller.host)?;
    let read = MergeAsk {
        repo: repo.clone(),
        number,
        approved_sha: None,
    };
    let ready = call(&host, read).await?;
    if !ready.repo.eq_ignore_ascii_case(&repo) || ready.number != number {
        bail!("the host answered for a different pull request than the one asked about");
    }
    let (gate_db, seen) = (db.clone(), ready.clone());
    match agent_host::blocking(move || gate(&gate_db, &seen)).await?? {
        Gate::Wait => Ok(waiting(&ready)),
        Gate::Declined => bail!(
            "the owner declined merging {}#{} at {}; do not ask again for this commit, tell the owner what you need",
            ready.repo,
            ready.number,
            ready.head_sha
        ),
        Gate::Merge => {
            let go = MergeAsk {
                repo,
                number,
                approved_sha: Some(ready.head_sha.clone()),
            };
            let done = call(&host, go).await?;
            record(db, session_id, &caller, &done);
            Ok(merged(&done))
        }
    }
}

fn waiting(ready: &MergeOutcome) -> Value {
    json!({
        "status": "waiting_for_owner",
        "repo": ready.repo,
        "pr": ready.number,
        "head_sha": ready.head_sha,
        "message": format!(
            "HQ asked the owner whether to merge {}#{} at {}. Nothing was merged. Tell the user you are \
             waiting on this, then call harness_session_request_merge again once they have answered. \
             A new commit on the branch cancels the request.",
            ready.repo, ready.number, ready.head_sha
        ),
    })
}

fn merged(done: &MergeOutcome) -> Value {
    json!({
        "status": "merged",
        "repo": done.repo,
        "pr": done.number,
        "url": done.url,
        "head_sha": done.head_sha,
    })
}

/// The merge already happened, so a failure to record it is logged and not returned.
fn record(db: &Database, session_id: &str, caller: &Caller, done: &MergeOutcome) {
    let detail = json!({ "repo": done.repo, "pr": done.number, "head_sha": done.head_sha });
    let result = db.with_conn(|c| {
        registry::record_event(
            c,
            session_id,
            EVENT_KIND,
            session_id,
            Some(&detail.to_string()),
        )?;
        if let Some(task) = &caller.task {
            let text = format!(
                "HQ merged {} ({}#{} at {}) after the owner approved it.",
                done.url, done.repo, done.number, done.head_sha
            );
            hq_db::tasks::add_comment(c, task, COMMENT_AUTHOR, &text, None)?;
        }
        Ok(())
    });
    if let Err(e) = result {
        tracing::warn!(session = session_id, error = %e, "merge succeeded but could not be recorded");
    }
}

pub struct HarnessSessionRequestMergeTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionRequestMergeTool {
    fn name(&self) -> &str {
        "harness_session_request_merge"
    }

    fn description(&self) -> &str {
        "Ask HQ to merge your green pull request. HQ never merges by itself: it checks the pull request (open, not a draft, targets the default branch, mergeable, every check passed, no merge commits), asks the owner once for this exact commit, and returns waiting_for_owner. Tell the user, then call again after they answer; if they approved and the pull request is unchanged and still green, HQ squash-merges it once and returns merged. A new commit or a failing check cancels the approval and asks again. Only repositories the owner enabled; a refusal says why."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pr": { "type": "string", "description": "Pull request address, or <owner>/<repo>#<number>" }
            },
            "required": ["pr"]
        })
    }

    fn category(&self) -> &str {
        "harness"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let Some(session) = super::caller_session(&args) else {
            bail!(
                "only a session HQ launched can ask for a merge; this call did not come from one"
            );
        };
        let cfg = super::agent_host_config().publish;
        let pr = arg_str(&args, "pr");
        request_merge_for(&self.db, &cfg, session, &pr, |name| {
            agent_host::host(Some(name))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_host::scripted::ScriptedHost;
    use hq_core::config::MergeConfig;
    use std::os::unix::fs::PermissionsExt;

    const SESSION: &str = "hs-merge";
    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
    const NEW_SHA: &str = "fedcba9876543210fedcba9876543210fedcba98";
    const PR: &str = "acme/widgets#7";

    fn outcome(sha: &str, merged: bool) -> String {
        format!(
            r#"{{"repo":"acme/widgets","number":7,"title":"Add things","url":"https://example.com/acme/widgets/pull/7","head_sha":"{sha}","base":"main","head_ref":"feat/x","checks":["ci","lint"],"merged":{merged}}}"#
        )
    }

    struct Fixture {
        dir: tempfile::TempDir,
        db: Arc<Database>,
    }

    /// A host script: a read (`"approved_sha":null`) prints `plan.json`; a merge prints the
    /// approved commit as merged unless `error.json` exists. Every call is logged.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("plan.json"), outcome(SHA, false)).unwrap();
        let script = "#!/bin/sh\nd=\"$(dirname \"$0\")\"\necho \"$@\" >> \"$d/calls.log\"\n\
            if echo \"$*\" | grep -q 'approved_sha\":null'; then\n\
              if [ -e \"$d/error.json\" ]; then cat \"$d/error.json\" >&2; exit 1; fi\n\
              cat \"$d/plan.json\"; exit 0; fi\n\
            sed 's/\"merged\":false/\"merged\":true/' \"$d/plan.json\"\n";
        let path = dir.path().join("scripted-host");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let db = Arc::new(Database::open_memory().unwrap());
        db.with_conn(|c| {
            registry::insert(
                c,
                &registry::NewSession {
                    id: SESSION,
                    harness: "codex",
                    label: "",
                    cwd: "/t",
                    mission_id: None,
                    placement: registry::Placement {
                        host: "local",
                        agent_name: SESSION,
                        workspace_id: "w",
                        pane_id: "p",
                    },
                    purpose: None,
                },
            )
        })
        .unwrap();
        Fixture { dir, db }
    }

    fn enabled() -> PublishConfig {
        PublishConfig {
            repos: vec!["Acme/Widgets".into()],
            merge: MergeConfig {
                enabled: true,
                repos: Vec::new(),
            },
            ..Default::default()
        }
    }

    impl Fixture {
        async fn ask(&self, cfg: &PublishConfig) -> Result<Value> {
            self.ask_pr(cfg, PR).await
        }

        async fn ask_pr(&self, cfg: &PublishConfig, pr: &str) -> Result<Value> {
            let host: Host = Arc::new(
                ScriptedHost::new(self.dir.path().join("scripted-host")).without_binary_preflight(),
            );
            request_merge_for(&self.db, cfg, SESSION, pr, |_| Ok(host)).await
        }

        fn calls(&self) -> String {
            std::fs::read_to_string(self.dir.path().join("calls.log")).unwrap_or_default()
        }

        fn merges(&self) -> usize {
            self.calls().matches("approved_sha\":\"").count()
        }

        fn set_head(&self, sha: &str) {
            std::fs::write(self.dir.path().join("plan.json"), outcome(sha, false)).unwrap();
        }

        fn requests(&self) -> Vec<(String, String)> {
            self.db
                .with_conn(|c| {
                    let mut stmt = c.prepare(
                        "SELECT id, body FROM value_items WHERE dedup_key LIKE 'merge-approval:%' ORDER BY rowid",
                    )?;
                    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
                })
                .unwrap()
        }
    }

    async fn err(f: &Fixture, cfg: &PublishConfig) -> String {
        f.ask(cfg).await.unwrap_err().to_string()
    }

    #[tokio::test]
    async fn off_by_default_and_nothing_reaches_the_host() {
        let f = fixture();
        let off = PublishConfig {
            repos: vec!["acme/widgets".into()],
            ..Default::default()
        };
        assert!(err(&f, &off).await.contains("turned off"));
        assert!(f.calls().is_empty());
    }

    #[tokio::test]
    async fn a_repository_off_the_list_or_on_never_merge_is_refused() {
        let f = fixture();
        let unlisted = PublishConfig {
            repos: Vec::new(),
            ..enabled()
        };
        assert!(err(&f, &unlisted).await.contains("does not merge there"));
        let own_list = PublishConfig {
            merge: MergeConfig {
                enabled: true,
                repos: vec!["acme/other".into()],
            },
            ..enabled()
        };
        assert!(err(&f, &own_list).await.contains("does not merge there"));
        let never = PublishConfig {
            never_merge: vec!["ACME/widgets".into()],
            ..enabled()
        };
        let msg = err(&f, &never).await;
        assert!(
            msg.contains("never merges") && msg.contains("never merge them"),
            "{msg}"
        );
        assert!(f.calls().is_empty(), "{}", f.calls());
    }

    #[tokio::test]
    async fn a_pull_request_that_is_not_ready_is_refused_with_the_hosts_reason() {
        let f = fixture();
        std::fs::write(
            f.dir.path().join("error.json"),
            r#"{"error":{"code":"refused","message":"checks failed: lint"}}"#,
        )
        .unwrap();
        let msg = err(&f, &enabled()).await;
        assert!(
            msg.contains("merge refused") && msg.contains("checks failed: lint"),
            "{msg}"
        );
        assert!(
            f.requests().is_empty(),
            "no approval is asked for a red pull request"
        );
    }

    #[tokio::test]
    async fn a_ready_pull_request_asks_the_owner_once_and_merges_nothing() {
        let f = fixture();
        let first = f.ask(&enabled()).await.unwrap();
        assert_eq!(first["status"], "waiting_for_owner");
        assert_eq!(
            f.ask(&enabled()).await.unwrap()["status"],
            "waiting_for_owner"
        );
        let requests = f.requests();
        assert_eq!(requests.len(), 1, "a waiting request is not filed twice");
        for want in ["acme/widgets", SHA, "ci, lint", "pull/7"] {
            assert!(requests[0].1.contains(want), "{want} in {}", requests[0].1);
        }
        assert_eq!(f.merges(), 0, "{}", f.calls());
    }

    #[tokio::test]
    async fn the_owners_yes_merges_once_and_is_spent() {
        let f = fixture();
        f.ask(&enabled()).await.unwrap();
        let id = f.requests().remove(0).0;
        assert!(value_items::approve_by_id(&f.db, &id).unwrap());
        let done = f.ask(&enabled()).await.unwrap();
        assert_eq!(done["status"], "merged");
        assert_eq!(f.merges(), 1);
        assert!(
            f.calls().contains(&format!("\"approved_sha\":\"{SHA}\"")),
            "{}",
            f.calls()
        );
        let again = f.ask(&enabled()).await.unwrap();
        assert_eq!(
            again["status"], "waiting_for_owner",
            "the yes was one merge, not a standing one"
        );
        assert_eq!(f.merges(), 1);
        let events =
            f.db.with_conn(|c| registry::list_events(c, SESSION, 10))
                .unwrap();
        assert!(events.iter().any(|e| e.kind == "merge"));
    }

    #[tokio::test]
    async fn a_new_commit_voids_the_approval_and_asks_again() {
        let f = fixture();
        f.ask(&enabled()).await.unwrap();
        let id = f.requests().remove(0).0;
        assert!(value_items::approve_by_id(&f.db, &id).unwrap());
        f.set_head(NEW_SHA);
        let said = f.ask(&enabled()).await.unwrap();
        assert_eq!(said["status"], "waiting_for_owner");
        assert_eq!(f.merges(), 0, "{}", f.calls());
        let requests = f.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].1.contains(NEW_SHA));
    }

    #[tokio::test]
    async fn a_decline_is_a_hard_no_for_that_commit_only() {
        let f = fixture();
        f.ask(&enabled()).await.unwrap();
        let id = f.requests().remove(0).0;
        assert!(value_items::dismiss_by_id(&f.db, &id).unwrap());
        assert!(err(&f, &enabled()).await.contains("declined"));
        f.set_head(NEW_SHA);
        assert_eq!(
            f.ask(&enabled()).await.unwrap()["status"],
            "waiting_for_owner"
        );
    }

    #[tokio::test]
    async fn a_helper_cannot_approve_for_the_owner() {
        let f = fixture();
        let tool = HarnessSessionRequestMergeTool { db: f.db.clone() };
        let schema = tool.parameters();
        let props: Vec<&String> = schema["properties"].as_object().unwrap().keys().collect();
        assert_eq!(
            props,
            ["pr"],
            "the helper names the pull request and nothing else"
        );
        let named = json!({ "pr": PR, "approved": true, "approved_sha": SHA, "session_id": SESSION, "from_session": SESSION });
        let said = tool.execute(named).await.unwrap_err().to_string();
        assert!(said.contains("only a session HQ launched"), "{said}");
        assert!(f.calls().is_empty());
        // The flow itself never takes an approval from its inputs: with no owner yes it only waits.
        let said = f.ask(&enabled()).await.unwrap();
        assert_eq!(said["status"], "waiting_for_owner");
        assert_eq!(f.merges(), 0);
    }

    #[tokio::test]
    async fn an_unreadable_pull_request_or_an_unknown_session_is_refused() {
        let f = fixture();
        for bad in [
            "",
            "acme/widgets",
            "acme/widgets#0",
            "https://example.com/acme/widgets/issues/7",
        ] {
            let msg = f.ask_pr(&enabled(), bad).await.unwrap_err().to_string();
            assert!(msg.contains("`pr` must be"), "{bad}: {msg}");
        }
        let host: Host = Arc::new(
            ScriptedHost::new(f.dir.path().join("scripted-host")).without_binary_preflight(),
        );
        let unknown = request_merge_for(&f.db, &enabled(), "hs-nope", PR, |_| Ok(host)).await;
        assert!(unknown.unwrap_err().to_string().contains("not known"));
        assert!(f.calls().is_empty());
    }

    #[test]
    fn titles_shown_to_the_owner_are_one_clean_bounded_line() {
        assert_eq!(shown_title("Fix\nthe\tthing"), "Fix the thing");
        assert!(shown_title(&"x".repeat(500)).chars().count() <= MAX_TITLE_CHARS);
    }
}
