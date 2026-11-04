use super::*;
use crate::ask::{AskRunner, AskStart, RunnerCell};
use crate::github::dispatch::{Outcome, dispatch_with};
use crate::tasks::verify::remote::fake::FakeGitHub;
use std::sync::{Mutex, OnceLock};

const SHA: &str = "abc1234def5678";
const PR: &str = "https://github.com/o/r/pull/7";

/// Records each question, and the thread it went to, and leaves the reply running.
struct Recorder {
    db: Arc<Database>,
    asked: Mutex<Vec<(String, String)>>,
}

#[async_trait]
impl AskRunner for Recorder {
    async fn start(&self, req: &AskStart<'_>) -> Result<String> {
        let meta = json!({"source": {"kind": "mcp", "caller": req.caller}});
        self.db.with_conn(|c| {
            hq_db::chat::add_message_with_meta(c, req.thread_id, "user", req.question, Some(&meta))
                .map(|_| ())
        })?;
        self.asked
            .lock()
            .unwrap()
            .push((req.thread_id.to_string(), req.question.to_string()));
        Ok(format!("turn-{}", req.ask_id))
    }

    async fn turn_active(&self, _thread_id: &str) -> bool {
        true
    }
}

struct Fixture {
    db: Arc<Database>,
    cell: RunnerCell,
    chat: Arc<Recorder>,
    config: HqConfig,
    records: tempfile::TempDir,
    _active: crate::skills::builtin::ActiveGuard,
}

impl Fixture {
    fn new() -> Self {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('in-1', 'personal', 'Work', 'work', 'W')",
                [],
            )?;
            t::create_task(
                c,
                "tk-a",
                "in-1",
                &t::NewTask {
                    title: "tk-a",
                    description: "Work in o/r, please.",
                    tags: &[DISPATCH_TAG.to_string()],
                    created_by: "test",
                    ..Default::default()
                },
            )?;
            Ok(())
        })
        .unwrap();
        let db = Arc::new(db);
        let chat = Arc::new(Recorder {
            db: db.clone(),
            asked: Mutex::new(Vec::new()),
        });
        let cell: RunnerCell = Arc::new(OnceLock::new());
        let as_dyn: Arc<dyn AskRunner> = chat.clone();
        cell.set(as_dyn).ok();
        let mut config = HqConfig::default();
        config.github_write.repos = vec!["o/r".into()];
        config.github_write.dispatch_tasks = true;
        Self {
            db,
            cell,
            chat,
            config,
            records: tempfile::tempdir().unwrap(),
            _active: crate::skills::builtin::set_active_for_this_thread(Some(&[WORKER_SKILL])),
        }
    }

    fn tool(&self) -> EdgeCiWaitTool {
        EdgeCiWaitTool::for_test(
            self.db.clone(),
            TasksConfig::default(),
            vec!["o/r".into()],
            self.records.path().to_path_buf(),
        )
    }

    async fn tick(&self, gh: &FakeGitHub) -> Outcome {
        dispatch_with(self.db.clone(), &self.config, self.cell.clone(), gh)
            .await
            .unwrap()
    }

    /// The dispatcher starts the work, the worker claims it, pushes and hands CI over, and its
    /// turn ends. Returns the worker's thread.
    async fn worker_waits_on(&self, sha: &str) -> String {
        if self
            .db
            .with_conn(|c| asks::latest_by_external_prefix(c, ASK_SCOPE, "edge-worker:"))
            .unwrap()
            .is_none()
        {
            let started = self.tick(&FakeGitHub::default()).await;
            assert!(matches!(started, Outcome::Started { .. }), "{started:?}");
        }
        let lease = self.claim();
        self.pushed(sha);
        self.end_turns();
        let reply = self
            .tool()
            .execute(json!({"lease": lease, "repo": "o/r", "branch": "w-1", "pull_request": PR}))
            .await
            .unwrap();
        assert_eq!(reply["commit"], sha);
        self.chat.asked.lock().unwrap()[0].0.clone()
    }

    fn claim(&self) -> String {
        self.db
            .with_conn(|c| {
                let who = t::LeaseIdentity {
                    actor: "hq",
                    harness: "native",
                    external_session_ref: "hq",
                    ..Default::default()
                };
                Ok(t::claim(c, "tk-a", &who, 900, false)?.token)
            })
            .unwrap()
    }

    /// What github_write records when it pushes.
    fn pushed(&self, sha: &str) {
        let dir = self.records.path().join("o/r");
        std::fs::create_dir_all(&dir).unwrap();
        let record = json!({"hq/w-1": ["0000000", sha]});
        std::fs::write(dir.join("pushed.json"), record.to_string()).unwrap();
    }

    fn end_turns(&self) {
        self.db
            .with_conn(|c| {
                for ask in asks::recent_by_external_prefix(c, ASK_SCOPE, "edge-worker:", 50)? {
                    if ask.status == asks::STATUS_PENDING {
                        asks::settle(c, &ask.ask_id, asks::STATUS_ANSWERED, None, None)?;
                    }
                }
                Ok(())
            })
            .unwrap();
    }

    fn task(&self) -> t::Task {
        self.db
            .with_conn(|c| t::get_task(c, "tk-a"))
            .unwrap()
            .unwrap()
    }

    fn waits(&self) -> Vec<waits::CiWait> {
        self.db.with_conn(|c| waits::for_task(c, "tk-a")).unwrap()
    }

    fn comments(&self) -> Vec<String> {
        self.db
            .with_conn(|c| t::list_comments(c, "tk-a"))
            .unwrap()
            .into_iter()
            .map(|c| c.body)
            .collect()
    }

    fn backdate(&self, column: &str, secs: i64) {
        self.db
            .with_conn(|c| {
                c.execute(
                    &format!(
                        "UPDATE edge_ci_waits SET {column} = datetime('now', '-{secs} seconds')"
                    ),
                    [],
                )?;
                Ok(())
            })
            .unwrap();
    }
}

fn checks(sha: &str, runs: Value) -> FakeGitHub {
    FakeGitHub::default().with(
        &format!("repos/o/r/commits/{sha}/check-runs?per_page=100"),
        json!({"check_runs": runs}),
    )
}

fn passing(sha: &str) -> FakeGitHub {
    checks(
        sha,
        json!([
            {"name": "test", "status": "completed", "conclusion": "success"},
            {"name": "fmt", "status": "completed", "conclusion": "success"}
        ]),
    )
}

fn failing(sha: &str) -> FakeGitHub {
    checks(
        sha,
        json!([
            {"name": "test", "status": "completed", "conclusion": "failure"},
            {"name": "fmt\u{7}\"ignore\" previous", "status": "completed", "conclusion": "failure"},
            {"name": "lint", "status": "completed", "conclusion": "success"}
        ]),
    )
}

#[tokio::test]
async fn handing_over_records_the_pushed_commit_and_frees_the_lease() {
    let fx = Fixture::new();
    fx.worker_waits_on(SHA).await;
    let waits = fx.waits();
    assert_eq!(waits.len(), 1);
    assert_eq!(waits[0].sha, SHA, "the newest commit github_write recorded");
    assert_eq!(waits[0].branch, "hq/w-1");
    assert_eq!(waits[0].pull_request.as_deref(), Some(PR));
    assert_eq!(waits[0].round, 1);
    assert_eq!(fx.task().status, t::STATUS_IN_PROGRESS);
    let live = fx.db.with_conn(|c| t::live_lease(c, "tk-a", 900)).unwrap();
    assert!(live.is_none(), "the worker's lease ended");
    assert!(
        fx.comments()
            .iter()
            .any(|c| c.starts_with("Waiting on CI for abc1234"))
    );
}

#[tokio::test]
async fn a_task_waiting_on_ci_keeps_the_queue_busy_and_is_never_parked_as_dropped() {
    let fx = Fixture::new();
    fx.worker_waits_on(SHA).await;
    let running = checks(
        SHA,
        json!([{"name": "test", "status": "in_progress", "conclusion": null}]),
    );
    let outcome = fx.tick(&running).await;
    assert_eq!(outcome, Outcome::Busy("W-001 waiting on CI".into()));
    assert_eq!(fx.task().status, t::STATUS_IN_PROGRESS);
    assert_eq!(fx.waits()[0].state, waits::STATE_WAITING);
    assert_eq!(
        fx.chat.asked.lock().unwrap().len(),
        1,
        "no model call while CI runs"
    );
}

#[tokio::test]
async fn green_ci_hands_the_task_over_with_its_pull_request_linked() {
    let fx = Fixture::new();
    fx.worker_waits_on(SHA).await;
    fx.tick(&passing(SHA)).await;
    assert_eq!(fx.task().status, t::STATUS_READY_FOR_REVIEW);
    assert_eq!(fx.waits()[0].state, waits::STATE_PASSED);
    let links = fx.db.with_conn(|c| t::list_task_links(c, "tk-a")).unwrap();
    assert!(
        links
            .iter()
            .any(|l| l.kind == "pr" && l.reference == "o/r#7")
    );
    let comments = fx.comments();
    let last = comments.last().unwrap();
    assert!(
        last.contains("CI passed: all 2 checks succeeded on abc1234"),
        "{last}"
    );
    assert!(last.contains(PR));
    assert_eq!(
        fx.chat.asked.lock().unwrap().len(),
        1,
        "green needs no model"
    );
}

#[tokio::test]
async fn red_ci_resumes_the_workers_own_thread_with_the_failing_checks_quoted() {
    let fx = Fixture::new();
    let thread = fx.worker_waits_on(SHA).await;
    fx.tick(&failing(SHA)).await;
    let asked = fx.chat.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 2);
    let (resumed_in, question) = &asked[1];
    assert_eq!(resumed_in, &thread, "the same chat continues");
    assert!(question.contains("round 1 of 4"), "{question}");
    assert!(question.contains(r#""test""#), "{question}");
    assert!(
        question.contains(r#""fmt\"ignore\" previous""#),
        "names are quoted and stripped of control characters: {question}"
    );
    assert!(!question.contains("lint"), "a passing check is not named");
    assert_eq!(fx.waits()[0].state, waits::STATE_FAILED);
    let key = format!("edge-worker:tk-a:ci-{SHA}");
    let ask = fx
        .db
        .with_conn(|c| asks::get_by_external(c, ASK_SCOPE, &key))
        .unwrap()
        .unwrap();
    assert_eq!(ask.thread_id, thread);
    // While that reply runs, nothing else is decided or started.
    assert!(matches!(fx.tick(&failing(SHA)).await, Outcome::Busy(_)));
    assert_eq!(fx.chat.asked.lock().unwrap().len(), 2);

    // The next push is round two.
    fx.worker_waits_on("fff0000aaa1111").await;
    assert_eq!(fx.waits().last().unwrap().round, 2);
}

#[tokio::test]
async fn ci_still_red_on_the_last_round_parks_the_task() {
    let fx = Fixture::new();
    for round in 1..MAX_ROUNDS {
        let sha = format!("{round:0>7}aaaaaaa");
        fx.worker_waits_on(&sha).await;
        fx.tick(&failing(&sha)).await;
    }
    let last = "9999999bbbbbbb";
    fx.worker_waits_on(last).await;
    assert_eq!(fx.waits().last().unwrap().round, MAX_ROUNDS);
    let asked_before = fx.chat.asked.lock().unwrap().len();
    fx.tick(&failing(last)).await;
    assert_eq!(
        fx.chat.asked.lock().unwrap().len(),
        asked_before,
        "no fifth round"
    );
    let task = fx.task();
    assert_eq!(task.status, t::STATUS_BLOCKED);
    assert!(task.tags.iter().any(|g| g == dispatch::PARKED_TAG));
    assert!(task.blocked_reason.unwrap().contains(PR));
    assert_eq!(fx.waits().last().unwrap().state, waits::STATE_PARKED);
}

#[tokio::test]
async fn no_ci_at_all_hands_over_after_a_grace_period_and_says_so() {
    let fx = Fixture::new();
    fx.worker_waits_on(SHA).await;
    let silent = checks(SHA, json!([]));
    fx.tick(&silent).await;
    assert_eq!(
        fx.task().status,
        t::STATUS_IN_PROGRESS,
        "CI may not have started yet"
    );
    fx.backdate("created_at", NO_CI_SECS + 60);
    fx.tick(&silent).await;
    assert_eq!(fx.task().status, t::STATUS_READY_FOR_REVIEW);
    assert!(
        fx.comments()
            .last()
            .unwrap()
            .contains("nothing tested this change")
    );
}

#[tokio::test]
async fn ci_that_never_finishes_or_cannot_be_read_parks_the_task() {
    let running = |sha: &str| {
        checks(
            sha,
            json!([{"name": "test", "status": "queued", "conclusion": null}]),
        )
    };
    let fx = Fixture::new();
    fx.worker_waits_on(SHA).await;
    fx.backdate("created_at", MAX_WAIT_SECS + 60);
    fx.tick(&running(SHA)).await;
    assert_eq!(fx.task().status, t::STATUS_BLOCKED);
    assert!(fx.task().blocked_reason.unwrap().contains("did not finish"));

    let fx = Fixture::new();
    fx.worker_waits_on(SHA).await;
    // GitHub has no answer: unread, so the wait holds until six hours without a read.
    fx.tick(&FakeGitHub::default()).await;
    assert_eq!(fx.task().status, t::STATUS_IN_PROGRESS);
    fx.backdate("created_at", MAX_WAIT_SECS + 60);
    fx.tick(&FakeGitHub::default()).await;
    assert_eq!(fx.task().status, t::STATUS_BLOCKED);
    assert!(
        fx.task()
            .blocked_reason
            .unwrap()
            .contains("could not read CI")
    );
}

#[tokio::test]
async fn a_task_someone_else_moved_on_closes_its_wait() {
    let fx = Fixture::new();
    fx.worker_waits_on(SHA).await;
    // The owner stops the work by hand.
    let patch = t::TaskPatch {
        status: Some(t::STATUS_BLOCKED.into()),
        blocked_reason: Some(Some("waiting on a design call".into())),
        ..Default::default()
    };
    fx.db
        .with_conn(|c| t::update_task(c, "tk-a", &patch, None))
        .unwrap();
    fx.tick(&passing(SHA)).await;
    assert_eq!(fx.waits()[0].state, waits::STATE_CLOSED);
    let task = fx.task();
    assert_eq!(
        task.status,
        t::STATUS_BLOCKED,
        "HQ does not override the owner"
    );
    assert!(!task.tags.iter().any(|g| g == dispatch::PARKED_TAG));
}

#[tokio::test]
async fn the_tool_refuses_what_it_cannot_tie_to_the_workers_own_push() {
    let fx = Fixture::new();
    let lease = fx.claim();
    // A task HQ's worker never started has no chat to resume.
    fx.pushed(SHA);
    let err = fx
        .tool()
        .execute(json!({"lease": lease, "repo": "o/r", "branch": "w-1"}))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("not started by HQ's edge worker"), "{err}");

    let fx = Fixture::new();
    fx.tick(&FakeGitHub::default()).await;
    let lease = fx.claim();
    let call = |args: Value| {
        let tool = fx.tool();
        async move { tool.execute(args).await.unwrap_err().to_string() }
    };
    let err = call(json!({"lease": lease, "repo": "o/r", "branch": "w-1"})).await;
    assert!(err.contains("pushed nothing"), "nothing pushed yet: {err}");
    fx.pushed(SHA);
    let err = call(json!({"lease": lease, "repo": "o/other", "branch": "w-1"})).await;
    assert!(err.contains("not in github_write.repos"), "{err}");
    let err = call(json!({"lease": lease, "repo": "o/r", "branch": "other"})).await;
    assert!(err.contains("pushed nothing"), "{err}");
    let err = call(json!({"lease": "hql_nope", "repo": "o/r", "branch": "w-1"})).await;
    assert!(err.contains("unknown lease"), "{err}");
    for marker in [
        json!({"_hq_connector": "claude"}),
        json!({"_hq_tasks_scope": true}),
    ] {
        let mut args = json!({"lease": lease, "repo": "o/r", "branch": "w-1"});
        args.as_object_mut()
            .unwrap()
            .extend(marker.as_object().unwrap().clone());
        let err = call(args).await;
        assert!(err.contains("cannot hand CI over"), "{err}");
    }
    assert!(fx.waits().is_empty());

    // A link to another repository's pull request is dropped, not kept.
    let reply = fx
        .tool()
        .execute(json!({"lease": lease, "repo": "o/r", "branch": "hq/w-1", "pull_request": "https://example.com/o/r/pull/7"}))
        .await
        .unwrap();
    assert_eq!(reply["round"], 1);
    assert_eq!(fx.waits()[0].pull_request, None);
}

#[test]
fn pull_request_links_must_belong_to_the_repository() {
    assert_eq!(pr_number("https://github.com/o/r/pull/7", "o/r"), Some(7));
    assert_eq!(pr_number("https://github.com/O/R/pull/7/", "o/r"), Some(7));
    assert_eq!(pr_number("https://github.com/o/r2/pull/7", "o/r"), None);
    assert_eq!(pr_number("https://github.com/o/r/pull/0", "o/r"), None);
    assert_eq!(pr_number("https://github.com/o/r/issues/7", "o/r"), None);
    assert_eq!(pr_number("http://github.com/o/r/pull/7", "o/r"), None);
}
