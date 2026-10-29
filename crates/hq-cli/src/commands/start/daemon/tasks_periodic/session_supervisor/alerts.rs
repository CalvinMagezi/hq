use super::*;

/// Fixed text, never screen text: the harness keeps showing a prompt HQ keeps dismissing.
pub(super) fn notify_dismiss_cap(db: &Database, row: &registry::HarnessSessionRow, why: &str) {
    let item = ValueItem::new(
        "session-supervisor",
        ValueKind::ActionNeeded,
        format!("Harness session '{}' keeps showing a prompt", session_label(row)),
        format!(
            "Session {} on {} {why}. HQ stopped dismissing it automatically (at most {} keys per session). Read it with `harness_session_logs` and clear it by hand.",
            row.id,
            row.host,
            registry::DISMISSAL_CAP
        ),
    )
    .with_dedup_key(format!("session-dismiss-cap-{}", row.id));
    let _ = hq_db::value_items::emit(db, &item);
}

/// An agent that finished its turn and sits waiting for the next instruction
/// never exits, so without this the operator would only hear about it when the
/// session is eventually stopped. The host reports `done` for a finished turn no
/// one has looked at yet.
pub(super) fn alert_finished(
    db: &Database,
    vault_path: &Path,
    row: &registry::HarnessSessionRow,
    agent: &AgentInfo,
    screen: &str,
    head: Option<&GitHead>,
) {
    let (id, seq) = (row.id.clone(), agent.state_change_seq);
    match db.with_conn(move |c| registry::claim_state_alert(c, &id, seq)) {
        Ok(true) => {}
        _ => return,
    }
    let link = record_on_task(db, row, Event::Finished);
    let excerpt = tail_excerpt(screen, MAX_TAIL_BYTES);
    // A commit made in the turn is named, so a reviewer checks out exactly that.
    let db_arc = Arc::new(db.clone());
    let commit = head
        .filter(|h| checkout::note_commit(&db_arc, &row.id, h))
        .map(checkout::commit_line);
    let (finished, finished_turn) = match &commit {
        Some(line) => (
            format!("finished and {line}"),
            format!("finished a turn and {line}"),
        ),
        None => ("finished".to_string(), "finished a turn".to_string()),
    };
    // A delegated session tells whoever delegated it, whatever else happens here.
    if let Err(e) = hq_tools::a2a::report_to_parent(db, row, &finished, &excerpt) {
        tracing::warn!(session = %row.id, error = %e, "session-supervisor: could not report to the parent");
    }
    report_to_observers(db, row, &finished_turn, &excerpt);
    if hand_to_chat(db, row, WAKE_FINISHED) {
        return;
    }
    let label = session_label(row);
    let task_line = task_line(link.as_ref());
    let commit_line = commit
        .as_deref()
        .map_or_else(String::new, |line| format!(" It {line}."));
    let body = format!(
        "Session {} on {} finished its task and is waiting for the next instruction.{commit_line}\n{OUTPUT_OPEN}\n{excerpt}\n{OUTPUT_CLOSE}\nGive it more work with `harness_session_send`, or end it with `harness_session_stop`.{task_line}",
        row.id, row.host,
    );
    post_relay_nudge(
        vault_path,
        &row.id,
        &format!("Harness session '{label}' finished its task"),
        &body,
        false,
    );
    tracing::info!(session = %row.id, host = %row.host, "session-supervisor: agent finished, alert sent");
}

/// Tells the monitors and reviewers watching this session what happened. A
/// failure is logged: the event is already recorded and alerted on.
pub(super) fn report_to_observers(
    db: &Database,
    row: &registry::HarnessSessionRow,
    event: &str,
    excerpt: &str,
) {
    match hq_tools::a2a::report_to_observers(db, row, event, excerpt) {
        Ok(told) if !told.is_empty() => {
            tracing::info!(session = %row.id, observers = ?told, "session-supervisor: told the observers");
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(session = %row.id, error = %e, "session-supervisor: could not tell the observers")
        }
    }
}

/// Untagged, so the notification gate sends it to the digest: a finished
/// session is a status update (FR-001 criterion 3). `interrupt` is for the few
/// that need the operator now; a session blocked on a dialog alerts through
/// `alert_blocked` instead.
pub(super) fn post_relay_nudge(
    vault_path: &Path,
    session_id: &str,
    subject: &str,
    body: &str,
    interrupt: bool,
) {
    let mut msg = hq_core::mailbox::new_message(
        "session-supervisor",
        "relay",
        MailboxMessageType::Nudge,
        Some(subject),
        body,
        None,
    );
    if interrupt {
        msg.meta.insert(
            hq_core::mailbox::META_INTERRUPT.to_string(),
            "true".to_string(),
        );
    }
    if let Err(e) = hq_core::mailbox::send_message(vault_path, &msg) {
        tracing::warn!(session = %session_id, error = %e, "session-supervisor: relay mailbox post failed");
    }
}

pub(super) fn alert_blocked(
    db: &Database,
    row: &registry::HarnessSessionRow,
    agent: &AgentInfo,
    screen: &str,
) {
    let (id, seq) = (row.id.clone(), agent.state_change_seq);
    match db.with_conn(move |c| registry::claim_state_alert(c, &id, seq)) {
        Ok(true) => {}
        _ => return,
    }
    let link = record_on_task(db, row, Event::Blocked);
    // A monitor hears about the block too; it can tell whoever can answer it.
    report_to_observers(
        db,
        row,
        "is blocked at a dialog",
        &tail_excerpt(screen, MAX_TAIL_BYTES),
    );
    if hand_to_chat(db, row, WAKE_BLOCKED) {
        badge_web_inbox(db, row, seq);
        return;
    }
    let label = session_label(row);
    let excerpt = last_lines(screen, BLOCKED_EXCERPT_LINES);
    let task_line = task_line(link.as_ref());
    let item = ValueItem::new(
        "session-supervisor",
        ValueKind::ActionNeeded,
        format!("Harness session '{label}' is waiting for you"),
        format!(
            "Session {} on {} is blocked on a prompt or approval:\n{OUTPUT_OPEN}\n{excerpt}\n{OUTPUT_CLOSE}\nRead it with `harness_session_pending` and answer with `harness_session_answer`, or with `harness_session_send` keys when HQ cannot read it.{task_line}",
            row.id, row.host
        ),
    )
    .with_dedup_key(format!("session-blocked-{}-{seq}", row.id));
    let fresh = item.dedup_key.as_deref().is_some_and(|key| {
        matches!(
            hq_db::value_items::find_active_by_dedup(db, item.kind, key),
            Ok(None)
        )
    });
    let _ = hq_db::value_items::emit(db, &item);
    if fresh {
        record_dialog(db, &item.id, &row.id, agent);
    }
    tracing::info!(session = %row.id, host = %row.host, "session-supervisor: agent blocked, alert sent");
}

/// Ties the alert to the dialog the host read, when it read one, so a notification button
/// answers that dialog and no other.
fn record_dialog(db: &Database, item_id: &str, session_id: &str, agent: &AgentInfo) {
    let (Some(dialog), Some(seq)) = (agent.pending.as_deref(), agent.pending_seq) else {
        return;
    };
    let options: Vec<hq_db::dialog_alerts::DialogOption> = dialog
        .options
        .iter()
        .map(|o| hq_db::dialog_alerts::DialogOption {
            id: o.id,
            label: o.label.clone(),
            remember: o.remember,
            typed: o.typed,
        })
        .collect();
    let recorded = hq_db::dialog_alerts::record(
        db,
        &hq_db::dialog_alerts::NewDialogAlert {
            item_id,
            session_id,
            pending_seq: seq,
            kind: &dialog.kind,
            question: dialog.question.as_deref(),
            command: dialog.command.as_deref(),
            options: &options,
        },
    );
    if let Err(e) = recorded {
        tracing::warn!(session = %session_id, error = %e, "session-supervisor: could not record the dialog behind an alert");
    }
}

/// Record `event` on the session's HQ task. Logged, never raised: one broken
/// task must not end the sweep for every other session.
pub(super) fn record_on_task(
    db: &Database,
    row: &registry::HarnessSessionRow,
    event: Event,
) -> Option<mission::TaskLink> {
    db.with_conn(|c| mission::record(c, row, event))
        .inspect_err(|e| tracing::warn!(session = %row.id, error = %e, "session-supervisor: task update failed"))
        .ok()
        .flatten()
}

pub(super) const WAKE_FINISHED: &str = "finished";
pub(super) const WAKE_BLOCKED: &str = "blocked";
pub(super) const WAKE_EXITED: &str = "exited";

/// For a session a web chat watches, leave the event as a durable wake for the
/// web driver and report true, so the caller sends nothing to the relay.
pub(super) fn hand_to_chat(db: &Database, row: &registry::HarnessSessionRow, reason: &str) -> bool {
    if row.owner_thread.is_none() {
        return false;
    }
    match db.with_conn(|c| registry::set_wake(c, &row.id, reason)) {
        Ok(woke) => woke,
        Err(e) => {
            tracing::warn!(session = %row.id, error = %e, "session-supervisor: chat wake failed, using the relay");
            false
        }
    }
}

/// A chat-watched session that blocks may sit unseen in a chat no one has
/// open, so it also gets a web-only inbox item, which badges the app.
pub(super) fn badge_web_inbox(db: &Database, row: &registry::HarnessSessionRow, seq: u64) {
    let item = ValueItem::new(
        WEB_INBOX_SOURCE,
        ValueKind::ActionNeeded,
        format!("Harness session '{}' needs an answer", session_label(row)),
        format!(
            "Session {} on {} is waiting at a prompt. Its chat has the details.",
            row.id, row.host
        ),
    )
    .with_dedup_key(format!("session-blocked-{}-{seq}", row.id));
    let _ = hq_db::value_items::emit(db, &item);
}

/// Must stay in `hq_daemon::value_bus::WEB_ONLY_SOURCES`, so it never reaches the relay.
pub(super) const WEB_INBOX_SOURCE: &str = "session_chat";

/// The line that tells the operator where the linked task now stands.
pub(super) fn task_line(link: Option<&mission::TaskLink>) -> String {
    link.map_or_else(String::new, |l| {
        format!("\nTask {} is {}.", l.display_id, l.status.replace('_', " "))
    })
}

/// An exit that just left the linked task blocked means work stopped with no
/// one on it: that interrupts. Every other exit is a status update.
pub(super) fn exit_interrupts(link: Option<&mission::TaskLink>) -> bool {
    link.is_some_and(|l| l.moved && l.status == hq_db::tasks::STATUS_BLOCKED)
}

pub(super) fn session_label(row: &registry::HarnessSessionRow) -> String {
    if row.label.is_empty() {
        row.harness.clone()
    } else {
        format!("{} ({})", row.label, row.harness)
    }
}

/// The line an exit message ends with when the session had a worktree.
pub(super) fn worktree_line(released: Option<&serde_json::Value>) -> String {
    let Some(v) = released else {
        return String::new();
    };
    match (
        v.get("removed").and_then(|p| p.as_str()),
        v.get("kept").and_then(|p| p.as_str()),
    ) {
        (Some(path), _) => {
            format!("\nIts worktree {path} was removed; its branch keeps every commit.")
        }
        (None, Some(path)) => format!(
            "\nIts worktree {path} was kept: it has changes nobody committed. `harness_session_resume` continues there."
        ),
        _ => String::new(),
    }
}
