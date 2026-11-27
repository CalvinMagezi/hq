#[test]
fn state_where_ignores_unknown_and_hostile_filters() {
    for filter in [
        "",
        "bogus",
        "pending' OR 1=1 --",
        "approved; DROP TABLE value_items",
    ] {
        assert_eq!(state_where(Some(filter)), "", "{filter}");
    }
    assert_eq!(state_where(None), "");
    assert!(state_where(Some("pending")).starts_with(" WHERE v.state IN"));
}

use super::*;

#[tokio::test]
async fn test_aggregate_empty_notifications() {
    let db = hq_db::Database::open_memory().unwrap();
    let res = aggregate_notifications(&db, Some("pending"), None, 50, true).unwrap();
    assert_eq!(res.notifications.len(), 0);
    assert_eq!(res.unread_count, 0);
    assert_eq!(res.pending_approvals_count, 0);
}

#[tokio::test]
async fn the_default_list_and_badge_hold_only_the_owners_items() {
    use hq_core::types::{ValueItem, ValueKind};
    let db = hq_db::Database::open_memory().unwrap();
    for i in 0..200 {
        let (source, kind) = match i % 4 {
            0 => ("harness-sessions", ValueKind::Fyi),
            1 => ("fleet-digest", ValueKind::Fyi),
            2 => ("session-supervisor", ValueKind::ActionNeeded),
            _ => ("notify_owner", ValueKind::Fyi),
        };
        let item = ValueItem::new(source, kind, format!("t{i}"), "b");
        hq_db::value_items::insert(&db, &item).unwrap();
    }
    let default = aggregate_notifications(&db, Some("pending"), None, 500, false).unwrap();
    assert_eq!(default.notifications.len(), 100);
    assert!(default.notifications.iter().all(|n| n.audience == "owner"));
    assert_eq!(default.unread_count, default.notifications.len());
    let all = aggregate_notifications(&db, Some("pending"), None, 500, true).unwrap();
    assert_eq!(all.notifications.len(), 200);
}

#[tokio::test]
async fn test_scan_value_items() {
    let db = hq_db::Database::open_memory().unwrap();
    db.with_conn(|conn| {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS value_items (
                    id TEXT PRIMARY KEY,
                    source_task TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    title TEXT NOT NULL,
                    body TEXT NOT NULL,
                    artifact_path TEXT,
                    score REAL NOT NULL,
                    state TEXT NOT NULL,
                    created_at TEXT NOT NULL
                )",
                [],
            )?;
            conn.execute(
                "INSERT INTO value_items (id, source_task, kind, title, body, artifact_path, score, state, created_at)
                 VALUES ('val_1', 'cron:triage', 'action_needed', 'High memory usage', 'Server is near RAM limit', NULL, 0.95, 'pending', '2026-08-18T10:00:00Z')",
                [],
            )?;
            Ok(())
        })
        .unwrap();

    let items = scan_value_items(&db, Some("pending")).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "val_val_1");
    assert_eq!(items[0].title, "High memory usage");
    assert_eq!(items[0].state, NotificationState::Pending);
    assert_eq!(items[0].kind, NotificationKind::ActionNeeded);
    assert_eq!(items[0].actions, ["approve", "reject"]);

    let res = aggregate_notifications(&db, Some("pending"), Some("all"), 50, true).unwrap();
    assert_eq!(res.notifications.len(), 1);
    assert_eq!(res.pending_approvals_count, 1);
}

#[test]
fn a_host_that_is_not_answering_offers_nothing_to_approve() {
    let db = hq_db::Database::open_memory().unwrap();
    let item = hq_core::types::ValueItem::new(
        "session-supervisor",
        hq_core::types::ValueKind::ActionNeeded,
        "Host 'laptop' is not answering",
        "Host laptop has not answered for 5 minutes.",
    )
    .with_dedup_key("host-unreachable-laptop-1000");
    hq_db::value_items::emit(&db, &item).unwrap();
    let items = scan_value_items(&db, Some("pending")).unwrap();
    assert_eq!(items[0].kind, NotificationKind::ActionNeeded);
    assert_eq!(items[0].actions, ["dismiss"]);
}

#[test]
fn a_blocked_sessions_alert_lists_its_dialog_and_its_one_button() {
    let db = hq_db::Database::open_memory().unwrap();
    let item = hq_core::types::ValueItem::new(
        "session-supervisor",
        hq_core::types::ValueKind::ActionNeeded,
        "Harness session 'copilot' is waiting for you",
        "Session hs-1 on laptop is blocked on a prompt or approval",
    );
    hq_db::value_items::insert(&db, &item).unwrap();
    let plain = hq_core::types::ValueItem::new(
        "notify_owner",
        hq_core::types::ValueKind::Fyi,
        "Deploy finished",
        "v1.2 is live",
    );
    hq_db::value_items::insert(&db, &plain).unwrap();
    let options = vec![
        hq_db::dialog_alerts::DialogOption {
            id: 1,
            label: "Yes".into(),
            remember: false,
            typed: false,
        },
        hq_db::dialog_alerts::DialogOption {
            id: 2,
            label: "Yes, and remember this path".into(),
            remember: true,
            typed: false,
        },
    ];
    hq_db::dialog_alerts::record(
        &db,
        &hq_db::dialog_alerts::NewDialogAlert {
            item_id: &item.id,
            session_id: "hs-1",
            pending_seq: 7,
            kind: "path",
            question: Some("Do you want to allow this?"),
            command: Some("/tmp"),
            options: &options,
        },
    )
    .unwrap();
    let items = scan_value_items(&db, Some("pending")).unwrap();
    let alert = items
        .iter()
        .find(|n| n.id == format!("val_{}", item.id))
        .unwrap();
    let dialog = alert
        .metadata
        .dialog
        .as_ref()
        .expect("the alert carries its dialog");
    assert_eq!(dialog["seq"], 7);
    assert_eq!(dialog["approve"]["label"], "Yes");
    assert_eq!(dialog["options"][1]["remember"], true);
    let news = items
        .iter()
        .find(|n| n.id == format!("val_{}", plain.id))
        .unwrap();
    assert!(news.metadata.dialog.is_none());
}

#[tokio::test]
async fn a_cleared_dialog_notice_comes_back_but_a_decision_never_does() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
    let db = &state.db;
    let mk = |title: &str| {
        let item = hq_core::types::ValueItem::new(
            "session-supervisor",
            hq_core::types::ValueKind::ActionNeeded,
            title,
            "blocked",
        );
        hq_db::value_items::insert(db, &item).unwrap();
        item.id
    };
    let notice = mk("Helper is waiting");
    let decision = mk("Allow this?");
    hq_db::dialog_alerts::record(
        db,
        &hq_db::dialog_alerts::NewDialogAlert {
            item_id: &notice,
            session_id: "hs-1",
            pending_seq: 1,
            kind: "path",
            question: None,
            command: None,
            options: &[],
        },
    )
    .unwrap();
    for id in [&notice, &decision] {
        hq_db::value_items::answer_by_id(db, id, "dismissed").unwrap();
    }
    let body = serde_json::json!({ "ids": [format!("val_{notice}"), format!("val_{decision}")] });
    let res = restore_cleared_handler(
        State(state.clone()),
        Json(serde_json::from_value(body).unwrap()),
    )
    .await;
    assert_eq!(res.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let out: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(out["restored"], 1, "only the dialog notice comes back");
    let pending = scan_value_items(db, Some("pending")).unwrap();
    assert!(pending.iter().any(|n| n.id == format!("val_{notice}")));
    assert!(!pending.iter().any(|n| n.id == format!("val_{decision}")));
}

#[tokio::test]
async fn an_unchanged_list_is_a_bodiless_304_and_a_change_is_a_new_etag() {
    use axum::http::{HeaderMap, HeaderValue, header};
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
    let insert = |id: &str| {
        let id = id.to_string();
        state
                .db
                .with_conn(move |c| {
                    Ok(c.execute(
                        "INSERT INTO value_items (id, source_task, kind, title, body, state, created_at)
                         VALUES (?1, 't', 'action_needed', 'T', 'B', 'pending', '2026-09-25T00:00:00Z')",
                        [id],
                    )?)
                })
                .unwrap();
    };
    let list = |held: Option<String>| {
        let mut headers = HeaderMap::new();
        if let Some(tag) = held {
            headers.insert(header::IF_NONE_MATCH, HeaderValue::from_str(&tag).unwrap());
        }
        let params = ListNotificationsParams {
            state: Some("pending".into()),
            category: None,
            limit: Some(30),
            include_agent: Some("1".into()),
        };
        list_notifications_handler(State(state.clone()), Query(params), headers)
    };
    insert("first111");
    let first = list(None).await;
    assert_eq!(first.status(), 200);
    let tag = first.headers()[header::ETAG].to_str().unwrap().to_string();
    assert!(tag.starts_with('"') && tag.ends_with('"'), "{tag}");

    let same = list(Some(tag.clone())).await;
    assert_eq!(same.status(), 304);
    let body = axum::body::to_bytes(same.into_body(), 1024).await.unwrap();
    assert!(body.is_empty(), "a 304 carries no list");

    insert("second22");
    let changed = list(Some(tag.clone())).await;
    assert_eq!(changed.status(), 200, "a new item is a new list");
    assert_ne!(changed.headers()[header::ETAG].to_str().unwrap(), tag);
}

#[test]
fn an_approval_names_the_web_chat_it_came_from() {
    assert_eq!(
        web_chat_of("owner-approval:web:t-42:bash:abc"),
        Some("t-42")
    );
    assert_eq!(web_chat_of("owner-approval:web:new:bash:abc"), None);
    assert_eq!(
        web_chat_of("owner-approval:relay:telegram:9:bash:abc"),
        None
    );
    assert_eq!(web_chat_of("disk-watchdog-2026-11-05"), None);
}

#[test]
fn counts_split_owner_only_decisions_from_helper_requests_and_list_the_tidied() {
    use hq_core::types::{ValueItem, ValueKind};
    let db = hq_db::Database::open_memory().unwrap();
    let put = |source: &str, key: &str, owner_only: bool| {
        let item = ValueItem::new(source, ValueKind::ActionNeeded, "t", "b")
            .with_dedup_key(key)
            .owner_only(owner_only);
        hq_db::value_items::insert(&db, &item).unwrap();
        item.id
    };
    put("device-sign-in", "sign-in-1", true);
    put("session-supervisor", "session-blocked-hs-1-1", false);
    put("session-supervisor", "session-blocked-hs-2-1", false);
    let gone = put("session-supervisor", "session-blocked-hs-3-1", false);
    db.with_conn(|c| {
        c.execute(
            "UPDATE value_items SET state = 'expired', expired_reason = 'auto_tidy:session_ended', \
                 expired_at = ?2 WHERE id = ?1",
            rusqlite::params![gone, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    })
    .unwrap();

    let res = aggregate_notifications(&db, None, None, 50, true).unwrap();
    assert_eq!(res.needs_you_owner_only, 1);
    assert_eq!(res.needs_you_helpers, 2);
    assert_eq!(res.pending_approvals_count, 3, "the old count is unchanged");
    assert_eq!(res.expired_count, 1);
    let tidied = res
        .notifications
        .iter()
        .find(|n| n.id == format!("val_{gone}"))
        .unwrap();
    assert_eq!(
        tidied.expired_reason.as_deref(),
        Some("auto_tidy:session_ended")
    );
    assert_eq!(tidied.source_group, "helpers");
    let sign_in = res.notifications.iter().find(|n| n.owner_only).unwrap();
    assert_eq!(sign_in.source_group, "you");
}

#[test]
fn a_notice_to_look_at_offers_only_dismiss() {
    let db = hq_db::Database::open_memory().unwrap();
    let item = hq_core::types::ValueItem::new(
        "disk_watchdog",
        hq_core::types::ValueKind::ActionNeeded,
        "Disk watchdog: threshold(s) breached",
        "Disk 96% used",
    )
    .with_dedup_key("disk-watchdog-2026-11-05");
    hq_db::value_items::insert(&db, &item).unwrap();
    let items = scan_value_items(&db, Some("pending")).unwrap();
    assert_eq!(items[0].actions, ["dismiss"]);
}

#[test]
fn an_owner_approval_quotes_its_call_and_marks_a_losing_one() {
    let db = hq_db::Database::open_memory().unwrap();
    let ask = |tool: &str, args: serde_json::Value| {
        let item = hq_core::types::ValueItem::new(
            "owner-approval",
            hq_core::types::ValueKind::ActionNeeded,
            format!("HQ wants to run {tool}"),
            "Approve to let it run once.",
        );
        hq_db::owner_approvals::record(&db, &item.id, tool, &args).unwrap();
        hq_db::value_items::insert(&db, &item).unwrap();
        format!("val_{}", item.id)
    };
    let delete = ask("task_delete", serde_json::json!({"id": "t-1"}));
    let list = ask("bash", serde_json::json!({"command": "ls -la"}));
    let items = scan_value_items(&db, Some("pending")).unwrap();
    let approval = |id: &str| {
        items
            .iter()
            .find(|n| n.id == id)
            .unwrap()
            .metadata
            .approval
            .clone()
            .unwrap()
    };
    let d = approval(&delete);
    assert!(
        d["thread_id"].is_null(),
        "an item with no web chat key names no chat"
    );
    assert_eq!(d["tool"], "task_delete");
    assert_eq!(d["args"]["id"], "t-1");
    assert_eq!(d["loses"], true);
    assert_eq!(d["verb"], "delete");
    let l = approval(&list);
    assert_eq!(l["loses"], false);
    assert!(l["verb"].is_null());
}

#[test]
fn a_helpers_dialog_that_deletes_is_marked_losing() {
    let db = hq_db::Database::open_memory().unwrap();
    let item = hq_core::types::ValueItem::new(
        "session-supervisor",
        hq_core::types::ValueKind::ActionNeeded,
        "waiting",
        "blocked",
    );
    hq_db::value_items::insert(&db, &item).unwrap();
    let options = vec![hq_db::dialog_alerts::DialogOption {
        id: 1,
        label: "Yes".into(),
        remember: false,
        typed: false,
    }];
    hq_db::dialog_alerts::record(
        &db,
        &hq_db::dialog_alerts::NewDialogAlert {
            item_id: &item.id,
            session_id: "hs-1",
            pending_seq: 3,
            kind: "command",
            question: Some("Run this?"),
            command: Some("git clean -fdx"),
            options: &options,
        },
    )
    .unwrap();
    let items = scan_value_items(&db, Some("pending")).unwrap();
    let dialog = items[0].metadata.dialog.clone().unwrap();
    assert_eq!(dialog["loses"], true);
    assert_eq!(dialog["verb"], "delete");
    assert!(items[0].metadata.approval.is_none());
}

fn fetch_request(db: &hq_db::Database, chat: &str, url: &str, digest: &str) -> String {
    let item = hq_core::types::ValueItem::new(
        "owner-approval",
        hq_core::types::ValueKind::ActionNeeded,
        "HQ wants to run web_fetch",
        "b",
    )
    .with_dedup_key(format!("owner-approval:{chat}:web_fetch:{digest}"));
    hq_db::owner_approvals::record(db, &item.id, "web_fetch", &serde_json::json!({"url": url}))
        .unwrap();
    hq_db::value_items::insert(db, &item).unwrap();
    item.id
}

fn still_waiting(db: &hq_db::Database, id: &str) -> bool {
    scan_value_items(db, Some("pending"))
        .unwrap()
        .iter()
        .any(|i| i.id == format!("val_{id}"))
}

#[test]
fn allowing_a_site_answers_only_that_sites_short_requests_in_that_chat() {
    let db = hq_db::Database::open_memory().unwrap();
    let chat = "web:t1";
    let asked = fetch_request(&db, chat, "https://a.example/one", "d1");
    let same = fetch_request(&db, chat, "https://www.a.example/two", "d2");
    let other = fetch_request(&db, chat, "https://b.example/", "d3");
    let long = format!("https://a.example/x?d={}", "a".repeat(250));
    let long = fetch_request(&db, chat, &long, "d4");
    let elsewhere = fetch_request(&db, "web:t2", "https://a.example/three", "d5");

    record_action(&db, &asked, ENGAGEMENT_APPROVED).unwrap();
    note_site_answer(&db, &asked, ENGAGEMENT_APPROVED, true).unwrap();

    assert!(!still_waiting(&db, &same), "the same site clears");
    assert!(still_waiting(&db, &other), "another site still asks");
    assert!(still_waiting(&db, &long), "a long address still asks");
    assert!(still_waiting(&db, &elsewhere), "another chat still asks");
}

#[test]
fn a_plain_approval_widens_to_the_site_only_on_the_third() {
    let db = hq_db::Database::open_memory().unwrap();
    for n in 1..=2 {
        let id = fetch_request(
            &db,
            "web:t1",
            &format!("https://a.example/{n}"),
            &format!("d{n}"),
        );
        record_action(&db, &id, ENGAGEMENT_APPROVED).unwrap();
        note_site_answer(&db, &id, ENGAGEMENT_APPROVED, false).unwrap();
    }
    let waiting = fetch_request(&db, "web:t1", "https://a.example/next", "dn");
    assert!(still_waiting(&db, &waiting));
    let third = fetch_request(&db, "web:t1", "https://a.example/3", "d3");
    record_action(&db, &third, ENGAGEMENT_APPROVED).unwrap();
    note_site_answer(&db, &third, ENGAGEMENT_APPROVED, false).unwrap();
    assert!(!still_waiting(&db, &waiting));
}

/// An owner-approval request of web chat `thread`, as the runtime raises it.
fn chat_request(db: &hq_db::Database, thread: &str, tool: &str, args: serde_json::Value) -> String {
    let digest = hq_core::approval_key::args_digest(&args);
    let item = hq_core::types::ValueItem::new(
        "owner-approval",
        hq_core::types::ValueKind::ActionNeeded,
        format!("HQ wants to run {tool}"),
        "Approve to let it run once.",
    )
    .with_dedup_key(format!("owner-approval:web:{thread}:{tool}:{digest}"))
    .owner_only(true);
    hq_db::owner_approvals::record(db, &item.id, tool, &args).unwrap();
    hq_db::value_items::insert(db, &item).unwrap();
    item.id
}

fn project_folder() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("hq-folder-api")
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .unwrap();
    let root = dir.path().canonicalize().unwrap().join("app");
    std::fs::create_dir_all(&root).unwrap();
    (dir, root)
}

#[test]
fn a_command_request_offers_the_folder_it_changes_into() {
    let db = hq_db::Database::open_memory().unwrap();
    let (_dir, root) = project_folder();
    let id = chat_request(
        &db,
        "t1",
        "bash",
        serde_json::json!({"command": format!("cd {} && make", root.display())}),
    );
    chat_request(&db, "t1", "bash", serde_json::json!({"command": "ls"}));
    let items = scan_value_items(&db, Some("pending")).unwrap();
    let with_folder = items.iter().find(|i| i.id == format!("val_{id}")).unwrap();
    let approval = with_folder.metadata.approval.clone().unwrap();
    assert_eq!(approval["folder"], root.to_string_lossy().as_ref());
    assert_eq!(approval["scope"], "web:t1");
    let bare = items.iter().find(|i| i.id != format!("val_{id}")).unwrap();
    assert!(bare.metadata.approval.as_ref().unwrap()["folder"].is_null());
}

#[test]
fn the_card_counts_the_chats_recent_requests_for_the_nudge() {
    let db = hq_db::Database::open_memory().unwrap();
    for n in 1..=3 {
        chat_request(
            &db,
            "t1",
            "bash",
            serde_json::json!({"command": format!("make {n}")}),
        );
    }
    chat_request(&db, "t2", "bash", serde_json::json!({"command": "make"}));
    let items = scan_value_items(&db, Some("pending")).unwrap();
    let counts: Vec<i64> = items
        .iter()
        .map(|i| {
            i.metadata.approval.as_ref().unwrap()["recent_requests"]
                .as_i64()
                .unwrap()
        })
        .collect();
    assert_eq!(counts.iter().filter(|n| **n == 3).count(), 3);
    assert_eq!(counts.iter().filter(|n| **n == 1).count(), 1);
}

#[test]
fn allowing_a_folder_reads_it_from_the_call_and_refuses_a_call_with_no_folder() {
    let db = hq_db::Database::open_memory().unwrap();
    let (_dir, root) = project_folder();
    let named = chat_request(
        &db,
        "t1",
        "bash",
        serde_json::json!({"command": format!("cd {} && make", root.display())}),
    );
    let (scope, folder) = folder_to_allow(&db, &named).unwrap();
    assert_eq!(scope, "web:t1");
    assert_eq!(folder, root.to_string_lossy());
    let bare = chat_request(&db, "t1", "bash", serde_json::json!({"command": "ls"}));
    assert!(folder_to_allow(&db, &bare).is_err());
    assert!(folder_to_allow(&db, "no-such-item").is_err());
    // A folder that covers too much is never offered, whatever the command says.
    let high = chat_request(
        &db,
        "t1",
        "bash",
        serde_json::json!({"command": "cd /usr && ls"}),
    );
    assert!(folder_to_allow(&db, &high).is_err());
}

fn site_approvals_of(db: &hq_db::Database, id: &str) -> Option<i64> {
    let items = scan_value_items(db, Some("pending")).unwrap();
    let item = items.iter().find(|i| i.id == format!("val_{id}"))?;
    item.metadata.approval.as_ref()?["site_approvals"].as_i64()
}

fn answer(db: &hq_db::Database, id: &str, engagement: &str, whole_site: bool) {
    record_action(db, id, engagement).unwrap();
    note_site_answer(db, id, engagement, whole_site).unwrap();
}

/// The list cuts to `limit` before it looks up each card's extras, so the cards it returns must
/// still carry them, and the counts must not depend on the cut.
#[test]
fn the_list_enriches_only_the_cards_it_returns() {
    let db = hq_db::Database::open_memory().unwrap();
    for n in 0..3 {
        let url = format!("https://a.example/{n}");
        let id = fetch_request(&db, "web:t1", &url, &format!("d{n}"));
        if n < 2 {
            answer(&db, &id, ENGAGEMENT_APPROVED, false);
        }
    }
    let all = aggregate_notifications(&db, Some("pending"), None, 50, false).unwrap();
    let cut = aggregate_notifications(&db, Some("pending"), None, 1, false).unwrap();

    assert_eq!(cut.notifications.len(), 1);
    let card = &cut.notifications[0];
    assert!(
        card.metadata.approval.as_ref().unwrap()["site_approvals"].is_number(),
        "a returned card still has its site count"
    );
    assert_eq!(all.unread_count, cut.unread_count, "counts ignore the cut");
    assert_eq!(all.needs_you_owner_only, cut.needs_you_owner_only);
    assert_eq!(all.needs_you_helpers, cut.needs_you_helpers);
}

#[test]
fn a_fetch_card_carries_how_often_its_site_was_allowed_once() {
    let db = hq_db::Database::open_memory().unwrap();
    let first = fetch_request(&db, "web:t1", "https://a.example/0", "d0");
    assert_eq!(site_approvals_of(&db, &first), Some(0));
    for n in 1..=2 {
        let url = format!("https://a.example/{n}");
        let id = fetch_request(&db, "web:t1", &url, &format!("d{n}"));
        answer(&db, &id, ENGAGEMENT_APPROVED, false);
        let next = fetch_request(&db, "web:t1", "https://a.example/n", &format!("n{n}"));
        assert_eq!(site_approvals_of(&db, &next), Some(n), "after {n}");
    }
}

#[test]
fn a_decline_and_allow_all_elsewhere_do_not_change_the_count() {
    let db = hq_db::Database::open_memory().unwrap();
    for n in 1..=2 {
        let id = fetch_request(
            &db,
            "web:t1",
            &format!("https://a.example/{n}"),
            &format!("d{n}"),
        );
        answer(&db, &id, ENGAGEMENT_APPROVED, false);
    }
    // A decline in the same chat does not lower it.
    let declined = fetch_request(&db, "web:t1", "https://a.example/no", "dn");
    answer(&db, &declined, "dismissed", false);
    // "Allow all" in another chat is not an allow-once answer.
    let all = fetch_request(&db, "web:t2", "https://b.example/x", "da");
    answer(&db, &all, ENGAGEMENT_APPROVED, true);
    let next_a = fetch_request(&db, "web:t3", "https://a.example/y", "dy");
    let next_b = fetch_request(&db, "web:t3", "https://b.example/y", "dz");
    assert_eq!(site_approvals_of(&db, &next_a), Some(2));
    assert_eq!(site_approvals_of(&db, &next_b), Some(0));
}

#[test]
fn a_helpers_dialog_never_counts_even_when_it_is_answered_like_an_approval() {
    let db = hq_db::Database::open_memory().unwrap();
    let helper = hq_core::types::ValueItem::new(
        "session-supervisor",
        hq_core::types::ValueKind::ActionNeeded,
        "waiting",
        "blocked",
    );
    hq_db::value_items::insert(&db, &helper).unwrap();
    answer(&db, &helper.id, ENGAGEMENT_APPROVED, false);
    let count: i64 = db
        .with_conn(|c| {
            Ok(c.query_row(
                "SELECT COALESCE(SUM(count),0) FROM site_allowed_once",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(site_approvals_of(&db, &helper.id), None);
}

#[test]
fn an_approval_item_carries_the_runtime_consequence_sentence() {
    let db = hq_db::Database::open_memory().unwrap();
    let id = fetch_request(&db, "web:t1", "https://a.example/x", "d1");
    let items = scan_value_items(&db, Some("pending")).unwrap();
    let item = items.iter().find(|i| i.id == format!("val_{id}")).unwrap();
    let approval = item.metadata.approval.as_ref().unwrap();
    assert_eq!(
        approval["consequence"],
        "HQ will open a.example and read the page."
    );
    assert_eq!(approval["loses"], false);
}

/// Only HQ's own sign-in item is shown as a new sign-in; an item from another source with the
/// same kind of key is an ordinary one, so it cannot offer "Sign it out" for a real device.
#[test]
fn only_hqs_own_sign_in_item_reads_as_one() {
    let db = hq_db::Database::open_memory().unwrap();
    let item = |source: &str| {
        hq_core::types::ValueItem::new(
            source,
            hq_core::types::ValueKind::ActionNeeded,
            "A new sign-in to your HQ",
            "b",
        )
        .with_dedup_key(format!("{}dev1", crate::device_auth::SIGN_IN_ITEM_PREFIX))
    };
    let real = item(crate::device_auth::DEVICE_SIGN_IN_SOURCE);
    let forged = item(hq_tools::notify::NOTIFY_SOURCE);
    hq_db::value_items::insert(&db, &real).unwrap();
    hq_db::value_items::insert(&db, &forged).unwrap();
    let items = scan_value_items(&db, Some("pending")).unwrap();
    let sign_in = |id: &str| {
        items
            .iter()
            .find(|n| n.id == format!("val_{id}"))
            .unwrap()
            .metadata
            .sign_in
            .clone()
    };
    assert_eq!(
        sign_in(&real.id),
        Some(serde_json::json!({ "device_id": "dev1" }))
    );
    assert_eq!(sign_in(&forged.id), None);
    assert!(
        hq_tools::notify::RESERVED_DEDUP_PREFIXES
            .contains(&crate::device_auth::SIGN_IN_ITEM_PREFIX),
        "notify_owner refuses this key"
    );
}

#[tokio::test]
async fn clearing_all_updates_leaves_every_needs_you_item_waiting() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
    let ask = hq_core::types::ValueItem::new(
        "owner-approval",
        hq_core::types::ValueKind::ActionNeeded,
        "HQ wants to run bash",
        "b",
    );
    let news = hq_core::types::ValueItem::new(
        "notify_owner",
        hq_core::types::ValueKind::Fyi,
        "Deploy finished",
        "b",
    );
    hq_db::value_items::insert(&state.db, &ask).unwrap();
    hq_db::value_items::insert(&state.db, &news).unwrap();
    assert_eq!(
        mark_all_read_handler(State(state.clone())).await.status(),
        200
    );
    let pending = scan_value_items(&state.db, Some("pending")).unwrap();
    let ids: Vec<&str> = pending.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, [format!("val_{}", ask.id)]);
}

#[tokio::test]
async fn action_approves_known_items_and_404s_unknown_ones() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
    state
            .db
            .with_conn(|c| {
                Ok(c.execute(
                    "INSERT INTO value_items (id, source_task, kind, title, body, state, created_at)
                     VALUES ('abc12345', 't', 'action_needed', 'T', 'B', 'pending', '2026-09-25T00:00:00Z')",
                    [],
                )?)
            })
            .unwrap();
    let act = |id: &str, action: &str| {
        let body = NotificationActionRequest {
            action: action.into(),
            starter: None,
            minutes: None,
            until: None,
        };
        notification_action_handler(State(state.clone()), AxumPath(id.into()), Json(body))
    };
    assert_eq!(act("val_abc12345", "approve").await.status(), 200);
    assert_eq!(act("val_missing99", "approve").await.status(), 404);
    assert_eq!(act("val_abc12345", "explode").await.status(), 400);
    let approved = aggregate_notifications(&state.db, Some("approved"), None, 10, true).unwrap();
    assert_eq!(approved.notifications[0].state, NotificationState::Approved);
}

/// One approve approves exactly the item it names. A short id used to approve and sign every
/// delivered item sharing that prefix, owner-approval requests included; an item that was
/// not delivered was marked approved without a signature; and a spent approval came back.
#[tokio::test]
async fn approving_by_id_approves_exactly_that_item_and_signs_it() {
    use hq_core::types::{ValueItem, ValueKind};
    use hq_db::value_items as vi;
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
    let ask = |dedup: &str, id: &str| {
        let mut item = ValueItem::new("owner-approval", ValueKind::ActionNeeded, "Run it?", "b")
            .with_dedup_key(dedup);
        item.id = id.into();
        vi::insert(&state.db, &item).unwrap();
        vi::set_delivered(&state.db, id).unwrap();
    };
    ask("owner:a", "feed0001-aaaa");
    ask("owner:b", "feed0002-bbbb");
    let act = |id: &str, action: &str| {
        let body = NotificationActionRequest {
            action: action.into(),
            starter: None,
            minutes: None,
            until: None,
        };
        notification_action_handler(State(state.clone()), AxumPath(id.into()), Json(body))
    };

    assert_eq!(act("val_feed", "approve").await.status(), 404);
    assert!(!vi::is_signed_approval(&state.db, "owner:a").unwrap());
    assert!(!vi::is_signed_approval(&state.db, "owner:b").unwrap());

    assert_eq!(act("val_feed0001-aaaa", "approve").await.status(), 200);
    assert!(vi::is_signed_approval(&state.db, "owner:a").unwrap());
    assert!(!vi::is_signed_approval(&state.db, "owner:b").unwrap());
    assert_eq!(
        act("val_feed0001-aaaa", "approve").await.status(),
        200,
        "the same answer again changes nothing"
    );

    // Spent by the one call it approved, it cannot be approved back into a second run.
    assert!(vi::consume_approval(&state.db, "owner:a").unwrap());
    assert_eq!(act("val_feed0001-aaaa", "approve").await.status(), 409);
    assert!(!vi::consume_approval(&state.db, "owner:a").unwrap());
    assert_eq!(act("val_feed0001-aaaa", "dismiss").await.status(), 409);

    // Never delivered: approved and signed all the same, or the gate would not run it.
    let mut waiting = ValueItem::new("owner-approval", ValueKind::ActionNeeded, "Run?", "b")
        .with_dedup_key("owner:c");
    waiting.id = "feed0003-cccc".into();
    vi::insert(&state.db, &waiting).unwrap();
    assert_eq!(act("val_feed0003-cccc", "approve").await.status(), 200);
    assert!(vi::is_signed_approval(&state.db, "owner:c").unwrap());

    assert_eq!(act("val_feed0002-bbbb", "dismiss").await.status(), 200);
    assert_eq!(
        act("val_feed0002-bbbb", "approve").await.status(),
        409,
        "a declined request is not approved afterwards"
    );
    assert!(!vi::is_signed_approval(&state.db, "owner:b").unwrap());
}

#[tokio::test]
async fn an_open_dialog_is_not_approved_through_its_notice() {
    // Approving the notice of a question once left the agent waiting with the alert gone.
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
    let item = hq_core::types::ValueItem::new(
        "session-supervisor",
        hq_core::types::ValueKind::ActionNeeded,
        "Harness session 'claude' is waiting for you",
        "Session hs-1 on laptop is blocked on a prompt or approval",
    );
    hq_db::value_items::insert(&state.db, &item).unwrap();
    let options = ["Red", "Blue"].map(|label| hq_db::dialog_alerts::DialogOption {
        id: if label == "Red" { 1 } else { 2 },
        label: label.into(),
        remember: false,
        typed: false,
    });
    hq_db::dialog_alerts::record(
        &state.db,
        &hq_db::dialog_alerts::NewDialogAlert {
            item_id: &item.id,
            session_id: "hs-1",
            pending_seq: 7,
            kind: "other",
            question: Some("Which colour should the test use?"),
            command: None,
            options: &options,
        },
    )
    .unwrap();
    let id = format!("val_{}", item.id);
    let listed = scan_value_items(&state.db, Some("pending")).unwrap();
    let notice = listed.iter().find(|n| n.id == id).unwrap();
    assert_eq!(notice.actions, ["dismiss"], "only its dialog answers it");

    let act = |action: &str| {
        let body = NotificationActionRequest {
            action: action.into(),
            starter: None,
            minutes: None,
            until: None,
        };
        notification_action_handler(State(state.clone()), AxumPath(id.clone()), Json(body))
    };
    for action in ["approve", "engage", "reject"] {
        assert_eq!(act(action).await.status(), 409, "{action}");
    }
    assert_eq!(
        scan_value_items(&state.db, Some("pending")).unwrap().len(),
        1,
        "a refused action leaves the notice in place"
    );
    assert_eq!(act("dismiss").await.status(), 200);
}

fn insert(state: &WsState, source: &str, kind: hq_core::types::ValueKind) -> String {
    let item = hq_core::types::ValueItem::new(source, kind, "t", "b");
    hq_db::value_items::insert(&state.db, &item).unwrap();
    format!("val_{}", item.id)
}

async fn json_of(res: Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn reading_keeps_an_update_in_its_list_and_drops_it_from_the_unread_count() {
    use hq_core::types::ValueKind;
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
    let news = insert(&state, "agent_notify", ValueKind::Fyi);
    insert(&state, "disk-watchdog", ValueKind::Fyi);
    let before = aggregate_notifications(&state.db, Some("pending"), None, 50, true).unwrap();
    assert_eq!(before.unread_updates_count, 2);

    let res = read_handler(State(state.clone()), AxumPath(news.clone())).await;
    assert_eq!(res.status(), 200);
    let after = aggregate_notifications(&state.db, Some("pending"), None, 50, true).unwrap();
    assert_eq!(after.unread_updates_count, 1);
    assert_eq!(after.notifications.len(), 2, "read is not put away");
    assert!(after.notifications.iter().any(|n| n.id == news && n.read));

    assert_eq!(read_all_handler(State(state.clone())).await.status(), 200);
    let after = aggregate_notifications(&state.db, Some("pending"), None, 50, true).unwrap();
    assert_eq!(
        (after.unread_updates_count, after.notifications.len()),
        (0, 2)
    );
}

#[tokio::test]
async fn putting_all_away_can_be_undone_and_done_can_be_cleared() {
    use hq_core::types::ValueKind;
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
    let news = insert(&state, "agent_notify", ValueKind::Fyi);
    let ask = insert(&state, "owner-approval", ValueKind::ActionNeeded);

    let body = json_of(archive_all_handler(State(state.clone())).await).await;
    assert_eq!(body["ids"], serde_json::json!([news]));
    let pending = scan_value_items(&state.db, Some("pending")).unwrap();
    assert_eq!(pending.iter().map(|n| &n.id).collect::<Vec<_>>(), [&ask]);

    let undo = RestoreManyRequest {
        ids: vec![news.clone()],
    };
    let res = restore_many_handler(State(state.clone()), Json(undo)).await;
    assert_eq!(json_of(res).await["restored"], 1);
    assert_eq!(
        scan_value_items(&state.db, Some("pending")).unwrap().len(),
        2
    );
    // A request can never be brought back by Undo: it was never put away.
    let undo = RestoreManyRequest {
        ids: vec![ask.clone()],
    };
    let res = restore_many_handler(State(state.clone()), Json(undo)).await;
    assert_eq!(res.status(), 409);

    archive_all_handler(State(state.clone())).await;
    assert_eq!(
        scan_value_items(&state.db, Some("history")).unwrap().len(),
        1
    );
    assert_eq!(clear_done_handler(State(state.clone())).await.status(), 200);
    assert!(
        scan_value_items(&state.db, Some("history"))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        scan_value_items(&state.db, None).unwrap().len(),
        1,
        "the request still waits"
    );
}

#[tokio::test]
async fn a_silenced_topic_is_listed_but_not_counted_and_hq_is_never_silenced() {
    use hq_core::types::ValueKind;
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
    let helper = insert(&state, "session-supervisor", ValueKind::ActionNeeded);
    insert(&state, "owner-approval", ValueKind::ActionNeeded);

    let ask = |topic: &str, minutes| SilenceRequest {
        topic: topic.into(),
        minutes,
    };
    let res = silence_handler(State(state.clone()), Json(ask("hq", None))).await;
    assert_eq!(res.status(), 400);
    let res = silence_handler(State(state.clone()), Json(ask("helpers", Some(0)))).await;
    assert_eq!(res.status(), 400);
    let res = silence_handler(State(state.clone()), Json(ask("helpers", Some(60)))).await;
    let body = json_of(res).await;
    let helpers = body["topics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["topic"] == "helpers");
    assert_eq!(helpers.unwrap()["silenced"], true);
    assert!(helpers.unwrap()["until"].is_string());

    let list = aggregate_notifications(&state.db, Some("pending"), None, 50, true).unwrap();
    assert_eq!(
        list.notifications.len(),
        2,
        "silenced items are still listed"
    );
    assert_eq!(
        list.pending_approvals_count, 1,
        "only HQ's own request counts"
    );
    let item = list.notifications.iter().find(|n| n.id == helper).unwrap();
    assert!(item.silenced && item.topic == "helpers");

    let res = unsilence_handler(State(state.clone()), AxumPath("helpers".into())).await;
    assert_eq!(res.status(), 200);
    let list = aggregate_notifications(&state.db, Some("pending"), None, 50, true).unwrap();
    assert_eq!(list.pending_approvals_count, 2);
}

#[test]
fn a_background_task_notice_does_not_count_as_needs_you() {
    let db = hq_db::Database::open_memory().unwrap();
    let put = |source: &str, key: &str, owner_only: bool| {
        let item = hq_core::types::ValueItem::new(
            source,
            hq_core::types::ValueKind::ActionNeeded,
            "t",
            "b",
        )
        .with_dedup_key(key)
        .owner_only(owner_only);
        hq_db::value_items::insert(&db, &item).unwrap();
    };
    put("subagent_runs", "subagent-run-abc", false);
    put("device-sign-in", "sign-in-1", true);
    put("cron:triage", "triage-1", false);

    let res = aggregate_notifications(&db, None, None, 50, true).unwrap();
    assert_eq!(res.notifications.len(), 3, "the notice is still listed");
    assert_eq!(res.pending_approvals_count, 2);
    assert_eq!(res.needs_you_owner_only, 1);
    assert_eq!(res.needs_you_helpers, 1);
}

#[tokio::test]
async fn a_legacy_delegated_work_notice_is_read_in_plain_words() {
    use hq_core::types::{ValueItem, ValueKind};
    let db = hq_db::Database::open_memory().unwrap();
    let item = ValueItem::new(
        "subagent_runs",
        ValueKind::ActionNeeded,
        "Delegated work needs attention",
        "Sub-agent run ee68156e (verifier) is partial and completed: it ended without a finished deliverable. Details: subagent_run_status.",
    )
    .with_dedup_key("subagent-run-ee68156e-0000");
    hq_db::value_items::insert(&db, &item).unwrap();
    let res = aggregate_notifications(&db, Some("pending"), None, 50, true).unwrap();
    let n = &res.notifications[0];
    let text = format!("{} {}", n.title, n.description);
    assert!(text.contains("A background task did not finish"), "{text}");
    for leaked in ["ee68156e", "verifier", "subagent_run_status", "Sub-agent"] {
        assert!(!text.contains(leaked), "{text}");
    }
}

#[tokio::test]
async fn a_reminders_ping_offers_done_and_snooze_and_moves_the_reminder() {
    use hq_core::config::RemindersConfig;
    use hq_core::types::ValueKind;
    use hq_db::reminders::{self, Limits, NewReminder};
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
    let now = chrono::Utc::now().timestamp();
    let make = |title: &str| {
        reminders::create(
            &state.db,
            &NewReminder {
                title,
                note: None,
                due_at: now - 10,
                tz: "Africa/Kampala",
                repeat_rule: None,
                nudge: true,
                task_id: None,
                thread_id: None,
                tainted: false,
                source: "chat",
            },
            now - 60,
            Limits {
                max_active: 10,
                max_per_hour: 10,
            },
        )
        .unwrap()
    };
    let done_me = make("Call Moses");
    let snooze_me = make("Send the invoice");
    hq_tools::reminders::fire::fire_due(&state.db, &RemindersConfig::default(), now).unwrap();
    let ping_of = |id: &str| {
        format!(
            "val_{}",
            reminders::get(&state.db, id)
                .unwrap()
                .unwrap()
                .last_item_id
                .unwrap()
        )
    };
    let listed = scan_value_items(&state.db, Some("pending")).unwrap();
    let item = listed
        .iter()
        .find(|n| n.id == ping_of(&done_me.id))
        .unwrap();
    assert_eq!(item.actions, ["done", "snooze", "dismiss"]);

    let act = |id: String, action: &str, minutes: Option<i64>| {
        let body = NotificationActionRequest {
            action: action.into(),
            starter: None,
            minutes,
            until: None,
        };
        notification_action_handler(State(state.clone()), AxumPath(id), Json(body))
    };
    assert_eq!(act(ping_of(&done_me.id), "done", None).await.status(), 200);
    assert_eq!(
        reminders::get(&state.db, &done_me.id)
            .unwrap()
            .unwrap()
            .status,
        reminders::STATUS_DONE
    );
    assert_eq!(
        act(ping_of(&snooze_me.id), "snooze", Some(0))
            .await
            .status(),
        400,
        "a bad snooze is refused before the ping is answered"
    );
    assert_eq!(
        act(ping_of(&snooze_me.id), "snooze", Some(30))
            .await
            .status(),
        200
    );
    let moved = reminders::get(&state.db, &snooze_me.id).unwrap().unwrap();
    assert!(moved.due_at >= now + 29 * 60 && moved.due_at <= now + 31 * 60 + 5);
    assert_eq!(moved.status, reminders::STATUS_ACTIVE);

    let other = insert(&state, "agent_notify", ValueKind::Fyi);
    assert_eq!(act(other, "done", None).await.status(), 400);
}
