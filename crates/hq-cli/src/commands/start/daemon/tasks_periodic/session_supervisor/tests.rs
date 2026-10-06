use super::*;
use hq_core::config::{HerdrConfig, LOCAL_HOST};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// A pane redraw as a terminal would have logged it: cursor hiding, screen
/// erase, an OSC title, colour runs, and carriage returns between rows.
const ANSI_SAMPLE: &str = "\x1b[?25l\x1b[2J\x1b[H\x1b]0;osc-window-title\x07\r\x1b[38;5;242m> working\x1b[0m\r\n\x1b[1mFINAL ANSWER: 42\x1b[0m\r\n\x1b[?25h";

/// A Herdr host whose `agent list` reports `agents` and whose `agent read`
/// prints `screen`.
fn fake_host(dir: &Path, agents: &[(&str, &str, u64)], screen: &str) -> HerdrHost {
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
    let path = dir.join("herdr");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let cfg = HerdrConfig {
        binary: path.to_string_lossy().to_string(),
        ..HerdrConfig::default()
    };
    HerdrHost::from_config(&cfg, LOCAL_HOST).unwrap()
}

fn gone_host(dir: &Path) -> HerdrHost {
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

async fn sweep(vault: &Path, db: &Database, host: &HerdrHost, s: Option<&Summarizer>) {
    let host = host.clone();
    let resolve: HostResolver = Arc::new(move |_| Ok(host.clone()));
    supervise(vault, db, s, resolve, TEST_HOST_BUDGET)
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
    supervise(tmp.path(), &db, None, resolve, HOST_SWEEP_BUDGET)
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
    let path = tmp.path().join("herdr");
    std::fs::write(&path, "#!/bin/sh\nsleep 30\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let cfg = HerdrConfig {
        binary: path.to_string_lossy().to_string(),
        command_timeout_secs: 30,
        ..HerdrConfig::default()
    };
    let hung = HerdrHost::from_config(&cfg, LOCAL_HOST).unwrap();
    let resolve: HostResolver = Arc::new(move |_| Ok(hung.clone()));

    let started = Instant::now();
    supervise(tmp.path(), &db, None, resolve, Duration::from_millis(500))
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

    assert_eq!(task_state(&db, &task), (hq_db::tasks::STATUS_BLOCKED.into(), 1));
    let body = only_body(tmp.path());
    assert!(body.contains("Task FR-001 is blocked"), "{body}");
    assert!(interrupts(tmp.path()), "stopped work on a task is a blocker, not digest news");
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
    assert_eq!(task_state(&db, &task), (hq_db::tasks::STATUS_IN_PROGRESS.into(), 0));
}

#[tokio::test]
async fn an_unreachable_host_leaves_the_task_untouched() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    let task = seed_with_task(&db, "hs-laptop");

    let resolve: HostResolver = Arc::new(|name| bail!("host '{name}' unreachable: no route"));
    supervise(tmp.path(), &db, None, resolve, HOST_SWEEP_BUDGET)
        .await
        .unwrap();

    assert_eq!(task_state(&db, &task), (hq_db::tasks::STATUS_IN_PROGRESS.into(), 0));
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
    assert!(!interrupts(tmp.path()), "a finished turn is a digest update");
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
    assert_eq!(task_state(&db, &task), (hq_db::tasks::STATUS_IN_PROGRESS.into(), 1));
}

fn watch_from_chat(db: &Database, id: &str) {
    db.with_conn(|c| registry::set_owner(c, id, Some("th-web"))).unwrap();
}

fn row_of(db: &Database, id: &str) -> registry::HarnessSessionRow {
    db.with_conn(|c| registry::get(c, id)).unwrap().unwrap()
}

fn value_item_count(db: &Database) -> i64 {
    db.with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM value_items", [], |r| r.get::<_, i64>(0))?))
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
    assert!(mailbox_files(tmp.path()).is_empty(), "web chat only: nothing for Telegram");
    assert_eq!(task_state(&db, &task).0, hq_db::tasks::STATUS_READY_FOR_REVIEW);

    let blocked = fake_host(tmp.path(), &[("hs-web", "blocked", 5)], "Approve the edit?");
    sweep(tmp.path(), &db, &blocked, None).await;
    assert_eq!(row_of(&db, "hs-web").pm_wake.as_deref(), Some(WAKE_BLOCKED));
    assert_eq!(value_item_count(&db), 1, "one web-inbox badge for the block");
    assert!(
        hq_daemon::value_bus::WEB_ONLY_SOURCES.contains(&WEB_INBOX_SOURCE),
        "the badge must never reach Telegram"
    );

    sweep(tmp.path(), &db, &gone_host(tmp.path()), None).await;
    let row = row_of(&db, "hs-web");
    assert_eq!(row.status, registry::STATUS_EXITED);
    assert_eq!(row.pm_wake.as_deref(), Some(WAKE_EXITED));
    assert!(mailbox_files(tmp.path()).is_empty());
    assert_eq!(value_item_count(&db), 1, "the exit adds nothing for the relay");
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
fn keylogging_host(dir: &Path, name: &str, status: &str, screen: &str) -> (HerdrHost, PathBuf) {
    keylogging_host_with(dir, name, status, screen, true)
}

fn keylogging_host_with(
    dir: &Path,
    name: &str,
    status: &str,
    screen: &str,
    reacts: bool,
) -> (HerdrHost, PathBuf) {
    let log = dir.join("keys.log");
    let tick = u8::from(reacts);
    let script = format!(
        "#!/bin/sh\ncase \"$*\" in\n  *\"agent list\"*) n=$(wc -l < '{log}' 2>/dev/null || echo 0); printf '{{\"id\":\"x\",\"result\":{{\"agents\":[{{\"agent\":\"claude\",\"agent_status\":\"{status}\",\"cwd\":\"/tmp\",\"name\":\"{name}\",\"pane_id\":\"w1:p1\",\"workspace_id\":\"w1\",\"state_change_seq\":%d}}]}}}}' \"$((3 + n))\" ;;\n  *\"agent read\"*) n=$(wc -l < '{log}' 2>/dev/null || echo 0); if [ {tick} = 1 ]; then printf '%s\\ntick %s' '{screen}' \"$n\"; else printf '%s' '{screen}'; fi ;;\n  *\"send-keys\"*) echo \"$*\" >> '{log}'; printf '%s' '{{\"id\":\"x\",\"result\":{{}}}}' ;;\n  *) exit 2 ;;\nesac\n",
        screen = screen.replace('\'', ""),
        log = log.display()
    );
    let path = dir.join("herdr");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let cfg = HerdrConfig {
        binary: path.to_string_lossy().to_string(),
        ..HerdrConfig::default()
    };
    (HerdrHost::from_config(&cfg, LOCAL_HOST).unwrap(), log)
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
    assert_eq!(wake(&db).as_deref(), Some(WAKE_FINISHED), "wake despite dismissal");

    // Herdr bumped state_change_seq after the key; the next finished turn must wake again.
    db.with_conn(|c| {
        c.execute("UPDATE harness_sessions SET pm_wake = NULL WHERE id = 'hs-f'", [])?;
        Ok(())
    })
    .unwrap();
    sweep(tmp.path(), &db, &host, None).await;
    assert_eq!(wake(&db).as_deref(), Some(WAKE_FINISHED), "new seq, new wake");
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
    assert_eq!(keys_logged(&log).len(), 1, "an unchanged tail means the key did nothing");
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
