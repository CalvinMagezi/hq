use super::*;
use hq_core::config::{FleetInboxConfig, HelperPolicy};

/// The quiet-fleet rules (`inbox.fleet`) as the sweep applies them. See `docs/plans/quiet-fleet.md`.
#[derive(Clone, Copy, Debug)]
pub(super) struct FleetQuiet {
    enabled: bool,
    digest: bool,
    grace: Duration,
    policy: HelperPolicy,
}

impl FleetQuiet {
    pub(super) fn from_config(c: &FleetInboxConfig) -> Self {
        Self {
            enabled: c.enabled,
            digest: c.enabled && c.digest,
            grace: c.grace(),
            policy: c.helper_policy,
        }
    }

    /// Today's behaviour: every prompt and every finish becomes an item.
    #[cfg(test)]
    pub(super) fn off() -> Self {
        Self {
            enabled: false,
            digest: false,
            grace: Duration::ZERO,
            policy: HelperPolicy::Everything,
        }
    }

    /// Whether a session's finishes and quiet exits fold into the day's digest.
    pub(super) fn digest_on(&self) -> bool {
        self.digest
    }

    /// Someone other than the owner answers for this session: a command policy, a driving or
    /// watching chat, the session it was started for, or the task it works on.
    fn owns(db: &Database, row: &registry::HarnessSessionRow) -> bool {
        row.owner_thread.is_some()
            || row.parent_session_id.is_some()
            || row.mission_id.is_some()
            || hq_tools::harness_session::policy::of(&Arc::new(db.clone()), &row.id).is_some()
    }

    /// How a block of `row` is treated. `driver_grace` is the old wait for a driving chat.
    pub(super) fn block_rule(
        &self,
        db: &Database,
        row: &registry::HarnessSessionRow,
        driver_grace: Duration,
    ) -> BlockRule {
        let owned = Self::owns(db, row);
        let quiet = self.enabled
            && owned
            && self.policy != HelperPolicy::Everything
            && !self.grace.is_zero();
        BlockRule {
            grace: if quiet { self.grace } else { driver_grace },
            quiet,
            owned,
            policy: if self.enabled {
                self.policy
            } else {
                HelperPolicy::Everything
            },
        }
    }
}

/// What the quiet rules say about one session's blocks.
#[derive(Clone, Copy, Debug)]
pub(super) struct BlockRule {
    /// How long a block may stand before the owner is told.
    pub(super) grace: Duration,
    quiet: bool,
    owned: bool,
    policy: HelperPolicy,
}

impl BlockRule {
    /// Nobody tells the owner yet: the fleet answers first and `raise_unanswered` speaks after
    /// `grace`. A policy escalation always speaks at once, and so does a prompt that loses
    /// work when the owner asked for those.
    fn waits(&self, escalated: bool, loses: bool) -> bool {
        self.quiet && !escalated && !(self.policy == HelperPolicy::LosesWork && loses)
    }

    /// The owner is told at once, even when a chat drives the session: a policy escalation, or
    /// a prompt that loses work when the owner asked to hear about those.
    fn asks_now(&self, escalated: bool, loses: bool) -> bool {
        escalated || (self.policy == HelperPolicy::LosesWork && loses)
    }

    /// Only the owner can decide it, so it counts on the badge.
    fn owner_only(&self, escalated: bool, loses: bool) -> bool {
        !self.owned || escalated || loses || self.policy == HelperPolicy::Everything
    }
}

fn loses_work(agent: &AgentInfo) -> bool {
    agent
        .pending
        .as_deref()
        .is_some_and(|d| d.losing_verb().is_some())
}

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
    row: &registry::HarnessSessionRow,
    agent: &AgentInfo,
    screen: &str,
    head: Option<&GitHead>,
    fleet: &FleetQuiet,
) {
    let (id, seq) = (row.id.clone(), agent.state_change_seq);
    match db.with_conn(move |c| registry::claim_state_alert(c, &id, seq)) {
        Ok(true) => {}
        _ => return,
    }
    let link = record_on_task(db, row, Event::Finished);
    record_event(db, &row.id, "finished", serde_json::json!({ "seq": seq }));
    hq_tools::harness_session::policy::post_summary(&Arc::new(db.clone()), row);
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
    report_to_observers(db, row, "finished", &finished_turn, &excerpt);
    if hand_to_chat(db, row, WAKE_FINISHED) {
        return;
    }
    if fleet.digest_on() {
        // One rolling item for the day instead of a row per helper; its counts come from the
        // event log written above.
        refresh_digest(db);
        tracing::info!(session = %row.id, host = %row.host, "session-supervisor: agent finished, counted in the digest");
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
    // One open item per session: a session that finishes turn after turn while the owner is
    // away says once that it waits, not once a turn.
    let item = ValueItem::new(
        "session-supervisor",
        ValueKind::Fyi,
        format!("Harness session '{label}' finished its task"),
        body,
    )
    .with_dedup_key(format!("session-finished-{}", row.id));
    let _ = hq_db::value_items::emit(db, &item);
    tracing::info!(session = %row.id, host = %row.host, "session-supervisor: agent finished, alert sent");
}

/// Rewrites today's fleet digest. Logged, never raised.
pub(super) fn refresh_digest(db: &Database) {
    if let Err(e) = hq_db::fleet_digest::refresh(db, chrono::Local::now()) {
        tracing::warn!(error = %e, "session-supervisor: fleet digest not updated");
    }
}

/// Tells the monitors and reviewers watching this session what happened. A
/// failure is logged: the event is already recorded and alerted on.
pub(super) fn report_to_observers(
    db: &Database,
    row: &registry::HarnessSessionRow,
    kind: &str,
    event: &str,
    excerpt: &str,
) {
    match hq_tools::a2a::report_to_observers(db, row, kind, event, excerpt) {
        Ok(told) if !told.is_empty() => {
            tracing::info!(session = %row.id, observers = ?told, "session-supervisor: told the observers");
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(session = %row.id, error = %e, "session-supervisor: could not tell the observers")
        }
    }
}

pub(super) fn alert_blocked(
    db: &Database,
    row: &registry::HarnessSessionRow,
    agent: &AgentInfo,
    screen: &str,
    resolve: &HostResolver,
    rule: BlockRule,
) {
    let (id, seq) = (row.id.clone(), agent.state_change_seq);
    match db.with_conn(move |c| registry::claim_state_alert(c, &id, seq)) {
        Ok(true) => {}
        _ => {
            raise_unanswered(db, row, agent, screen, rule);
            return;
        }
    }
    record_block(db, row, agent);
    let note = match by_policy(db, row, agent, screen, resolve, rule) {
        PolicyStep::Answering => return,
        PolicyStep::Escalated(note) => Some(note),
        PolicyStep::NotJudged => None,
    };
    raise_blocked(db, row, agent, screen, note.as_deref(), rule);
}

/// The key every alert about one block of a session carries: one per `state_change_seq`.
fn blocked_key(session_id: &str, seq: u64) -> String {
    format!("{}{seq}", blocked_prefix(session_id))
}

fn blocked_prefix(session_id: &str) -> String {
    format!("session-blocked-{session_id}-")
}

/// A block that whoever drives the session did not clear in `grace` reaches the owner after
/// all. It covers a driving chat that could not answer, one that closed after the block was
/// handed to it, and a command policy whose answer never landed. Nothing is said twice: a
/// block that already has an inbox item, in any state, is left alone.
fn raise_unanswered(
    db: &Database,
    row: &registry::HarnessSessionRow,
    agent: &AgentInfo,
    screen: &str,
    rule: BlockRule,
) {
    let grace = rule.grace;
    if grace.is_zero() {
        return;
    }
    let seq = agent.state_change_seq;
    if !matches!(
        hq_db::value_items::any_with_dedup_from(
            db,
            &blocked_key(&row.id, seq),
            &[SUPERVISOR_SOURCE, WEB_INBOX_SOURCE]
        ),
        Ok(false)
    ) {
        return;
    }
    let id = row.id.clone();
    let waited = db
        .with_conn(move |c| hq_db::session_events::blocked_for_ms(c, &id))
        .ok()
        .flatten()
        .unwrap_or(0);
    if u128::try_from(waited).unwrap_or(0) < grace.as_millis() {
        return;
    }
    tracing::info!(session = %row.id, "session-supervisor: block not cleared by its driver, telling the owner");
    // The escalation is in the event log, so a re-raise keeps what the first raise knew.
    let owner_only = rule.owner_only(last_block_escalated(db, &row.id), loses_work(agent));
    if row.owner_thread.is_some() {
        badge_web_inbox(db, row, seq, owner_only);
    } else {
        let link = record_on_task(db, row, Event::Blocked);
        raise_to_owner(db, row, agent, screen, None, link.as_ref(), owner_only);
    }
}

/// Whether the command policy left the session's latest block to the owner: its newest
/// block-related event is the `escalation` that `record_escalation` writes after `record_block`.
fn last_block_escalated(db: &Database, session_id: &str) -> bool {
    let id = session_id.to_string();
    db.with_conn(move |c| registry::list_events(c, &id, 200))
        .ok()
        .and_then(|events| {
            events
                .into_iter()
                .rev()
                .find(|e| matches!(e.kind.as_str(), "blocked" | "question" | "escalation"))
        })
        .is_some_and(|e| e.kind == "escalation")
}

/// Source of the supervisor's own inbox items.
const SUPERVISOR_SOURCE: &str = "session-supervisor";

/// A session that is no longer blocked asks nothing: its waiting "needs an answer" items leave
/// the inbox, however the block was cleared (keys from a chat, the agent's own page, a timeout).
pub(super) fn clear_blocked_alerts(db: &Database, session_id: &str) {
    match hq_db::value_items::retire_waiting_numbered(db, &blocked_prefix(session_id)) {
        Ok(0) => {}
        Ok(n) => {
            tracing::info!(session = %session_id, retired = n, "session-supervisor: block cleared, its alerts left the inbox")
        }
        Err(e) => {
            tracing::warn!(session = %session_id, error = %e, "session-supervisor: could not clear the alerts of a cleared block")
        }
    }
}

/// What the session's command policy did with the dialog it blocked at.
enum PolicyStep {
    /// It allows or denies it, and is answering it in the background.
    Answering,
    /// It left it to the owner; the line says why.
    Escalated(String),
    /// No policy, no dialog HQ can read, or a kind a policy does not judge.
    NotJudged,
}

/// A dialog the policy decides is answered off the sweep (an answer waits for the screen),
/// and raised after all if the answer does not take.
fn by_policy(
    db: &Database,
    row: &registry::HarnessSessionRow,
    agent: &AgentInfo,
    screen: &str,
    resolve: &HostResolver,
    rule: BlockRule,
) -> PolicyStep {
    use hq_core::command_policy::Decision;
    use hq_tools::harness_session::policy;
    let db_arc = Arc::new(db.clone());
    let (Some(dialog), Some(seq)) = (agent.pending.as_deref(), agent.pending_seq) else {
        return PolicyStep::NotJudged;
    };
    let Some(rules) = policy::of(&db_arc, &row.id) else {
        return PolicyStep::NotJudged;
    };
    let Some(decision) = policy::judge(&rules, dialog, &row.cwd) else {
        return PolicyStep::NotJudged;
    };
    let host = match (resolve.as_ref())(&row.host) {
        Ok(host) => host,
        Err(_) => return PolicyStep::NotJudged,
    };
    if let Decision::Escalate { .. } = decision {
        let acted = policy::act(&db_arc, &host, row, &rules, dialog, seq);
        let reason = match acted {
            Some(policy::Acted::Escalated { reason }) => reason,
            _ => decision.why().to_string(),
        };
        record_escalation(db, &row.id, &rules.name, dialog, &reason);
        return PolicyStep::Escalated(format!(
            "Its '{}' command policy left this to you: {reason}.",
            rules.name
        ));
    }
    let (db2, row2, agent2, screen2, dialog2) = (
        db_arc.clone(),
        row.clone(),
        agent.clone(),
        screen.to_string(),
        dialog.clone(),
    );
    tokio::task::spawn_blocking(move || {
        match policy::act(&db2, &host, &row2, &rules, &dialog2, seq) {
            Some(policy::Acted::Escalated { reason }) => {
                record_escalation(&db2, &row2.id, &rules.name, &dialog2, &reason);
                let note = format!(
                    "Its '{}' command policy could not settle this: {reason}.",
                    rules.name
                );
                raise_blocked(&db2, &row2, &agent2, &screen2, Some(&note), rule);
            }
            Some(acted) => {
                tracing::info!(session = %row2.id, ?acted, "session-supervisor: dialog answered by the command policy");
            }
            None => raise_blocked(&db2, &row2, &agent2, &screen2, None, rule),
        }
    });
    PolicyStep::Answering
}

/// The "waiting for you" alert for a blocked session, with `note` (what the command policy
/// made of it) when there is one.
fn raise_blocked(
    db: &Database,
    row: &registry::HarnessSessionRow,
    agent: &AgentInfo,
    screen: &str,
    note: Option<&str>,
    rule: BlockRule,
) {
    let seq = agent.state_change_seq;
    let (escalated, loses) = (note.is_some(), loses_work(agent));
    let owner_only = rule.owner_only(escalated, loses);
    let link = record_on_task(db, row, Event::Blocked);
    // A monitor hears about the block too; it can tell whoever can answer it.
    report_to_observers(
        db,
        row,
        "blocked",
        "is blocked at a dialog",
        &tail_excerpt(screen, MAX_TAIL_BYTES),
    );
    if hand_to_chat(db, row, WAKE_BLOCKED) {
        // A chat that drives the session answers the prompt itself, and asks the owner in
        // their inbox when the answer is theirs to give. The owner hears about the block only
        // if it is still there after `grace` (`raise_unanswered`). A chat that only watches
        // cannot answer, so its owner is told at once.
        if rule.asks_now(escalated, loses) {
            // Only the owner can allow it: the chat does not get to sit on it for the grace.
            raise_to_owner(db, row, agent, screen, note, link.as_ref(), true);
        } else if !row.drive || rule.grace.is_zero() {
            badge_web_inbox(db, row, seq, owner_only);
        }
        return;
    }
    if rule.waits(escalated, loses) {
        // The fleet answers first. `raise_unanswered` makes the one item if it does not.
        return;
    }
    raise_to_owner(db, row, agent, screen, note, link.as_ref(), owner_only);
}

/// The inbox item for a block the owner has to answer.
fn raise_to_owner(
    db: &Database,
    row: &registry::HarnessSessionRow,
    agent: &AgentInfo,
    screen: &str,
    note: Option<&str>,
    link: Option<&mission::TaskLink>,
    owner_only: bool,
) {
    let seq = agent.state_change_seq;
    let label = session_label(row);
    let excerpt = last_lines(screen, BLOCKED_EXCERPT_LINES);
    let task_line = task_line(link);
    let note_line = note.map_or_else(String::new, |n| format!("\n{n}"));
    let item = ValueItem::new(
        "session-supervisor",
        ValueKind::ActionNeeded,
        format!("Harness session '{label}' is waiting for you"),
        format!(
            "Session {} on {} is blocked on a prompt or approval:\n{OUTPUT_OPEN}\n{excerpt}\n{OUTPUT_CLOSE}{note_line}\nRead it with `harness_session_pending` and answer with `harness_session_answer`, or with `harness_session_send` keys when HQ cannot read it.{task_line}",
            row.id, row.host
        ),
    )
    .with_dedup_key(blocked_key(&row.id, seq))
    .with_link(session_page(&row.id))
    .owner_only(owner_only);
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
pub(super) fn badge_web_inbox(
    db: &Database,
    row: &registry::HarnessSessionRow,
    seq: u64,
    owner_only: bool,
) {
    let item = ValueItem::new(
        WEB_INBOX_SOURCE,
        ValueKind::ActionNeeded,
        format!("Harness session '{}' needs an answer", session_label(row)),
        format!(
            "Session {} on {} is waiting at a prompt. Its chat has the details.",
            row.id, row.host
        ),
    )
    .with_dedup_key(blocked_key(&row.id, seq))
    .with_link(session_page(&row.id))
    .owner_only(owner_only);
    let _ = hq_db::value_items::emit(db, &item);
}

/// The web app's page for one session, where every option of its dialog is a button.
pub(super) fn session_page(session_id: &str) -> String {
    format!("/helpers/{session_id}")
}

pub(super) const WEB_INBOX_SOURCE: &str = "session_chat";

/// The line that tells the operator where the linked task now stands.
pub(super) fn task_line(link: Option<&mission::TaskLink>) -> String {
    link.map_or_else(String::new, |l| {
        format!("\nTask {} is {}.", l.display_id, l.status.replace('_', " "))
    })
}

/// An exit that just left the linked task blocked means work stopped with no
/// one on it: the owner has to act. Every other exit is a status update.
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

/// Longest question or command text an event carries; `harness_session_pending` has the rest.
const EVENT_TEXT: usize = 300;

fn capped(text: Option<&str>) -> Option<String> {
    text.map(|t| {
        if t.len() <= EVENT_TEXT {
            t.to_string()
        } else {
            format!("{}…", &t[..t.floor_char_boundary(EVENT_TEXT)])
        }
    })
}

/// Appends one event to the session's log (`harness_session_events`), which
/// `harness_session_events` reads as a stream. Logged, never raised.
pub(super) fn record_event(db: &Database, session_id: &str, kind: &str, detail: serde_json::Value) {
    let (id, kind_owned, detail) = (session_id.to_string(), kind.to_string(), detail.to_string());
    if let Err(e) = db.with_conn(move |c| {
        registry::record_event(c, &id, &kind_owned, "supervisor", Some(&detail))
    }) {
        tracing::warn!(session = %session_id, kind, error = %e, "session-supervisor: event not recorded");
    }
}

/// A session blocked: `blocked`, or `question` when the agent asks the owner something, with
/// the dialog when HQ can read it. Its text is the agent's output, capped.
fn record_block(db: &Database, row: &registry::HarnessSessionRow, agent: &AgentInfo) {
    let dialog = agent.pending.as_deref();
    let kind = match dialog {
        Some(d) if d.kind == "question" => "question",
        _ => "blocked",
    };
    let detail = match (dialog, agent.pending_seq) {
        (Some(d), Some(seq)) => serde_json::json!({
            "seq": seq,
            "dialog": {
                "kind": d.kind,
                "question": capped(d.question.as_deref()),
                "command": capped(d.command.as_deref()),
                "options": d.options.iter().map(|o| serde_json::json!({
                    "id": o.id,
                    "label": capped(Some(&o.label)),
                    "remember": o.remember,
                })).collect::<Vec<_>>(),
                "truncated": d.truncated,
            },
        }),
        _ => serde_json::json!({ "dialog": null }),
    };
    record_event(db, &row.id, kind, detail);
}

/// The command policy left a dialog to the owner.
fn record_escalation(
    db: &Database,
    session_id: &str,
    policy: &str,
    dialog: &hq_tools::agent_host::PendingDialog,
    reason: &str,
) {
    record_event(
        db,
        session_id,
        "escalation",
        serde_json::json!({
            "policy": policy,
            "kind": dialog.kind,
            "command": capped(dialog.command.as_deref()),
            "reason": reason,
        }),
    );
}
