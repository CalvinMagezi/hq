use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone)]
enum Plan {
    /// Settle the ask as answered after a short delay, like a quick reply.
    AnswerSoon(&'static str),
    /// Leave the turn running until the test settles it.
    Hold,
    Refuse(&'static str),
}

/// Ask id, thread id, mode, scope and caller of each start the runner saw.
type Started = (String, String, AskMode, AskScope, String);

struct FakeRunner {
    db: Arc<Database>,
    plan: Plan,
    running: AtomicBool,
    starts: Mutex<Vec<Started>>,
}

impl FakeRunner {
    fn new(db: &Arc<Database>, plan: Plan) -> Arc<Self> {
        Arc::new(Self {
            db: db.clone(),
            plan,
            running: AtomicBool::new(false),
            starts: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl AskRunner for FakeRunner {
    async fn start(&self, req: &AskStart<'_>) -> Result<String> {
        if let Plan::Refuse(why) = &self.plan {
            bail!("{why}");
        }
        let meta = json!({"source": {"kind": "mcp", "caller": req.caller}});
        self.db.with_conn(|c| {
            hq_db::chat::add_message_with_meta(c, req.thread_id, "user", req.question, Some(&meta))
                .map(|_| ())
        })?;
        self.starts.lock().unwrap().push((
            req.ask_id.into(),
            req.thread_id.into(),
            req.mode,
            req.scope,
            req.caller.into(),
        ));
        self.running.store(true, Ordering::SeqCst);
        if let Plan::AnswerSoon(text) = &self.plan {
            let (db, ask_id, text) = (self.db.clone(), req.ask_id.to_string(), text.to_string());
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(60)).await;
                let _ = db.with_conn(|c| {
                    asks::settle(c, &ask_id, STATUS_ANSWERED, Some((&text, "m1")), None)
                });
            });
        }
        Ok(format!("turn-{}", req.ask_id))
    }

    async fn turn_active(&self, _thread_id: &str) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

struct Rig {
    db: Arc<Database>,
    runner: Arc<FakeRunner>,
    ask: Box<dyn HqTool>,
    result: Box<dyn HqTool>,
}

fn rig(plan: Plan) -> Rig {
    let db = Arc::new(Database::open_memory().unwrap());
    let runner = FakeRunner::new(&db, plan);
    let cell: RunnerCell = Arc::new(OnceLock::new());
    let as_dyn: Arc<dyn AskRunner> = runner.clone();
    cell.set(as_dyn).ok();
    let mut tools = create_ask_tools_with(db.clone(), cell, None);
    let result = tools.pop().unwrap();
    let ask = tools.pop().unwrap();
    Rig {
        db,
        runner,
        ask,
        result,
    }
}

fn user_messages(db: &Database, thread: &str) -> usize {
    db.with_conn(|c| hq_db::chat::get_messages(c, thread, 100))
        .unwrap()
        .iter()
        .filter(|m| m.role == "user")
        .count()
}

fn handoff(mut args: Value) -> Value {
    args[crate::harness_session::HANDOFF_SCOPE_ARG] = json!(true);
    args
}

#[tokio::test]
async fn a_quick_reply_is_returned_in_the_same_call() {
    let r = rig(Plan::AnswerSoon("It is 42."));
    let out = r
        .ask
        .execute(json!({"question": "What is the answer?\nSecond line.", "caller": "claude-code"}))
        .await
        .unwrap();
    assert_eq!(out["status"], "answered");
    assert_eq!(out["answer"], "It is 42.");
    let thread = out["thread_id"].as_str().unwrap();
    assert_eq!(out["links"]["chat"], format!("/chat?thread={thread}"));
    assert!(out["turn_id"].as_str().unwrap().starts_with("turn-"));
    assert_eq!(out["mode"], "read_only");
    assert!(out["untrusted_data"].is_string());
    assert_eq!(user_messages(&r.db, thread), 1);
    let title =
        r.db.with_conn(|c| hq_db::chat::get_thread(c, thread))
            .unwrap()
            .unwrap()
            .title;
    assert_eq!(title, "What is the answer?");
    let starts = r.runner.starts.lock().unwrap();
    assert_eq!(starts.len(), 1);
    assert_eq!(
        (starts[0].2, starts[0].3, starts[0].4.as_str()),
        (AskMode::ReadOnly, AskScope::Full, "claude-code")
    );
}

#[tokio::test]
async fn a_slow_reply_is_pending_and_a_later_request_collects_it() {
    let r = rig(Plan::Hold);
    let first = r
        .ask
        .execute(json!({"question": "slow one", "wait_secs": 0}))
        .await
        .unwrap();
    assert_eq!(first["status"], "pending");
    assert!(first.get("answer").is_none());
    let ask_id = first["ask_id"].as_str().unwrap().to_string();

    let still = r
        .result
        .execute(json!({"ask_id": ask_id, "wait_secs": 0}))
        .await
        .unwrap();
    assert_eq!(still["status"], "pending");

    r.db.with_conn(|c| asks::settle(c, &ask_id, STATUS_ANSWERED, Some(("done", "m")), None))
        .unwrap();
    let got = r
        .result
        .execute(json!({"ask_id": ask_id, "wait_secs": 2}))
        .await
        .unwrap();
    assert_eq!(
        (got["status"].as_str(), got["answer"].as_str()),
        (Some("answered"), Some("done"))
    );
}

#[tokio::test]
async fn a_result_call_waits_for_the_answer_that_lands_while_it_waits() {
    let r = rig(Plan::Hold);
    let first = r
        .ask
        .execute(json!({"question": "q", "wait_secs": 0}))
        .await
        .unwrap();
    let ask_id = first["ask_id"].as_str().unwrap().to_string();
    let db = r.db.clone();
    let id = ask_id.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        db.with_conn(|c| asks::settle(c, &id, STATUS_ANSWERED, Some(("late answer", "m")), None))
            .unwrap();
    });
    let got = r
        .result
        .execute(json!({"ask_id": ask_id, "wait_secs": 5}))
        .await
        .unwrap();
    assert_eq!(got["answer"], "late answer");
}

#[tokio::test]
async fn the_same_external_id_posts_the_question_once() {
    let r = rig(Plan::Hold);
    let args = json!({"question": "only once", "external_id": "job-1", "wait_secs": 0});
    let a = r.ask.execute(args.clone()).await.unwrap();
    let b = r.ask.execute(args).await.unwrap();
    assert_eq!(a["ask_id"], b["ask_id"]);
    assert_eq!(a["thread_id"], b["thread_id"]);
    assert!(a.get("deduplicated").is_none());
    assert_eq!(b["deduplicated"], true);
    assert_eq!(r.runner.starts.lock().unwrap().len(), 1);
    assert_eq!(user_messages(&r.db, a["thread_id"].as_str().unwrap()), 1);
}

#[tokio::test]
async fn reusing_an_external_id_for_another_question_is_refused() {
    let r = rig(Plan::Hold);
    r.ask
        .execute(json!({"question": "first", "external_id": "k", "wait_secs": 0}))
        .await
        .unwrap();
    let err = r
        .ask
        .execute(json!({"question": "second", "external_id": "k", "wait_secs": 0}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("different question"), "{err}");
    assert_eq!(r.runner.starts.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn an_existing_thread_is_continued_and_a_missing_one_is_refused() {
    let r = rig(Plan::Hold);
    let thread =
        r.db.with_conn(|c| hq_db::chat::create_thread(c, "mine", "user", "user"))
            .unwrap();
    let out = r
        .ask
        .execute(json!({"question": "follow up", "thread_id": thread.thread_id, "wait_secs": 0}))
        .await
        .unwrap();
    assert_eq!(out["thread_id"], thread.thread_id);
    let err = r
        .ask
        .execute(json!({"question": "x", "thread_id": "nope", "wait_secs": 0}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("does not exist"), "{err}");
}

#[tokio::test]
async fn the_handoff_key_cannot_use_full_mode_but_can_ask_read_only() {
    let r = rig(Plan::Hold);
    let err = r
        .ask
        .execute(handoff(
            json!({"question": "q", "mode": "full", "wait_secs": 0}),
        ))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("full-scope key"), "{err}");
    assert!(
        r.runner.starts.lock().unwrap().is_empty(),
        "nothing was posted"
    );

    let ok = r
        .ask
        .execute(handoff(json!({"question": "q", "wait_secs": 0})))
        .await
        .unwrap();
    assert_eq!(ok["mode"], "read_only");
    assert_eq!(r.runner.starts.lock().unwrap()[0].3, AskScope::Handoff);

    let full = r
        .ask
        .execute(json!({"question": "q2", "mode": "full", "wait_secs": 0}))
        .await
        .unwrap();
    assert_eq!(full["mode"], "full");
    assert_eq!(r.runner.starts.lock().unwrap()[1].2, AskMode::Full);
}

#[tokio::test]
async fn an_unknown_mode_is_refused_rather_than_guessed() {
    let r = rig(Plan::Hold);
    let err = r
        .ask
        .execute(json!({"question": "q", "mode": "write"}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("read_only"), "{err}");
}

#[tokio::test]
async fn a_key_reads_only_its_own_asks() {
    let r = rig(Plan::Hold);
    let full = r
        .ask
        .execute(json!({"question": "full's", "wait_secs": 0}))
        .await
        .unwrap();
    let theirs = r
        .ask
        .execute(handoff(json!({"question": "handoff's", "wait_secs": 0})))
        .await
        .unwrap();

    let err = r
        .result
        .execute(handoff(json!({"ask_id": full["ask_id"], "wait_secs": 0})))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no ask with id"), "{err}");
    assert!(
        r.result
            .execute(handoff(json!({"ask_id": theirs["ask_id"], "wait_secs": 0})))
            .await
            .is_ok()
    );
    assert!(
        r.result
            .execute(json!({"ask_id": theirs["ask_id"], "wait_secs": 0}))
            .await
            .is_ok(),
        "the full key sees both"
    );
    assert!(
        r.result
            .execute(json!({"ask_id": "ask-unknown", "wait_secs": 0}))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn the_question_length_is_capped() {
    let r = rig(Plan::Hold);
    let long = "a".repeat(MAX_QUESTION_CHARS + 1);
    let err = r
        .ask
        .execute(json!({"question": long, "wait_secs": 0}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("limit is 20000"), "{err}");
    let at_limit = "é".repeat(MAX_QUESTION_CHARS);
    assert!(
        r.ask
            .execute(json!({"question": at_limit, "wait_secs": 0}))
            .await
            .is_ok(),
        "the cap counts characters"
    );
    assert!(
        r.ask
            .execute(json!({"question": "   ", "wait_secs": 0}))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn new_questions_are_rate_limited_per_key_but_a_replay_is_free() {
    let r = rig(Plan::Hold);
    for i in 0..ASKS_PER_WINDOW {
        let args =
            json!({"question": format!("q{i}"), "external_id": format!("e{i}"), "wait_secs": 0});
        let out = r.ask.execute(args).await.unwrap();
        // Answered at once, so the limit under test is the rate and not the pending cap.
        let id = out["ask_id"].as_str().unwrap();
        r.db.with_conn(|c| asks::settle(c, id, STATUS_ANSWERED, Some(("a", "m")), None))
            .unwrap();
    }
    let err = r
        .ask
        .execute(json!({"question": "one more", "wait_secs": 0}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("too many questions"), "{err}");
    let replay = r
        .ask
        .execute(json!({"question": "q0", "external_id": "e0", "wait_secs": 0}))
        .await
        .unwrap();
    assert_eq!(replay["deduplicated"], true);
    assert!(
        r.ask
            .execute(handoff(json!({"question": "other key", "wait_secs": 0})))
            .await
            .is_ok()
    );
}

#[test]
fn the_limiter_forgets_calls_once_the_window_passes() {
    let limiter = RateLimiter::new(2, Duration::from_millis(80));
    assert!(limiter.try_acquire("k").is_ok() && limiter.try_acquire("k").is_ok());
    let wait = limiter.try_acquire("k").unwrap_err();
    assert!(wait <= Duration::from_millis(80));
    assert!(limiter.try_acquire("other").is_ok());
    std::thread::sleep(Duration::from_millis(100));
    assert!(limiter.try_acquire("k").is_ok());
}

#[tokio::test]
async fn a_pending_ask_whose_turn_is_gone_resolves_to_failed() {
    let r = rig(Plan::Hold);
    let first = r
        .ask
        .execute(json!({"question": "q", "wait_secs": 0}))
        .await
        .unwrap();
    assert_eq!(first["status"], "pending");
    r.runner.running.store(false, Ordering::SeqCst);
    let got = r
        .result
        .execute(json!({"ask_id": first["ask_id"], "wait_secs": 5}))
        .await
        .unwrap();
    assert_eq!(got["status"], "failed");
    assert!(
        got["error"]
            .as_str()
            .unwrap()
            .contains("stopped without recording an answer")
    );
}

#[tokio::test]
async fn a_failed_start_leaves_nothing_behind_and_the_call_can_be_retried() {
    let db = Arc::new(Database::open_memory().unwrap());
    let refusing = FakeRunner::new(&db, Plan::Refuse("a reply is still running in that thread"));
    let cell: RunnerCell = Arc::new(OnceLock::new());
    let as_dyn: Arc<dyn AskRunner> = refusing;
    cell.set(as_dyn).ok();
    let ask = create_ask_tools_with(db.clone(), cell, None).remove(0);
    let args = json!({"question": "q", "external_id": "retry-me", "wait_secs": 0});
    let err = ask.execute(args.clone()).await.unwrap_err();
    assert!(format!("{err:#}").contains("still running"), "{err:#}");
    assert!(format!("{err:#}").contains("nothing was posted"), "{err:#}");
    assert!(
        db.with_conn(|c| asks::get_by_external(c, "full", "retry-me"))
            .unwrap()
            .is_none()
    );
    let active = db
        .with_conn(|c| hq_db::chat::list_threads(c, 10, 0, Some("web")))
        .unwrap();
    assert!(
        active.iter().all(|t| t.status != "active"),
        "the thread the ask made is archived"
    );
}

#[tokio::test]
async fn without_a_running_web_server_the_tool_says_so() {
    let db = Arc::new(Database::open_memory().unwrap());
    let cell: RunnerCell = Arc::new(OnceLock::new());
    let ask = create_ask_tools_with(db.clone(), cell, None).remove(0);
    let err = ask.execute(json!({"question": "q"})).await.unwrap_err();
    assert!(err.to_string().contains("running HQ web server"), "{err}");
    assert!(
        db.with_conn(|c| hq_db::chat::list_threads(c, 10, 0, None))
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn family_guests_are_refused_by_both_tools() {
    let db = Arc::new(Database::open_memory().unwrap());
    let guest = FamilyGuestContext {
        name: "Carol".into(),
        origin_channel_id: 1,
        owner_name: "Owner".into(),
        allowed_harnesses: Vec::new(),
    };
    let tools = create_ask_tools_with(db, Arc::new(OnceLock::new()), Some(guest));
    for tool in tools {
        let err = tool
            .execute(json!({"question": "q", "ask_id": "a"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Carol cannot ask HQ"), "{err}");
    }
}

#[test]
fn waits_are_capped_below_the_transport_timeout() {
    assert_eq!(
        clamp_wait(&json!({})),
        Duration::from_secs(DEFAULT_WAIT_SECS)
    );
    assert_eq!(
        clamp_wait(&json!({"wait_secs": 999})),
        Duration::from_secs(MAX_WAIT_SECS)
    );
    assert_eq!(clamp_wait(&json!({"wait_secs": 0})), Duration::ZERO);
    const { assert!(MAX_WAIT_SECS < 60) };
}

#[test]
fn the_caller_label_is_trimmed_to_harmless_text() {
    assert_eq!(clean_caller("claude-code"), "claude-code");
    assert_eq!(
        clean_caller("<script>alert(1)</script>"),
        "scriptalert1/script"
    );
    assert_eq!(clean_caller("  "), DEFAULT_CALLER);
    assert_eq!(clean_caller(&"x".repeat(100)).len(), MAX_CALLER_CHARS);
}

#[test]
fn the_view_carries_the_answer_text_and_nothing_else_from_the_turn() {
    let row = |status: &str, answer: Option<&str>, error: Option<&str>| AskRow {
        ask_id: "ask-1".into(),
        thread_id: "t1".into(),
        turn_id: Some("turn-1".into()),
        external_id: None,
        scope: "full".into(),
        mode: "read_only".into(),
        caller: "c".into(),
        fingerprint: "f".into(),
        status: status.into(),
        answer: answer.map(Into::into),
        answer_message_id: Some("m1".into()),
        error: error.map(Into::into),
        created_at: "2026-10-02T00:00:00Z".into(),
        answered_at: Some("2026-10-02T00:00:05Z".into()),
    };
    let answered = view(&row("answered", Some("the answer"), None), false);
    let keys: Vec<&str> = answered
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    for key in keys {
        assert!(
            [
                "ask_id",
                "thread_id",
                "turn_id",
                "links",
                "status",
                "mode",
                "created_at",
                "answer",
                "answered_at",
                "untrusted_data"
            ]
            .contains(&key),
            "unexpected field {key}"
        );
    }
    let failed = view(&row("failed", None, Some("boom")), false);
    assert_eq!(
        (failed["status"].as_str(), failed["error"].as_str()),
        (Some("failed"), Some("boom"))
    );
    assert!(failed.get("answer").is_none());

    let long = "é".repeat(MAX_ANSWER_BYTES);
    let cut = view(&row("answered", Some(&long), None), false);
    let kept = cut["answer"].as_str().unwrap();
    assert!(kept.len() <= MAX_ANSWER_BYTES && kept.len() >= MAX_ANSWER_BYTES - 1);
    assert_eq!(
        (
            cut["answer_truncated"].as_bool(),
            cut["answer_chars"].as_u64()
        ),
        (Some(true), Some(MAX_ANSWER_BYTES as u64))
    );
}

/// The gateway cuts a result past this many tokens in the middle, which would break the JSON.
#[test]
fn even_the_worst_case_answer_stays_under_the_gateways_cut() {
    let row = |answer: &str| AskRow {
        ask_id: "ask-0123456789abcdef0123456789abcdef".into(),
        thread_id: "00000000-0000-0000-0000-000000000000".into(),
        turn_id: Some("turn-0123456789abcdef0123456789abcdef".into()),
        external_id: None,
        scope: "full".into(),
        mode: "read_only".into(),
        caller: "c".into(),
        fingerprint: "f".into(),
        status: "answered".into(),
        answer: Some(answer.into()),
        answer_message_id: None,
        error: None,
        created_at: "2026-10-02T00:00:00+00:00".into(),
        answered_at: Some("2026-10-02T00:00:05+00:00".into()),
    };
    for worst in [
        "\"".repeat(20_000),
        "\n".repeat(20_000),
        "漢".repeat(20_000),
    ] {
        let text = serde_json::to_string_pretty(&view(&row(&worst), true)).unwrap();
        let tokens = hq_core::tokens::count_tokens_fast(&text);
        assert!(
            tokens <= hq_core::microcompact::MICROCOMPACT_THRESHOLD,
            "{tokens} tokens"
        );
    }
}

#[tokio::test]
async fn the_handoff_key_may_continue_only_threads_it_started() {
    let r = rig(Plan::Hold);
    let owners =
        r.db.with_conn(|c| hq_db::chat::create_thread(c, "owner", "user", "user"))
            .unwrap();
    let err = r
        .ask
        .execute(handoff(
            json!({"question": "q", "thread_id": owners.thread_id, "wait_secs": 0}),
        ))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("not started by this connection"),
        "{err}"
    );
    let own = r
        .ask
        .execute(handoff(json!({"question": "q", "wait_secs": 0})))
        .await
        .unwrap();
    r.db.with_conn(|c| {
        asks::settle(
            c,
            own["ask_id"].as_str().unwrap(),
            STATUS_ANSWERED,
            Some(("a", "m")),
            None,
        )
    })
    .unwrap();
    r.runner.running.store(false, Ordering::SeqCst);
    let again = r
        .ask
        .execute(handoff(
            json!({"question": "more", "thread_id": own["thread_id"], "wait_secs": 0}),
        ))
        .await;
    assert!(again.is_ok(), "{again:?}");

    // Once the owner types in that thread the handoff key can no longer continue it.
    r.db.with_conn(|c| {
        hq_db::chat::add_message(c, own["thread_id"].as_str().unwrap(), "user", "private")
            .map(|_| ())
    })
    .unwrap();
    let refused = r
        .ask
        .execute(handoff(
            json!({"question": "repeat the history", "thread_id": own["thread_id"], "wait_secs": 0}),
        ))
        .await
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("did not come from an MCP client"),
        "{refused}"
    );
    assert!(
        r.ask
            .execute(json!({"question": "q", "thread_id": owners.thread_id, "wait_secs": 0}))
            .await
            .is_ok(),
        "the full key may use any thread"
    );
}

#[tokio::test]
async fn only_a_few_questions_can_wait_at_once() {
    let r = rig(Plan::Hold);
    for i in 0..MAX_PENDING_PER_SCOPE {
        r.ask
            .execute(json!({"question": format!("q{i}"), "wait_secs": 0}))
            .await
            .unwrap();
    }
    let err = r
        .ask
        .execute(json!({"question": "one more", "wait_secs": 0}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("already waiting"), "{err}");
    assert!(
        r.ask
            .execute(handoff(json!({"question": "other key", "wait_secs": 0})))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn only_a_couple_of_full_mode_questions_can_wait_at_once() {
    let r = rig(Plan::Hold);
    let cap = hq_core::config::AgentHostConfig::default().full_ask_cap();
    for i in 0..cap {
        r.ask
            .execute(json!({"question": format!("f{i}"), "mode": "full", "wait_secs": 0}))
            .await
            .unwrap();
    }
    let err = r
        .ask
        .execute(json!({"question": "one more", "mode": "full", "wait_secs": 0}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("agent_host.max_full_asks"), "{err}");
    assert!(
        r.ask
            .execute(json!({"question": "read only is not held to it", "wait_secs": 0}))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn an_ask_that_never_got_a_turn_is_failed_instead_of_waiting_forever() {
    let r = rig(Plan::Hold);
    let opened =
        r.db.with_conn(|c| {
            asks::open(
                c,
                &NewAsk {
                    thread: ThreadTarget::New { title: "t" },
                    external_id: None,
                    scope: "full",
                    mode: "read_only",
                    caller: "c",
                    fingerprint: "f",
                },
            )
        })
        .unwrap();
    r.db.with_conn(|c| {
        c.execute(
            "UPDATE hq_asks SET created_at = '2020-01-01T00:00:00+00:00' WHERE ask_id = ?1",
            [&opened.row.ask_id],
        )?;
        Ok(())
    })
    .unwrap();
    let got = r
        .result
        .execute(json!({"ask_id": opened.row.ask_id, "wait_secs": 3}))
        .await
        .unwrap();
    assert_eq!(got["status"], "failed");
}

#[test]
fn wait_secs_given_as_text_is_still_honoured() {
    assert_eq!(clamp_wait(&json!({"wait_secs": "0"})), Duration::ZERO);
    assert_eq!(clamp_wait(&json!({"wait_secs": -5})), Duration::ZERO);
    assert_eq!(clamp_wait(&json!({"wait_secs": "soon"})), Duration::ZERO);
    assert_eq!(
        clamp_wait(&json!({"wait_secs": 7.9})),
        Duration::from_secs(7)
    );
    assert_eq!(clamp_wait(&json!({"wait_secs": -0.5})), Duration::ZERO);
    assert_eq!(
        clamp_wait(&json!({"wait_secs": null})),
        Duration::from_secs(DEFAULT_WAIT_SECS)
    );
    assert_eq!(
        clamp_wait(&json!({"wait_secs": " 7 "})),
        Duration::from_secs(7)
    );
}

#[test]
fn a_session_hq_spawned_may_ask_read_only_but_not_in_full_mode() {
    let marker = crate::harness_session::SPAWNED_SESSION_ARG;
    let ask = |mode: &str, marked: bool| {
        let mut args = json!({"question": "q", "mode": mode});
        if marked {
            args[marker] = json!("hs-claude-code-1");
        }
        AskArgs::parse(&args)
    };
    assert_eq!(ask("read_only", true).unwrap().mode, AskMode::ReadOnly);
    assert_eq!(ask("", true).unwrap().mode, AskMode::ReadOnly);
    let err = ask("full", true).err().unwrap().to_string();
    assert_eq!(err, crate::harness_session::SPAWNED_REFUSAL);
    assert_eq!(ask("full", false).unwrap().mode, AskMode::Full);
}
