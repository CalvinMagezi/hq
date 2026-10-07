use super::*;

/// A web chat is not framed as part of the Telegram conversation.
#[tokio::test]
async fn isolated_session_carries_no_other_interface_turns() {
    let vault = tempfile::tempdir().unwrap();
    let threads = vault.path().join("_threads");
    std::fs::create_dir_all(&threads).unwrap();
    let entry = serde_json::json!({
        "ts": chrono::Utc::now(), "source": "telegram", "role": "user",
        "content": "TELEGRAM-ONLY-MARKER", "session_key": "tg"
    });
    std::fs::write(threads.join("telegram.jsonl"), format!("{entry}\n")).unwrap();

    let config = HqConfig {
        vault_path: vault.path().to_path_buf(),
        openrouter_api_key: Some("test-key-no-network".to_string()),
        ..Default::default()
    };
    let web = hq_core::identity::RequestIdentity::from_proxy_user("web", vec!["*".into()], None);

    let prompt_for = |isolated: bool| {
        let mut hooks = NativeHqHooks {
            identity: Some(web.clone()),
            isolated,
            ..Default::default()
        };
        native_builder(
            &config,
            "hi",
            String::new(),
            vault.path().to_path_buf(),
            SessionConfig {
                context_window: 200_000,
                ..SessionConfig::default()
            },
            &mut hooks,
        )
    };
    let shared = prompt_for(false).build().await.unwrap();
    let isolated = prompt_for(true).build().await.unwrap();

    // Merged thread lines never reach the system prompt (only System
    // blocks do); what leaked was the cross-interface framing note.
    assert!(shared.system_prompt().unwrap().contains("[via <interface>"));
    let isolated = isolated.system_prompt().unwrap();
    assert!(!isolated.contains("[via <interface>"));
    assert!(!isolated.contains("TELEGRAM-ONLY-MARKER"));
}

/// Telegram and web never saw `report_progress` notes because this wiring
/// was missing: the tool answered "not available" instead.
#[tokio::test]
async fn report_progress_reaches_the_surface_sink() {
    let vault = tempfile::TempDir::new().unwrap();
    let config = HqConfig {
        openrouter_api_key: Some("test-key-no-network".to_string()),
        vault_path: vault.path().to_path_buf(),
        ..Default::default()
    };

    let seen = Arc::new(std::sync::Mutex::new(Vec::<ProgressEvent>::new()));
    let sink: ProgressSink = {
        let seen = seen.clone();
        Arc::new(move |event| seen.lock().unwrap().push(event))
    };
    let mut hooks = NativeHqHooks {
        turn_id: Some("t1".to_string()),
        on_progress: Some(sink),
        ..Default::default()
    };
    let session = native_builder(
        &config,
        "migrate the calendar store",
        String::new(),
        vault.path().to_path_buf(),
        SessionConfig::default(),
        &mut hooks,
    )
    .build()
    .await
    .expect("session builds without network");

    let result = session
        .call_tool_for_test("report_progress", serde_json::json!({"message": "halfway"}))
        .await
        .unwrap();
    assert!(!result.content[0].text.contains("not available"));
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].turn_id, "t1");
    assert_eq!(seen[0].note.as_deref(), Some("halfway"));
}

fn event(note: Option<&str>) -> ProgressEvent {
    ProgressEvent {
        turn_id: "t1".to_string(),
        elapsed_secs: 90,
        note: note.map(str::to_string),
        blocked_on: None,
        resumed_with_assumption: None,
    }
}

#[test]
fn soft_limit_replies_carry_an_incomplete_marker() {
    use hq_core::types::SessionResult;
    let broke = result_text(&SessionResult::BudgetExhausted(String::new()));
    assert!(broke.starts_with("_[incomplete:"), "{broke}");
    assert_eq!(result_text(&SessionResult::Complete("done".into())), "done");
}

#[test]
fn parked_ack_quotes_a_short_ref_only_for_a_tracked_turn() {
    let tracked = parked_ack(Some("0123456789abcdef"));
    assert!(tracked.contains("Parked as turn `01234567`"), "{tracked}");
    assert!(!tracked.contains("89abcdef"), "{tracked}");
    let untracked = parked_ack(None);
    assert!(!untracked.contains('`'), "{untracked}");
    assert!(untracked.contains(&parked_marker("")), "{untracked}");
}

#[test]
fn note_without_a_turn_id_is_labelled_progress() {
    let msg = render_progress_note("", &event(Some("halfway")));
    assert_eq!(msg, "Progress: halfway");
}

#[test]
fn plain_note_renders_unadorned() {
    let msg = render_progress_note("t1", &event(Some("halfway through the migration")));
    assert_eq!(msg, "Turn `t1`: halfway through the migration");
}

/// The distinction this field exists to make: a peer chat surface should
/// be able to tell "still working" apart from "stopped to wait on you",
/// not just read the same free-text note either way.
#[test]
fn blocked_on_is_framed_distinctly_from_plain_progress() {
    let mut ev = event(Some("which OpenRouter key to use"));
    ev.blocked_on = Some("Alex".to_string());
    let msg = render_progress_note("t1", &ev);
    assert!(msg.contains("blocked, waiting on Alex"));
    assert!(msg.contains("which OpenRouter key to use"));
}

#[test]
fn blocked_on_without_a_note_still_says_something_useful() {
    let mut ev = event(None);
    ev.blocked_on = Some("Alex".to_string());
    let msg = render_progress_note("t1", &ev);
    assert!(msg.contains("blocked, waiting on Alex"));
    assert!(msg.contains("no detail given"));
}

/// The escape hatch this field exists for: an agent that gave up waiting
/// and proceeded on a guess must read as "acted on an assumption", not as
/// ordinary progress, so the operator knows to go check it.
#[test]
fn resumed_with_assumption_takes_priority_over_blocked_on() {
    let mut ev = event(Some("continuing the deploy"));
    ev.blocked_on = Some("Alex".to_string());
    ev.resumed_with_assumption = Some("using the staging key".to_string());
    let msg = render_progress_note("t1", &ev);
    assert!(msg.contains("proceeding on assumption (using the staging key)"));
    assert!(!msg.contains("blocked, waiting on"));
}

/// The originally-reported gap: a `TaskType::Vault` prompt ("note") describing
/// real code work ("bug") must be recognized as carrying a code signal so it
/// keeps bash/edit/read/write instead of being downgraded to `SessionProfile::Weak`.
#[test]
fn fault_vocabulary_bug_is_a_code_signal() {
    assert!(prompt_has_code_signal(
        "Fix the login bug and add a note about what caused it"
    ));
}

/// The explicitly-flagged false-positive risk of the rejected "add fix/edit as
/// signals" approach: a genuinely vault-only prompt that happens to contain the
/// generic verb "fix" must NOT trip a code signal. Only bug/error/crash/exception
/// are added here precisely because they don't appear in prompts like this one.
#[test]
fn generic_verb_fix_does_not_false_positive() {
    assert!(!prompt_has_code_signal(
        "remember to fix the note about mom's birthday"
    ));
}

/// Word-boundary correctness: "debugging" contains "bug" as a substring but is
/// not the standalone word "bug". Treating it as a signal would misfire on
/// ordinary language (e.g. "debugging my sleep schedule" is not a code-fault
/// report), so whole-word matching must not match it. This is judged an
/// acceptable residual risk in the other direction (a real "I'm debugging the
/// parser, note it" prompt won't trip this particular signal) because the other
/// 9 signals plus "error"/"crash"/"exception" still catch the overwhelming
/// majority of real code-fault prompts, and widening to a substring match would
/// reopen the false-positive risk this whole change is designed to avoid.
#[test]
fn debugging_substring_does_not_match_bug_signal() {
    assert!(!prompt_has_code_signal(
        "I've been debugging my sleep schedule, note it in the vault"
    ));
}

/// A second genuine code-fault example, using "error" instead of "bug", to prove
/// the new signal set isn't a one-word special case.
#[test]
fn fault_vocabulary_error_is_a_code_signal() {
    assert!(prompt_has_code_signal(
        "there's an error in the parser, remember to check it tomorrow"
    ));
}

/// "crash" and "exception" round out the 4 new fault-vocabulary signals; both
/// must independently trip the guard.
#[test]
fn fault_vocabulary_crash_and_exception_are_code_signals() {
    assert!(prompt_has_code_signal(
        "note this down: the app has a crash on startup"
    ));
    assert!(prompt_has_code_signal(
        "note to self: exception thrown when parsing empty input"
    ));
}

/// Regression guard on the pre-existing 9 signals: none of the original
/// extension/fence/keyword checks should be affected by adding the new
/// word-set check alongside them.
#[test]
fn pre_existing_signals_still_match() {
    assert!(prompt_has_code_signal("edit fibonacci.rs"));
    assert!(prompt_has_code_signal("```rust\nfn main() {}\n```"));
    assert!(prompt_has_code_signal("add a def helper() in utils.py"));
}

/// Baseline: a prompt with none of the 9 original signals and none of the 4
/// new fault words should still return false.
#[test]
fn no_signal_returns_false() {
    assert!(!prompt_has_code_signal(
        "remember to buy milk and call mom this weekend"
    ));
}

/// Detached-supervisor heartbeats: with a 1s tick interval and a fake
/// prompt future slow enough to outlive several ticks, at least one
/// `note: None` heartbeat event must fire before the future completes.
/// Time is paused so the test is deterministic and instant.
#[tokio::test(start_paused = true)]
async fn detached_supervisor_emits_heartbeat_before_completion() {
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink: ProgressSink = Arc::new({
        let events = events.clone();
        move |ev| events.lock().unwrap().push(ev)
    });
    let fut = Box::pin(async {
        tokio::time::sleep(Duration::from_secs(5)).await;
        "done"
    });
    let out = supervise_detached(fut, Some(sink), Some(1), "t-1".to_string(), Instant::now()).await;
    assert_eq!(out, "done");
    let evs = events.lock().unwrap();
    assert!(
        !evs.is_empty(),
        "expected at least one heartbeat before completion"
    );
    assert!(
        evs.iter().all(|e| e.note.is_none()),
        "supervisor ticks are heartbeats (note: None), got: {evs:?}"
    );
    assert!(evs.iter().all(|e| e.turn_id == "t-1"));
}

/// Guard on the opt-in nature of heartbeats: without an interval (or a
/// sink) the supervisor is a plain await and no events fire.
#[tokio::test(start_paused = true)]
async fn detached_supervisor_without_interval_never_ticks() {
    let events = Arc::new(std::sync::Mutex::new(Vec::<ProgressEvent>::new()));
    let sink: ProgressSink = Arc::new({
        let events = events.clone();
        move |ev| events.lock().unwrap().push(ev)
    });
    let fut = Box::pin(async {
        tokio::time::sleep(Duration::from_secs(5)).await;
        "done"
    });
    let out = supervise_detached(fut, Some(sink), None, "t-2".to_string(), Instant::now()).await;
    assert_eq!(out, "done");
    assert!(events.lock().unwrap().is_empty());
}

/// What an `hq_ask` in read-only mode relies on: the preset set through the hooks denies
/// every tool that changes something (including the coder delegation tool; read-only specialists are
/// allowed because their children inherit the mode, see the AgentService test) while the lookups a question needs still run.
#[tokio::test]
async fn a_read_only_preset_denies_mutating_tools_and_keeps_the_reads() {
    use hq_core::types::PermissionPreset;

    let vault = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
    std::fs::write(vault.path().join("Notebooks/a.md"), "# A\nhello").unwrap();
    let config = HqConfig {
        openrouter_api_key: Some("test-key-no-network".to_string()),
        vault_path: vault.path().to_path_buf(),
        ..HqConfig::default()
    };
    let web = hq_core::identity::RequestIdentity::from_proxy_user("web", vec!["*".into()], None);

    let build = |preset: Option<PermissionPreset>| {
        let mut hooks = NativeHqHooks {
            identity: Some(web.clone()),
            isolated: true,
            permission_preset: preset,
            ..Default::default()
        };
        native_builder(
            &config,
            "migrate the calendar store",
            String::new(),
            vault.path().to_path_buf(),
            // Live, as a web turn is, so the denial below is the preset's and not the liveness gate's.
            SessionConfig {
                is_live_user_turn: true,
                ..SessionConfig::default()
            },
            &mut hooks,
        )
        .build()
    };
    let read_only = build(Some(PermissionPreset::ReadOnly))
        .await
        .expect("session builds");
    let mutating = [
        (
            "vault_write_note",
            serde_json::json!({"path": "Notebooks/x.md", "title": "x", "content": "x"}),
        ),
        ("bash", serde_json::json!({"command": "touch pwned"})),
        (
            "write_file",
            serde_json::json!({"path": "pwned.txt", "content": "x"}),
        ),
        ("task_create", serde_json::json!({"title": "x"})),
        (
            "harness_session_spawn",
            serde_json::json!({"harness": "claude-code", "cwd": "/tmp/x"}),
        ),
        (
            "call_code_reasoner",
            serde_json::json!({"task": "write a file"}),
        ),
    ];
    for (tool, args) in mutating {
        let denied = read_only
            .call_tool_for_test(tool, args)
            .await
            .unwrap()
            .content[0]
            .text
            .clone();
        assert!(
            denied.contains("requires write access"),
            "{tool} was not denied: {denied}"
        );
    }
    assert!(!vault.path().join("Notebooks/x.md").exists());
    assert!(!vault.path().join("pwned").exists() && !vault.path().join("pwned.txt").exists());

    for (tool, args) in [
        ("vault_list", serde_json::json!({"directory": "Notebooks"})),
        ("vault_read", serde_json::json!({"path": "Notebooks/a.md"})),
        ("vault_search", serde_json::json!({"query": "hello"})),
    ] {
        let out = read_only
            .call_tool_for_test(tool, args)
            .await
            .unwrap()
            .content[0]
            .text
            .clone();
        assert!(
            !out.contains("requires write access"),
            "{tool} must stay available: {out}"
        );
    }

    let open = build(None).await.expect("session builds");
    let out = open
        .call_tool_for_test(
            "vault_write_note",
            serde_json::json!({"path": "Notebooks/x.md", "title": "x", "content": "x"}),
        )
        .await
        .unwrap()
        .content[0]
        .text
        .clone();
    assert!(
        !out.contains("requires write access"),
        "without the preset the write is allowed: {out}"
    );
}

/// The handoff key was kept off these tools on purpose; a read-only ask on that key must not get them back.
#[tokio::test]
async fn a_handoff_ask_turn_has_none_of_the_tools_the_handoff_key_is_kept_off() {
    use hq_core::types::PermissionPreset;

    let vault = tempfile::TempDir::new().unwrap();
    let config = HqConfig {
        openrouter_api_key: Some("test-key-no-network".to_string()),
        vault_path: vault.path().to_path_buf(),
        ..HqConfig::default()
    };
    let names = |deny: Vec<String>| {
        let builder = crate::builder::SessionBuilder::from_config(&config)
            .working_dir(vault.path().to_path_buf())
            .session_config(SessionConfig {
                is_live_user_turn: true,
                ..SessionConfig::default()
            })
            .permission_preset(PermissionPreset::ReadOnly)
            .deny_tool_prefixes(deny);
        async move { builder.build().await.unwrap().tool_names().await }
    };

    let handoff = names(
        hq_tools::ask::HANDOFF_ASK_DENIED_PREFIXES
            .iter()
            .map(|p| p.to_string())
            .collect(),
    )
    .await;
    for denied in [
        "host_read",
        "host_agents",
        "harness_session_logs",
        "harness_session_list",
        "harness_session_status",
        "subagent_run_result",
        "read_file",
        "grep",
        "find_files",
        "list_dir",
        "git_status",
        "git_diff",
        "call_web_researcher",
        "system_info",
    ] {
        assert!(
            !handoff.iter().any(|n| n == denied),
            "{denied} must be gone: {handoff:?}"
        );
    }
    for kept in ["vault_search", "vault_read", "task_list"] {
        assert!(
            handoff.iter().any(|n| n == kept),
            "{kept} must stay: {handoff:?}"
        );
    }
    let full = names(Vec::new()).await;
    for present in ["read_file", "grep", "harness_session_list", "host_agents"] {
        assert!(
            full.iter().any(|n| n == present),
            "an unrestricted turn keeps {present}: {full:?}"
        );
    }
}
