use super::*;
use crate::ask::{AskRunner, AskStart, RunnerCell};
use crate::github::app::{AppRecord, Store};
use crate::github::repos::SCAN_FILE;
use std::sync::{Mutex, OnceLock};

/// Records each question and leaves the reply running, as a real chat would while it works.
struct Recorder {
    db: Arc<Database>,
    questions: Mutex<Vec<String>>,
    untrusted: Mutex<Vec<bool>>,
    models: Mutex<Vec<Option<String>>>,
    budgets: Mutex<Vec<Option<f64>>>,
    presets: Mutex<Vec<bool>>,
}

#[async_trait::async_trait]
impl AskRunner for Recorder {
    async fn start(&self, req: &AskStart<'_>) -> Result<String> {
        let meta = json!({"source": {"kind": "mcp", "caller": req.caller}});
        self.db.with_conn(|c| {
            hq_db::chat::add_message_with_meta(c, req.thread_id, "user", req.question, Some(&meta))
                .map(|_| ())
        })?;
        self.questions
            .lock()
            .unwrap()
            .push(req.question.to_string());
        self.untrusted.lock().unwrap().push(req.untrusted);
        self.models
            .lock()
            .unwrap()
            .push(req.model.map(str::to_string));
        self.budgets.lock().unwrap().push(req.budget_usd);
        self.presets.lock().unwrap().push(req.worker_preset);
        Ok(format!("turn-{}", req.ask_id))
    }

    async fn turn_active(&self, _thread_id: &str) -> bool {
        true
    }
}

fn chat(db: &Arc<Database>) -> (RunnerCell, Arc<Recorder>) {
    let recorder = Arc::new(Recorder {
        db: db.clone(),
        questions: Mutex::new(Vec::new()),
        untrusted: Mutex::new(Vec::new()),
        models: Mutex::new(Vec::new()),
        budgets: Mutex::new(Vec::new()),
        presets: Mutex::new(Vec::new()),
    });
    let cell: RunnerCell = Arc::new(OnceLock::new());
    let as_dyn: Arc<dyn AskRunner> = recorder.clone();
    cell.set(as_dyn).ok();
    (cell, recorder)
}

/// Shadows the real entry point so these tests never read the developer's own App.
async fn dispatch(db: Arc<Database>, config: &HqConfig, runner: RunnerCell) -> Result<Outcome> {
    let nowhere = std::env::temp_dir().join(format!("hq-no-app-{}", uuid::Uuid::new_v4().simple()));
    let app = AppClient::new(Store::at(nowhere), "http://127.0.0.1:1/")?;
    dispatch_with(db, config, runner, &app).await
}

struct Connected {
    _dir: tempfile::TempDir,
    app: AppClient,
}

fn connected(confirmed: bool, needs_reconnect: bool, secrets: &[&str]) -> Connected {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(dir.path().join("github"));
    store
        .save_record(&AppRecord {
            app_id: 1,
            client_id: "Iv-test".into(),
            slug: "hq-coder-abc".into(),
            owner_login: "o".into(),
            owner_id: 42,
            owner_name: None,
            installation_id: Some(9),
            repository_selection: Some("selected".into()),
            needs_reconnect,
            first_task_confirmed: confirmed,
        })
        .unwrap();
    let scanned = json!([{"repo": "o/r", "private": true, "default_branch": "main",
        "protected": false, "has_ci": true, "secrets": secrets, "previews": [], "unchecked": false}]);
    store
        .write_private(SCAN_FILE, scanned.to_string().as_bytes())
        .unwrap();
    let app = AppClient::new(store, "http://127.0.0.1:1/").unwrap();
    Connected { _dir: dir, app }
}

fn by_default() -> HqConfig {
    HqConfig::default()
}

#[test]
fn dispatch_is_on_by_default_and_arms_only_with_something_to_work_on() {
    let _active = worker_on();
    assert!(by_default().github_write.dispatch_tasks);
    let nowhere = tempfile::tempdir().unwrap();
    let empty = AppClient::new(
        Store::at(nowhere.path().join("github")),
        "http://127.0.0.1:1/",
    )
    .unwrap();
    assert!(!armed(&by_default(), &empty), "nothing connected");
    assert!(armed(&by_default(), &connected(true, false, &[]).app));
}

#[tokio::test]
async fn the_first_task_after_connecting_waits_for_the_owner() {
    let _active = worker_on();
    let db = Arc::new(db());
    let (cell, recorder) = chat(&db);
    let c = connected(false, false, &[]);
    add(&db, "tk-a", &[DISPATCH_TAG], &[], None);

    let held = dispatch_with(db.clone(), &by_default(), cell.clone(), &c.app)
        .await
        .unwrap();
    assert!(matches!(held, Outcome::AwaitingConfirmation(_)), "{held:?}");
    assert!(recorder.questions.lock().unwrap().is_empty());
    assert_eq!(task(&db, "tk-a").status, t::STATUS_TO_DO);

    assert!(c.app.confirm_first_task().unwrap());
    let started = dispatch_with(db, &by_default(), cell, &c.app)
        .await
        .unwrap();
    assert!(matches!(started, Outcome::Started { .. }), "{started:?}");
}

#[tokio::test]
async fn a_flagged_repository_is_not_taken() {
    let _active = worker_on();
    let db = Arc::new(db());
    let (cell, recorder) = chat(&db);
    let c = connected(true, false, &["FOO"]);
    add(&db, "tk-a", &[DISPATCH_TAG], &[], None);
    let outcome = dispatch_with(db, &by_default(), cell, &c.app)
        .await
        .unwrap();
    assert_eq!(outcome, Outcome::Idle);
    assert!(recorder.questions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_dead_connection_takes_nothing_even_with_a_config_list() {
    let _active = worker_on();
    let db = Arc::new(db());
    let (cell, recorder) = chat(&db);
    let c = connected(true, true, &[]);
    add(&db, "tk-a", &[DISPATCH_TAG], &[], None);
    let outcome = dispatch_with(db, &on(), cell, &c.app).await.unwrap();
    assert_eq!(outcome, Outcome::Idle);
    assert!(recorder.questions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_config_list_narrows_the_apps_repositories() {
    let c = connected(true, false, &[]);
    let mut narrowed = by_default();
    narrowed.github_write.repos = vec!["o/other".into()];
    assert!(allowed_repos(&narrowed, &c.app).is_empty());
    assert_eq!(allowed_repos(&by_default(), &c.app), ["o/r"]);
}

fn no_chat() -> RunnerCell {
    Arc::new(OnceLock::new())
}

fn db() -> Database {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('in-1', 'personal', 'Work', 'work', 'W')",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    db
}

fn add(db: &Database, id: &str, tags: &[&str], assignees: &[&str], priority: Option<&str>) {
    add_about(db, id, tags, assignees, priority, "Work in O/R, please.");
}

fn add_about(
    db: &Database,
    id: &str,
    tags: &[&str],
    assignees: &[&str],
    priority: Option<&str>,
    description: &str,
) {
    let tags: Vec<String> = tags.iter().map(|s| s.to_string()).collect();
    let assignees: Vec<String> = assignees.iter().map(|s| s.to_string()).collect();
    db.with_conn(|c| {
        t::create_task(
            c,
            id,
            "in-1",
            &t::NewTask {
                title: id,
                description,
                tags: &tags,
                assignees: &assignees,
                priority,
                created_by: "test",
                ..Default::default()
            },
        )?;
        Ok(())
    })
    .unwrap();
}

fn set_status(db: &Database, id: &str, status: &str) {
    let patch = t::TaskPatch {
        status: Some(status.into()),
        ..Default::default()
    };
    db.with_conn(|c| t::update_task(c, id, &patch, None))
        .unwrap();
}

fn task(db: &Database, id: &str) -> t::Task {
    db.with_conn(|c| t::get_task(c, id)).unwrap().unwrap()
}

fn next(db: &Database) -> Option<String> {
    db.with_conn(|c| candidates(c, &["o/r".to_string()]))
        .unwrap()
        .into_iter()
        .next()
        .map(|t| t.id)
}

fn on() -> HqConfig {
    let mut config = HqConfig::default();
    config.github_write.repos = vec!["o/r".into()];
    config.github_write.dispatch_tasks = true;
    config
}

fn worker_on() -> crate::skills::builtin::ActiveGuard {
    crate::skills::builtin::set_active_for_this_thread(Some(&[WORKER_SKILL]))
}

fn claim(db: &Database, id: &str) {
    db.with_conn(|c| {
        let who = t::LeaseIdentity {
            actor: "hq",
            harness: "native",
            external_session_ref: "hq",
            ..Default::default()
        };
        t::claim(c, id, &who, 900, false).map(|_| ())
    })
    .unwrap();
}

fn settle_all(db: &Database, error: &str) {
    db.with_conn(|c| asks::fail_all_pending(c, error)).unwrap();
}

#[test]
fn the_most_urgent_free_worker_task_is_taken() {
    let db = db();
    add(&db, "tk-other", &["chores"], &[], Some("urgent"));
    add(
        &db,
        "tk-theirs",
        &[DISPATCH_TAG],
        &["someone"],
        Some("urgent"),
    );
    add(
        &db,
        "tk-shared",
        &[DISPATCH_TAG],
        &["someone", HQ_ASSIGNEE],
        Some("urgent"),
    );
    add(&db, "tk-low", &[DISPATCH_TAG], &[], Some("low"));
    add(
        &db,
        "tk-high",
        &[DISPATCH_TAG],
        &[HQ_ASSIGNEE],
        Some("high"),
    );
    assert_eq!(next(&db).as_deref(), Some("tk-high"));
    set_status(&db, "tk-high", t::STATUS_READY_FOR_REVIEW);
    assert_eq!(next(&db).as_deref(), Some("tk-low"));
}

#[test]
fn a_blocked_task_is_not_taken() {
    let db = db();
    add(&db, "tk-first", &["chores"], &[], None);
    add(&db, "tk-a", &[DISPATCH_TAG], &[], None);
    db.with_conn(|c| t::add_dependency(c, "tk-a", "tk-first", "test"))
        .unwrap();
    assert!(next(&db).is_none());
}

#[test]
fn the_repository_must_be_named_as_a_whole_word() {
    let db = db();
    add(&db, "tk-a", &[DISPATCH_TAG], &[], None);
    let elsewhere = db
        .with_conn(|c| candidates(c, &["someone/else".to_string()]))
        .unwrap();
    assert!(elsewhere.is_empty());
    assert!(
        next(&db).is_some(),
        "the repository match ignores case and a full stop"
    );
    for near_miss in ["Fix o/r2 please", "See xo/r", "In o/r-other", "o/rbar"] {
        let db = self::db();
        add_about(&db, "tk-b", &[DISPATCH_TAG], &[], None, near_miss);
        assert!(next(&db).is_none(), "{near_miss}");
    }
    let db = self::db();
    add_about(
        &db,
        "tk-c",
        &[DISPATCH_TAG],
        &[],
        None,
        "repo: https://github.com/x (o/r)",
    );
    assert!(next(&db).is_some());
}

#[tokio::test]
async fn dispatch_stays_off_without_the_switch_an_active_worker_skill_or_a_chat() {
    let db = Arc::new(db());
    add(&db, "tk-a", &[DISPATCH_TAG], &[], None);
    let (cell, _) = chat(&db);
    let mut no_switch = on();
    no_switch.github_write.dispatch_tasks = false;
    let active = worker_on();
    assert_eq!(
        dispatch(db.clone(), &no_switch, cell.clone())
            .await
            .unwrap(),
        Outcome::Off
    );
    assert_eq!(
        dispatch(db.clone(), &on(), no_chat()).await.unwrap(),
        Outcome::Off,
        "no chat to work in"
    );
    drop(active);
    let _inactive = crate::skills::builtin::set_active_for_this_thread(Some(&[]));
    assert_eq!(dispatch(db, &on(), cell).await.unwrap(), Outcome::Off);
}

#[tokio::test]
async fn a_task_is_asked_about_once_and_the_worker_waits_for_the_reply() {
    let _active = worker_on();
    let db = Arc::new(db());
    let (cell, recorder) = chat(&db);
    assert_eq!(
        dispatch(db.clone(), &on(), cell.clone()).await.unwrap(),
        Outcome::Idle
    );
    add(&db, "tk-a", &[DISPATCH_TAG], &[], None);
    add(&db, "tk-b", &[DISPATCH_TAG], &[], None);
    let first = dispatch(db.clone(), &on(), cell.clone()).await.unwrap();
    let Outcome::Started { task, .. } = first else {
        panic!("expected a start, got {first:?}");
    };
    let question = recorder.questions.lock().unwrap()[0].clone();
    assert_eq!(
        question,
        format!(
            "Load the {WORKER_SKILL} skill with load_skill and follow it to work HQ task {task} end to end."
        )
    );
    assert_eq!(*recorder.models.lock().unwrap(), [None], "the chat's chain");
    assert_eq!(
        *recorder.untrusted.lock().unwrap(),
        [true],
        "the reply starts under the untrusted-content policy"
    );
    // An edit before the reply claims the task does not start a second one.
    db.with_conn(|c| t::add_comment(c, "tk-a", "owner", "also this", None))
        .unwrap();
    set_status(&db, "tk-a", t::STATUS_TO_DO);
    let again = dispatch(db, &on(), cell).await.unwrap();
    assert!(matches!(again, Outcome::Busy(_)), "{again:?}");
    assert_eq!(recorder.questions.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_reply_that_ended_without_the_task_parks_it_and_the_queue_moves_on() {
    let _active = worker_on();
    let db = Arc::new(db());
    let (cell, recorder) = chat(&db);
    add(&db, "tk-a", &[DISPATCH_TAG], &[], Some("high"));
    add(&db, "tk-b", &[DISPATCH_TAG], &[], Some("low"));
    dispatch(db.clone(), &on(), cell.clone()).await.unwrap();
    settle_all(&db, "provider error");
    let next = dispatch(db.clone(), &on(), cell).await.unwrap();
    assert!(matches!(next, Outcome::Started { .. }), "{next:?}");
    assert_eq!(recorder.questions.lock().unwrap().len(), 2);
    let parked = task(&db, "tk-a");
    assert_eq!(parked.status, t::STATUS_BLOCKED);
    assert!(parked.tags.iter().any(|g| g == PARKED_TAG));
    assert!(
        parked
            .blocked_reason
            .unwrap_or_default()
            .contains("provider error")
    );
}

#[tokio::test]
async fn a_task_the_worker_left_in_progress_is_parked_once_its_lease_is_gone() {
    let _active = worker_on();
    let db = Arc::new(db());
    let (cell, _) = chat(&db);
    add(&db, "tk-a", &[DISPATCH_TAG], &[], None);
    dispatch(db.clone(), &on(), cell.clone()).await.unwrap();
    claim(&db, "tk-a");
    settle_all(&db, "the reply ended");
    let held = dispatch(db.clone(), &on(), cell.clone()).await.unwrap();
    assert_eq!(held, Outcome::Busy(task(&db, "tk-a").display_id));
    db.with_conn(|c| {
        c.execute(
            "UPDATE task_work_sessions SET last_heartbeat_at = datetime('now', '-1 day')",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        dispatch(db.clone(), &on(), cell).await.unwrap(),
        Outcome::Idle
    );
    assert_eq!(task(&db, "tk-a").status, t::STATUS_BLOCKED);
    let notes = hq_db::value_items::list_by_state(&db, hq_core::types::ValueState::Pending)
        .unwrap_or_default();
    assert!(
        notes.iter().any(|n| n.source_task == INBOX_SOURCE),
        "the owner is told"
    );
}

#[tokio::test]
async fn a_task_the_owner_moved_to_in_progress_is_left_alone() {
    let _active = worker_on();
    let db = Arc::new(db());
    let (cell, _) = chat(&db);
    add(&db, "tk-a", &[DISPATCH_TAG], &[], None);
    set_status(&db, "tk-a", t::STATUS_IN_PROGRESS);
    for _ in 0..2 {
        let tick = dispatch(db.clone(), &on(), cell.clone()).await.unwrap();
        assert!(matches!(tick, Outcome::Busy(_)), "{tick:?}");
    }
    assert_eq!(task(&db, "tk-a").status, t::STATUS_IN_PROGRESS);
}

#[test]
fn a_github_url_names_its_repository() {
    for text in [
        "https://github.com/O/R",
        "see https://github.com/o/r/issues/3.",
        "clone github.com/o/r.git",
    ] {
        let db = self::db();
        add_about(&db, "tk-u", &[DISPATCH_TAG], &[], None, text);
        assert!(next(&db).is_some(), "{text}");
    }
    let db = self::db();
    add_about(
        &db,
        "tk-v",
        &[DISPATCH_TAG],
        &[],
        None,
        "https://github.com/o/r2",
    );
    assert!(next(&db).is_none());
}

#[test]
fn scoped_connections_cannot_queue_work_or_rewrite_queued_tasks() {
    let queue = vec![DISPATCH_TAG.to_uppercase()];
    let other = vec!["chores".to_string()];
    assert!(refuse_queue_write(true, Some(&queue), &[], false).is_err());
    assert!(refuse_queue_write(true, None, &queue, true).is_err());
    assert!(
        refuse_queue_write(true, Some(&queue), &queue, false).is_ok(),
        "already queued"
    );
    assert!(refuse_queue_write(true, Some(&other), &other, true).is_ok());
    assert!(
        refuse_queue_write(false, Some(&queue), &[], true).is_ok(),
        "the owner may"
    );
}

#[tokio::test]
async fn a_parked_task_set_back_to_do_is_skipped_until_the_tag_is_removed() {
    let _active = worker_on();
    let db = Arc::new(db());
    let (cell, recorder) = chat(&db);
    add(&db, "tk-a", &[DISPATCH_TAG], &[], None);
    dispatch(db.clone(), &on(), cell.clone()).await.unwrap();
    settle_all(&db, "provider error");
    assert_eq!(
        dispatch(db.clone(), &on(), cell.clone()).await.unwrap(),
        Outcome::Idle
    );
    set_status(&db, "tk-a", t::STATUS_TO_DO);
    assert_eq!(
        dispatch(db.clone(), &on(), cell.clone()).await.unwrap(),
        Outcome::Idle
    );
    assert_eq!(
        recorder.questions.lock().unwrap().len(),
        1,
        "the same text is not run again"
    );
    let unparked = t::TaskPatch {
        tags: Some(vec![DISPATCH_TAG.into()]),
        ..Default::default()
    };
    db.with_conn(|c| t::update_task(c, "tk-a", &unparked, None))
        .unwrap();
    // The owner unparks later than the park; timestamps have one-second resolution.
    db.with_conn(|c| {
        c.execute(
            "UPDATE tasks SET updated_at = datetime('now', '+1 minute') WHERE id = 'tk-a'",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    let again = dispatch(db, &on(), cell).await.unwrap();
    assert!(matches!(again, Outcome::Started { .. }), "{again:?}");
}

#[test]
fn a_restricted_caller_cannot_change_a_queued_tasks_tags() {
    let parked = vec![DISPATCH_TAG.to_string(), PARKED_TAG.to_string()];
    let queue = vec![DISPATCH_TAG.to_string()];
    assert!(
        refuse_queue_write(true, Some(&queue), &parked, false).is_err(),
        "unparking"
    );
    assert!(refuse_queue_write(true, Some(&[]), &queue, false).is_err());
    let reordered = vec![PARKED_TAG.to_uppercase(), DISPATCH_TAG.to_string()];
    assert!(refuse_queue_write(true, Some(&reordered), &parked, false).is_ok());
    assert!(
        refuse_queue_write(false, Some(&queue), &parked, false).is_ok(),
        "the owner may"
    );
}

#[tokio::test]
async fn the_driver_runs_on_the_configured_dispatch_model() {
    let _active = worker_on();
    let db = Arc::new(db());
    let (cell, recorder) = chat(&db);
    add(&db, "tk-a", &[DISPATCH_TAG], &[], None);
    let mut config = on();
    config.github_write.dispatch_model = Some("anthropic-haiku".into());
    dispatch(db, &config, cell).await.unwrap();
    assert_eq!(
        *recorder.models.lock().unwrap(),
        [Some("anthropic-haiku".to_string())]
    );
}

#[tokio::test]
async fn every_task_runs_under_the_configured_budget() {
    let _active = worker_on();
    let db = Arc::new(db());
    let (cell, recorder) = chat(&db);
    add(&db, "tk-a", &[DISPATCH_TAG], &[], None);
    let mut config = on();
    config.github_write.task_budget_usd = 0.2;
    dispatch(db, &config, cell).await.unwrap();
    assert_eq!(*recorder.budgets.lock().unwrap(), [Some(0.2)]);
    assert_eq!(
        *recorder.presets.lock().unwrap(),
        [true],
        "every queued task runs as the worker"
    );
}

#[tokio::test]
async fn an_undeclared_dispatch_model_stops_dispatch_without_parking_anything() {
    let _active = worker_on();
    let db = Arc::new(db());
    let (cell, recorder) = chat(&db);
    add(&db, "tk-a", &[DISPATCH_TAG], &[], None);
    let mut config = on();
    config.backends = hq_core::config::BackendsConfig {
        primary: "luna".into(),
        backends: vec![hq_core::config::BackendEntry {
            name: "luna".into(),
            kind: hq_core::config::BackendKind::GithubCopilotCli,
            endpoint: None,
            credential_env: None,
            model: Some("gpt-luna".into()),
            effort: None,
            wire: Default::default(),
            enabled: true,
            unix_socket: None,
            prompted_tools: false,
        }],
        ..Default::default()
    };
    config.github_write.dispatch_model = Some("anthropic-haiku-typo".into());
    let err = dispatch(db.clone(), &config, cell.clone())
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("names no enabled backend"),
        "{err}"
    );
    assert!(
        recorder.questions.lock().unwrap().is_empty(),
        "nothing started"
    );
    assert_eq!(task(&db, "tk-a").status, t::STATUS_TO_DO, "nothing parked");
    for declared in ["luna", "gpt-luna"] {
        config.github_write.dispatch_model = Some(declared.into());
        assert!(model_is_declared(&config, declared));
    }
}

#[tokio::test]
async fn recent_runs_name_the_task_each_ask_worked_newest_first() {
    let _active = worker_on();
    let db = Arc::new(db());
    let (cell, _) = chat(&db);
    add(&db, "tk-a", &[DISPATCH_TAG], &[], Some("high"));
    add(&db, "tk-b", &[DISPATCH_TAG], &[], Some("low"));
    dispatch(db.clone(), &on(), cell.clone()).await.unwrap();
    settle_all(&db, "provider error");
    dispatch(db.clone(), &on(), cell).await.unwrap();
    let runs = db.with_conn(|c| recent_runs(c, 10)).unwrap();
    let ids: Vec<&str> = runs.iter().map(|r| r.task_id.as_str()).collect();
    assert_eq!(ids, ["tk-b", "tk-a"]);
    assert_eq!(runs[0].status, "pending");
    assert_eq!(runs[1].status, "failed");
    assert_eq!(runs[1].task.as_ref().unwrap().status, t::STATUS_BLOCKED);
    assert!(db.with_conn(|c| recent_runs(c, 1)).unwrap().len() == 1);
}
