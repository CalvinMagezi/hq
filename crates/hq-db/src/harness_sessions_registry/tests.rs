#[test]
fn nudge_column_is_one_of_two_schema_columns() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::migrations::run(&conn).unwrap();
    for keys in [false, true] {
        let column = nudge_column(keys);
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM pragma_table_info('harness_sessions') WHERE name = ?1",
                [column],
                |r| r.get(0),
            )
            .unwrap();
        assert!(exists, "{column} is not a harness_sessions column");
    }
    assert_ne!(nudge_column(true), nudge_column(false));
}

use super::*;
use crate::pool::Database;

fn add(c: &Connection, id: &str, harness: &str) -> Result<()> {
    insert(
        c,
        &NewSession {
            id,
            harness,
            label: "auth refactor",
            cwd: "/tmp",
            mission_id: None,
            placement: Placement {
                host: "local",
                agent_name: id,
                workspace_id: "w1",
                pane_id: "w1:p1",
            },
        },
    )
}

const GOAL: &str = "Add rate limiting to the login endpoint";
const DONE: &str = "Login returns 429 after 5 failed attempts and cargo test passes";

fn watched(c: &Connection, id: &str) -> Result<()> {
    add(c, id, "pi")?;
    watch_from_chat(c, id, "th-1", false)?;
    Ok(())
}

#[test]
fn drive_needs_a_goal_that_passes_the_gate_and_refusals_are_audited() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        watched(c, "hs-g")?;
        assert_eq!(request_drive(c, "hs-missing", true, ACTOR_USER)?, DriveChange::NotWatched);

        let DriveChange::Refused(gaps) = request_drive(c, "hs-g", true, ACTOR_USER)? else {
            panic!("a session with no goal must not be driven");
        };
        assert!(gaps.iter().any(|g| g.starts_with("goal is missing")), "{gaps:?}");
        assert!(!get(c, "hs-g")?.unwrap().drive);

        set_goal(c, "hs-g", Some("TBD"), Some("when done"), ACTOR_USER)?;
        assert!(matches!(request_drive(c, "hs-g", true, ACTOR_HQ)?, DriveChange::Refused(_)), "ambiguous");

        set_goal(c, "hs-g", Some(GOAL), Some(DONE), ACTOR_USER)?;
        assert_eq!(request_drive(c, "hs-g", true, ACTOR_USER)?, DriveChange::Changed(true));
        assert!(get(c, "hs-g")?.unwrap().drive);

        let kinds: Vec<String> = list_events(c, "hs-g", 20)?.into_iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            [EVENT_DRIVE_REFUSED, EVENT_GOAL_SET, EVENT_DRIVE_REFUSED, EVENT_GOAL_SET, EVENT_DRIVE_ON]
        );
        let last = list_events(c, "hs-g", 1)?.pop().unwrap();
        assert_eq!(last.goal.as_deref(), Some(GOAL), "the audit record carries the goal in force");
        assert_eq!(last.done_criteria.as_deref(), Some(DONE));
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_changed_goal_that_fails_the_gate_stops_driving() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        watched(c, "hs-c")?;
        set_goal(c, "hs-c", Some(GOAL), Some(DONE), ACTOR_USER)?;
        request_drive(c, "hs-c", true, ACTOR_USER)?;

        let same = set_goal(c, "hs-c", Some("Add rate limiting to the signup endpoint too"), None, ACTOR_USER)?.unwrap();
        assert!(!same.drive_stopped && same.gaps.is_empty());
        assert!(get(c, "hs-c")?.unwrap().drive, "a still-valid edit keeps driving");

        let update = set_goal(c, "hs-c", None, Some("done"), ACTOR_USER)?.unwrap();
        assert!(update.drive_stopped, "{update:?}");
        assert!(!get(c, "hs-c")?.unwrap().drive);
        let last = list_events(c, "hs-c", 1)?.pop().unwrap();
        assert_eq!((last.kind.as_str(), last.actor.as_str()), (EVENT_DRIVE_OFF, ACTOR_GATE));
        assert_eq!(set_goal(c, "hs-nope", Some(GOAL), None, ACTOR_USER)?, None);
        Ok(())
    })
    .unwrap();
}

#[test]
fn enforce_gate_stops_a_driven_row_that_predates_the_gate() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        watched(c, "hs-old")?;
        set_drive(c, "hs-old", true)?;
        assert!(!enforce_gate(c, "hs-old")?);
        assert!(!get(c, "hs-old")?.unwrap().drive);

        watched(c, "hs-ok")?;
        set_goal(c, "hs-ok", Some(GOAL), Some(DONE), ACTOR_USER)?;
        set_drive(c, "hs-ok", true)?;
        assert!(enforce_gate(c, "hs-ok")?);
        Ok(())
    })
    .unwrap();
}

#[test]
fn drive_off_always_works_and_an_ended_session_cannot_be_driven() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        watched(c, "hs-e")?;
        set_goal(c, "hs-e", Some(GOAL), Some(DONE), ACTOR_USER)?;
        request_drive(c, "hs-e", true, ACTOR_USER)?;
        set_status(c, "hs-e", STATUS_EXITED)?;

        assert_eq!(request_drive(c, "hs-e", false, ACTOR_USER)?, DriveChange::Changed(false));
        let DriveChange::Refused(why) = request_drive(c, "hs-e", true, ACTOR_USER)? else {
            panic!("an exited session must not be driven");
        };
        assert!(why[0].contains("not running"), "{why:?}");

        relaunch(c, "hs-e", &Placement { host: "local", agent_name: "hs-e", workspace_id: "w2", pane_id: "w2:p1" })?;
        assert_eq!(request_drive(c, "hs-e", true, ACTOR_USER)?, DriveChange::Changed(true), "goal survives a resume");
        Ok(())
    })
    .unwrap();
}

#[test]
fn crud_roundtrip() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        add(c, "hs-1", "claude-code")?;
        let s = get(c, "hs-1")?.unwrap();
        assert_eq!(s.harness, "claude-code");
        assert_eq!(s.status, STATUS_RUNNING);
        assert_eq!(s.host, "local");
        assert_eq!(s.pane_id.as_deref(), Some("w1:p1"));

        set_resume_token(c, "hs-1", "sess-abc")?;
        set_status(c, "hs-1", STATUS_EXITED)?;
        let s = get(c, "hs-1")?.unwrap();
        assert_eq!(s.resume_token.as_deref(), Some("sess-abc"));
        assert_eq!(s.status, STATUS_EXITED);

        assert_eq!(list(c, Some(STATUS_EXITED), 10)?.len(), 1);
        assert_eq!(list(c, Some(STATUS_RUNNING), 10)?.len(), 0);
        assert_eq!(list(c, None, 10)?.len(), 1);
        Ok(())
    })
    .unwrap();
}

#[test]
fn mission_queries_see_only_that_missions_sessions() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        for (id, mission) in [("hs-a", Some("tk-1")), ("hs-b", Some("tk-1")), ("hs-c", None)] {
            insert(
                c,
                &NewSession {
                    id,
                    harness: "pi",
                    label: "",
                    cwd: "/tmp",
                    mission_id: mission,
                    placement: Placement {
                        host: "local",
                        agent_name: id,
                        workspace_id: "w1",
                        pane_id: "w1:p1",
                    },
                },
            )?;
        }
        assert_eq!(list_for_mission(c, "tk-1")?.len(), 2);
        assert_eq!(count_running_for_mission(c, "tk-1", "hs-a")?, 1);
        set_status(c, "hs-b", STATUS_EXITED)?;
        assert_eq!(count_running_for_mission(c, "tk-1", "hs-a")?, 0);
        assert!(list_for_mission(c, "tk-other")?.is_empty());
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_wake_needs_a_watching_thread_and_is_claimed_once() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        add(c, "hs-w", "pi")?;
        assert!(!set_wake(c, "hs-w", "finished")?, "no thread watches it yet");
        assert!(!set_drive(c, "hs-w", true)?);
        assert!(set_owner(c, "hs-w", Some("th-1"))?);
        assert!(set_wake(c, "hs-w", "blocked")?);
        assert!(set_wake(c, "hs-w", "finished")?);
        assert!(!claim_wake(c, "hs-w", "blocked")?, "a newer wake replaced it");
        assert_eq!(list_due_for_driver(c, 30)?.len(), 1);
        assert!(claim_wake(c, "hs-w", "finished")?);
        assert!(!claim_wake(c, "hs-w", "finished")?);
        assert!(list_due_for_driver(c, 30)?.is_empty(), "not driven, nothing due");
        assert_eq!(list_for_thread(c, "th-1")?.len(), 1);
        Ok(())
    })
    .unwrap();
}

#[test]
fn only_a_session_no_chat_watched_takes_the_new_watch_drive() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        let drive = |c: &Connection, id: &str| get(c, id).map(|s| s.unwrap().drive);
        assert!(!watch_from_chat(c, "hs-missing", "th-1", true)?);

        add(c, "hs-new", "pi")?;
        assert!(watch_from_chat(c, "hs-new", "th-1", true)?);
        assert!(drive(c, "hs-new")?, "a new watch drives by default");

        add(c, "hs-optout", "pi")?;
        watch_from_chat(c, "hs-optout", "th-1", false)?;
        assert!(!drive(c, "hs-optout")?, "drive=false opts out");

        add(c, "hs-off", "pi")?;
        set_owner(c, "hs-off", Some("th-1"))?;
        watch_from_chat(c, "hs-off", "th-1", true)?;
        assert!(!drive(c, "hs-off")?, "watching again never turns the user's switch on");
        set_drive(c, "hs-off", true)?;
        watch_from_chat(c, "hs-off", "th-1", false)?;
        assert!(drive(c, "hs-off")?, "nor off: the switch stays where the user left it");

        watch_from_chat(c, "hs-off", "th-2", true)?;
        let moved = get(c, "hs-off")?.unwrap();
        assert_eq!(moved.owner_thread.as_deref(), Some("th-2"));
        assert!(!moved.drive, "a session taken from another chat starts with Drive off");
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_driven_session_checks_in_when_due_and_unwatching_stops_it() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        add(c, "hs-d", "pi")?;
        set_owner(c, "hs-d", Some("th-1"))?;
        assert!(set_drive(c, "hs-d", true)?);
        assert!(list_due_for_driver(c, 30)?.is_empty(), "never seen by a sweep: host unknown");
        set_seen(c, "hs-d", "working")?;
        assert_eq!(list_due_for_driver(c, 30)?.len(), 1, "never driven: due at once");
        assert!(claim_checkin(c, "hs-d", 30)?);
        assert!(!claim_checkin(c, "hs-d", 30)?);
        assert!(list_due_for_driver(c, 30)?.is_empty());
        c.execute("UPDATE harness_sessions SET last_driven_at = datetime('now', '-31 minutes') WHERE id = 'hs-d'", [])?;
        assert_eq!(list_due_for_driver(c, 30)?.len(), 1);

        c.execute("UPDATE harness_sessions SET last_seen_at = datetime('now', '-2 hours') WHERE id = 'hs-d'", [])?;
        assert!(list_due_for_driver(c, 30)?.is_empty(), "an unreachable host gets no check-in");
        assert!(!claim_checkin(c, "hs-d", 30)?);
        set_wake(c, "hs-d", "exited")?;
        assert_eq!(list_due_for_driver(c, 30)?.len(), 1, "a wake is due whatever the host");
        claim_wake(c, "hs-d", "exited")?;

        set_wake(c, "hs-d", "finished")?;
        set_owner(c, "hs-d", None)?;
        let s = get(c, "hs-d")?.unwrap();
        assert!(!s.drive && s.pm_wake.is_none() && s.owner_thread.is_none());
        assert!(list_due_for_driver(c, 30)?.is_empty());
        Ok(())
    })
    .unwrap();
}

#[test]
fn seen_status_is_stored_and_only_a_change_notifies() {
    let db = Database::open_memory().unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sink = seen.clone();
    on_change(move |id| {
        if id == "hs-seen" {
            sink.lock().unwrap().push(id.to_string());
        }
    });
    db.with_conn(|c| {
        add(c, "hs-seen", "pi")?;
        set_seen(c, "hs-seen", "working")?;
        set_seen(c, "hs-seen", "working")?;
        set_seen(c, "hs-seen", "done")?;
        let s = get(c, "hs-seen")?.unwrap();
        assert_eq!(s.last_agent_status.as_deref(), Some("done"));
        assert!(s.last_seen_at.is_some());
        Ok(())
    })
    .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[test]
fn a_relaunch_resets_the_alert_claim_for_the_new_agent() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        add(c, "hs-re", "pi")?;
        assert!(claim_state_alert(c, "hs-re", 9)?);
        set_status(c, "hs-re", STATUS_EXITED)?;
        relaunch(c, "hs-re", &Placement { host: "local", agent_name: "hs-re", workspace_id: "w2", pane_id: "w2:p1" })?;
        assert!(claim_state_alert(c, "hs-re", 2)?, "the new agent's first state alerts");
        Ok(())
    })
    .unwrap();
}

#[test]
fn relaunch_moves_the_session_and_marks_it_running() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        add(c, "hs-r", "pi")?;
        set_status(c, "hs-r", STATUS_EXITED)?;
        relaunch(
            c,
            "hs-r",
            &Placement {
                host: "laptop",
                agent_name: "hs-r",
                workspace_id: "w7",
                pane_id: "w7:p1",
            },
        )?;
        let s = get(c, "hs-r")?.unwrap();
        assert_eq!(s.status, STATUS_RUNNING);
        assert_eq!(s.host, "laptop");
        assert_eq!(s.workspace_id.as_deref(), Some("w7"));
        Ok(())
    })
    .unwrap();
}

#[test]
fn exit_claim_is_won_once() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        add(c, "hs-2", "pi")?;
        assert!(set_status_exited_if_running(c, "hs-2")?);
        assert!(!set_status_exited_if_running(c, "hs-2")?);
        assert!(!set_status_exited_if_running(c, "hs-missing")?);
        assert_eq!(get(c, "hs-2")?.unwrap().status, STATUS_EXITED);
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_state_alert_is_claimed_once_per_state_change() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        add(c, "hs-b", "pi")?;
        assert!(claim_state_alert(c, "hs-b", 5)?);
        assert!(!claim_state_alert(c, "hs-b", 5)?);
        assert!(!claim_state_alert(c, "hs-b", 4)?);
        assert!(claim_state_alert(c, "hs-b", 9)?);
        Ok(())
    })
    .unwrap();
}

#[test]
fn snapshot_roundtrips_without_touching_updated_at() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        add(c, "hs-3", "pi")?;
        assert_eq!(last_snapshot(c, "hs-3")?, None);
        c.execute(
            "UPDATE harness_sessions SET updated_at = '2020-01-01 00:00:00' WHERE id = 'hs-3'",
            [],
        )?;

        set_last_snapshot(c, "hs-3", "pane text")?;
        assert_eq!(last_snapshot(c, "hs-3")?.as_deref(), Some("pane text"));
        assert_eq!(get(c, "hs-3")?.unwrap().updated_at, "2020-01-01 00:00:00");
        assert_eq!(last_snapshot(c, "hs-missing")?, None);
        Ok(())
    })
    .unwrap();
}

#[test]
fn the_herdr_migration_orphans_tmux_era_running_rows() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!("../../sql/031_harness_sessions.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../../sql/036_harness_session_snapshot.sql"))
        .unwrap();
    conn.execute(
        "INSERT INTO harness_sessions (id, harness, tmux_session, cwd, logfile) VALUES ('old', 'pi', 'hq-old', '/tmp', '/l')",
        [],
    )
    .unwrap();
    conn.execute_batch(include_str!("../../sql/043_harness_sessions_herdr.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../../sql/061_harness_session_driver.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../../sql/064_harness_session_goal.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../../sql/070_harness_session_drive_guards.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../../sql/071_harness_session_dismissals.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../../sql/072_harness_session_dismiss_tail.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../../sql/076_session_parent.sql"))
        .unwrap();
    let s = get(&conn, "old").unwrap().unwrap();
    assert_eq!(s.status, STATUS_ORPHANED);
    assert_eq!(s.agent_name, "hq-old");
    assert_eq!(s.host, "local");
}

#[test]
fn a_guard_stop_keeps_its_reason_until_someone_switches_drive_again() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        watched(c, "hs-g")?;
        set_goal(c, "hs-g", Some(GOAL), Some(DONE), ACTOR_USER)?;
        request_drive(c, "hs-g", true, ACTOR_USER)?;
        assert!(stop_drive(c, "hs-g", ACTOR_GUARD, "budget used")?);
        assert!(
            !stop_drive(c, "hs-g", ACTOR_GUARD, "again")?,
            "only a driven session is stopped, once"
        );
        let row = get(c, "hs-g")?.unwrap();
        assert!(!row.drive);
        assert_eq!(row.drive_off_reason.as_deref(), Some("budget used"));
        let events = list_events(c, "hs-g", 10)?;
        let last = events.last().unwrap();
        assert_eq!(
            (
                last.kind.as_str(),
                last.actor.as_str(),
                last.detail.as_deref()
            ),
            (EVENT_DRIVE_OFF, ACTOR_GUARD, Some("budget used"))
        );
        request_drive(c, "hs-g", true, ACTOR_USER)?;
        assert!(
            get(c, "hs-g")?.unwrap().drive_off_reason.is_none(),
            "switching Drive on clears the reason"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn only_the_user_switching_drive_on_refills_the_budget() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        watched(c, "hs-n")?;
        set_goal(c, "hs-n", Some(GOAL), Some(DONE), ACTOR_USER)?;
        request_drive(c, "hs-n", true, ACTOR_USER)?;
        for _ in 0..2 {
            assert!(reserve_nudge(c, "hs-n", "th-1", false, 8)?);
            note_send(c, "hs-n", true, false)?;
        }
        assert!(reserve_nudge(c, "hs-n", "th-1", true, 8)?);
        set_progress(c, "hs-n", 2, Some("Bash(cargo test)"))?;
        request_drive(c, "hs-n", false, ACTOR_USER)?;
        request_drive(c, "hs-n", true, ACTOR_HQ)?;
        let row = get(c, "hs-n")?.unwrap();
        assert_eq!(
            (row.nudges_sent, row.no_progress_streak),
            (2, 2),
            "HQ cannot top itself up"
        );
        request_drive(c, "hs-n", false, ACTOR_USER)?;
        request_drive(c, "hs-n", true, ACTOR_USER)?;
        let row = get(c, "hs-n")?.unwrap();
        assert_eq!(
            (row.nudges_sent, row.no_progress_streak, row.progress_mark),
            (0, 0, None)
        );
        let nudges = list_events(c, "hs-n", 50)?
            .iter()
            .filter(|e| e.kind == EVENT_NUDGE)
            .count();
        assert_eq!(
            nudges, 2,
            "every driver instruction is an event without its text"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn the_gate_stopping_drive_leaves_a_reason_for_the_panel() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        watched(c, "hs-q")?;
        set_goal(c, "hs-q", Some(GOAL), Some(DONE), ACTOR_USER)?;
        request_drive(c, "hs-q", true, ACTOR_USER)?;
        set_goal(c, "hs-q", None, Some("done"), ACTOR_USER)?;
        let row = get(c, "hs-q")?.unwrap();
        assert!(!row.drive);
        assert!(row.drive_off_reason.is_some());
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_reservation_is_atomic_per_counter_and_only_for_the_driving_chat() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        watched(c, "hs-a")?;
        set_goal(c, "hs-a", Some(GOAL), Some(DONE), ACTOR_USER)?;
        request_drive(c, "hs-a", true, ACTOR_USER)?;
        assert!(reserve_nudge(c, "hs-a", "th-1", false, 2)?);
        assert!(reserve_nudge(c, "hs-a", "th-1", false, 2)?);
        assert!(!reserve_nudge(c, "hs-a", "th-1", false, 2)?, "the limit holds, not one more");
        assert!(reserve_nudge(c, "hs-a", "th-1", true, 2)?, "keys have their own allowance");
        assert!(!reserve_nudge(c, "hs-a", "th-other", true, 9)?, "not the chat that drives it");
        refund_nudge(c, "hs-a", false)?;
        assert!(reserve_nudge(c, "hs-a", "th-1", false, 2)?, "a failed send gives its slot back");
        stop_drive(c, "hs-a", ACTOR_GUARD, "x")?;
        assert!(!reserve_nudge(c, "hs-a", "th-1", true, 9)?, "Drive is off");
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_non_user_cannot_drive_past_the_cap_but_the_user_can() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        for id in ["hs-1", "hs-2"] {
            watched(c, id)?;
            set_goal(c, id, Some(GOAL), Some(DONE), ACTOR_USER)?;
        }
        assert_eq!(request_drive_capped(c, "hs-1", true, ACTOR_HQ, Some(1))?, DriveChange::Changed(true));
        assert!(matches!(request_drive_capped(c, "hs-2", true, ACTOR_HQ, Some(1))?, DriveChange::Refused(_)));
        assert_eq!(request_drive_capped(c, "hs-2", true, ACTOR_USER, Some(1))?, DriveChange::Changed(true));
        Ok(())
    })
    .unwrap();
}

#[test]
fn the_caps_count_running_driven_sessions_and_sessions_by_origin() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        watched(c, "hs-1")?;
        watched(c, "hs-2")?;
        set_goal(c, "hs-1", Some(GOAL), Some(DONE), ACTOR_USER)?;
        request_drive(c, "hs-1", true, ACTOR_USER)?;
        assert_eq!(count_driven_running(c)?, 1);
        assert_eq!(count_running_with_origin(c, ORIGIN_ASK)?, 0);
        set_origin(c, "hs-2", ORIGIN_ASK)?;
        set_origin(c, "hs-1", ORIGIN_MCP)?;
        assert_eq!((count_running_with_origin(c, ORIGIN_ASK)?, count_running_with_origin(c, ORIGIN_MCP)?), (1, 1));
        set_status(c, "hs-1", STATUS_STOPPED)?;
        assert_eq!(count_driven_running(c)?, 0);
        assert_eq!(count_running_with_origin(c, ORIGIN_MCP)?, 0);
        Ok(())
    })
    .unwrap();
}

fn insert_session(c: &rusqlite::Connection, id: &str) {
    insert(
        c,
        &NewSession {
            id,
            harness: "claude-code",
            label: "t",
            cwd: "/t",
            mission_id: None,
            placement: Placement { host: "native", agent_name: id, workspace_id: id, pane_id: id },
        },
    )
    .unwrap();
}

#[test]
fn a_child_remembers_its_parent_and_depth_and_only_running_children_are_listed() {
    let db = crate::Database::open_memory().unwrap();
    db.with_conn(|c| {
        for id in ["hs-p", "hs-c1", "hs-c2", "hs-x"] {
            insert_session(c, id);
        }
        set_parent(c, "hs-c1", "hs-p", 1)?;
        set_parent(c, "hs-c2", "hs-p", 1)?;
        let row = get(c, "hs-c1")?.unwrap();
        assert_eq!((row.parent_session_id.as_deref(), row.spawn_depth), (Some("hs-p"), 1));
        assert_eq!(get(c, "hs-x")?.unwrap().parent_session_id, None);

        assert_eq!(running_children(c, "hs-p")?.len(), 2);
        set_status(c, "hs-c2", STATUS_STOPPED)?;
        let running = running_children(c, "hs-p")?;
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].id, "hs-c1");
        assert_eq!(children_started_since(c, "hs-p", 60)?, 2, "stopped ones still count toward the rate");
        assert_eq!(children_started_since(c, "hs-x", 60)?, 0);
        Ok(())
    })
    .unwrap();
}
