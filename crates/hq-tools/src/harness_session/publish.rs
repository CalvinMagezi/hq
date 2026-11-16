//! `harness_session_publish`: a sandboxed helper that committed work in its worktree asks HQ to
//! push the branch and open the pull request. The host does both with its own login after
//! mechanical checks (`hq_host::publish`), so the helper never holds a credential. The session is
//! the one the gateway attested, never one the model names. A repository the owner has not
//! allowed asks the owner once, through the same inbox approval as a new folder.

use super::*;
use crate::agent_host::{AgentHostError, PublishAsk, PublishOutcome};
use crate::registry::HqTool;
use crate::util::arg_str;
use async_trait::async_trait;
use hq_core::config::PublishConfig;
use hq_core::publish_check::{MAX_BODY_CHARS, MAX_TITLE_CHARS, env_secrets, holds_secret};
use hq_core::types::{ValueItem, ValueKind};
use hq_db::{session_checkouts, value_items};
use value_items::RequestState;

const SOURCE: &str = "session-publish";
const KEY_PREFIX: &str = "publish-grant:";
const EVENT_KIND: &str = "publish";
/// How long the owner's yes for one repository lasts, and how long a request or a no is kept.
const GRANT_DAYS: i64 = 30;
const COMMENT_AUTHOR: &str = "HQ";

/// Where a request for a repository stands with the owner.
#[derive(Debug, PartialEq, Eq)]
enum Gate {
    Allowed,
    Pending,
    Declined,
}

fn request_key(repo: &str) -> String {
    format!("{KEY_PREFIX}{}", repo.to_ascii_lowercase())
}

/// The owner's standing answer for `repo`, filing one request when there is none. A request
/// already waiting is never filed twice.
fn gate(db: &Database, repo: &str) -> Result<Gate> {
    let key = request_key(repo);
    if value_items::is_signed_approval(db, &key)?
        && super::folder_request::approval_unspent(db, &key)?
    {
        return Ok(Gate::Allowed);
    }
    let latest = value_items::latest_engagement_by_dedup(db, &key)?;
    match RequestState::from_latest(latest, chrono::Utc::now()) {
        RequestState::Declined => Ok(Gate::Declined),
        RequestState::Wait => Ok(Gate::Pending),
        RequestState::Ask => {
            let mut item = ValueItem::new(
                SOURCE,
                ValueKind::ActionNeeded,
                format!("Allow HQ to open PRs for {repo} for helpers?"),
                format!(
                    "A coding helper finished work in {repo} and asked HQ to push its branch and open a \
                     pull request with this machine's GitHub login. HQ checks the change first and never \
                     merges, forces, touches the default branch or publishes workflow files. Approving \
                     allows this for every helper for {GRANT_DAYS} days. Dismiss to refuse."
                ),
            )
            .with_dedup_key(key);
            item.expires_at = Some(chrono::Utc::now() + chrono::Duration::days(GRANT_DAYS));
            value_items::insert(db, &item)?;
            Ok(Gate::Pending)
        }
    }
}

/// Publishes since a day ago, which `max_per_session_per_day` limits.
fn published_today(db: &Database, session_id: &str) -> Result<i64> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT COUNT(*) FROM harness_session_events
             WHERE session_id = ?1 AND kind = ?2 AND created_at > datetime('now', '-1 day')",
            rusqlite::params![session_id, EVENT_KIND],
            |r| r.get(0),
        )?)
    })
}

fn check_text(title: &str, body: &str) -> Result<()> {
    if title.trim().is_empty() {
        bail!("`title` is required");
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        bail!("`title` is longer than {MAX_TITLE_CHARS} characters");
    }
    if body.chars().count() > MAX_BODY_CHARS {
        bail!("`body` is longer than {MAX_BODY_CHARS} characters");
    }
    let secrets = env_secrets();
    for (name, text) in [("title", title), ("body", body)] {
        if holds_secret(text, &secrets) {
            bail!("`{name}` holds something shaped like a credential; nothing was published");
        }
    }
    Ok(())
}

/// What the host said, as a refusal the helper can act on.
fn host_refusal(e: AgentHostError) -> anyhow::Error {
    match e {
        AgentHostError::Api { code, message } => {
            anyhow::anyhow!("publish refused ({code}): {message}")
        }
        other => anyhow::anyhow!("publish failed: {other}"),
    }
}

struct Worktree {
    host: String,
    path: String,
    branch: String,
    task: Option<String>,
}

fn worktree_of(db: &Database, session_id: &str) -> Result<Worktree> {
    let Some(row) = db.with_conn(|c| registry::get(c, session_id))? else {
        bail!("session {session_id} is not known to HQ");
    };
    let checkout = db.with_conn(|c| session_checkouts::get(c, session_id))?;
    let Some(checkout) = checkout.filter(|c| c.removed_at.is_none()) else {
        bail!("this session has no worktree of its own to publish from");
    };
    let (Some(path), Some(branch)) = (checkout.worktree, checkout.branch) else {
        bail!(
            "only a session working on a branch in its own worktree can publish; a reviewer cannot"
        );
    };
    Ok(Worktree {
        host: row.host,
        path,
        branch,
        task: row.mission_id,
    })
}

/// The whole flow, with the host passed in so a test can script it.
pub(super) async fn publish_for(
    db: &Arc<Database>,
    cfg: &PublishConfig,
    session_id: &str,
    (title, body): (&str, &str),
    host: impl FnOnce(&str) -> Result<Host>,
) -> Result<Value> {
    if !cfg.enabled {
        bail!("publishing for helpers is turned off (agent_host.publish.enabled)");
    }
    check_text(title, body)?;
    let wt = worktree_of(db, session_id)?;
    if published_today(db, session_id)? >= i64::from(cfg.per_day()) {
        bail!(
            "this session already published {} times today; the limit is agent_host.publish.max_per_session_per_day",
            cfg.per_day()
        );
    }
    let host = host(&wt.host)?;
    let mut ask = PublishAsk {
        path: wt.path.clone(),
        branch: wt.branch.clone(),
        title: title.into(),
        body: body.into(),
        repo_slug: None,
        check_only: true,
        max_files: cfg.files(),
        max_bytes: cfg.bytes(),
    };
    let plan = call(&host, &ask).await?;
    if !cfg.lists(&plan.repo) {
        let (db, repo) = (db.clone(), plan.repo.clone());
        match agent_host::blocking(move || gate(&db, &repo)).await?? {
            Gate::Allowed => {}
            Gate::Pending => return Ok(pending(&plan.repo)),
            Gate::Declined => bail!(
                "the owner declined pull requests for {} from helpers; do not ask again, tell the owner what you need",
                plan.repo
            ),
        }
    }
    ask.check_only = false;
    ask.repo_slug = Some(plan.repo.clone());
    let done = call(&host, &ask).await?;
    record(db, session_id, &wt, &done);
    Ok(published(&done))
}

async fn call(host: &Host, ask: &PublishAsk) -> Result<PublishOutcome> {
    let (host, ask) = (host.clone(), ask.clone());
    agent_host::blocking(move || host.worktree_publish(&ask).map_err(host_refusal)).await?
}

fn pending(repo: &str) -> Value {
    json!({
        "status": "pending_owner_approval",
        "repo": repo,
        "message": format!(
            "HQ asked the owner to allow pull requests for {repo}. Nothing was pushed. Tell the user you \
             are waiting on this, then call harness_session_publish again once they have answered."
        ),
    })
}

fn published(done: &PublishOutcome) -> Value {
    json!({
        "status": "published",
        "url": done.url,
        "repo": done.repo,
        "branch": done.branch,
        "base": done.base,
        "files": done.files,
        "commits": done.commits,
        "existing_pull_request": done.existing,
    })
}

/// The PR address goes on the session's task and into its event log. Neither is allowed to
/// undo a publish that already happened, so a failure here is logged and not returned.
fn record(db: &Database, session_id: &str, wt: &Worktree, done: &PublishOutcome) {
    let url = done.url.as_deref().unwrap_or_default();
    let detail =
        json!({ "repo": done.repo, "branch": done.branch, "url": url, "files": done.files });
    let result = db.with_conn(|c| {
        registry::record_event(
            c,
            session_id,
            EVENT_KIND,
            session_id,
            Some(&detail.to_string()),
        )?;
        if let Some(task) = &wt.task {
            let text = format!(
                "HQ opened a pull request for {} ({}): {url}",
                done.branch, done.repo
            );
            hq_db::tasks::add_comment(c, task, COMMENT_AUTHOR, &text, None)?;
        }
        Ok(())
    });
    if let Err(e) = result {
        tracing::warn!(session = session_id, error = %e, "publish succeeded but could not be recorded");
    }
}

pub struct HarnessSessionPublishTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionPublishTool {
    fn name(&self) -> &str {
        "harness_session_publish"
    }

    fn description(&self) -> &str {
        "Ask HQ to publish the branch you committed in your own worktree: HQ checks it, pushes it and opens a pull request with the host's GitHub login, so you need no credential. Commit first; uncommitted changes are refused. Never merges, never touches the default branch, never publishes .github/ or other CI and hook files, files over the size cap, or anything shaped like a credential. A repository the owner has not allowed asks the owner once and returns pending_owner_approval: tell the user, then call again. A refusal says exactly why; fix that and call again. Returns the pull request address."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Pull request title" },
                "body": { "type": "string", "description": "Pull request description" }
            },
            "required": ["title"]
        })
    }

    fn category(&self) -> &str {
        "harness"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let Some(session) = super::caller_session(&args) else {
            bail!("only a session HQ launched can publish; this call did not come from one");
        };
        let cfg = super::agent_host_config().publish;
        let (title, body) = (arg_str(&args, "title"), arg_str(&args, "body"));
        publish_for(&self.db, &cfg, session, (&title, &body), |name| {
            agent_host::host(Some(name))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_host::scripted::ScriptedHost;
    use std::os::unix::fs::PermissionsExt;

    const SESSION: &str = "hs-pub";
    const URL: &str = "https://example.com/acme/widgets/pull/7";
    const PLAN: &str = r#"{"repo":"acme/widgets","branch":"feat/x","base":"main","files":2,"bytes":10,"commits":1}"#;
    const DONE: &str = r#"{"repo":"acme/widgets","branch":"feat/x","base":"main","files":2,"bytes":10,"commits":1,"url":"https://example.com/acme/widgets/pull/7","existing":false}"#;
    const DENIED: &str = r#"{"error":{"code":"refused","message":".github/ci.yml is never published: CI configuration"}}"#;

    struct Fixture {
        dir: tempfile::TempDir,
        db: Arc<Database>,
    }

    /// A host script that answers `worktree publish` with the plan for a check-only ask and the
    /// outcome for a real one, and logs every call.
    fn fixture(reply_real: &str) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("plan.json"), PLAN).unwrap();
        std::fs::write(dir.path().join("real.json"), reply_real).unwrap();
        let script = "#!/bin/sh\nd=\"$(dirname \"$0\")\"\necho \"$@\" >> \"$d/calls.log\"\n\
            if echo \"$*\" | grep -q 'check_only\":true'; then cat \"$d/plan.json\"; exit 0; fi\n\
            if grep -q '\"error\"' \"$d/real.json\"; then cat \"$d/real.json\" >&2; exit 1; fi\n\
            cat \"$d/real.json\"\n";
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
            )?;
            session_checkouts::record(
                c,
                &session_checkouts::NewCheckout {
                    session_id: SESSION,
                    folder: "/t",
                    worktree: Some("/h/HQ/worktrees/t/hs-pub"),
                    repo: Some("/t"),
                    branch: Some("feat/x"),
                    detached: None,
                    owns: &[],
                    head: Some("abc"),
                },
            )
        })
        .unwrap();
        Fixture { dir, db }
    }

    impl Fixture {
        fn host(&self) -> Host {
            Arc::new(
                ScriptedHost::new(self.dir.path().join("scripted-host")).without_binary_preflight(),
            )
        }

        async fn publish(&self, cfg: &PublishConfig) -> Result<Value> {
            let host = self.host();
            publish_for(&self.db, cfg, SESSION, ("Add things", "Because."), |_| {
                Ok(host)
            })
            .await
        }

        fn calls(&self) -> String {
            std::fs::read_to_string(self.dir.path().join("calls.log")).unwrap_or_default()
        }

        fn requests(&self) -> Vec<String> {
            self.db
                .with_conn(|c| {
                    let mut stmt = c.prepare("SELECT id FROM value_items WHERE dedup_key = ?1")?;
                    let rows = stmt.query_map([request_key("acme/widgets")], |r| r.get(0))?;
                    Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
                })
                .unwrap()
        }
    }

    fn listed() -> PublishConfig {
        PublishConfig {
            repos: vec!["Acme/Widgets".into()],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_listed_repository_publishes_and_the_url_is_recorded() {
        let f = fixture(DONE);
        let said = f.publish(&listed()).await.unwrap();
        assert_eq!(said["status"], "published");
        assert_eq!(said["url"], URL);
        let calls = f.calls();
        assert_eq!(calls.matches("worktree publish").count(), 2, "{calls}");
        assert!(calls.contains("\"repo_slug\":\"acme/widgets\""), "{calls}");
        let events =
            f.db.with_conn(|c| registry::list_events(c, SESSION, 10))
                .unwrap();
        assert!(
            events.iter().any(
                |e| e.kind == "publish" && e.detail.as_deref().is_some_and(|d| d.contains(URL))
            )
        );
    }

    #[tokio::test]
    async fn an_unlisted_repository_asks_the_owner_once_and_pushes_nothing() {
        let f = fixture(DONE);
        let cfg = PublishConfig::default();
        let first = f.publish(&cfg).await.unwrap();
        assert_eq!(first["status"], "pending_owner_approval");
        let again = f.publish(&cfg).await.unwrap();
        assert_eq!(again["status"], "pending_owner_approval");
        assert_eq!(
            f.requests().len(),
            1,
            "a waiting request is not filed twice"
        );
        assert!(!f.calls().contains("\"check_only\":false"), "{}", f.calls());
    }

    #[tokio::test]
    async fn the_owners_approval_lets_the_next_call_publish() {
        let f = fixture(DONE);
        let cfg = PublishConfig::default();
        f.publish(&cfg).await.unwrap();
        let id = f.requests().remove(0);
        assert!(value_items::approve_by_id(&f.db, &id).unwrap());
        let said = f.publish(&cfg).await.unwrap();
        assert_eq!(said["status"], "published");
        let again = f.publish(&cfg).await.unwrap();
        assert_eq!(
            again["status"], "published",
            "the yes is a standing one, not spent"
        );
    }

    #[tokio::test]
    async fn a_declined_repository_is_a_hard_no() {
        let f = fixture(DONE);
        let cfg = PublishConfig::default();
        f.publish(&cfg).await.unwrap();
        let id = f.requests().remove(0);
        assert!(value_items::dismiss_by_id(&f.db, &id).unwrap());
        let err = f.publish(&cfg).await.unwrap_err().to_string();
        assert!(err.contains("declined"), "{err}");
    }

    #[tokio::test]
    async fn a_host_refusal_reaches_the_helper_with_its_reason() {
        let f = fixture(DENIED);
        let err = f.publish(&listed()).await.unwrap_err().to_string();
        assert!(
            err.contains("refused") && err.contains(".github/ci.yml is never published"),
            "{err}"
        );
        let events =
            f.db.with_conn(|c| registry::list_events(c, SESSION, 10))
                .unwrap();
        assert!(events.iter().all(|e| e.kind != "publish"));
    }

    #[tokio::test]
    async fn the_day_limit_stops_a_session() {
        let f = fixture(DONE);
        let cfg = PublishConfig {
            max_per_session_per_day: 1,
            ..listed()
        };
        f.publish(&cfg).await.unwrap();
        let err = f.publish(&cfg).await.unwrap_err().to_string();
        assert!(err.contains("already published"), "{err}");
    }

    #[tokio::test]
    async fn off_means_off_and_text_that_looks_like_a_credential_is_refused() {
        let f = fixture(DONE);
        let off = PublishConfig {
            enabled: false,
            ..listed()
        };
        assert!(
            f.publish(&off)
                .await
                .unwrap_err()
                .to_string()
                .contains("turned off")
        );
        let host = f.host();
        let leaked = publish_for(
            &f.db,
            &listed(),
            SESSION,
            ("t", "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789"),
            |_| Ok(host),
        )
        .await;
        assert!(leaked.unwrap_err().to_string().contains("credential"));
        assert!(f.calls().is_empty(), "nothing reached the host");
    }

    #[tokio::test]
    async fn a_session_without_its_own_branch_cannot_publish() {
        let f = fixture(DONE);
        f.db.with_conn(|c| {
            session_checkouts::record(
                c,
                &session_checkouts::NewCheckout {
                    session_id: SESSION,
                    folder: "/t",
                    worktree: Some("/h/HQ/worktrees/t/hs-pub"),
                    repo: Some("/t"),
                    branch: None,
                    detached: Some("abc"),
                    owns: &[],
                    head: Some("abc"),
                },
            )
        })
        .unwrap();
        let err = f.publish(&listed()).await.unwrap_err().to_string();
        assert!(err.contains("working on a branch"), "{err}");
    }

    #[tokio::test]
    async fn the_caller_is_the_attested_session_never_one_the_model_names() {
        let f = fixture(DONE);
        let tool = HarnessSessionPublishTool { db: f.db.clone() };
        let named = json!({ "title": "t", "session_id": SESSION, "session": SESSION, "from_session": SESSION });
        let err = tool.execute(named).await.unwrap_err().to_string();
        assert!(err.contains("only a session HQ launched"), "{err}");
        assert!(f.calls().is_empty());
        let schema = tool.parameters().to_string();
        assert!(
            !schema.contains("session") && !schema.contains("path") && !schema.contains("branch")
        );
    }
}
