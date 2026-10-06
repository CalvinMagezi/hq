use super::*;

#[test]
fn saved_state_still_carries_harness_for_rollback() {
    let json = serde_json::to_value(ChannelState::new_default()).unwrap();
    assert_eq!(json["harness"], "hq");
    let back: ChannelState = serde_json::from_value(json).unwrap();
    assert!(back.messages.is_empty());
}

fn msg(role: hq_core::types::MessageRole, content: &str) -> hq_core::types::ChatMessage {
    hq_core::types::ChatMessage {
        role,
        content: content.to_string(),
        tool_calls: vec![],
        tool_call_id: None,
        reasoning_content: None,
        image_parts: vec![],
    }
}

#[test]
fn split_current_turn_images_pops_the_trailing_user_message_and_returns_its_images() {
    let img = hq_core::types::ImageAttachment {
        path: std::path::PathBuf::from("/tmp/x.jpg"),
        mime_type: "image/jpeg".to_string(),
    };
    let mut current = msg(hq_core::types::MessageRole::User, "describe this");
    current.image_parts = vec![img.clone()];
    let history = vec![
        msg(hq_core::types::MessageRole::User, "earlier turn"),
        msg(hq_core::types::MessageRole::Assistant, "earlier reply"),
        current,
    ];

    let (remaining, images) = split_current_turn_images(history);

    assert_eq!(remaining.len(), 2, "current turn must be popped off");
    assert_eq!(remaining[0].content, "earlier turn");
    assert_eq!(remaining[1].content, "earlier reply");
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].path, img.path);
}

#[test]
fn split_current_turn_images_on_empty_history_returns_nothing() {
    let (remaining, images) = split_current_turn_images(vec![]);
    assert!(remaining.is_empty());
    assert!(images.is_empty());
}

#[test]
fn split_current_turn_images_leaves_a_trailing_assistant_message_untouched() {
    // Defensive: only pop when the trailing entry is actually a User
    // turn, so an unexpected history shape doesn't silently drop the
    // last assistant reply.
    let history = vec![
        msg(hq_core::types::MessageRole::User, "turn"),
        msg(hq_core::types::MessageRole::Assistant, "reply"),
    ];
    let (remaining, images) = split_current_turn_images(history);
    assert_eq!(remaining.len(), 2);
    assert!(images.is_empty());
}

/// Workstream G (event-sourced history audit) regression test: a failed
/// turn must not leave `messages` with a dangling unanswered user turn,
/// or the next successful turn would resubmit two consecutive user
/// messages to the LLM with no record a failure happened in between.
#[test]
fn replace_parked_reply_swaps_the_ack_in_place() {
    let mut state = ChannelState::new_default();
    state.record_turn_outcome(&Ok(hq_agent::native_hq::parked_ack(Some(
        "t9-0000-long-uuid",
    ))));
    let before = state.messages.len();
    state.replace_parked_reply("t9-0000-long-uuid", "Done: 3 files migrated.");
    assert_eq!(state.messages.len(), before);
    assert_eq!(
        state.messages.last().unwrap().content,
        "Done: 3 files migrated."
    );
    state.record_turn_outcome(&Ok(hq_agent::native_hq::parked_ack(None)));
    state.replace_parked_reply("", "Untracked result.");
    assert_eq!(state.messages.last().unwrap().content, "Untracked result.");
}

#[test]
fn ack_quotes_an_id_only_when_the_registry_row_exists() {
    let db = hq_db::Database::open_memory().unwrap();
    let tracked = register_turn(Some(&db), "abcdef123456", "telegram", "c1", None, "p");
    assert_eq!(tracked.as_deref(), Some("abcdef123456"));
    assert!(lookup_turn(&db, "`abcdef12`").is_ok());
    assert!(hq_agent::native_hq::parked_ack(tracked.as_deref()).contains("`abcdef12`"));

    let untracked = register_turn(None, "fedcba654321", "telegram", "c1", None, "p");
    assert!(untracked.is_none());
    assert!(!hq_agent::native_hq::parked_ack(untracked.as_deref()).contains('`'));
}

#[test]
fn watch_done_marker_is_stripped_and_closes_only_the_watch_row() {
    assert_eq!(
        take_watch_done("still pending"),
        ("still pending".to_string(), false)
    );
    assert_eq!(
        take_watch_done("PR merged. `WATCH_DONE`"),
        ("PR merged.".to_string(), true)
    );

    let db = hq_db::Database::open_memory().unwrap();
    db.with_conn(|c| {
        hq_db::background_turns::insert_watch(
            c, "w-1", "telegram", "1", None, None, "p", 0, 300, None,
        )
    })
    .unwrap();
    let (text, done) = settle_watch_firing(Some(&db), "w-1", "Deploy is green.\nWATCH_DONE");
    assert!(done);
    assert_eq!(text, "Deploy is green.");
    let row = db
        .with_conn(|c| hq_db::background_turns::get(c, "w-1"))
        .unwrap()
        .unwrap();
    assert_eq!(row.status, hq_db::background_turns::STATUS_COMPLETED);
    assert_eq!(row.result_text.as_deref(), Some("Deploy is green."));

    let (_, done) = settle_watch_firing(Some(&db), "w-1", "no marker");
    assert!(!done);
}

#[test]
fn truncate_chars_cuts_on_characters_not_bytes() {
    assert_eq!(truncate_chars("héllo".to_string(), 10), "héllo");
    assert_eq!(truncate_chars("héllo wörld".to_string(), 5), "héll…");
}

#[test]
fn progress_sink_sends_notes_and_heartbeats() {
    let vault = tempfile::TempDir::new().unwrap();
    let sent = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let send: ChatSender = {
        let sent = sent.clone();
        Arc::new(move |m| sent.lock().unwrap().push(m))
    };
    let sink = progress_sink(
        None,
        vault.path().to_path_buf(),
        hq_core::identity::RequestIdentity::from_discord(7),
        1900,
        send,
    );
    let event = |note: Option<&str>| hq_agent::native_hq::ProgressEvent {
        turn_id: "t1".to_string(),
        elapsed_secs: 180,
        note: note.map(str::to_string),
        blocked_on: None,
        resumed_with_assumption: None,
    };
    sink(event(Some("halfway")));
    sink(event(None));
    let sent = sent.lock().unwrap();
    assert_eq!(sent[0], "Turn `t1`: halfway");
    assert_eq!(sent[1], "Turn `t1` still running (elapsed 3m).");
}

#[test]
fn record_turn_outcome_appends_assistant_reply_on_success() {
    let mut state = ChannelState::new_default();
    state.messages.push(hq_core::types::ChatMessage {
        image_parts: Vec::new(),
        role: hq_core::types::MessageRole::User,
        content: "hello".to_string(),
        tool_calls: vec![],
        tool_call_id: None,
        reasoning_content: None,
    });

    state.record_turn_outcome(&Ok("hi there".to_string()));

    assert_eq!(state.messages.len(), 2);
    assert_eq!(
        state.messages[1].role,
        hq_core::types::MessageRole::Assistant
    );
    assert_eq!(state.messages[1].content, "hi there");
}

#[test]
fn record_turn_outcome_appends_failure_marker_instead_of_leaving_a_dangling_user_turn() {
    let mut state = ChannelState::new_default();
    state.messages.push(hq_core::types::ChatMessage {
        image_parts: Vec::new(),
        role: hq_core::types::MessageRole::User,
        content: "run the deploy".to_string(),
        tool_calls: vec![],
        tool_call_id: None,
        reasoning_content: None,
    });

    state.record_turn_outcome(&Err(anyhow::anyhow!("harness timed out")));

    assert_eq!(state.messages.len(), 2);
    assert_eq!(
        state.messages[1].role,
        hq_core::types::MessageRole::Assistant
    );
    assert!(state.messages[1].content.contains("turn failed"));
    assert!(state.messages[1].content.contains("harness timed out"));

    // A subsequent successful turn's user message no longer lands right
    // after an unanswered one — the log stays a coherent alternation.
    state.messages.push(hq_core::types::ChatMessage {
        image_parts: Vec::new(),
        role: hq_core::types::MessageRole::User,
        content: "try again".to_string(),
        tool_calls: vec![],
        tool_call_id: None,
        reasoning_content: None,
    });
    state.record_turn_outcome(&Ok("done".to_string()));
    assert_eq!(state.messages.len(), 4);
    let roles: Vec<_> = state.messages.iter().map(|m| m.role.clone()).collect();
    assert_eq!(
        roles,
        vec![
            hq_core::types::MessageRole::User,
            hq_core::types::MessageRole::Assistant,
            hq_core::types::MessageRole::User,
            hq_core::types::MessageRole::Assistant,
        ]
    );
}

#[test]
fn watch_parse_plain_form() {
    let text = "/watch 15 check whether the agy review is done";
    let lower = text.to_lowercase();
    let args = parse_watch_command(&lower, text).unwrap().unwrap();
    assert_eq!(args.interval_mins, 15);
    // Omitting `for <N>h` no longer means "forever" — it means
    // DEFAULT_WATCH_EXPIRY_HOURS (see that const's doc comment for why).
    assert_eq!(args.expiry_hours, Some(DEFAULT_WATCH_EXPIRY_HOURS));
    assert_eq!(args.prompt, "check whether the agy review is done");
}

#[test]
fn watch_parse_for_hours_form() {
    let text = "/watch 30 for 4h keep an eye on the build";
    let lower = text.to_lowercase();
    let args = parse_watch_command(&lower, text).unwrap().unwrap();
    assert_eq!(args.interval_mins, 30);
    assert_eq!(args.expiry_hours, Some(4));
    assert_eq!(args.prompt, "keep an eye on the build");
}

#[test]
fn watch_parse_interval_is_clamped() {
    let text = "/watch 0 ping me";
    let lower = text.to_lowercase();
    let args = parse_watch_command(&lower, text).unwrap().unwrap();
    assert_eq!(args.interval_mins, MIN_WATCH_INTERVAL_MINS);

    let text = "/watch 99999 ping me";
    let lower = text.to_lowercase();
    let args = parse_watch_command(&lower, text).unwrap().unwrap();
    assert_eq!(args.interval_mins, 1440);
}

#[test]
fn watch_parse_prompt_keeps_original_case() {
    let text = "/Watch 5 Check The PR Status";
    let lower = text.to_lowercase();
    let args = parse_watch_command(&lower, text).unwrap().unwrap();
    assert_eq!(args.prompt, "Check The PR Status");
}

#[test]
fn watch_parse_missing_prompt_is_usage_error() {
    let lower = "/watch 15";
    assert_eq!(parse_watch_command(lower, lower), Some(None));
    let lower = "/watch";
    assert_eq!(parse_watch_command(lower, lower), Some(None));
    let lower = "/watch 15 for 2h";
    assert_eq!(parse_watch_command(lower, lower), Some(None));
}

#[test]
fn watch_parse_bad_interval_is_usage_error() {
    let lower = "/watch soon check the thing";
    assert_eq!(parse_watch_command(lower, lower), Some(None));
    let lower = "/watch 15 for xh check the thing";
    assert_eq!(parse_watch_command(lower, lower), Some(None));
}

#[test]
fn watch_parse_ignores_non_watch_messages() {
    assert_eq!(
        parse_watch_command("watchdog status", "watchdog status"),
        None
    );
    assert_eq!(parse_watch_command("hello there", "hello there"), None);
}

#[test]
fn unwatch_parse_forms() {
    assert_eq!(
        parse_unwatch_command("/unwatch abc-123"),
        Some(Some("abc-123".to_string()))
    );
    assert_eq!(
        parse_unwatch_command("unwatch abc-123"),
        Some(Some("abc-123".to_string()))
    );
    assert_eq!(parse_unwatch_command("/unwatch"), Some(None));
    assert_eq!(parse_unwatch_command("/unwatchx foo"), None);
    assert_eq!(parse_unwatch_command("hello"), None);
}

#[test]
fn a_busy_chat_steers_when_it_can_and_says_so_when_it_cannot() {
    let mut state = ChannelState::new_default();
    assert_eq!(state.busy_reply("x"), None);
    assert_eq!(state.claim_turn("first"), Ok(()));
    assert_eq!(state.claim_turn("second"), Err(NO_STEER_INBOX_REPLY));
    let inbox = Arc::new(std::sync::Mutex::new(None));
    state.pending_steer = Some(inbox.clone());
    assert_eq!(state.claim_turn("go left"), Err(STEERING_REPLY));
    assert_eq!(inbox.lock().unwrap().as_deref(), Some("go left"));
}

#[test]
fn state_files_written_before_the_harness_field_went_still_load() {
    let old = r#"{"messages":[],"harness":"cursor","model_override":"x","pinned_permission_preset":null}"#;
    assert!(serde_json::from_str::<ChannelState>(old).is_ok());
}

#[test]
fn staging_refreshes_the_system_prompt_and_trims_to_the_recent_tail() {
    let mut state = ChannelState::new_default();
    for i in 0..MAX_HISTORY {
        state.stage_turn(&format!("sys {i}"), &format!("msg {i}"), Vec::new());
    }
    assert_eq!(state.messages.len(), 1 + KEPT_HISTORY);
    assert_eq!(state.messages[0].content, "sys 29");
    assert_eq!(state.messages.last().unwrap().content, "msg 29");
}

#[test]
fn staging_never_overwrites_a_leading_user_message() {
    let mut state = ChannelState::new_default();
    state.messages.push(chat_message(
        hq_core::types::MessageRole::User,
        "[focus] switching topic to: tax",
        Vec::new(),
    ));
    state.stage_turn("prompt", "next", Vec::new());
    let contents: Vec<&str> = state.messages.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(contents, ["prompt", "[focus] switching topic to: tax", "next"]);
    assert_eq!(state.messages[0].role, hq_core::types::MessageRole::System);
}

#[test]
fn steers_queue_instead_of_overwriting() {
    let inbox = std::sync::Mutex::new(None);
    queue_steer(&inbox, "first".into());
    queue_steer(&inbox, "second".into());
    assert_eq!(inbox.lock().unwrap().as_deref(), Some("first\n\nsecond"));
}
