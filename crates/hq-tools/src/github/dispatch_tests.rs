use super::*;
use crate::ask::{AskRunner, AskStart, RunnerCell};
use std::sync::{Mutex, OnceLock};

/// Records each question and leaves the reply running, as a real chat would while it works.
struct Recorder {
    db: Arc<Database>,
    questions: Mutex<Vec<String>>,
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
    });
    let cell: RunnerCell = Arc::new(OnceLock::new());
    let as_dyn: Arc<dyn AskRunner> = recorder.clone();
    cell.set(as_dyn).ok();
    (cell, recorder)
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
    let tags: Vec<String> = tags.iter().map(|s| s.to_string()).collect();
    let assignees: Vec<String> = assignees.iter().map(|s| s.to_string()).collect();
    db.with_conn(|c| {
        t::create_task(
            c,
            id,
            "in-1",
            &t::NewTask {
                title: id,
                description: "Work in O/R, please.",
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

fn next(db: &Database) -> Option<String> {
    db.with_conn(|c| next_task(c, &["o/r".to_string()]))
        .unwrap()
        .map(|t| t.id)
}

fn on() -> HqConfig {
    let mut config = HqConfig::default();
    config.github_write.repos = vec!["o/r".into()];
    config.github_write.dispatch_tasks = true;
    config
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
fn a_task_that_names_no_allowed_repository_is_not_taken() {
    let db = db();
    add(&db, "tk-a", &[DISPATCH_TAG], &[], None);
    let elsewhere = db
        .with_conn(|c| next_task(c, &["someone/else".to_string()]))
        .unwrap();
    assert!(elsewhere.is_none());
    assert!(next(&db).is_some(), "the repository match ignores case");
}

#[test]
fn a_worker_task_in_progress_makes_the_queue_wait() {
    let db = db();
    add(&db, "tk-a", &[DISPATCH_TAG], &[], None);
    assert!(db.with_conn(running_task).unwrap().is_none());
    set_status(&db, "tk-a", t::STATUS_IN_PROGRESS);
    assert!(db.with_conn(running_task).unwrap().is_some());
}

#[tokio::test]
async fn dispatch_stays_off_without_the_switch_an_active_worker_skill_or_a_chat() {
    let db = Arc::new(db());
    add(&db, "tk-a", &[DISPATCH_TAG], &[], None);
    let (cell, _) = chat(&db);
    let mut no_switch = on();
    no_switch.github_write.dispatch_tasks = false;
    assert_eq!(
        dispatch(db.clone(), &no_switch, cell.clone())
            .await
            .unwrap(),
        Outcome::Off
    );
    let mut no_repos = on();
    no_repos.github_write.repos.clear();
    assert_eq!(
        dispatch(db.clone(), &no_repos, cell.clone()).await.unwrap(),
        Outcome::Off
    );
    let mut skill_off = on();
    skill_off.skills.builtin.off = vec![WORKER_SKILL.into()];
    assert_eq!(
        dispatch(db.clone(), &skill_off, cell).await.unwrap(),
        Outcome::Off
    );
    assert_eq!(
        dispatch(db, &on(), no_chat()).await.unwrap(),
        Outcome::Off,
        "no chat to work in"
    );
}

#[tokio::test]
async fn dispatch_waits_while_busy_and_idles_on_an_empty_queue() {
    let db = Arc::new(db());
    let (cell, _) = chat(&db);
    assert_eq!(
        dispatch(db.clone(), &on(), cell.clone()).await.unwrap(),
        Outcome::Idle
    );
    add(&db, "tk-a", &[DISPATCH_TAG], &[], None);
    set_status(&db, "tk-a", t::STATUS_IN_PROGRESS);
    assert!(matches!(
        dispatch(db, &on(), cell).await.unwrap(),
        Outcome::Busy(_)
    ));
}

#[tokio::test]
async fn a_task_is_asked_about_once_until_it_changes() {
    let db = Arc::new(db());
    let (cell, recorder) = chat(&db);
    add(&db, "tk-a", &[DISPATCH_TAG], &[], None);
    let first = dispatch(db.clone(), &on(), cell.clone()).await.unwrap();
    let Outcome::Started { task, .. } = first else {
        panic!("expected a start, got {first:?}");
    };
    let question = recorder.questions.lock().unwrap()[0].clone();
    assert_eq!(question, format!("Work HQ task {task} end to end."));
    let again = dispatch(db, &on(), cell).await.unwrap();
    assert!(matches!(again, Outcome::AlreadyAsked { .. }), "{again:?}");
    assert_eq!(
        recorder.questions.lock().unwrap().len(),
        1,
        "no second reply was started"
    );
}
