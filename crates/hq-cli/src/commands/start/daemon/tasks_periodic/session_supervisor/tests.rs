use super::*;
use hq_tools::agent_host::scripted::ScriptedHost;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// A pane redraw as a terminal would have logged it: cursor hiding, screen
/// erase, an OSC title, colour runs, and carriage returns between rows.
const ANSI_SAMPLE: &str = "\x1b[?25l\x1b[2J\x1b[H\x1b]0;osc-window-title\x07\r\x1b[38;5;242m> working\x1b[0m\r\n\x1b[1mFINAL ANSWER: 42\x1b[0m\r\n\x1b[?25h";

/// A host whose `agent list` reports `agents` and whose `agent read`
/// prints `screen`.
fn fake_host(dir: &Path, agents: &[(&str, &str, u64)], screen: &str) -> ScriptedHost {
    let list: Vec<String> = agents
        .iter()
        .map(|(name, status, seq)| {
            format!(
                r#"{{"agent":"claude","agent_status":"{status}","cwd":"/tmp","name":"{name}","pane_id":"w1:p1","workspace_id":"w1","state_change_seq":{seq}}}"#
            )
        })
        .collect();
    let script = format!(
        "#!/bin/sh\ncase \"$*\" in\n  *\"agent list\"*) printf '%s' '{{\"id\":\"x\",\"result\":{{\"agents\":[{}]}}}}' ;;\n  *\"agent read\"*) printf '%s' '{}' ;;\n  *) exit 2 ;;\nesac\n",
        list.join(","),
        screen.replace('\'', "")
    );
    let path = dir.join("scripted-host");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    ScriptedHost::new(path)
}

fn gone_host(dir: &Path) -> ScriptedHost {
    fake_host(dir, &[], "")
}

fn seed(db: &Database, id: &str) {
    seed_for(db, id, None);
}

/// An in-progress HQ task, FR-001, with one session working on it.
fn seed_with_task(db: &Database, id: &str) -> String {
    use hq_db::tasks as t;
    let task = db
        .with_conn(|c| {
            c.execute(
                "INSERT INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('in-1', 'personal', 'Work', 'work', 'FR')",
                [],
            )?;
            let new = t::NewTask {
                title: "durable missions",
                created_by: "test",
                ..Default::default()
            };
            let task = t::create_task(c, "tk-1", "in-1", &new)?;
            let patch = t::TaskPatch {
                status: Some(t::STATUS_IN_PROGRESS.into()),
                ..Default::default()
            };
            t::update_task(c, &task.id, &patch, None)?;
            Ok(task.id)
        })
        .unwrap();
    seed_for(db, id, Some(&task));
    task
}

fn task_state(db: &Database, task: &str) -> (String, usize) {
    db.with_conn(|c| {
        let status = hq_db::tasks::get_task(c, task)?.unwrap().status;
        Ok((status, hq_db::tasks::list_comments(c, task)?.len()))
    })
    .unwrap()
}

fn seed_for(db: &Database, id: &str, mission: Option<&str>) {
    let id = id.to_string();
    db.with_conn(move |c| {
        registry::insert(
            c,
            &registry::NewSession {
                id: &id,
                harness: "claude-code",
                label: "auth refactor",
                cwd: "/tmp",
                mission_id: mission,
                placement: registry::Placement {
                    host: "local",
                    agent_name: &id,
                    workspace_id: "w1",
                    pane_id: "w1:p1",
                },
                purpose: None,
            },
        )
    })
    .unwrap();
}

fn store_snapshot(db: &Database, id: &str, snapshot: &str) {
    let (id, snapshot) = (id.to_string(), snapshot.to_string());
    db.with_conn(move |c| registry::set_last_snapshot(c, &id, &snapshot))
        .unwrap();
}

fn mailbox_dir(vault: &Path) -> PathBuf {
    vault.join(hq_core::mailbox::MAILBOX_DIR).join("relay")
}

fn mailbox_files(vault: &Path) -> Vec<String> {
    std::fs::read_dir(mailbox_dir(vault))
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.ends_with(".json"))
                .collect()
        })
        .unwrap_or_default()
}

fn only_message(vault: &Path) -> String {
    let files = mailbox_files(vault);
    assert_eq!(files.len(), 1, "expected one relay message, got {files:?}");
    std::fs::read_to_string(mailbox_dir(vault).join(&files[0])).unwrap()
}

/// The message body as the operator receives it, unescaped from the JSON.
fn only_body(vault: &Path) -> String {
    let raw = only_message(vault);
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    value
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap()
        .to_string()
}

/// Summarizer stub. `Ok` text is delivered as the summary, `Err` forces the
/// raw-excerpt fallback. No LLM is reachable from a test.
fn stub_summarizer(outcome: std::result::Result<&'static str, &'static str>) -> Summarizer {
    Arc::new(move |_text: String| -> SummaryFuture {
        Box::pin(async move {
            match outcome {
                Ok(summary) => Ok(summary.to_string()),
                Err(e) => bail!("{e}"),
            }
        })
    })
}

/// The production budget is sized for a real sweep. A loaded test machine
/// can take longer than that to fork the fake host's shell, which reads as
/// an unreachable host and posts nothing, so behaviour tests get room.
const TEST_HOST_BUDGET: Duration = Duration::from_secs(60);

async fn sweep(vault: &Path, db: &Database, host: &ScriptedHost, s: Option<&Summarizer>) {
    let host = host.clone();
    let resolve: HostResolver = Arc::new(move |_| Ok(Arc::new(host.clone()) as Host));
    supervise(vault, db, s, resolve, Sweep::within(TEST_HOST_BUDGET))
        .await
        .unwrap();
}

fn status_of(db: &Database, id: &str) -> String {
    let id = id.to_string();
    db.with_conn(move |c| registry::get(c, &id))
        .unwrap()
        .unwrap()
        .status
}

#[tokio::test]
async fn dead_session_posts_final_output_to_relay() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-dead");
    // Backticks in the output are what a markdown fence would choke on.
    store_snapshot(
        &db,
        "hs-dead",
        "```rust\nfn main() {}\n```\nFINAL ANSWER: 42\n",
    );

    sweep(tmp.path(), &db, &gone_host(tmp.path()), None).await;

    assert_eq!(status_of(&db, "hs-dead"), registry::STATUS_EXITED);
    let msg = only_message(tmp.path());
    assert!(msg.contains("hs-dead"));
    assert!(msg.contains("FINAL ANSWER: 42"));
    assert!(msg.contains("harness_session_resume"));
    assert!(msg.contains(OUTPUT_OPEN));
    assert!(msg.contains(OUTPUT_CLOSE));
}

#[tokio::test]
async fn second_sweep_sends_nothing_more() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-once");
    store_snapshot(&db, "hs-once", "done");
    let host = gone_host(tmp.path());

    sweep(tmp.path(), &db, &host, None).await;
    sweep(tmp.path(), &db, &host, None).await;

    assert_eq!(mailbox_files(tmp.path()).len(), 1);
}

#[tokio::test]
async fn a_session_that_died_before_any_snapshot_still_notifies() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-nosnap");

    sweep(tmp.path(), &db, &gone_host(tmp.path()), None).await;

    let body = only_body(tmp.path());
    assert!(body.contains("hs-nosnap"));
    assert!(body.contains("no output was captured for this session"));
}

#[tokio::test]
async fn ansi_dense_snapshot_is_delivered_clean() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-ansi");
    store_snapshot(&db, "hs-ansi", ANSI_SAMPLE);

    sweep(tmp.path(), &db, &gone_host(tmp.path()), None).await;

    let body = only_body(tmp.path());
    assert!(body.contains("FINAL ANSWER: 42"));
    assert!(body.contains("> working"));
    assert!(!body.contains('\x1b'), "escape survived: {body:?}");
    assert!(!body.contains('\r'), "carriage return survived: {body:?}");
    assert!(!body.contains("[0m"));
    assert!(
        !body.contains("osc-window-title"),
        "OSC title survived: {body:?}"
    );
}

#[tokio::test]
async fn alive_session_is_snapshotted_not_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-alive");
    let host = fake_host(tmp.path(), &[("hs-alive", "working", 3)], "PANE-READY");

    sweep(tmp.path(), &db, &host, None).await;

    assert_eq!(status_of(&db, "hs-alive"), registry::STATUS_RUNNING);
    assert!(mailbox_files(tmp.path()).is_empty());
    let snapshot = db
        .with_conn(|c| registry::last_snapshot(c, "hs-alive"))
        .unwrap()
        .unwrap_or_default();
    assert!(snapshot.contains("PANE-READY"));
}

#[tokio::test]
async fn an_unreachable_host_leaves_its_sessions_running() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-away");
    store_snapshot(&db, "hs-away", "last known screen");

    let resolve: HostResolver = Arc::new(|name| bail!("host '{name}' unreachable: no route"));
    supervise(
        tmp.path(),
        &db,
        None,
        resolve,
        Sweep::within(HOST_SWEEP_BUDGET),
    )
    .await
    .unwrap();

    assert_eq!(status_of(&db, "hs-away"), registry::STATUS_RUNNING);
    assert!(
        mailbox_files(tmp.path()).is_empty(),
        "laptop asleep is not an exit"
    );
}

#[tokio::test]
async fn a_host_that_never_answers_is_cut_off_at_the_host_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-hung");
    let path = tmp.path().join("scripted-host");
    std::fs::write(&path, "#!/bin/sh\nsleep 30\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let hung = ScriptedHost::new(path).with_command_timeout(Duration::from_secs(30));
    let resolve: HostResolver = Arc::new(move |_| Ok(Arc::new(hung.clone()) as Host));

    let started = Instant::now();
    supervise(
        tmp.path(),
        &db,
        None,
        resolve,
        Sweep::within(Duration::from_millis(500)),
    )
    .await
    .unwrap();

    assert!(
        started.elapsed() < Duration::from_secs(8),
        "a hung host held the sweep for {:?}",
        started.elapsed()
    );
    assert_eq!(status_of(&db, "hs-hung"), registry::STATUS_RUNNING);
    assert!(mailbox_files(tmp.path()).is_empty());
}

fn alert_count(db: &Database) -> i64 {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT COUNT(*) FROM value_items WHERE dedup_key LIKE 'session-blocked-%'",
            [],
            |r| r.get::<_, i64>(0),
        )?)
    })
    .unwrap()
}

#[tokio::test]
async fn a_blocked_agent_alerts_once_until_its_state_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-blocked");
    let blocked = fake_host(
        tmp.path(),
        &[("hs-blocked", "blocked", 4)],
        "Do you want to proceed?",
    );

    sweep(tmp.path(), &db, &blocked, None).await;
    sweep(tmp.path(), &db, &blocked, None).await;
    assert_eq!(alert_count(&db), 1, "same block must not re-alert");
    assert_eq!(status_of(&db, "hs-blocked"), registry::STATUS_RUNNING);

    let blocked_again = fake_host(
        tmp.path(),
        &[("hs-blocked", "blocked", 9)],
        "Approve the edit?",
    );
    sweep(tmp.path(), &db, &blocked_again, None).await;
    assert_eq!(alert_count(&db), 2, "a new block is a new alert");
}

/// A host whose one agent is blocked at a dialog it read, numbered `seq`.
fn host_with_dialog(dir: &Path, name: &str, seq: u64) -> ScriptedHost {
    let pending = r#"{"kind":"path","question":"Do you want to allow this?","command":"/tmp","options":[{"id":1,"label":"Yes"},{"id":2,"label":"Yes, and remember this path","remember":true},{"id":3,"label":"No (Esc)"}]}"#;
    let agent = format!(
        r#"{{"agent":"copilot","agent_status":"blocked","cwd":"/tmp","name":"{name}","pane_id":"w1:p1","workspace_id":"w1","state_change_seq":{seq},"pending_seq":{seq},"pending":{pending}}}"#
    );
    let script = format!(
        "#!/bin/sh\ncase \"$*\" in\n  *\"agent list\"*) printf '%s' '{{\"id\":\"x\",\"result\":{{\"agents\":[{agent}]}}}}' ;;\n  *\"agent read\"*) printf '%s' 'Do you want to allow this?' ;;\n  *) exit 2 ;;\nesac\n"
    );
    let path = dir.join("scripted-host");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    ScriptedHost::new(path)
}

#[tokio::test]
async fn a_block_at_a_dialog_hq_can_read_ties_the_alert_to_that_dialog() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-path");
    let host = host_with_dialog(tmp.path(), "hs-path", 12);

    sweep(tmp.path(), &db, &host, None).await;
    sweep(tmp.path(), &db, &host, None).await;
    assert_eq!(alert_count(&db), 1);
    let item_id: String = db
        .with_conn(|c| {
            Ok(c.query_row(
                "SELECT id FROM value_items WHERE dedup_key = 'session-blocked-hs-path-12'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    let dialog = hq_db::dialog_alerts::get(&db, &item_id).unwrap().unwrap();
    assert_eq!(dialog.session_id, "hs-path");
    assert_eq!(dialog.pending_seq, 12);
    assert_eq!(dialog.kind, "path");
    assert_eq!(dialog.options.len(), 3);
    assert_eq!(
        dialog.approve_option,
        Some(1),
        "the button answers once, never remember"
    );
}

#[tokio::test]
async fn a_finished_agent_alerts_once_and_stays_running() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-done");
    let done = fake_host(
        tmp.path(),
        &[("hs-done", "done", 6)],
        "All tests pass. Nothing left to do.",
    );

    sweep(tmp.path(), &db, &done, None).await;
    sweep(tmp.path(), &db, &done, None).await;

    let body = only_body(tmp.path());
    assert!(body.contains("finished its task"), "{body}");
    assert!(body.contains("All tests pass"), "{body}");
    let msg: serde_json::Value = serde_json::from_str(&only_message(tmp.path())).unwrap();
    assert!(
        msg["meta"].get(hq_core::mailbox::META_INTERRUPT).is_none(),
        "a finished turn goes to the digest, not an interrupt: {msg}"
    );
    assert_eq!(status_of(&db, "hs-done"), registry::STATUS_RUNNING);

    let idle = fake_host(tmp.path(), &[("hs-done", "idle", 6)], "");
    sweep(tmp.path(), &db, &idle, None).await;
    assert_eq!(mailbox_files(tmp.path()).len(), 1, "idle is not an alert");
}

#[tokio::test]
async fn an_exit_is_recorded_on_the_task_once_across_sweeps() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    let task = seed_with_task(&db, "hs-task");
    store_snapshot(&db, "hs-task", "panic: boom");
    let host = gone_host(tmp.path());

    // The second sweep stands in for a daemon restart re-reading the registry.
    sweep(tmp.path(), &db, &host, None).await;
    sweep(tmp.path(), &db, &host, None).await;

    assert_eq!(
        task_state(&db, &task),
        (hq_db::tasks::STATUS_BLOCKED.into(), 1)
    );
    let body = only_body(tmp.path());
    assert!(body.contains("Task FR-001 is blocked"), "{body}");
    assert!(
        interrupts(tmp.path()),
        "stopped work on a task is a blocker, not digest news"
    );
}

fn interrupts(vault: &Path) -> bool {
    let msg: serde_json::Value = serde_json::from_str(&only_message(vault)).unwrap();
    msg["meta"].get(hq_core::mailbox::META_INTERRUPT).is_some()
}

#[tokio::test]
async fn an_exit_without_a_task_stays_a_digest_update() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-plain");

    sweep(tmp.path(), &db, &gone_host(tmp.path()), None).await;

    assert!(!interrupts(tmp.path()));
}

#[tokio::test]
async fn routine_sweeps_of_a_working_task_session_notify_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    let task = seed_with_task(&db, "hs-busy");
    let working = fake_host(tmp.path(), &[("hs-busy", "working", 2)], "editing files");

    for _ in 0..3 {
        sweep(tmp.path(), &db, &working, None).await;
    }

    assert!(mailbox_files(tmp.path()).is_empty());
    assert_eq!(alert_count(&db), 0);
    assert_eq!(
        task_state(&db, &task),
        (hq_db::tasks::STATUS_IN_PROGRESS.into(), 0)
    );
}

#[tokio::test]
async fn an_unreachable_host_leaves_the_task_untouched() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    let task = seed_with_task(&db, "hs-laptop");

    let resolve: HostResolver = Arc::new(|name| bail!("host '{name}' unreachable: no route"));
    supervise(
        tmp.path(),
        &db,
        None,
        resolve,
        Sweep::within(HOST_SWEEP_BUDGET),
    )
    .await
    .unwrap();

    assert_eq!(
        task_state(&db, &task),
        (hq_db::tasks::STATUS_IN_PROGRESS.into(), 0)
    );
}

#[tokio::test]
async fn a_finished_turn_puts_the_task_up_for_review_not_complete() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    let task = seed_with_task(&db, "hs-review");
    let done = fake_host(tmp.path(), &[("hs-review", "done", 3)], "All tests pass.");

    sweep(tmp.path(), &db, &done, None).await;
    sweep(tmp.path(), &db, &done, None).await;

    assert_eq!(
        task_state(&db, &task),
        (hq_db::tasks::STATUS_READY_FOR_REVIEW.into(), 1)
    );
    assert!(only_body(tmp.path()).contains("Task FR-001 is ready for review"));
    assert!(
        !interrupts(tmp.path()),
        "a finished turn is a digest update"
    );
}

#[tokio::test]
async fn a_task_session_blocked_on_a_dialog_raises_one_action_item() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    let task = seed_with_task(&db, "hs-ask");
    let blocked = fake_host(tmp.path(), &[("hs-ask", "blocked", 7)], "Approve the edit?");

    sweep(tmp.path(), &db, &blocked, None).await;
    sweep(tmp.path(), &db, &blocked, None).await;

    assert_eq!(alert_count(&db), 1);
    let body: String = db
        .with_conn(|c| {
            Ok(c.query_row(
                "SELECT body FROM value_items WHERE dedup_key LIKE 'session-blocked-%'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert!(body.contains("Task FR-001 is in progress"), "{body}");
    assert_eq!(
        task_state(&db, &task),
        (hq_db::tasks::STATUS_IN_PROGRESS.into(), 1)
    );
}

fn watch_from_chat(db: &Database, id: &str) {
    db.with_conn(|c| registry::set_owner(c, id, Some("th-web")))
        .unwrap();
}

fn row_of(db: &Database, id: &str) -> registry::HarnessSessionRow {
    db.with_conn(|c| registry::get(c, id)).unwrap().unwrap()
}

fn value_item_count(db: &Database) -> i64 {
    db.with_conn(|c| {
        Ok(c.query_row("SELECT COUNT(*) FROM value_items", [], |r| {
            r.get::<_, i64>(0)
        })?)
    })
    .unwrap()
}

#[tokio::test]
async fn a_watched_session_wakes_its_chat_instead_of_the_relay() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    let task = seed_with_task(&db, "hs-web");
    watch_from_chat(&db, "hs-web");

    let done = fake_host(tmp.path(), &[("hs-web", "done", 4)], "All tests pass.");
    sweep(tmp.path(), &db, &done, None).await;
    let row = row_of(&db, "hs-web");
    assert_eq!(row.pm_wake.as_deref(), Some(WAKE_FINISHED));
    assert_eq!(row.last_agent_status.as_deref(), Some("done"));
    assert!(
        mailbox_files(tmp.path()).is_empty(),
        "web chat only: nothing for Telegram"
    );
    assert_eq!(
        task_state(&db, &task).0,
        hq_db::tasks::STATUS_READY_FOR_REVIEW
    );

    let blocked = fake_host(tmp.path(), &[("hs-web", "blocked", 5)], "Approve the edit?");
    sweep(tmp.path(), &db, &blocked, None).await;
    assert_eq!(row_of(&db, "hs-web").pm_wake.as_deref(), Some(WAKE_BLOCKED));
    assert_eq!(
        value_item_count(&db),
        1,
        "one web-inbox badge for the block"
    );
    assert!(
        hq_daemon::value_bus::WEB_ONLY_SOURCES.contains(&WEB_INBOX_SOURCE),
        "the badge must never reach Telegram"
    );

    sweep(tmp.path(), &db, &gone_host(tmp.path()), None).await;
    let row = row_of(&db, "hs-web");
    assert_eq!(row.status, registry::STATUS_EXITED);
    assert_eq!(row.pm_wake.as_deref(), Some(WAKE_EXITED));
    assert!(mailbox_files(tmp.path()).is_empty());
    assert_eq!(
        value_item_count(&db),
        1,
        "the exit adds nothing for the relay"
    );
}

#[tokio::test]
async fn a_working_watched_session_only_updates_its_seen_status() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-quiet");
    watch_from_chat(&db, "hs-quiet");
    let working = fake_host(tmp.path(), &[("hs-quiet", "working", 1)], "editing");

    sweep(tmp.path(), &db, &working, None).await;

    let row = row_of(&db, "hs-quiet");
    assert_eq!(row.last_agent_status.as_deref(), Some("working"));
    assert!(row.pm_wake.is_none());
}

#[tokio::test]
async fn summary_replaces_the_raw_excerpt() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-sum");
    store_snapshot(&db, "hs-sum", "RAW PANE TEXT\nFINAL ANSWER: 42");

    let summarizer = stub_summarizer(Ok("Done: refactored auth, no open items."));
    sweep(tmp.path(), &db, &gone_host(tmp.path()), Some(&summarizer)).await;

    let body = only_body(tmp.path());
    assert!(body.contains("Done: refactored auth, no open items."));
    assert!(!body.contains("RAW PANE TEXT"));
    assert!(!body.contains(OUTPUT_OPEN));
    assert!(body.contains("harness_session_resume"));
}

#[tokio::test]
async fn summarizer_failure_falls_back_to_the_raw_excerpt() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-fail");
    store_snapshot(&db, "hs-fail", "RAW PANE TEXT\nFINAL ANSWER: 42");

    let summarizer = stub_summarizer(Err("provider unreachable"));
    sweep(tmp.path(), &db, &gone_host(tmp.path()), Some(&summarizer)).await;

    let body = only_body(tmp.path());
    assert!(body.contains("RAW PANE TEXT"));
    assert!(body.contains("FINAL ANSWER: 42"));
    assert!(body.contains(OUTPUT_OPEN));
}

#[test]
fn disabled_summary_config_means_no_summarizer() {
    let off = HqConfig {
        relay: hq_core::config::RelayConfig {
            summarize_session_exits: false,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(summarizer_for(&off).is_none());
    assert!(summarizer_for(&HqConfig::default()).is_some());
}

#[test]
fn tail_excerpt_keeps_the_end_char_safely() {
    let short = "all of it";
    assert_eq!(tail_excerpt(short, 100), short);

    let long: String = (0..200)
        .map(|i| char::from(b'a' + (i % 26) as u8))
        .collect();
    let cut = tail_excerpt(&long, 50);
    assert_eq!(cut.chars().count(), 51); // 50 + ellipsis
    assert!(cut.ends_with(&long[long.len() - 10..]));
}

#[test]
fn tail_excerpt_caps_bytes_on_multibyte_text() {
    // Four bytes per rocket: a byte cap that lands mid-character must walk
    // forward, never split, and never exceed the cap.
    let emoji = "🚀".repeat(200);
    let cut = tail_excerpt(&emoji, 50);
    let kept = cut.strip_prefix('…').unwrap();
    assert_eq!(kept.len(), 48);
    assert_eq!(kept.chars().count(), 12);
    assert!(emoji.ends_with(kept));

    let mixed = format!("{}{}", "é".repeat(100), "tail");
    let cut = tail_excerpt(&mixed, 25);
    let kept = cut.strip_prefix('…').unwrap();
    assert!(kept.len() <= 25);
    assert!(kept.ends_with("tail"));
}

#[test]
fn clean_pty_text_flattens_carriage_returns() {
    let cleaned = clean_pty_text("first\rsecond\r\nthird\r");
    assert_eq!(cleaned, "first\nsecond\nthird");
}

#[test]
fn last_lines_keeps_the_tail() {
    let text = (0..10)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(last_lines(&text, 3), "7\n8\n9");
    assert_eq!(last_lines(&text, 50), text);
}

const SURVEY_SCREEN: &str = "6\n\n● How is Claude doing this session? (optional)\n  1: Bad  2: Fine  3: Good  0: Dismiss\n  Update installed · Restart to update";

/// Like `fake_host`, but `agent send-keys` appends its arguments to a log file. The screen
/// gains a `tick N` line per key received, as a pane that reacted to the key would.
fn keylogging_host(dir: &Path, name: &str, status: &str, screen: &str) -> (ScriptedHost, PathBuf) {
    keylogging_host_with(dir, name, status, screen, true)
}

fn keylogging_host_with(
    dir: &Path,
    name: &str,
    status: &str,
    screen: &str,
    reacts: bool,
) -> (ScriptedHost, PathBuf) {
    let log = dir.join("keys.log");
    let tick = u8::from(reacts);
    let script = format!(
        "#!/bin/sh\ncase \"$*\" in\n  *\"agent list\"*) n=$(wc -l < '{log}' 2>/dev/null || echo 0); printf '{{\"id\":\"x\",\"result\":{{\"agents\":[{{\"agent\":\"claude\",\"agent_status\":\"{status}\",\"cwd\":\"/tmp\",\"name\":\"{name}\",\"pane_id\":\"w1:p1\",\"workspace_id\":\"w1\",\"state_change_seq\":%d}}]}}}}' \"$((3 + n))\" ;;\n  *\"agent read\"*) n=$(wc -l < '{log}' 2>/dev/null || echo 0); if [ {tick} = 1 ]; then printf '%s\\ntick %s' '{screen}' \"$n\"; else printf '%s' '{screen}'; fi ;;\n  *\"send-keys\"*) echo \"$*\" >> '{log}'; printf '%s' '{{\"id\":\"x\",\"result\":{{}}}}' ;;\n  *) exit 2 ;;\nesac\n",
        screen = screen.replace('\'', ""),
        log = log.display()
    );
    let path = dir.join("scripted-host");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    (ScriptedHost::new(path), log)
}

fn drive_on(db: &Database, id: &str) {
    let id = id.to_string();
    db.with_conn(move |c| {
        registry::set_owner(c, &id, Some("th-1"))?;
        registry::set_drive(c, &id, true)?;
        Ok(())
    })
    .unwrap();
}

fn keys_logged(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn a_driven_session_has_the_survey_dismissed_without_spending_allowances() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    let task = seed_with_task(&db, "hs-s");
    drive_on(&db, "hs-s");
    let (host, log) = keylogging_host(tmp.path(), "hs-s", "done", SURVEY_SCREEN);

    sweep(tmp.path(), &db, &host, None).await;
    sweep(tmp.path(), &db, &host, None).await;

    let keys = keys_logged(&log);
    assert_eq!(keys.len(), 2, "{keys:?}");
    assert!(keys.iter().all(|k| k.ends_with("hs-s 0")), "{keys:?}");
    let row = db.with_conn(|c| registry::get(c, "hs-s")).unwrap().unwrap();
    assert_eq!((row.dismissals, row.nudges_sent, row.keys_sent), (2, 0, 0));
    let survey_comments = db
        .with_conn(|c| hq_db::tasks::list_comments(c, &task))
        .unwrap()
        .iter()
        .filter(|c| c.body.contains("feedback survey"))
        .count();
    assert_eq!(survey_comments, 1, "one comment per session");
    assert_eq!(alert_count(&db), 0);
}

#[tokio::test]
async fn an_observe_only_session_is_never_typed_into() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-o");
    let (host, log) = keylogging_host(tmp.path(), "hs-o", "done", SURVEY_SCREEN);
    sweep(tmp.path(), &db, &host, None).await;
    assert!(keys_logged(&log).is_empty());
}

#[tokio::test]
async fn a_trust_dialog_is_reported_not_answered() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-t");
    drive_on(&db, "hs-t");
    let screen = format!(
        "Do you trust the files in this folder?\n 1. Yes, proceed\n 2. No, exit\n{SURVEY_SCREEN}"
    );
    let (host, log) = keylogging_host(tmp.path(), "hs-t", "blocked", &screen);
    sweep(tmp.path(), &db, &host, None).await;
    assert!(keys_logged(&log).is_empty());
    assert_eq!(alert_count(&db), 1);
}

#[tokio::test]
async fn dismissals_stop_at_the_cap_and_notify_once() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-c");
    drive_on(&db, "hs-c");
    let (host, log) = keylogging_host(tmp.path(), "hs-c", "done", SURVEY_SCREEN);
    for _ in 0..registry::DISMISSAL_CAP + 3 {
        sweep(tmp.path(), &db, &host, None).await;
    }
    assert_eq!(keys_logged(&log).len() as i64, registry::DISMISSAL_CAP);
    let notices: i64 = db
        .with_conn(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM value_items WHERE dedup_key = 'session-dismiss-cap-hs-c'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(notices, 1, "one cap notice");
}

#[tokio::test]
async fn a_finished_alert_and_wake_survive_a_state_change_after_the_keypress() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-f");
    drive_on(&db, "hs-f");
    let (host, log) = keylogging_host(tmp.path(), "hs-f", "done", SURVEY_SCREEN);
    let wake = |db: &Database| {
        db.with_conn(|c| registry::get(c, "hs-f"))
            .unwrap()
            .unwrap()
            .pm_wake
    };

    sweep(tmp.path(), &db, &host, None).await;
    assert_eq!(keys_logged(&log).len(), 1);
    assert_eq!(
        wake(&db).as_deref(),
        Some(WAKE_FINISHED),
        "wake despite dismissal"
    );

    // The host bumped state_change_seq after the key; the next finished turn must wake again.
    db.with_conn(|c| {
        c.execute(
            "UPDATE harness_sessions SET pm_wake = NULL WHERE id = 'hs-f'",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    sweep(tmp.path(), &db, &host, None).await;
    assert_eq!(
        wake(&db).as_deref(),
        Some(WAKE_FINISHED),
        "new seq, new wake"
    );
}

#[tokio::test]
async fn survey_text_on_a_working_agent_gets_no_key() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-w");
    drive_on(&db, "hs-w");
    let (host, log) = keylogging_host(tmp.path(), "hs-w", "working", SURVEY_SCREEN);
    sweep(tmp.path(), &db, &host, None).await;
    assert!(keys_logged(&log).is_empty());
}

#[tokio::test]
async fn a_survey_beside_a_drafted_input_box_gets_no_key() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-x");
    drive_on(&db, "hs-x");
    let screen = format!("{SURVEY_SCREEN}\n│ > fix the login bug │");
    let (host, log) = keylogging_host(tmp.path(), "hs-x", "done", &screen);
    sweep(tmp.path(), &db, &host, None).await;
    assert!(keys_logged(&log).is_empty());
}

#[tokio::test]
async fn a_survey_that_does_not_close_is_dismissed_once_then_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-u");
    drive_on(&db, "hs-u");
    let (host, log) = keylogging_host_with(tmp.path(), "hs-u", "done", SURVEY_SCREEN, false);
    for _ in 0..4 {
        sweep(tmp.path(), &db, &host, None).await;
    }
    assert_eq!(
        keys_logged(&log).len(),
        1,
        "an unchanged tail means the key did nothing"
    );
    let notices: i64 = db
        .with_conn(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM value_items WHERE dedup_key = 'session-dismiss-cap-hs-u'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(notices, 1);
}

#[tokio::test]
async fn the_observed_draft_pane_gets_no_key() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-g");
    drive_on(&db, "hs-g");
    let screen = format!("{SURVEY_SCREEN}\n\n\n❯ 00000");
    let (host, log) = keylogging_host(tmp.path(), "hs-g", "done", &screen);
    sweep(tmp.path(), &db, &host, None).await;
    assert!(keys_logged(&log).is_empty());
}

/// A host whose one agent's process has ended with code 0: the pane is still
/// listed, with `status: exited`, and its screen can be read. Every call is logged.
fn exited_host(dir: &Path, name: &str, screen: &str) -> ScriptedHost {
    exited_host_with_code(dir, name, screen, Some(0))
}

/// `exited_host` with the exit code the host reports; `None` is a host that
/// reports none.
fn exited_host_with_code(dir: &Path, name: &str, screen: &str, code: Option<u32>) -> ScriptedHost {
    let code = code
        .map(|c| format!(r#","exit_code":{c}"#))
        .unwrap_or_default();
    let agent = format!(
        r#"{{"agent":"hermes","agent_status":"unknown","status":"exited","cwd":"/tmp","name":"{name}","pane_id":"w1:p1","workspace_id":"w1","state_change_seq":3{code}}}"#
    );
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> \"$(dirname \"$0\")/calls.log\"\ncase \"$*\" in\n  *\"agent list\"*) printf '%s' '{{\"id\":\"x\",\"result\":{{\"agents\":[{agent}]}}}}' ;;\n  *\"agent read\"*) printf '%s' '{}' ;;\n  *\"workspace close\"*) printf '%s' '{{\"id\":\"x\",\"result\":{{\"type\":\"ok\"}}}}' ;;\n  *) exit 2 ;;\nesac\n",
        screen.replace('\'', "")
    );
    let path = dir.join("scripted-host");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    ScriptedHost::new(path)
}

#[tokio::test]
async fn a_stack_run_whose_process_ended_is_read_closed_and_put_into_review() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    let task = seed_with_task(&db, "hs-run");
    db.with_conn(|c| {
        registry::set_origin(c, "hs-run", registry::ORIGIN_STACK)?;
        hq_db::tasks::open_for_session(c, "tk-1", "hs-run", "hs-run", "hermes", "local", "/tmp")
            .map(|_| ())
    })
    .unwrap();
    let host = exited_host(tmp.path(), "hs-run", "REPORT\nThree suppliers found.\n");

    sweep(tmp.path(), &db, &host, None).await;

    assert_eq!(status_of(&db, "hs-run"), registry::STATUS_EXITED);
    let (status, _) = task_state(&db, &task);
    assert_eq!(
        status,
        hq_db::tasks::STATUS_READY_FOR_REVIEW,
        "a run with output is a finished turn"
    );
    let snap = db
        .with_conn(|c| registry::last_snapshot(c, "hs-run"))
        .unwrap()
        .unwrap();
    assert!(
        snap.contains("Three suppliers found"),
        "the final screen is kept: {snap}"
    );
    let calls = std::fs::read_to_string(tmp.path().join("calls.log")).unwrap();
    assert!(
        calls.contains("workspace close w1"),
        "the dead pane is closed: {calls}"
    );
    assert!(only_message(tmp.path()).contains("Three suppliers found"));
}

#[tokio::test]
async fn a_stack_run_that_exited_non_zero_blocks_its_task_whatever_is_on_screen() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    let task = seed_with_task(&db, "hs-crash");
    db.with_conn(|c| {
        registry::set_origin(c, "hs-crash", registry::ORIGIN_STACK)?;
        // An earlier sweep stored a snapshot while the run was alive.
        registry::set_last_snapshot(c, "hs-crash", "Starting up...\nFetching sources\n")
    })
    .unwrap();
    let host = exited_host_with_code(
        tmp.path(),
        "hs-crash",
        "Traceback (most recent call last)\n",
        Some(1),
    );

    sweep(tmp.path(), &db, &host, None).await;

    assert_eq!(status_of(&db, "hs-crash"), registry::STATUS_EXITED);
    let (status, _) = task_state(&db, &task);
    assert_eq!(
        status,
        hq_db::tasks::STATUS_BLOCKED,
        "a non-zero exit is a crash, however much it printed before"
    );
    let notes: Vec<String> = db
        .with_conn(|c| {
            Ok(hq_db::tasks::list_comments(c, &task)?
                .into_iter()
                .map(|c| c.body)
                .collect())
        })
        .unwrap();
    assert!(
        notes
            .iter()
            .any(|n| n.contains("without finishing") && n.contains("never resumed")),
        "{notes:?}"
    );
}

#[tokio::test]
async fn a_stack_run_on_a_host_that_reports_no_exit_code_is_judged_by_its_screen() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    let task = seed_with_task(&db, "hs-empty");
    db.with_conn(|c| registry::set_origin(c, "hs-empty", registry::ORIGIN_STACK))
        .unwrap();
    let host = exited_host_with_code(tmp.path(), "hs-empty", "   \n", None);

    sweep(tmp.path(), &db, &host, None).await;

    assert_eq!(status_of(&db, "hs-empty"), registry::STATUS_EXITED);
    let (status, _) = task_state(&db, &task);
    assert_eq!(
        status,
        hq_db::tasks::STATUS_BLOCKED,
        "no code and no output means the run crashed"
    );
}

fn host_items(db: &Database) -> Vec<(String, String, String)> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT kind, title, state FROM value_items WHERE dedup_key LIKE 'host-%' ORDER BY created_at, rowid",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
    .unwrap()
}

fn poll(host: &str, answer: std::result::Result<(), &str>) -> hq_tools::harness_session::HostPoll {
    let mut polled = hq_tools::harness_session::HostPoll::new();
    polled.insert(
        host.into(),
        answer.map(|_| Vec::new()).map_err(String::from),
    );
    polled
}

#[test]
fn a_silent_host_alerts_once_after_the_wait_and_says_when_it_is_back() {
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-a");
    seed(&db, "hs-b");
    let rows = vec![row_of(&db, "hs-a"), row_of(&db, "hs-b")];
    let after = Some(Duration::from_secs(300));
    let silent = poll(
        "local",
        Err("ssh: connect to host laptop: Operation timed out\nmore"),
    );

    hosts::watch_hosts(&db, &silent, &rows, after, 10_000);
    hosts::watch_hosts(&db, &silent, &rows, after, 10_240);
    assert!(
        host_items(&db).is_empty(),
        "four minutes is not yet an outage worth a word"
    );

    hosts::watch_hosts(&db, &silent, &rows, after, 10_300);
    hosts::watch_hosts(&db, &silent, &rows, after, 10_360);
    let items = host_items(&db);
    assert_eq!(items.len(), 1, "one alert per outage: {items:?}");
    assert_eq!(items[0].0, "action_needed");
    let body: String = db
        .with_conn(|c| {
            Ok(c.query_row(
                "SELECT body FROM value_items WHERE dedup_key LIKE 'host-unreachable-%'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert!(
        body.contains("5 minutes") && body.contains("2 sessions"),
        "{body}"
    );
    assert!(body.contains("hs-a") && body.contains("hs-b"), "{body}");
    assert!(
        body.contains("Operation timed out") && !body.contains("more"),
        "{body}"
    );

    hosts::watch_hosts(&db, &poll("local", Ok(())), &rows, after, 10_900);
    let items = host_items(&db);
    assert_eq!(items.len(), 2, "{items:?}");
    assert_eq!(
        items[0].2, "expired",
        "the alert leaves the inbox: {items:?}"
    );
    assert_eq!(items[1].0, "fyi");
    assert!(items[1].1.contains("answering again"), "{items:?}");

    hosts::watch_hosts(&db, &poll("local", Ok(())), &rows, after, 11_000);
    assert_eq!(host_items(&db).len(), 2, "back is said once");
}

#[test]
fn a_short_silence_or_an_alert_turned_off_says_nothing() {
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-a");
    let rows = vec![row_of(&db, "hs-a")];
    let silent = poll("local", Err("timed out"));
    let after = Some(Duration::from_secs(300));

    hosts::watch_hosts(&db, &silent, &rows, after, 1_000);
    hosts::watch_hosts(&db, &poll("local", Ok(())), &rows, after, 1_120);
    assert!(host_items(&db).is_empty(), "a blip is not news");
    assert!(hq_db::host_outages::list(&db).unwrap().is_empty());

    hosts::watch_hosts(&db, &silent, &rows, None, 2_000);
    hosts::watch_hosts(&db, &silent, &rows, None, 9_000);
    assert!(host_items(&db).is_empty(), "the alert is off");
    assert_eq!(
        hq_db::host_outages::list(&db).unwrap().len(),
        1,
        "still tracked"
    );
}

#[test]
fn an_outage_whose_sessions_all_stopped_ends_quietly() {
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-a");
    let rows = vec![row_of(&db, "hs-a")];
    let after = Some(Duration::from_secs(60));
    let silent = poll("local", Err("timed out"));
    hosts::watch_hosts(&db, &silent, &rows, after, 1_000);
    hosts::watch_hosts(&db, &silent, &rows, after, 1_100);
    assert_eq!(host_items(&db).len(), 1);

    hosts::watch_hosts(&db, &Default::default(), &[], after, 1_200);
    let items = host_items(&db);
    assert_eq!(
        items.len(),
        1,
        "no back notice for a host nobody polls: {items:?}"
    );
    assert_eq!(items[0].2, "expired");
    assert!(hq_db::host_outages::list(&db).unwrap().is_empty());
}

#[tokio::test]
async fn a_sweep_records_a_silent_host_as_an_outage() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-away");
    let resolve: HostResolver = Arc::new(|name| bail!("host '{name}' unreachable: no route"));
    let sweep = Sweep {
        host_budget: HOST_SWEEP_BUDGET,
        unreachable_after: Some(Duration::ZERO),
    };
    supervise(tmp.path(), &db, None, resolve, sweep)
        .await
        .unwrap();

    let open = hq_db::host_outages::list(&db).unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].host, "local");
    assert!(
        open[0].alert_item_id.is_some(),
        "a zero wait alerts at once"
    );
    assert_eq!(status_of(&db, "hs-away"), registry::STATUS_RUNNING);
}

/// A host whose finished agent's folder is at `sha`.
fn host_at_commit(dir: &Path, name: &str, seq: u64, sha: &str) -> ScriptedHost {
    let list = format!(
        r#"{{"agent":"claude","agent_status":"done","cwd":"/tmp","name":"{name}","pane_id":"w1:p1","workspace_id":"w1","state_change_seq":{seq}}}"#
    );
    let head =
        format!(r#"{{"head":{{"sha":"{sha}","subject":"add the parser","branch":"feature"}}}}"#);
    let script = format!(
        "#!/bin/sh\ncase \"$*\" in\n  *\"agent list\"*) printf '%s' '{{\"result\":{{\"agents\":[{list}]}}}}' ;;\n  *\"agent read\"*) printf '%s' 'done' ;;\n  *\"agent head\"*) printf '%s' '{head}' ;;\n  *) exit 2 ;;\nesac\n"
    );
    let path = dir.join(format!("host-{seq}"));
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    ScriptedHost::new(path)
}

#[tokio::test]
async fn a_finished_turn_names_the_commit_it_made_once() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-w");
    db.with_conn(|c| {
        hq_db::session_checkouts::record(
            c,
            &hq_db::session_checkouts::NewCheckout {
                session_id: "hs-w",
                folder: "/tmp",
                worktree: None,
                repo: None,
                branch: None,
                detached: None,
                owns: &[],
                head: Some("aaa111"),
            },
        )
    })
    .unwrap();

    sweep(
        tmp.path(),
        &db,
        &host_at_commit(tmp.path(), "hs-w", 3, "bbb222"),
        None,
    )
    .await;
    let body = only_body(tmp.path());
    assert!(
        body.contains("It committed bbb222 \"add the parser\" on branch feature."),
        "{body}"
    );
    let events = db
        .with_conn(|c| registry::list_events(c, "hs-w", 10))
        .unwrap();
    assert_eq!(events.iter().filter(|e| e.kind == "commit").count(), 1);

    // The next finished turn with nothing new committed says nothing about a commit.
    std::fs::remove_dir_all(mailbox_dir(tmp.path())).unwrap();
    sweep(
        tmp.path(),
        &db,
        &host_at_commit(tmp.path(), "hs-w", 4, "bbb222"),
        None,
    )
    .await;
    let body = only_body(tmp.path());
    assert!(!body.contains("committed"), "{body}");
}

/// A host whose agent waits at a command dialog for `command`; it logs every call, and an
/// answer makes the dialog go.
fn host_at_command(dir: &Path, name: &str, seq: u64, command: &str) -> ScriptedHost {
    let pending = format!(
        r#"{{"kind":"command","question":"Do you want to run this command?","command":"{command}","options":[{{"id":1,"label":"Yes"}},{{"id":2,"label":"Yes, and do not ask again for {command}","remember":true}},{{"id":3,"label":"No, and tell me what to do differently","typed":true}}]}}"#
    );
    let agent = format!(
        r#"{{"agent":"codex","agent_status":"blocked","cwd":"/tmp","name":"{name}","pane_id":"w1:p1","workspace_id":"w1","state_change_seq":{seq},"pending_seq":{seq},"pending":{pending}}}"#
    );
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> \"$(dirname \"$0\")/calls.log\"\ncase \"$*\" in\n  *\"agent list\"*) printf '%s' '{{\"result\":{{\"agents\":[{agent}]}}}}' ;;\n  *\"agent read\"*) printf '%s' 'Do you want to run this command?' ;;\n  *\"agent pending\"*) printf '%s' '{{\"result\":{{\"pending_seq\":{seq},\"rule\":\"codex-command\",\"pending\":{pending}}}}}' ;;\n  *\"agent answer\"*) printf '%s' '{{\"result\":{{\"registered\":true}}}}' ;;\n  *) exit 2 ;;\nesac\n"
    );
    let path = dir.join(format!("host-{seq}"));
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    ScriptedHost::new(path)
}

fn with_builder(db: &Database, id: &str) {
    let builder = hq_core::command_policy::resolve("builder", &Default::default())
        .unwrap()
        .unwrap();
    db.with_conn(|c| hq_db::session_policies::record(c, id, &builder))
        .unwrap();
}

/// Waits for the background answer to record its decision.
async fn policy_outcome(db: &Database, id: &str) -> serde_json::Value {
    for _ in 0..100 {
        let events = db.with_conn(|c| registry::list_events(c, id, 10)).unwrap();
        if let Some(e) = events.iter().find(|e| e.kind == "policy_decision") {
            return serde_json::from_str(e.detail.as_deref().unwrap()).unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the policy never decided");
}

fn answers(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("calls.log"))
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains("agent answer"))
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn a_command_the_policy_allows_is_answered_once_and_raises_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-ok");
    with_builder(&db, "hs-ok");
    let host = host_at_command(tmp.path(), "hs-ok", 5, "cargo test");

    sweep(tmp.path(), &db, &host, None).await;
    let outcome = policy_outcome(&db, "hs-ok").await;
    assert_eq!(outcome["decision"], "allow");
    assert_eq!(outcome["outcome"], "allowed");
    let answered = answers(tmp.path());
    assert_eq!(answered.len(), 1, "{answered:?}");
    assert!(
        answered[0].contains("--option 1"),
        "the one-time yes, never remember: {answered:?}"
    );
    assert_eq!(alert_count(&db), 0, "nothing for the owner");
}

#[tokio::test]
async fn a_command_the_policy_denies_is_declined_without_asking_anyone() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-push");
    with_builder(&db, "hs-push");
    let host = host_at_command(tmp.path(), "hs-push", 6, "git push origin main");

    sweep(tmp.path(), &db, &host, None).await;
    let outcome = policy_outcome(&db, "hs-push").await;
    assert_eq!(outcome["decision"], "deny");
    assert_eq!(outcome["rule"], "git push");
    assert!(answers(tmp.path())[0].contains("--decline"));
    assert_eq!(alert_count(&db), 0);
}

#[tokio::test]
async fn a_command_the_policy_does_not_name_reaches_the_owner_with_the_reason() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-curl");
    with_builder(&db, "hs-curl");
    let host = host_at_command(tmp.path(), "hs-curl", 7, "curl https://example.com");

    sweep(tmp.path(), &db, &host, None).await;
    assert!(answers(tmp.path()).is_empty(), "nothing is pressed");
    assert_eq!(alert_count(&db), 1);
    let body: String = db
        .with_conn(|c| {
            Ok(c.query_row(
                "SELECT body FROM value_items WHERE dedup_key = 'session-blocked-hs-curl-7'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert!(
        body.contains("'builder' command policy left this to you")
            && body.contains("does not name curl"),
        "{body}"
    );
    let outcome = policy_outcome(&db, "hs-curl").await;
    assert_eq!(outcome["outcome"], "escalated");
}

#[tokio::test]
async fn without_a_policy_every_dialog_still_goes_to_the_owner() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-old");
    let host = host_at_command(tmp.path(), "hs-old", 8, "cargo test");
    sweep(tmp.path(), &db, &host, None).await;
    assert!(answers(tmp.path()).is_empty());
    assert_eq!(alert_count(&db), 1);
}

fn event_kinds(db: &Database, id: &str) -> Vec<(String, Option<String>)> {
    db.with_conn(|c| registry::list_events(c, id, 20))
        .unwrap()
        .into_iter()
        .filter(|e| e.actor == "supervisor")
        .map(|e| (e.kind, e.detail))
        .collect()
}

#[tokio::test]
async fn a_session_s_life_is_written_as_events_for_the_stream() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-life");

    sweep(
        tmp.path(),
        &db,
        &host_with_dialog(tmp.path(), "hs-life", 4),
        None,
    )
    .await;
    let dir2 = tempfile::tempdir().unwrap();
    let working = fake_host(dir2.path(), &[("hs-life", "working", 5)], "compiling");
    sweep(tmp.path(), &db, &working, None).await;
    let dir3 = tempfile::tempdir().unwrap();
    let done = fake_host(dir3.path(), &[("hs-life", "done", 6)], "All done.");
    sweep(tmp.path(), &db, &done, None).await;
    let dir4 = tempfile::tempdir().unwrap();
    sweep(tmp.path(), &db, &gone_host(dir4.path()), None).await;

    let events = event_kinds(&db, "hs-life");
    let kinds: Vec<&str> = events.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(kinds, ["blocked", "unblocked", "finished", "exited"]);
    let blocked: serde_json::Value = serde_json::from_str(events[0].1.as_deref().unwrap()).unwrap();
    assert_eq!(blocked["seq"], 4);
    assert_eq!(blocked["dialog"]["kind"], "path");
    assert_eq!(blocked["dialog"]["options"][1]["remember"], true);
    let unblocked: serde_json::Value =
        serde_json::from_str(events[1].1.as_deref().unwrap()).unwrap();
    assert!(unblocked["blocked_ms"].as_i64().is_some(), "{unblocked}");
    assert_eq!(unblocked["status"], "working");
}

#[test]
fn a_silent_host_is_an_event_on_each_of_its_sessions() {
    let db = Database::open_memory().unwrap();
    seed(&db, "hs-a");
    seed(&db, "hs-b");
    let rows = vec![row_of(&db, "hs-a"), row_of(&db, "hs-b")];
    let silent = poll("local", Err("timed out"));
    hosts::watch_hosts(&db, &silent, &rows, Some(Duration::ZERO), 100);
    hosts::watch_hosts(
        &db,
        &poll("local", Ok(())),
        &rows,
        Some(Duration::ZERO),
        400,
    );
    for id in ["hs-a", "hs-b"] {
        let kinds: Vec<String> = event_kinds(&db, id).into_iter().map(|(k, _)| k).collect();
        assert_eq!(kinds, ["host_unreachable", "host_back"], "{id}");
    }
}
