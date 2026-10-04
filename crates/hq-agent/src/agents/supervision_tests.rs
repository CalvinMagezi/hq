//! Tests for the durable run registry and per-child delivery on the
//! [`AgentService`](super::AgentService) path (FR-076 slices 1 and 2).

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use hq_db::Database;
use hq_db::subagent_runs::{self as runs, ListFilter, Origin};

use super::tests::{MarkerProvider, event_collector, goal_task, service_with, wait_for_events};
use super::tool::SpawnSubagentsTool;
use super::types::{ChildExecContext, ChildPlan, ChildStatus};

fn temp_db() -> Arc<Database> {
    let path = std::env::temp_dir().join(format!("hq-supervision-{}.db", uuid::Uuid::new_v4()));
    Arc::new(Database::open(&path).expect("open file-backed test db"))
}

fn origin(chat: &str) -> Origin {
    Origin {
        platform: "web".into(),
        chat_id: chat.into(),
        thread_id: Some(chat.into()),
        identity: None,
    }
}

fn ctx_for(chat: &str) -> ChildExecContext {
    ChildExecContext {
        origin: Some(origin(chat)),
        parent_turn_id: Some("turn-1".into()),
        ..Default::default()
    }
}

fn svc(db: &Arc<Database>) -> (super::service::AgentService, Arc<MarkerProvider>) {
    let provider = MarkerProvider::new("hq-mock");
    let service =
        service_with(provider.clone(), 0, 3, vec![std::env::temp_dir()]).with_db(Some(db.clone()));
    (service, provider)
}

fn row(db: &Arc<Database>, run_id: &str) -> runs::RunRow {
    db.with_conn(|c| runs::get(c, run_id))
        .unwrap()
        .expect("run row exists")
}

#[tokio::test]
async fn blocker_past_the_preview_limits_is_flagged_and_retrievable() {
    let db = temp_db();
    let (service, _) = svc(&db);
    let goal = format!("{}\nBLOCKER: no image tool available", "x".repeat(900));
    let outcomes = service
        .execute(ChildPlan::single(goal_task("t1", &goal)), ctx_for("chat-a"))
        .await;

    let o = &outcomes[0];
    assert_eq!(o.status, ChildStatus::Completed);
    let info = o.run.as_ref().expect("settled run info");
    assert_eq!(info.accept_status, runs::ACCEPT_PARTIAL);
    assert!(info.blocker.as_deref().unwrap().contains("no image tool"));

    let stored = row(&db, &info.run_id);
    assert!(
        stored
            .output_full
            .unwrap()
            .contains("no image tool available")
    );
    assert!(stored.output_preview.unwrap().len() <= runs::RESULT_PREVIEW_BYTES + 3);
}

#[tokio::test]
async fn blocker_reaches_the_parent_even_when_the_200_byte_preview_cuts_it() {
    let db = temp_db();
    let (service, _) = svc(&db);
    let tool = SpawnSubagentsTool::new(Arc::new(service)).with_context(ctx_for("chat-a"));
    let goal = format!("{}\nMISSING: example image", "y".repeat(400));
    let result = crate::tools::AgentTool::execute(
        &tool,
        "call-1",
        serde_json::json!({ "task": { "id": "t1", "goal": goal } }),
    )
    .await
    .unwrap();
    let text = &result.content[0].text;
    assert!(text.contains("accept=partial"), "{text}");
    assert!(text.contains("MISSING: example image"), "{text}");
    assert!(text.contains("subagent_run_result"), "{text}");
}

#[tokio::test]
async fn missing_capability_blocks_before_the_child_runs() {
    let db = temp_db();
    let (service, provider) = svc(&db);
    let mut req = goal_task("t1", "write the vault note");
    req.required_tools = vec!["vault_write_note".into()];
    let outcomes = service
        .execute(ChildPlan::single(req), ctx_for("chat-a"))
        .await;

    let o = &outcomes[0];
    assert_eq!(o.status, ChildStatus::Blocked);
    assert!(o.error.as_deref().unwrap().contains("vault_write_note"));
    assert_eq!(provider.completions.load(Ordering::SeqCst), 0);
    let info = o.run.as_ref().unwrap();
    assert_eq!(info.accept_status, runs::ACCEPT_BLOCKED);
    assert_eq!(row(&db, &info.run_id).exec_status, runs::EXEC_BLOCKED);
}

#[tokio::test]
async fn a_tool_the_child_has_does_not_block_it() {
    let db = temp_db();
    let (service, provider) = svc(&db);
    let mut req = goal_task("t1", "read a file");
    req.required_tools = vec!["read_file".into(), "bash".into()];
    let outcomes = service
        .execute(ChildPlan::single(req), ctx_for("chat-a"))
        .await;
    assert_eq!(
        outcomes[0].status,
        ChildStatus::Completed,
        "{:?}",
        outcomes[0]
    );
    assert_eq!(provider.completions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn finished_session_with_unmet_criteria_stays_unverified() {
    let db = temp_db();
    let (service, _) = svc(&db);
    let mut req = goal_task("t1", "update the doc");
    req.success_criteria = vec!["image inserted".into(), "render checked".into()];
    let outcomes = service
        .execute(ChildPlan::single(req), ctx_for("chat-a"))
        .await;
    let info = outcomes[0].run.as_ref().unwrap();
    assert_eq!(outcomes[0].status, ChildStatus::Completed);
    assert_eq!(info.accept_status, runs::ACCEPT_UNVERIFIED);
    let stored = row(&db, &info.run_id);
    assert_eq!(stored.success_criteria.len(), 2);
    assert_eq!(stored.accept_status, runs::ACCEPT_UNVERIFIED);
}

#[tokio::test]
async fn fast_blocked_child_is_delivered_while_a_slow_sibling_still_runs() {
    let db = temp_db();
    let (service, _) = svc(&db);
    let (events, sink) = event_collector();
    let mut ctx = ctx_for("chat-a");
    ctx.completion_sink = Some(sink);

    let mut plan = ChildPlan::parallel(
        vec![
            goal_task("slow", "slow work [[SLEEP]]"),
            goal_task("fast", "quick\nBLOCKER: no access to the doc"),
        ],
        2,
    );
    plan.blocking = Some(false);
    let started = std::time::Instant::now();
    let (inline, run_ids) = service.execute_with_runs(plan, ctx).await;
    assert!(inline.is_empty());
    assert_eq!(run_ids.len(), 2);

    wait_for_events(&events, 1, Duration::from_secs(2)).await;
    assert!(
        started.elapsed() < Duration::from_millis(2500),
        "fast child waited on its slow sibling"
    );
    {
        let seen = events.lock().unwrap();
        assert_eq!(seen.len(), 1, "slow sibling must still be running");
        assert_eq!(seen[0].task_id, "fast");
        assert_eq!(seen[0].accept_status.as_deref(), Some(runs::ACCEPT_PARTIAL));
        assert!(seen[0].run_id.is_some());
    }

    wait_for_events(&events, 2, Duration::from_secs(10)).await;
    let slow_run = run_ids.iter().find(|(t, _)| t == "slow").unwrap().1.clone();
    assert_eq!(row(&db, &slow_run).exec_status, runs::EXEC_COMPLETED);
}

#[tokio::test]
async fn each_detached_child_writes_exactly_one_outbox_event() {
    let db = temp_db();
    let (service, _) = svc(&db);
    let (events, sink) = event_collector();
    let mut ctx = ctx_for("chat-a");
    ctx.completion_sink = Some(sink);
    let mut plan = ChildPlan::parallel(vec![goal_task("a", "one"), goal_task("b", "two")], 2);
    plan.blocking = Some(false);
    let (_, run_ids) = service.execute_with_runs(plan, ctx).await;
    wait_for_events(&events, 2, Duration::from_secs(10)).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(events.lock().unwrap().len(), 2);
    for (_, run_id) in run_ids {
        let rows = db.with_conn(|c| runs::events_for_run(c, &run_id)).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, runs::EVENT_PENDING);
    }
}

#[tokio::test]
async fn background_without_a_sink_is_queued_durably_with_a_registry() {
    let db = temp_db();
    let (service, _) = svc(&db);
    let mut plan = ChildPlan::single(goal_task("t1", "work"));
    plan.blocking = Some(false);
    let (inline, run_ids) = service.execute_with_runs(plan, ctx_for("chat-a")).await;
    assert!(inline.is_empty(), "accepted, not rejected");
    let run_id = run_ids[0].1.clone();
    for _ in 0..100 {
        if row(&db, &run_id).settled_at.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let events = db.with_conn(|c| runs::events_for_run(c, &run_id)).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].status, runs::EVENT_PENDING);
}

#[tokio::test]
async fn background_with_no_route_at_all_is_rejected_not_silently_dropped() {
    let provider = MarkerProvider::new("hq-mock");
    let service = service_with(provider.clone(), 0, 3, vec![std::env::temp_dir()]);
    let mut plan = ChildPlan::single(goal_task("t1", "work"));
    plan.blocking = Some(false);
    let outcomes = service.execute(plan, ChildExecContext::default()).await;
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].status, ChildStatus::Rejected);
    assert!(
        outcomes[0]
            .error
            .as_deref()
            .unwrap()
            .contains("no completion route")
    );
    assert_eq!(provider.completions.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cancelled_run_is_not_resurrected_when_the_child_finishes() {
    let db = temp_db();
    let (service, _) = svc(&db);
    let mut plan = ChildPlan::single(goal_task("t1", "slow [[SLEEP]]"));
    plan.blocking = Some(false);
    let (_, run_ids) = service.execute_with_runs(plan, ctx_for("chat-a")).await;
    let run_id = run_ids[0].1.clone();

    assert!(db.with_conn(|c| runs::cancel(c, &run_id, 1)).unwrap());
    tokio::time::sleep(Duration::from_millis(3600)).await;

    let stored = row(&db, &run_id);
    assert_eq!(stored.exec_status, runs::EXEC_CANCELLED);
    assert!(
        db.with_conn(|c| runs::events_for_run(c, &run_id))
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn runs_carry_their_task_link_and_stay_inside_their_chat() {
    let db = temp_db();
    let (service, _) = svc(&db);
    let mut req = goal_task("t1", "work");
    req.task_id = Some("task-123".into());
    service
        .execute(ChildPlan::single(req), ctx_for("chat-a"))
        .await;
    service
        .execute(
            ChildPlan::single(goal_task("t2", "other")),
            ctx_for("chat-b"),
        )
        .await;

    let linked = db
        .with_conn(|c| {
            runs::list(
                c,
                &ListFilter {
                    task_id: Some("task-123".into()),
                    ..Default::default()
                },
            )
        })
        .unwrap();
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0].chat_id.as_deref(), Some("chat-a"));

    let chat_b = db
        .with_conn(|c| {
            runs::list(
                c,
                &ListFilter {
                    scope: Some(origin("chat-b")),
                    ..Default::default()
                },
            )
        })
        .unwrap();
    assert_eq!(chat_b.len(), 1);
    assert_eq!(chat_b[0].child_id, "t2");
}

#[tokio::test]
async fn followup_turn_children_inherit_depth_and_stop_waking_past_the_cap() {
    let db = temp_db();
    let (service, _) = svc(&db);
    let first = service
        .execute(
            ChildPlan::single(goal_task("t1", "work")),
            ctx_for("chat-a"),
        )
        .await;
    let mut run_id = first[0].run.as_ref().unwrap().run_id.clone();

    for _ in 0..=runs::MAX_FOLLOWUP_DEPTH {
        let mut ctx = ctx_for("chat-a");
        ctx.parent_turn_id = Some(format!("{}{run_id}", runs::FOLLOWUP_TURN_PREFIX));
        let mut plan = ChildPlan::single(goal_task("t1", "work"));
        plan.blocking = Some(false);
        let (_, ids) = service.execute_with_runs(plan, ctx).await;
        run_id = ids[0].1.clone();
        for _ in 0..100 {
            if row(&db, &run_id).settled_at.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    assert!(row(&db, &run_id).followup_depth > runs::MAX_FOLLOWUP_DEPTH);
    let events = db.with_conn(|c| runs::events_for_run(c, &run_id)).unwrap();
    assert_eq!(events[0].status, runs::EVENT_SUPPRESSED);
}
