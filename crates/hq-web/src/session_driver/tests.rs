use super::*;

fn row(drive: bool) -> HarnessSessionRow {
    HarnessSessionRow {
        id: "hs-1".into(),
        harness: "claude-code".into(),
        label: "auth".into(),
        host: "laptop".into(),
        agent_name: "hs-1".into(),
        workspace_id: None,
        pane_id: None,
        cwd: "/repo".into(),
        status: "running".into(),
        resume_token: None,
        mission_id: None,
        created_at: String::new(),
        updated_at: String::new(),
        owner_thread: Some("th-1".into()),
        drive,
        pm_wake: None,
        last_driven_at: None,
        last_agent_status: None,
        last_seen_at: None,
        goal: Some(GOAL.into()),
        done_criteria: Some(DONE.into()),
        keys_sent: 0,
        origin: "user".into(),
        dismissals: 0,
        last_dismiss_tail: None,
        nudges_sent: 0,
        last_wake_nudges: None,
        no_progress_streak: 0,
        progress_mark: None,
        drive_off_reason: None,
    }
}

const GOAL: &str = "Add rate limiting to the login endpoint";
const DONE: &str = "Login returns 429 after 5 failed attempts and cargo test passes";

fn test_state() -> Arc<WsState> {
    Arc::new(WsState::new(
        std::env::temp_dir().join(format!("hq-driver-test-{}", uuid::Uuid::new_v4())),
        None,
    ))
}

/// A chat watching session `hs-w`, which has a `blocked` wake pending.
fn watched_with_wake(state: &WsState) -> String {
    let thread = state
        .db
        .with_conn(|c| Ok(hq_db::chat::create_thread(c, "Work", "user", "user")?.thread_id))
        .unwrap();
    state
        .db
        .with_conn(|c| {
            registry::insert(
                c,
                &registry::NewSession {
                    id: "hs-w",
                    harness: "pi",
                    label: "",
                    cwd: "/repo",
                    mission_id: None,
                    placement: registry::Placement { host: "laptop", agent_name: "hs-w", workspace_id: "w1", pane_id: "w1:p1" },
                },
            )?;
            registry::set_owner(c, "hs-w", Some(&thread))?;
            registry::set_wake(c, "hs-w", "blocked")?;
            Ok(())
        })
        .unwrap();
    thread
}

fn messages(state: &WsState, thread: &str) -> Vec<hq_db::chat::ChatMessage> {
    state.db.with_conn(|c| hq_db::chat::get_messages(c, thread, 100)).unwrap()
}

#[tokio::test]
async fn a_driven_session_over_its_daily_cap_gets_updates_instead_of_turns() {
    let state = test_state();
    let thread = watched_with_wake(&state);
    state
        .db
        .with_conn(|c| {
            registry::set_goal(c, "hs-w", Some(GOAL), Some(DONE), registry::ACTOR_USER)?;
            registry::set_drive(c, "hs-w", true)
        })
        .unwrap();
    let drove = json!({"driver": {"session_id": "hs-w", "reason": "finished", "mode": "drive"}});
    for _ in 0..MAX_DRIVE_TURNS_PER_DAY {
        crate::ws::post_assistant_message(&state, &thread, "sent the next step", &drove);
    }
    assert_eq!(drive_turns_today(&state, "hs-w"), MAX_DRIVE_TURNS_PER_DAY);
    assert_eq!(drive_turns_today(&state, "hs-other"), 0);

    drive_due(&state).await;

    let last = messages(&state, &thread).pop().unwrap();
    assert!(last.content.contains("Drive is paused"), "{}", last.content);
    assert!(state.active_chat_turns.read().await.is_empty(), "no driver turn was started");
}

#[tokio::test]
async fn a_wake_for_a_chat_a_read_only_ask_owns_is_dropped_with_one_notice_not_requeued() {
    let state = test_state();
    let thread = watched_with_wake(&state);
    state
        .db
        .with_conn(|c| {
            registry::set_goal(c, "hs-w", Some(GOAL), Some(DONE), registry::ACTOR_USER)?;
            registry::set_drive(c, "hs-w", true)?;
            hq_db::ask_requests::open(
                c,
                &hq_db::ask_requests::NewAsk {
                    thread: hq_db::ask_requests::ThreadTarget::Existing(&thread),
                    external_id: None,
                    scope: "handoff",
                    mode: "read_only",
                    caller: "c",
                    fingerprint: "f",
                },
            )?;
            Ok(())
        })
        .unwrap();

    drive_due(&state).await;
    drive_due(&state).await;

    let notes = messages(&state, &thread);
    assert_eq!(notes.len(), 1, "one notice, and the wake did not come back");
    assert!(notes[0].content.contains("read-only question"), "{}", notes[0].content);
    assert!(state.active_chat_turns.read().await.is_empty(), "no driver turn was started");
    let row = state.db.with_conn(|c| registry::get(c, "hs-w")).unwrap().unwrap();
    assert!(row.pm_wake.is_none(), "the wake is dropped, not handed back");
}

#[tokio::test]
async fn an_archived_chat_lets_go_of_its_sessions() {
    let state = test_state();
    let thread = watched_with_wake(&state);
    state.db.with_conn(|c| hq_db::chat::archive_thread(c, &thread)).unwrap();

    drive_due(&state).await;

    assert!(messages(&state, &thread).is_empty());
    let row = state.db.with_conn(|c| registry::get(c, "hs-w")).unwrap().unwrap();
    assert!(row.owner_thread.is_none(), "its events go back to the relay");
}

#[tokio::test]
async fn a_watched_session_wake_posts_one_update_into_its_chat() {
    let state = test_state();
    let thread = watched_with_wake(&state);

    drive_due(&state).await;
    drive_due(&state).await;

    let messages = state.db.with_conn(|c| hq_db::chat::get_messages(c, &thread, 10)).unwrap();
    assert_eq!(messages.len(), 1, "one update, the wake is claimed once");
    assert!(messages[0].content.contains("needs an answer"), "{}", messages[0].content);
    let row = state.db.with_conn(|c| registry::get(c, "hs-w")).unwrap().unwrap();
    assert!(row.pm_wake.is_none());
}

#[tokio::test]
async fn a_driven_session_whose_goal_stopped_passing_the_gate_is_observed_not_driven() {
    let state = test_state();
    let thread = watched_with_wake(&state);
    state
        .db
        .with_conn(|c| {
            registry::set_goal(c, "hs-w", Some(GOAL), Some(DONE), registry::ACTOR_USER)?;
            registry::set_drive(c, "hs-w", true)?;
            c.execute("UPDATE harness_sessions SET done_criteria = 'done' WHERE id = 'hs-w'", [])?;
            Ok(())
        })
        .unwrap();

    drive_due(&state).await;

    let last = messages(&state, &thread).pop().unwrap();
    assert!(last.content.contains("needs an answer"), "an update, not a driver turn: {}", last.content);
    assert!(state.active_chat_turns.read().await.is_empty(), "no driver turn was started");
    let row = state.db.with_conn(|c| registry::get(c, "hs-w")).unwrap().unwrap();
    assert!(!row.drive);
    let events = state.db.with_conn(|c| registry::list_events(c, "hs-w", 10)).unwrap();
    assert_eq!(events.last().unwrap().kind, registry::EVENT_DRIVE_OFF);
}

/// `hs-w` driven with a goal, a wake of `reason` pending, and an optional linked task.
fn driven(
    state: &WsState,
    reason: &str,
    task_status: Option<&str>,
) -> (String, Option<String>) {
    let thread = watched_with_wake(state);
    let task = task_status.map(|status| {
        state
            .db
            .with_conn(|c| {
                c.execute(
                    "INSERT INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('in-1', 'personal', 'Work', 'work', 'FR')",
                    [],
                )?;
                let task = t::create_task(c, "tk-1", "in-1", &t::NewTask { title: "login limits", created_by: "test", ..Default::default() })?;
                c.execute("UPDATE tasks SET status = ?1 WHERE id = ?2", rusqlite::params![status, task.id])?;
                Ok(task.id)
            })
            .unwrap()
    });
    state
        .db
        .with_conn(|c| {
            registry::set_goal(c, "hs-w", Some(GOAL), Some(DONE), registry::ACTOR_USER)?;
            registry::request_drive(c, "hs-w", true, registry::ACTOR_USER)?;
            registry::set_wake(c, "hs-w", reason)?;
            if let Some(id) = &task {
                registry::set_mission(c, "hs-w", id)?;
            }
            Ok(())
        })
        .unwrap();
    (thread, task)
}

fn row_of(state: &WsState) -> HarnessSessionRow {
    state
        .db
        .with_conn(|c| registry::get(c, "hs-w"))
        .unwrap()
        .unwrap()
}

/// The row went off by a guard: one notice, an event, no turn, no wake left, and a second pass says nothing more.
async fn assert_switched_off(state: &Arc<WsState>, thread: &str, why: &str) {
    let notices: Vec<_> = messages(state, thread)
        .into_iter()
        .filter(|m| m.content.contains("Drive switched off"))
        .collect();
    assert_eq!(notices.len(), 1, "one notice");
    assert!(notices[0].content.contains(why), "{}", notices[0].content);
    let row = row_of(state);
    assert!(!row.drive && row.pm_wake.is_none());
    assert!(row.drive_off_reason.as_deref().unwrap().contains(why));
    let events = state
        .db
        .with_conn(|c| registry::list_events(c, "hs-w", 20))
        .unwrap();
    let last = events.last().unwrap();
    assert_eq!(
        (last.kind.as_str(), last.actor.as_str()),
        (registry::EVENT_DRIVE_OFF, registry::ACTOR_GUARD)
    );
    assert!(
        state.active_chat_turns.read().await.is_empty(),
        "no driver turn was started"
    );
    let before = messages(state, thread).len();
    drive_due(state).await;
    assert_eq!(
        messages(state, thread).len(),
        before,
        "the notice does not wake the driver again"
    );
}

#[tokio::test]
async fn a_session_that_used_its_nudge_budget_is_switched_off_with_one_notice() {
    let state = test_state();
    let (thread, _) = driven(&state, "finished", None);
    let budget = herdr_config(&state).nudge_budget();
    state
        .db
        .with_conn(|c| {
            Ok(c.execute(
                "UPDATE harness_sessions SET nudges_sent = ?1 WHERE id = 'hs-w'",
                [budget],
            )?)
        })
        .unwrap();

    drive_due(&state).await;

    assert_switched_off(&state, &thread, "nudge budget is used").await;
}

#[tokio::test]
async fn finished_turns_with_no_new_tool_activity_switch_drive_off() {
    let state = test_state();
    let (thread, _) = driven(&state, "finished", None);
    let limit = herdr_config(&state).no_progress_limit();
    let screen = "⏺ Bash(cargo test)\n  ⎿ 12 passed\n⏺ All done, ready for your review.";
    let mark = tool_activity(screen).join("\n");
    state
        .db
        .with_conn(|c| {
            registry::set_last_snapshot(c, "hs-w", screen)?;
            registry::set_last_wake_nudges(c, "hs-w", 0)?;
            registry::set_progress(c, "hs-w", limit - 1, Some(&mark))
        })
        .unwrap();

    drive_due(&state).await;

    assert_switched_off(&state, &thread, "no new tool activity").await;
}

#[test]
fn progress_is_new_tool_lines_and_a_harness_that_prints_none_is_left_to_the_budget() {
    let first =
        tool_activity("⏺ Bash(cargo test 12 passed)\nplain chatter\n⏺ Update(src/lib.rs)");
    assert_eq!(first, ["Bash(cargo test ## passed)", "Update(src/lib.rs)"]);
    assert_eq!(
        tool_activity("• Ran cargo fmt\n> Do it again"),
        ["Ran cargo fmt"]
    );
    let again = tool_activity("⏺ Bash(cargo test 99 passed)\nI wrapped up.");
    assert_eq!(
        judge_progress(&first, &again),
        Progress::Stalled,
        "the same action with another count is not new"
    );
    let edited = tool_activity("⏺ Update(src/auth.rs)");
    assert_eq!(judge_progress(&first, &edited), Progress::Moved);
    assert_eq!(
        judge_progress(&[], &first),
        Progress::Moved,
        "the first finished turn is progress"
    );
    assert_eq!(judge_progress(&[], &[]), Progress::Unknown);
}

#[tokio::test]
async fn tracking_progress_counts_a_stall_and_resets_on_new_work() {
    let state = test_state();
    driven(&state, "finished", None);
    let set_screen = |s: &str| {
        state
            .db
            .with_conn(|c| registry::set_last_snapshot(c, "hs-w", s))
            .unwrap()
    };

    set_screen("⏺ Bash(cargo test)");
    assert_eq!(track_progress(&state, &row_of(&state)), 0);
    set_screen("⏺ Bash(cargo test)\nwrapped up");
    assert_eq!(track_progress(&state, &row_of(&state)), 1);
    set_screen("wrapped up again");
    assert_eq!(
        track_progress(&state, &row_of(&state)),
        2,
        "no tool lines now but there were before: a stall"
    );
    set_screen("⏺ Update(src/lib.rs)");
    assert_eq!(track_progress(&state, &row_of(&state)), 0);
}

#[tokio::test]
async fn a_task_the_user_completed_switches_drive_off() {
    let state = test_state();
    let (thread, _) = driven(&state, "blocked", Some("complete"));

    drive_due(&state).await;

    assert_switched_off(&state, &thread, "no longer in progress").await;
    let comments = state.db.with_conn(|c| t::list_comments(c, "tk-1")).unwrap();
    assert!(
        comments
            .iter()
            .any(|c| c.body.contains("switched Drive off")),
        "the task records it"
    );
}

#[tokio::test]
async fn a_check_in_on_a_task_that_left_in_progress_switches_drive_off() {
    let state = test_state();
    let (thread, _) = driven(&state, "blocked", Some("ready_for_review"));
    state
        .db
        .with_conn(|c| {
            registry::claim_wake(c, "hs-w", "blocked")?;
            registry::set_seen(c, "hs-w", "idle")?;
            Ok(c.execute(
                "UPDATE harness_sessions SET last_driven_at = NULL WHERE id = 'hs-w'",
                [],
            )?)
        })
        .unwrap();

    drive_due(&state).await;

    assert_switched_off(&state, &thread, "ready for review").await;
}

#[tokio::test]
async fn a_finished_turn_that_follows_no_instruction_is_not_driven_a_second_time() {
    let state = test_state();
    let (thread, _) = driven(&state, "finished", None);
    state
        .db
        .with_conn(|c| registry::set_last_wake_nudges(c, "hs-w", 0))
        .unwrap();

    drive_due(&state).await;

    let last = messages(&state, &thread).pop().unwrap();
    assert!(
        last.content.contains("not a reply to an instruction"),
        "{}",
        last.content
    );
    assert!(
        state.active_chat_turns.read().await.is_empty(),
        "no driver turn was started"
    );
    assert!(row_of(&state).drive, "this is not a reason to stop driving");
}

#[tokio::test]
async fn a_driven_row_whose_session_ended_is_switched_off() {
    let state = test_state();
    let (thread, _) = driven(&state, "finished", None);
    state
        .db
        .with_conn(|c| {
            Ok(c.execute(
                "UPDATE harness_sessions SET status = 'exited' WHERE id = 'hs-w'",
                [],
            )?)
        })
        .unwrap();

    drive_due(&state).await;

    assert_switched_off(&state, &thread, "session ended").await;
}

#[test]
fn running_the_same_tool_again_is_work_but_an_unchanged_screen_is_not() {
    let once = tool_activity("⏺ Bash(cargo test)\n⏺ Update(src/lib.rs)");
    let twice = tool_activity("⏺ Bash(cargo test)\nok\n⏺ Bash(cargo test)\n⏺ Update(src/lib.rs)");
    assert_eq!(twice.len(), 3, "repeats are kept");
    assert_eq!(judge_progress(&once, &twice), Progress::Moved, "another run of the same command is new work");
    assert_eq!(judge_progress(&twice, &twice), Progress::Stalled);
    assert_eq!(judge_progress(&twice, &once), Progress::Stalled, "fewer lines scrolled out is not work");
}

#[tokio::test]
async fn a_finished_turn_after_a_user_typed_send_is_not_counted_as_a_stall() {
    let state = test_state();
    let (thread, _) = driven(&state, "finished", None);
    let screen = "⏺ Bash(cargo test)";
    state
        .db
        .with_conn(|c| {
            registry::set_last_snapshot(c, "hs-w", screen)?;
            registry::set_progress(c, "hs-w", herdr_config(&state).no_progress_limit() - 1, Some(&tool_activity(screen).join("\n")))
        })
        .unwrap();
    assert!(row_of(&state).last_wake_nudges.is_none());

    let stop = driver_guard(&state, &row_of(&state), None, WAKE_FINISHED, &herdr_config(&state));

    assert!(stop.is_none(), "{stop:?}");
    assert_eq!(row_of(&state).no_progress_streak, herdr_config(&state).no_progress_limit() - 1, "left alone");
    assert!(messages(&state, &thread).is_empty());
}

#[tokio::test]
async fn the_key_allowance_stops_drive_separately_from_the_instruction_budget() {
    let state = test_state();
    let (thread, _) = driven(&state, "blocked", None);
    let keys = herdr_config(&state).key_allowance();
    state.db.with_conn(|c| Ok(c.execute("UPDATE harness_sessions SET keys_sent = ?1 WHERE id = 'hs-w'", [keys])?)).unwrap();

    drive_due(&state).await;

    assert_switched_off(&state, &thread, "key allowance is used").await;
}

#[tokio::test]
async fn a_check_in_does_not_stop_drive_on_the_task_rule_while_the_agent_is_working() {
    let state = test_state();
    driven(&state, "blocked", Some("ready_for_review"));
    let cfg = herdr_config(&state);
    let task = state.db.with_conn(|c| t::get_task(c, "tk-1")).unwrap();

    let mut row = row_of(&state);
    row.last_agent_status = Some("working".into());
    assert!(driver_guard(&state, &row, task.as_ref(), CHECK_IN, &cfg).is_none(), "someone is typing in the pane");
    row.last_agent_status = Some("idle".into());
    assert!(driver_guard(&state, &row, task.as_ref(), CHECK_IN, &cfg).unwrap().contains("ready for review"));
}

#[test]
fn a_driver_prompt_carries_the_goal_and_the_escalation_limits() {
    let prompt = driver_prompt(&row(true), None, "blocked", 8);
    assert!(
        prompt.contains(GOAL) && prompt.contains(DONE),
        "goal and definition of done"
    );
    assert!(prompt.contains("has not shown the goal is met"));
    assert!(prompt.contains("harness_session_logs"));
    assert!(prompt.contains("approve it with `harness_session_send` keys"));
    assert!(prompt.contains("Never mark the task complete"));
    assert!(prompt.contains("credentials"));
    assert!(prompt.contains("waiting at a prompt"));
}

#[test]
fn an_update_names_the_session_and_its_task() {
    let text = update_text(&row(false), None, "finished");
    assert!(text.contains("hs-1") && text.contains("finished a turn"), "{text}");
}
