use super::*;

fn conn() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    crate::migrations::run(&c).unwrap();
    c
}

fn origin(chat: &str) -> Origin {
    Origin {
        platform: "web".into(),
        chat_id: chat.into(),
        thread_id: Some(chat.into()),
        identity: None,
    }
}

fn new_run(id: &str, chat: &str) -> NewRun {
    NewRun {
        run_id: id.into(),
        parent_run_id: None,
        parent_turn_id: Some("turn-1".into()),
        origin: Some(origin(chat)),
        task_id: None,
        child_id: format!("child-{id}"),
        role: "general".into(),
        goal: "do it".into(),
        success_criteria: vec!["doc updated".into()],
        required_tools: vec![],
        detached: true,
        owner_pid: Some(4242),
        followup_depth: 0,
    }
}

fn settlement(exec: &str, accept: &str, out: &str) -> Settlement {
    Settlement {
        exec_status: exec.into(),
        accept_status: accept.into(),
        missing_deliverables: vec![],
        blocker_reason: None,
        next_action: None,
        output_full: out.into(),
        output_preview: preview(out, 10),
        error: None,
        resolved_backend: "hq".into(),
    }
}

#[test]
fn settle_is_idempotent_and_writes_one_event() {
    let c = conn();
    insert_run(&c, &new_run("run-aaaaaa", "t1"), 100).unwrap();
    let s = settlement(EXEC_COMPLETED, ACCEPT_UNVERIFIED, "done");
    assert!(settle(&c, "run-aaaaaa", &s, 110, true).unwrap());
    assert!(!settle(&c, "run-aaaaaa", &s, 111, true).unwrap());
    assert_eq!(events_for_run(&c, "run-aaaaaa").unwrap().len(), 1);
}

#[test]
fn full_output_survives_beyond_preview_limits() {
    let c = conn();
    insert_run(&c, &new_run("run-bbbbbb", "t1"), 100).unwrap();
    let long = format!("{}BLOCKER: no image tool", "x".repeat(900));
    let s = settlement(EXEC_COMPLETED, ACCEPT_PARTIAL, &long);
    settle(&c, "run-bbbbbb", &s, 110, false).unwrap();
    let row = get(&c, "run-bbbbbb").unwrap().unwrap();
    assert!(row.output_full.unwrap().ends_with("BLOCKER: no image tool"));
    assert!(row.output_preview.unwrap().len() < 20);
}

#[test]
fn two_claimants_cannot_both_win_one_event() {
    let c = conn();
    insert_run(&c, &new_run("run-cccccc", "t1"), 100).unwrap();
    settle(
        &c,
        "run-cccccc",
        &settlement(EXEC_COMPLETED, ACCEPT_UNVERIFIED, "ok"),
        110,
        true,
    )
    .unwrap();
    let first = claim_due(&c, "worker-a", "web", 120, 10).unwrap();
    let second = claim_due(&c, "worker-b", "web", 120, 10).unwrap();
    assert_eq!(first.len(), 1);
    assert!(second.is_empty());
}

#[test]
fn stale_claim_is_reclaimed_after_ttl() {
    let c = conn();
    insert_run(&c, &new_run("run-dddddd", "t1"), 100).unwrap();
    settle(
        &c,
        "run-dddddd",
        &settlement(EXEC_COMPLETED, ACCEPT_UNVERIFIED, "ok"),
        110,
        true,
    )
    .unwrap();
    assert_eq!(claim_due(&c, "a", "web", 120, 10).unwrap().len(), 1);
    assert!(
        claim_due(&c, "b", "web", 120 + CLAIM_TTL_SECS - 1, 10)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        claim_due(&c, "b", "web", 120 + CLAIM_TTL_SECS, 10)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn delivered_event_is_never_claimed_again() {
    let c = conn();
    insert_run(&c, &new_run("run-eeeeee", "t1"), 100).unwrap();
    settle(
        &c,
        "run-eeeeee",
        &settlement(EXEC_COMPLETED, ACCEPT_UNVERIFIED, "ok"),
        110,
        true,
    )
    .unwrap();
    let (ev, _) = claim_due(&c, "a", "web", 120, 10).unwrap().remove(0);
    assert!(mark_delivered(&c, ev.id, 125).unwrap());
    assert!(claim_due(&c, "a", "web", 10_000, 10).unwrap().is_empty());
}

#[test]
fn send_failure_retries_with_backoff_then_fails_for_good() {
    let c = conn();
    insert_run(&c, &new_run("run-ffffff", "t1"), 100).unwrap();
    settle(
        &c,
        "run-ffffff",
        &settlement(EXEC_COMPLETED, ACCEPT_UNVERIFIED, "ok"),
        110,
        true,
    )
    .unwrap();
    let mut now = 120;
    for attempt in 1..=MAX_DELIVERY_ATTEMPTS {
        let mut got = claim_due(&c, "a", "web", now, 10).unwrap();
        assert_eq!(got.len(), 1, "attempt {attempt} should be claimable");
        let (ev, _) = got.remove(0);
        let status = release_failed(&c, ev.id, "send failed", now).unwrap();
        if attempt < MAX_DELIVERY_ATTEMPTS {
            assert_eq!(status, EVENT_PENDING);
            assert!(
                claim_due(&c, "a", "web", now, 10).unwrap().is_empty(),
                "backoff holds the retry"
            );
        } else {
            assert_eq!(status, EVENT_FAILED);
        }
        now += RETRY_CAP_SECS + 1;
    }
    assert!(claim_due(&c, "a", "web", now, 10).unwrap().is_empty());
}

#[test]
fn deferred_claim_does_not_spend_an_attempt() {
    let c = conn();
    insert_run(&c, &new_run("run-gggggg", "t1"), 100).unwrap();
    settle(
        &c,
        "run-gggggg",
        &settlement(EXEC_COMPLETED, ACCEPT_UNVERIFIED, "ok"),
        110,
        true,
    )
    .unwrap();
    let (ev, _) = claim_due(&c, "a", "web", 120, 10).unwrap().remove(0);
    defer(&c, ev.id, 120, 15).unwrap();
    assert_eq!(get_event(&c, ev.id).unwrap().attempts, 0);
    assert!(claim_due(&c, "a", "web", 125, 10).unwrap().is_empty());
    assert_eq!(claim_due(&c, "a", "web", 135, 10).unwrap().len(), 1);
}

#[test]
fn cancelled_run_is_not_settled_or_woken_afterwards() {
    let c = conn();
    insert_run(&c, &new_run("run-hhhhhh", "t1"), 100).unwrap();
    settle(
        &c,
        "run-hhhhhh",
        &settlement(EXEC_COMPLETED, ACCEPT_UNVERIFIED, "ok"),
        110,
        true,
    )
    .unwrap();
    // Cancel after the event exists: settled already, so cancel is a no-op on the run.
    assert!(!cancel(&c, "run-hhhhhh", 112).unwrap());

    insert_run(&c, &new_run("run-iiiiii", "t1"), 100).unwrap();
    assert!(cancel(&c, "run-iiiiii", 105).unwrap());
    let late = settlement(EXEC_COMPLETED, ACCEPT_UNVERIFIED, "finished anyway");
    assert!(!settle(&c, "run-iiiiii", &late, 120, true).unwrap());
    let row = get(&c, "run-iiiiii").unwrap().unwrap();
    assert_eq!(row.exec_status, EXEC_CANCELLED);
    assert!(events_for_run(&c, "run-iiiiii").unwrap().is_empty());
}

#[test]
fn reconcile_interrupts_runs_whose_process_is_gone_exactly_once() {
    let c = conn();
    insert_run(&c, &new_run("run-jjjjjj", "t1"), 100).unwrap();
    mark_running(&c, "run-jjjjjj", 100, 600).unwrap();
    insert_run(&c, &new_run("run-kkkkkk", "t1"), 100).unwrap();
    settle(
        &c,
        "run-kkkkkk",
        &settlement(EXEC_COMPLETED, ACCEPT_UNVERIFIED, "ok"),
        105,
        false,
    )
    .unwrap();

    let ids = reconcile_dead_owners(&c, 210, |_| false).unwrap();
    assert_eq!(ids, vec!["run-jjjjjj".to_string()]);
    let row = get(&c, "run-jjjjjj").unwrap().unwrap();
    assert_eq!(row.exec_status, EXEC_INTERRUPTED);
    assert!(row.settled_at.is_some());
    assert!(
        reconcile_dead_owners(&c, 310, |_| false)
            .unwrap()
            .is_empty()
    );
    assert_eq!(events_for_run(&c, "run-jjjjjj").unwrap().len(), 1);
}

#[test]
fn reconcile_leaves_runs_of_a_live_process_alone() {
    let c = conn();
    insert_run(&c, &new_run("run-xxxxxx", "t1"), 100).unwrap();
    assert!(
        reconcile_dead_owners(&c, 210, |pid| pid == 4242)
            .unwrap()
            .is_empty()
    );
    assert!(get(&c, "run-xxxxxx").unwrap().unwrap().settled_at.is_none());

    let mut ownerless = new_run("run-yyyyyy", "t1");
    ownerless.owner_pid = None;
    insert_run(&c, &ownerless, 100).unwrap();
    assert!(reconcile_dead_owners(&c, 210, |_| false).unwrap().len() == 1);
    assert!(get(&c, "run-yyyyyy").unwrap().unwrap().settled_at.is_none());
}

#[test]
fn claims_are_per_platform_and_skip_old_backlog() {
    let c = conn();
    insert_run(&c, &new_run("run-zzzzzz", "t1"), 100).unwrap();
    settle(
        &c,
        "run-zzzzzz",
        &settlement(EXEC_COMPLETED, ACCEPT_UNVERIFIED, "ok"),
        110,
        true,
    )
    .unwrap();
    assert!(claim_due(&c, "a", "telegram", 120, 10).unwrap().is_empty());
    assert!(
        claim_due(&c, "a", "web", 110 + MAX_EVENT_AGE_SECS + 1, 10)
            .unwrap()
            .is_empty()
    );
    assert_eq!(claim_due(&c, "a", "web", 120, 10).unwrap().len(), 1);
}

#[test]
fn restart_after_delivery_ack_does_not_redeliver() {
    let c = conn();
    insert_run(&c, &new_run("run-llllll", "t1"), 100).unwrap();
    settle(
        &c,
        "run-llllll",
        &settlement(EXEC_COMPLETED, ACCEPT_UNVERIFIED, "ok"),
        110,
        true,
    )
    .unwrap();
    let (ev, _) = claim_due(&c, "a", "web", 120, 10).unwrap().remove(0);
    mark_delivered(&c, ev.id, 121).unwrap();
    reconcile_dead_owners(&c, 510, |_| false).unwrap();
    assert!(claim_due(&c, "a", "web", 1000, 10).unwrap().is_empty());
    assert_eq!(events_for_run(&c, "run-llllll").unwrap().len(), 1);
}

#[test]
fn restart_before_delivery_ack_redelivers_after_ttl() {
    let c = conn();
    insert_run(&c, &new_run("run-mmmmmm", "t1"), 100).unwrap();
    settle(
        &c,
        "run-mmmmmm",
        &settlement(EXEC_COMPLETED, ACCEPT_UNVERIFIED, "ok"),
        110,
        true,
    )
    .unwrap();
    claim_due(&c, "crashed-worker", "web", 120, 10).unwrap();
    // Worker died with the claim held and never acknowledged.
    let again = claim_due(&c, "new-worker", "web", 120 + CLAIM_TTL_SECS, 10).unwrap();
    assert_eq!(again.len(), 1);
}

#[test]
fn liveness_separates_unknown_stale_and_hung() {
    let c = conn();
    insert_run(&c, &new_run("run-nnnnnn", "t1"), 1000).unwrap();
    mark_running(&c, "run-nnnnnn", 1000, 600).unwrap();
    let row = get(&c, "run-nnnnnn").unwrap().unwrap();
    assert_eq!(row.liveness(1100), Liveness::Working);
    assert_eq!(row.liveness(1000 + STALE_AFTER_SECS + 1), Liveness::Unknown);
    touch(&c, "run-nnnnnn", 1050).unwrap();
    let row = get(&c, "run-nnnnnn").unwrap().unwrap();
    assert_eq!(row.liveness(1050 + STALE_AFTER_SECS + 1), Liveness::Stale);
    assert_eq!(row.liveness(1600 + HUNG_GRACE_SECS + 1), Liveness::Hung);
}

#[test]
fn flag_stalled_reports_once_and_leaves_run_open() {
    let c = conn();
    insert_run(&c, &new_run("run-oooooo", "t1"), 1000).unwrap();
    mark_running(&c, "run-oooooo", 1000, 6000).unwrap();
    touch(&c, "run-oooooo", 1010).unwrap();
    let now = 1010 + STALE_AFTER_SECS + 5;
    assert_eq!(flag_stalled(&c, now).unwrap().len(), 1);
    assert!(flag_stalled(&c, now + 10).unwrap().is_empty());
    assert!(get(&c, "run-oooooo").unwrap().unwrap().settled_at.is_none());
}

#[test]
fn runs_are_invisible_across_chats() {
    let c = conn();
    insert_run(&c, &new_run("run-pppppp", "chat-a"), 100).unwrap();
    insert_run(&c, &new_run("run-qqqqqq", "chat-b"), 100).unwrap();
    assert!(
        find(&c, "run-pppppp", Some(&origin("chat-a")))
            .unwrap()
            .is_some()
    );
    assert!(
        find(&c, "run-pppppp", Some(&origin("chat-b")))
            .unwrap()
            .is_none()
    );
    assert!(
        find(&c, "run-ppp", Some(&origin("chat-b")))
            .unwrap()
            .is_none()
    );
    let mine = list(
        &c,
        &ListFilter {
            scope: Some(origin("chat-a")),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].run_id, "run-pppppp");
}

#[test]
fn task_link_filters_runs() {
    let c = conn();
    let mut a = new_run("run-rrrrrr", "t1");
    a.task_id = Some("task-1".into());
    insert_run(&c, &a, 100).unwrap();
    insert_run(&c, &new_run("run-ssssss", "t1"), 100).unwrap();
    let rows = list(
        &c,
        &ListFilter {
            task_id: Some("task-1".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(rows.len(), 1);
}

#[test]
fn review_needs_a_settled_run_and_honest_missing_list() {
    let c = conn();
    insert_run(&c, &new_run("run-tttttt", "t1"), 100).unwrap();
    assert!(review(&c, "run-tttttt", ACCEPT_ACCEPTED, &[], None, 110).is_err());
    settle(
        &c,
        "run-tttttt",
        &settlement(EXEC_COMPLETED, ACCEPT_UNVERIFIED, "ok"),
        105,
        false,
    )
    .unwrap();
    assert!(
        review(
            &c,
            "run-tttttt",
            ACCEPT_ACCEPTED,
            &["image".into()],
            None,
            110
        )
        .is_err()
    );
    review(
        &c,
        "run-tttttt",
        ACCEPT_PARTIAL,
        &["image".into()],
        Some("retry"),
        111,
    )
    .unwrap();
    let row = get(&c, "run-tttttt").unwrap().unwrap();
    assert_eq!(row.accept_status, ACCEPT_PARTIAL);
    assert_eq!(row.missing_deliverables, vec!["image".to_string()]);
}

#[test]
fn followup_depth_bounds_automatic_wakes() {
    let c = conn();
    insert_run(&c, &new_run("run-uuuuuu", "t1"), 100).unwrap();
    let turn = format!("{FOLLOWUP_TURN_PREFIX}run-uuuuuu");
    assert_eq!(followup_depth_for(&c, Some("turn-1")).unwrap(), 0);
    assert_eq!(followup_depth_for(&c, Some(&turn)).unwrap(), 1);

    let mut deep = new_run("run-vvvvvv", "t1");
    deep.followup_depth = MAX_FOLLOWUP_DEPTH + 1;
    insert_run(&c, &deep, 100).unwrap();
    settle(
        &c,
        "run-vvvvvv",
        &settlement(EXEC_COMPLETED, ACCEPT_UNVERIFIED, "ok"),
        110,
        true,
    )
    .unwrap();
    assert_eq!(
        events_for_run(&c, "run-vvvvvv").unwrap()[0].status,
        EVENT_SUPPRESSED
    );
    assert!(claim_due(&c, "a", "web", 200, 10).unwrap().is_empty());
}

#[test]
fn runs_without_a_chat_route_stay_queued_not_claimed() {
    let c = conn();
    let mut cli = new_run("run-wwwwww", "x");
    cli.origin = None;
    insert_run(&c, &cli, 100).unwrap();
    settle(
        &c,
        "run-wwwwww",
        &settlement(EXEC_COMPLETED, ACCEPT_UNVERIFIED, "ok"),
        110,
        true,
    )
    .unwrap();
    assert!(claim_due(&c, "a", "web", 200, 10).unwrap().is_empty());
    assert_eq!(
        events_for_run(&c, "run-wwwwww").unwrap()[0].status,
        EVENT_PENDING
    );
}

#[test]
fn preview_cuts_on_a_char_boundary() {
    assert_eq!(preview("héllo", 2), "h...");
    assert_eq!(preview("short", 50), "short");
}

#[test]
fn unresolved_lists_partial_and_blocked_but_not_accepted_or_cancelled() {
    let c = conn();
    for (id, accept) in [
        ("run-aa1111", ACCEPT_PARTIAL),
        ("run-bb2222", ACCEPT_BLOCKED),
        ("run-cc3333", ACCEPT_UNVERIFIED),
    ] {
        insert_run(&c, &new_run(id, "t1"), 100).unwrap();
        settle(&c, id, &settlement(EXEC_COMPLETED, accept, "x"), 110, false).unwrap();
    }
    insert_run(&c, &new_run("run-dd4444", "t1"), 100).unwrap();
    cancel(&c, "run-dd4444", 110).unwrap();
    let ids: Vec<String> = list_unresolved(&c, 0, 1000)
        .unwrap()
        .into_iter()
        .map(|r| r.run_id)
        .collect();
    assert_eq!(
        ids,
        vec!["run-aa1111".to_string(), "run-bb2222".to_string()]
    );
    assert!(list_unresolved(&c, 0, 100).unwrap().is_empty());
}

#[test]
fn overdue_runs_are_closed_even_when_their_pid_is_alive() {
    let c = conn();
    insert_run(&c, &new_run("run-ov1111", "t1"), 1000).unwrap();
    mark_running(&c, "run-ov1111", 1000, 100).unwrap();
    assert!(reconcile_overdue(&c, 1100 + OVERDUE_GRACE_SECS).unwrap().is_empty());
    let closed = reconcile_overdue(&c, 1100 + OVERDUE_GRACE_SECS + 1).unwrap();
    assert_eq!(closed, vec!["run-ov1111".to_string()]);
    assert_eq!(events_for_run(&c, "run-ov1111").unwrap().len(), 1);
    assert!(reconcile_overdue(&c, 99_999).unwrap().is_empty());
}

#[test]
fn queued_run_gets_a_deadline_and_is_not_flagged_stale() {
    let c = conn();
    insert_run(&c, &new_run("run-qq2222", "t1"), 1000).unwrap();
    let row = get(&c, "run-qq2222").unwrap().unwrap();
    assert_eq!(row.deadline_at, Some(1000 + QUEUE_DEADLINE_SECS));
    assert!(flag_stalled(&c, 1000 + STALE_AFTER_SECS * 4).unwrap().is_empty());
}

#[test]
fn no_telemetry_is_reported_as_unknown_but_never_raises_a_stale_event() {
    let c = conn();
    insert_run(&c, &new_run("run-nt3333", "t1"), 1000).unwrap();
    mark_running(&c, "run-nt3333", 1000, 6000).unwrap();
    let at = 1000 + STALE_AFTER_SECS + 5;
    assert_eq!(get(&c, "run-nt3333").unwrap().unwrap().liveness(at), Liveness::Unknown);
    assert!(flag_stalled(&c, at).unwrap().is_empty());
}

#[test]
fn only_a_completed_run_can_be_accepted_and_a_cancelled_one_cannot_be_reviewed() {
    let c = conn();
    insert_run(&c, &new_run("run-rv4444", "t1"), 100).unwrap();
    settle(&c, "run-rv4444", &settlement(EXEC_FAILED, ACCEPT_BLOCKED, "x"), 110, false).unwrap();
    assert!(review(&c, "run-rv4444", ACCEPT_ACCEPTED, &[], None, 120).is_err());
    review(&c, "run-rv4444", ACCEPT_PARTIAL, &["x".into()], None, 121).unwrap();

    insert_run(&c, &new_run("run-rv5555", "t1"), 100).unwrap();
    cancel(&c, "run-rv5555", 110).unwrap();
    assert!(review(&c, "run-rv5555", ACCEPT_PARTIAL, &[], None, 120).is_err());
    assert_eq!(get(&c, "run-rv5555").unwrap().unwrap().accept_status, ACCEPT_BLOCKED);
}

#[test]
fn unscoped_callers_see_only_unrouted_runs() {
    let c = conn();
    insert_run(&c, &new_run("run-sc6666", "chat-a"), 100).unwrap();
    let mut cli = new_run("run-sc7777", "x");
    cli.origin = None;
    insert_run(&c, &cli, 100).unwrap();
    let scope = Some(Origin::unrouted());
    assert!(find(&c, "run-sc6666", scope.as_ref()).unwrap().is_none());
    assert!(find(&c, "run-sc7777", scope.as_ref()).unwrap().is_some());
    let rows = list(&c, &ListFilter { scope, ..Default::default() }).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].run_id, "run-sc7777");
}

#[test]
fn escalation_is_raised_once_and_interrupted_runs_count() {
    let c = conn();
    insert_run(&c, &new_run("run-es8888", "t1"), 100).unwrap();
    mark_running(&c, "run-es8888", 100, 600).unwrap();
    reconcile_dead_owners(&c, 200, |_| false).unwrap();
    let rows = list_unresolved(&c, 0, 1000).unwrap();
    assert_eq!(rows.len(), 1);
    mark_escalated(&c, "run-es8888", 300).unwrap();
    assert!(list_unresolved(&c, 0, 1000).unwrap().is_empty());
}
