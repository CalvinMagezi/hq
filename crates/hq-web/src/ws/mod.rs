//! WebSocket protocol logic: connection management and agent chat.

use crate::WsState;
use crate::chat_uploads::{self, ChatAttachment, ResolvedAttachment};
use axum::extract::{
    State,
    ws::{Message, WebSocket, WebSocketUpgrade},
};
use axum::response::IntoResponse;
use futures::{SinkExt, StreamExt};
use hq_core::types::{ChatMessage as AgentMsg, MessageRole, SessionEvent};
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::info;

mod ask;
mod record;
use record::{SharedRecord, TurnRecord};

pub(crate) use ask::{WebAskRunner, reconcile_asks_after_restart};

/// The reply running in a chat: how to stop it, and what it has done so far.
pub(crate) struct ChatTurnSlot {
    abort: tokio::task::AbortHandle,
    record: SharedRecord,
    /// The `hq_ask` this reply answers, so a stop can end it too.
    ask_id: Option<String>,
}

pub(crate) type ChatTurnMap = Arc<tokio::sync::RwLock<std::collections::HashMap<String, ChatTurnSlot>>>;

const BUSY_REASON: &str = "A reply is still running in this chat. Stop it or wait for it to finish.";
const GONE_REASON: &str = "That message is no longer in this chat, so it could not be edited.";

/// What a browser may send over the socket. Unknown types are logged and ignored.
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    Chat {
        #[serde(default)]
        content: String,
        thread_id: Option<String>,
        /// Files uploaded through `/api/chat/uploads` for this message.
        #[serde(default)]
        attachments: Vec<ChatAttachment>,
        /// The sender's id for its local copy, echoed on turn_start so it can swap in the saved one.
        client_id: Option<String>,
        /// Edit or regenerate: this message and everything after it are replaced.
        replace_from: Option<String>,
    },
    Stop {
        thread_id: Option<String>,
    },
    #[serde(other)]
    Other,
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<WsState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(socket: WebSocket, state: Arc<WsState>) {
    let mut rx = state.tx.subscribe();
    let (mut sender, mut receiver) = socket.split();

    let mut send_task = tokio::spawn(async move {
        while let Some(frame) = next_frame(&mut rx).await {
            if sender.send(Message::Text(frame.into())).await.is_err() {
                break;
            }
        }
    });

    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(text) => dispatch(&state, &text).await,
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    // Whichever half ends first takes the other with it, so a gone client leaves no subscriber behind.
    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }
}

/// The next frame for one client. A client that fell behind the broadcast is
/// told how much it missed (its reply may be incomplete, and a `turn_end` may
/// be among the lost events) instead of the gap going unnoticed. `None` ends the socket.
async fn next_frame(rx: &mut tokio::sync::broadcast::Receiver<String>) -> Option<String> {
    use tokio::sync::broadcast::error::RecvError;
    match rx.recv().await {
        Ok(frame) => Some(frame),
        Err(RecvError::Lagged(skipped)) => {
            tracing::warn!(skipped, "ws client lagged behind broadcast");
            Some(json!({"type": "stream_lag", "skipped": skipped}).to_string())
        }
        Err(RecvError::Closed) => None,
    }
}

async fn dispatch(state: &Arc<WsState>, text: &str) {
    let Ok(msg) = serde_json::from_str::<ClientMsg>(text) else {
        info!(msg = %text, "ws client message");
        return;
    };
    match msg {
        ClientMsg::Chat { content, thread_id, attachments, client_id, replace_from } => {
            let text = chat_uploads::strip_marker(&content);
            let files = chat_uploads::resolve(&state.vault_path, attachments).await;
            let trimmed = text.trim();
            if (!trimmed.is_empty() || !files.is_empty()) && trimmed != "/reset" {
                handle_chat(state, ChatRequest { text, thread_id, files, client_id, replace_from }).await;
            }
        }
        ClientMsg::Stop { thread_id: Some(tid) } if !tid.is_empty() => stop_chat_turn(state, &tid).await,
        ClientMsg::Stop { .. } => info!("ws chat stop requested with no thread_id"),
        ClientMsg::Other => info!(msg = %text, "ws client message"),
    }
}

/// Abort the in-flight turn for a thread (if any), keep what it produced so
/// far, and end it for every tab. Aborting the driver task drops the harness stream.
async fn stop_chat_turn(state: &Arc<WsState>, thread_id: &str) {
    // Settled before the slot goes, so a waiting hq_ask never sees an idle thread and a reason of its own.
    let ask_id = state.active_chat_turns.read().await.get(thread_id).and_then(|s| s.ask_id.clone());
    if let Some(ask_id) = &ask_id {
        ask::settle_failed(&state.db, ask_id, ask::STOPPED_REASON);
    }
    let slot = state.active_chat_turns.write().await.remove(thread_id);
    let mut message_id = None;
    if let Some(slot) = slot {
        slot.abort.abort();
        message_id = save_reply(&state.db, thread_id, &slot.record, None, true);
        info!(thread_id, "chat turn stopped by user");
    }
    state.broadcast(
        &json!({"type": "turn_end", "thread_id": thread_id, "stopped": true, "message_id": message_id}).to_string(),
    );
}

/// Writes a reply once, whichever of finish or Stop calls first. Returns its id.
fn save_reply(
    db: &hq_db::Database,
    thread_id: &str,
    record: &SharedRecord,
    final_text: Option<&str>,
    stopped: bool,
) -> Option<String> {
    save_reply_message(db, thread_id, record, final_text, stopped).map(|m| m.message_id)
}

/// `save_reply`, keeping the saved message so an `hq_ask` can answer with its text.
fn save_reply_message(
    db: &hq_db::Database,
    thread_id: &str,
    record: &SharedRecord,
    final_text: Option<&str>,
    stopped: bool,
) -> Option<hq_db::chat::ChatMessage> {
    let reply = record.lock().unwrap_or_else(|e| e.into_inner()).take(final_text, stopped)?;
    db.with_conn(|conn| hq_db::chat::add_message_with_meta(conn, thread_id, "assistant", &reply.content, reply.meta.as_ref()))
        .inspect_err(|e| tracing::warn!(thread_id, "could not save chat reply: {e}"))
        .ok()
}

/// One chat message as the browser sent it, after its files were resolved.
struct ChatRequest {
    text: String,
    thread_id: Option<String>,
    files: Vec<ResolvedAttachment>,
    client_id: Option<String>,
    replace_from: Option<String>,
}

/// Tells every tab a send was refused; only the sender has the local copy to drop.
fn reject(state: &WsState, thread_id: &str, client_id: Option<&str>, reason: &str, running: bool) {
    state.broadcast(
        &json!({"type": "chat_rejected", "thread_id": thread_id, "client_id": client_id, "reason": reason, "running": running})
            .to_string(),
    );
}

/// Starts a reply. The busy check, an edit's delete, the save and the
/// registration of the new turn all happen under one lock, so two tabs
/// sending at once cannot both start a reply in the same chat.
async fn handle_chat(state: &Arc<WsState>, req: ChatRequest) {
    let mut turns = state.active_chat_turns.write().await;
    if let Some(t) = &req.thread_id {
        if turns.contains_key(t) {
            reject(state, t, req.client_id.as_deref(), BUSY_REASON, true);
            return;
        }
        if let Some(from) = &req.replace_from {
            if ask::replaces_mcp_question(&state.db, t, from) {
                reject(state, t, req.client_id.as_deref(), ask::MCP_EDIT_REASON, false);
                return;
            }
            let deleted = state.db.with_conn(|conn| hq_db::chat::delete_messages_from(conn, t, from)).unwrap_or(0);
            if deleted == 0 {
                reject(state, t, req.client_id.as_deref(), GONE_REASON, false);
                return;
            }
        }
    }
    let (tid, history) = open_thread(state, &req);
    let Some(config) = load_config(state, &tid) else { return };
    let record: SharedRecord = Arc::new(std::sync::Mutex::new(TurnRecord::default()));
    let turn = ChatTurn {
        db: state.db.clone(),
        tx: state.tx.clone(),
        turns: state.active_chat_turns.clone(),
        tid: tid.clone(),
        record: record.clone(),
        ask: None,
    };
    let abort = spawn_supervised(turn, config, req.text, history, req.files);
    if let Some(t) = tid {
        turns.insert(t, ChatTurnSlot { abort, record, ask_id: None });
    }
}

/// Creates the thread when needed, saves the user message (with its
/// attachment marker), acks the turn with that saved message, and names a new
/// chat. Returns the thread id and the history the model sees.
fn open_thread(state: &WsState, req: &ChatRequest) -> (Option<String>, Vec<AgentMsg>) {
    let (message, files) = (req.text.as_str(), req.files.as_slice());
    let db = &state.db;
    let tid = req.thread_id.clone().or_else(|| {
        db.with_conn(|conn| hq_db::chat::create_thread(conn, "New Chat", "user", "user").map(|t| t.thread_id))
            .ok()
    });
    let history = tid.as_deref().map(|t| thread_history(db, t)).unwrap_or_default();

    let Some(t) = tid.as_deref() else {
        state.broadcast(&json!({"type": "turn_start", "thread_id": tid}).to_string());
        return (tid, history);
    };
    // The stored copy drops the ephemeral <hq-context> block; the model still gets it.
    let user_clean = hq_agent::threads::strip_hq_context(message);
    let metas: Vec<ChatAttachment> = files.iter().map(|f| f.meta.clone()).collect();
    let stored = format!("{user_clean}{}", chat_uploads::marker(&metas));
    let saved = db.with_conn(|conn| hq_db::chat::add_message(conn, t, "user", stored.trim_start())).ok();
    // Ack before the first token. Carrying the saved message lets every other
    // tab show the question, and the sender swap its local copy for this one;
    // `replace_from` tells the other tabs which messages an edit replaced.
    state.broadcast(
        &json!({
            "type": "turn_start",
            "thread_id": t,
            "user_message": saved,
            "client_id": req.client_id,
            "replace_from": req.replace_from,
        })
        .to_string(),
    );
    // Name a new chat after its opening message so parallel chats are tellable apart.
    let mut title = thread_title(&user_clean);
    if title.is_empty() {
        title = metas.first().map(|a| thread_title(&a.name)).unwrap_or_default();
    }
    if history.is_empty()
        && !title.is_empty()
        && db.with_conn(|conn| hq_db::chat::set_thread_title(conn, t, &title)).is_ok()
    {
        state.broadcast(&json!({"type": "thread_title", "thread_id": t, "title": title}).to_string());
    }
    (tid, history)
}

/// The last messages of a thread, as the model sees them.
fn thread_history(db: &hq_db::Database, thread_id: &str) -> Vec<AgentMsg> {
    db.with_conn(|conn| hq_db::chat::get_messages_page(conn, thread_id, 20, None))
        .unwrap_or_default()
        .into_iter()
        .map(|stored| {
            let (m, meta) = (stored.message, stored.meta);
            let text = chat_uploads::history_text(&m.content);
            let content = match (m.role.as_str(), ask::mcp_caller(meta.as_ref())) {
                ("user", Some(caller)) => ask::untrusted_question(&caller, &text),
                _ => text,
            };
            (m, content)
        })
        .map(|(m, content)| AgentMsg {
            image_parts: Vec::new(),
            role: if m.role == "user" { MessageRole::User } else { MessageRole::Assistant },
            content,
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning_content: None,
        })
        .collect()
}

/// Starts a reply the user did not type: the session driver's turn in a chat
/// that drives a harness session. It streams and saves like any reply, the
/// prompt itself is not stored. `Busy` while the chat already has a reply
/// running (try again later), `Refused` for a chat that must never get one.
pub(crate) async fn start_driver_turn(state: &Arc<WsState>, thread_id: &str, prompt: String, driver: serde_json::Value) -> DriverStart {
    let mut turns = state.active_chat_turns.write().await;
    if turns.contains_key(thread_id) {
        return DriverStart::Busy;
    }
    // A thread a read-only ask owns never gets a full-power reply the owner did not type. When the
    // lookup itself fails the answer is also no: a retry loop on a broken database helps nobody.
    let read_only_ask = state
        .db
        .with_conn(|c| hq_db::ask_requests::thread_has_ask(c, thread_id, None, Some("read_only")))
        .unwrap_or_else(|e| {
            tracing::warn!(thread_id, "could not check whether a read-only ask owns the chat, refusing the driver turn: {e}");
            true
        });
    if read_only_ask {
        return DriverStart::Refused;
    }
    let tid = Some(thread_id.to_string());
    let Some(config) = load_config(state, &tid) else { return DriverStart::Busy };
    let history = thread_history(&state.db, thread_id);
    let record: SharedRecord = Arc::new(std::sync::Mutex::new(TurnRecord::with_driver(driver)));
    state.broadcast(&json!({"type": "turn_start", "thread_id": thread_id}).to_string());
    let turn = ChatTurn {
        db: state.db.clone(),
        tx: state.tx.clone(),
        turns: state.active_chat_turns.clone(),
        tid,
        record: record.clone(),
        ask: None,
    };
    let abort = spawn_supervised(turn, config, prompt, history, Vec::new());
    turns.insert(thread_id.to_string(), ChatTurnSlot { abort, record, ask_id: None });
    DriverStart::Started
}

/// Tool only a session-driver turn lacks: `harness_session_send` is the metered path for typing
/// into a pane. Every driver-started turn also loses `config_manage`.
const DRIVER_DENIED_TOOL: &str = "herdr_send";
const CONFIG_TOOL: &str = "config_manage";

/// What `start_driver_turn` did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DriverStart {
    Started,
    /// A reply is already running; the caller may try again.
    Busy,
    /// This chat never takes such a turn; the caller must not retry.
    Refused,
}

/// Posts a finished assistant message into a chat and tells every tab to
/// reload it. Returns the message id.
pub(crate) fn post_assistant_message(state: &WsState, thread_id: &str, content: &str, meta: &serde_json::Value) -> Option<String> {
    let saved = state
        .db
        .with_conn(|conn| hq_db::chat::add_message_with_meta(conn, thread_id, "assistant", content, Some(meta)))
        .inspect_err(|e| tracing::warn!(thread_id, "could not save session update: {e}"))
        .ok()?;
    state.broadcast(&json!({"type": "turn_end", "thread_id": thread_id, "message_id": saved.message_id}).to_string());
    Some(saved.message_id)
}

/// Reloads config from disk so edits apply without a restart. With none at all
/// the turn ends at once with a message.
fn load_config(state: &WsState, tid: &Option<String>) -> Option<Arc<hq_core::config::HqConfig>> {
    if let Ok(c) = hq_core::config::HqConfig::load() {
        return Some(Arc::new(c));
    }
    if let Some(c) = &state.hq_config {
        return Some(c.clone());
    }
    state.broadcast(&json!({"type": "text_delta", "thread_id": tid, "content": "Config unavailable"}).to_string());
    state.broadcast(&json!({"type": "turn_end", "thread_id": tid}).to_string());
    None
}

/// The spawned half of a chat turn: runs the agent and streams it to every tab.
#[derive(Clone)]
struct ChatTurn {
    db: hq_db::Database,
    tx: tokio::sync::broadcast::Sender<String>,
    turns: ChatTurnMap,
    tid: Option<String>,
    record: SharedRecord,
    /// Set when `hq_ask` started this reply: it ends the ask and limits the tools.
    ask: Option<ask::AskTurn>,
}

/// Runs a turn and watches it. A panic inside the run would otherwise leave no
/// `turn_end`, so every tab would wait forever and the chat would stay busy.
fn spawn_supervised(
    turn: ChatTurn,
    config: Arc<hq_core::config::HqConfig>,
    message: String,
    history: Vec<AgentMsg>,
    files: Vec<ResolvedAttachment>,
) -> tokio::task::AbortHandle {
    let watcher = turn.clone();
    let task = tokio::spawn(turn.drive(config, message, history, files));
    let abort = task.abort_handle();
    tokio::spawn(async move { watcher.supervise(task).await });
    abort
}

impl ChatTurn {
    /// Waits for the run; when it panicked, keeps what it produced, says so, and ends the turn.
    /// A cancelled run was stopped by the user, which already ended the turn.
    async fn supervise(&self, task: tokio::task::JoinHandle<()>) {
        let Err(err) = task.await else { return };
        if !err.is_panic() {
            return;
        }
        tracing::error!(thread_id = ?self.tid, "chat turn panicked: {err}");
        self.send(json!({"type": "error", "thread_id": self.tid, "content": "The reply stopped unexpectedly. What was written so far is kept; try again."}));
        self.finish("", true, err.id(), Some(ask::PANIC_REASON)).await;
    }

    fn send(&self, v: serde_json::Value) {
        let _ = self.tx.send(v.to_string());
    }

    async fn drive(
        self,
        config: Arc<hq_core::config::HqConfig>,
        message: String,
        history: Vec<AgentMsg>,
        files: Vec<ResolvedAttachment>,
    ) {
        // Reading files happens here, after turn_start, so a slow PDF never holds up the socket.
        let (message, image_parts) = chat_uploads::build_prompt(&message, &files).await;
        let streamed = Arc::new(AtomicBool::new(false));
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let (driver_turn, session_driver, followup_turn_id) = {
            let record = self.record.lock().unwrap_or_else(|e| e.into_inner());
            (record.is_driver_turn(), record.is_session_driver_turn(), record.followup_turn_id())
        };
        let hooks = hq_agent::native_hq::NativeHqHooks {
            history,
            turn_id: followup_turn_id,
            on_child_completion: Some(self.child_sink()),
            on_event: Some(self.event_sink(streamed.clone())),
            on_progress: Some(self.progress_sink()),
            timeout: Some(std::time::Duration::from_secs(match &self.ask {
                Some(_) => config.chat_turn_timeout_secs.clamp(60, ask::ASK_TURN_TIMEOUT_SECS),
                None => config.chat_turn_timeout_secs.max(60),
            })),
            skip_memory_ingestion: self.ask.is_some(),
            deny_tool_prefixes: self
                .ask
                .as_ref()
                .map(ask::AskTurn::denied_tool_prefixes)
                .unwrap_or_default()
                .into_iter()
                .chain(session_driver.then(|| DRIVER_DENIED_TOOL.to_string()))
                .chain(driver_turn.then(|| CONFIG_TOOL.to_string()))
                .collect(),
            permission_preset: self.ask.as_ref().and_then(ask::AskTurn::permission_preset),
            identity: Some(match self.tid.as_deref() {
                Some(t) => hq_core::identity::RequestIdentity::from_web_thread(t, driver_turn, session_driver),
                None => hq_core::identity::RequestIdentity::from_proxy_user("web", vec!["*".into()], None),
            }),
            isolated: true,
            image_parts,
            ..Default::default()
        };
        // Same shared runner as the Telegram/Discord relays: session model, tool
        // gating, history injection, thread-file append and memory ingestion.
        let run = hq_agent::native_hq::run_native_hq(
            &config,
            &message,
            hq_agent::session_presets::chat_harness_instructions(&cwd),
            cwd.clone(),
            self.session_config(&config),
            hooks,
        )
        .await;
        let (content, failure) = match run {
            Ok(r) => (r.text.clone(), (!r.success).then(|| ask::failure_reason(&r.text))),
            Err(e) => {
                self.send(json!({"type": "error", "thread_id": self.tid, "content": format!("Session error: {e}")}));
                (String::new(), Some(ask::failure_reason(&format!("Session error: {e}"))))
            }
        };
        self.finish(&content, streamed.load(Ordering::Relaxed), tokio::task::id(), failure.as_deref()).await;
    }

    fn session_config(&self, config: &hq_core::config::HqConfig) -> hq_agent::session::SessionConfig {
        let mut session = hq_agent::session_presets::chat_session_config(config);
        if let Some(ask) = &self.ask {
            session.is_live_user_turn = ask.is_live_user_turn();
            // The asker is not the owner typing, so the exchange must not feed skill self-edits.
            session.background_review = false;
        }
        session
    }

    fn event_sink(&self, streamed: Arc<AtomicBool>) -> Box<dyn Fn(SessionEvent) + Send + Sync + 'static> {
        let (tx, tid, record) = (self.tx.clone(), self.tid.clone(), self.record.clone());
        Box::new(move |event| {
            if matches!(event, SessionEvent::TextDelta(_)) {
                streamed.store(true, Ordering::Relaxed);
            }
            record.lock().unwrap_or_else(|e| e.into_inner()).observe(&event);
            if let Some(msg) = session_event_json(&event, &tid) {
                let _ = tx.send(msg);
            }
        })
    }

    /// A background child settled. With automatic follow-up on, the follow-up
    /// turn speaks for it; otherwise a plain notice lands in the chat so the
    /// result is not lost. The notice never claims HQ will act on it.
    fn child_sink(&self) -> hq_agent::agents::CompletionSink {
        let (db, tx, tid) = (self.db.clone(), self.tx.clone(), self.tid.clone());
        Arc::new(move |event| {
            let followup_on = hq_core::config::HqConfig::load()
                .map(|c| c.collaboration.supervision_followup)
                .unwrap_or(false);
            let Some(thread) = tid.as_deref().filter(|_| !followup_on) else { return };
            let state = if event.success { "finished" } else { "did not finish cleanly" };
            let run = event.run_id.as_deref().unwrap_or("-");
            let content = format!(
                "Sub-agent `{}` ({}) {state} (run `{run}`). This is a notice only: ask me to review it, or check `subagent_run_list`.",
                event.task_id, event.role
            );
            let saved = db.with_conn(|c| {
                hq_db::chat::add_message_with_meta(c, thread, "assistant", &content, Some(&json!({"subagent_notice": event.run_id})))
            });
            match saved {
                Ok(m) => {
                    let _ = tx.send(json!({"type": "turn_end", "thread_id": thread, "message_id": m.message_id}).to_string());
                }
                Err(e) => tracing::warn!(thread, "could not save sub-agent notice: {e}"),
            }
        })
    }

    /// report_progress notes stream into the reply bubble; web has no
    /// background_turns row, so there is nothing else to update.
    fn progress_sink(&self) -> hq_agent::native_hq::ProgressSink {
        let (tx, tid) = (self.tx.clone(), self.tid.clone());
        Arc::new(move |event| {
            if event.note.is_none() {
                return;
            }
            let note = hq_agent::native_hq::render_progress_note("", &event);
            let content = format!("\n\n> {note}\n\n");
            let _ = tx.send(json!({"type": "text_delta", "thread_id": tid, "content": content, "thinking": false}).to_string());
        })
    }

    /// Saves the reply (a tools-only turn too) and ends it for every tab.
    /// `failure` says why the run did not complete, which only an `hq_ask` cares about.
    async fn finish(&self, content: &str, streamed: bool, task_id: tokio::task::Id, failure: Option<&str>) {
        let saved = self.tid.as_deref().and_then(|t| save_reply_message(&self.db, t, &self.record, Some(content), false));
        let message_id = saved.as_ref().map(|m| m.message_id.clone());
        // Before the thread is freed below, so an ask never reads as pending on an idle thread.
        if let Some(turn) = &self.ask {
            ask::settle_finished(&self.db, &turn.ask_id, saved.as_ref(), failure);
        }
        // A turn that never streamed (timeout, buffered failure) still needs its text shown.
        if !content.is_empty() && !streamed {
            self.send(json!({"type": "text_delta", "thread_id": self.tid, "content": content}));
        }
        self.send(json!({"type": "turn_end", "thread_id": self.tid, "message_id": message_id}));
        let Some(t) = &self.tid else { return };
        let mut guard = self.turns.write().await;
        if guard.get(t).is_some_and(|slot| slot.abort.id() == task_id) {
            guard.remove(t);
        }
    }
}

const THREAD_TITLE_CHARS: usize = 60;

/// First line of a message, cut to a sidebar-sized title on a char boundary.
fn thread_title(message: &str) -> String {
    let line = message.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    match line.char_indices().nth(THREAD_TITLE_CHARS) {
        Some((cut, _)) => format!("{}...", line[..cut].trim_end()),
        None => line.to_string(),
    }
}

/// Longest tool arguments sent live; the stored copy has its own, smaller cap.
const LIVE_ARGS_CHARS: usize = 4_000;

/// Map a session event to the web chat wire format. `TurnEnd` is per agent-loop
/// iteration, so it is not forwarded: the driver sends one `turn_end` per reply.
fn session_event_json(event: &SessionEvent, tid: &Option<String>) -> Option<String> {
    let v = match event {
        SessionEvent::TextDelta(s) => {
            serde_json::json!({"type": "text_delta", "thread_id": tid, "content": s, "thinking": false})
        }
        SessionEvent::Reasoning(s) => {
            serde_json::json!({"type": "reasoning_delta", "thread_id": tid, "content": s})
        }
        SessionEvent::ToolStart {
            tool_name,
            tool_call_id,
            arguments,
        } => {
            let args = record::args_preview(arguments, LIVE_ARGS_CHARS);
            serde_json::json!({"type": "tool_start", "thread_id": tid, "tool_name": tool_name, "tool_call_id": tool_call_id, "args": args})
        }
        SessionEvent::ToolProgress {
            tool_call_id,
            message,
            ..
        } => {
            serde_json::json!({"type": "tool_progress", "thread_id": tid, "tool_call_id": tool_call_id, "message": message})
        }
        SessionEvent::ToolEnd {
            tool_name,
            tool_call_id,
            result,
        } => {
            let preview = record::result_preview(result, record::LIVE_RESULT_CHARS);
            serde_json::json!({"type": "tool_end", "thread_id": tid, "tool_call_id": tool_call_id, "tool_name": tool_name, "result": preview})
        }
        SessionEvent::StepCredits { turn, delta, .. } => {
            serde_json::json!({"type": "step_credits", "thread_id": tid, "turn": turn, "delta": delta})
        }
        SessionEvent::Error(msg) => {
            serde_json::json!({"type": "error", "thread_id": tid, "content": msg})
        }
        SessionEvent::BudgetExhausted { spent, budget } => serde_json::json!({
            "type": "error",
            "thread_id": tid,
            "content": format!("Budget exhausted: spent ${spent:.2} of ${budget:.2}"),
        }),
        _ => return None,
    };
    Some(v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_messages_parse_and_unknown_types_are_other() {
        let parse = |s: &str| serde_json::from_str::<ClientMsg>(s).unwrap();
        assert!(matches!(
            parse(r#"{"type":"chat","content":"hi","thread_id":null}"#),
            ClientMsg::Chat { content, thread_id: None, attachments, client_id: None, replace_from: None }
                if content == "hi" && attachments.is_empty()
        ));
        let edit = parse(r#"{"type":"chat","content":"again","thread_id":"t1","client_id":"local-1","replace_from":"m7"}"#);
        assert!(matches!(
            edit,
            ClientMsg::Chat { client_id: Some(c), replace_from: Some(r), .. } if c == "local-1" && r == "m7"
        ));
        let with_files = parse(
            r#"{"type":"chat","content":"","thread_id":"t1","attachments":[{"name":"a.png","path":"_media/web/d/a.png","mime":"image/png","size":3}]}"#,
        );
        assert!(matches!(with_files, ClientMsg::Chat { attachments, .. } if attachments.len() == 1));
        assert!(matches!(parse(r#"{"type":"stop","thread_id":"t1"}"#), ClientMsg::Stop { thread_id: Some(_) }));
        assert!(matches!(parse(r#"{"type":"ping"}"#), ClientMsg::Other));
    }

    fn test_state() -> Arc<WsState> {
        let vault = std::env::temp_dir().join(format!("hq-ws-test-{}", uuid::Uuid::new_v4()));
        Arc::new(WsState::new(vault, None))
    }

    /// A running reply in `tid`, as handle_chat would register it.
    async fn plant_turn(state: &Arc<WsState>, tid: &str, streamed: &str) -> SharedRecord {
        let record: SharedRecord = Arc::new(std::sync::Mutex::new(TurnRecord::default()));
        record.lock().unwrap().observe(&SessionEvent::TextDelta(streamed.into()));
        let task = tokio::spawn(std::future::pending::<()>());
        let slot = ChatTurnSlot { abort: task.abort_handle(), record: record.clone(), ask_id: None };
        state.active_chat_turns.write().await.insert(tid.to_string(), slot);
        record
    }

    fn request(tid: &str, text: &str, replace_from: Option<&str>) -> ChatRequest {
        ChatRequest {
            text: text.into(),
            thread_id: Some(tid.into()),
            files: Vec::new(),
            client_id: Some("local-1".into()),
            replace_from: replace_from.map(Into::into),
        }
    }

    fn next_event(rx: &mut tokio::sync::broadcast::Receiver<String>) -> serde_json::Value {
        serde_json::from_str(&rx.try_recv().expect("an event was broadcast")).unwrap()
    }

    #[tokio::test]
    async fn a_second_send_while_a_reply_runs_is_refused_and_not_saved() {
        let state = test_state();
        let tid = state.db.with_conn(|c| hq_db::chat::create_thread(c, "t", "user", "user")).unwrap().thread_id;
        plant_turn(&state, &tid, "").await;
        let mut rx = state.tx.subscribe();

        handle_chat(&state, request(&tid, "again", None)).await;

        let ev = next_event(&mut rx);
        assert_eq!(ev["type"], "chat_rejected");
        assert_eq!(ev["client_id"], "local-1");
        assert_eq!(ev["running"], true);
        assert!(state.db.with_conn(|c| hq_db::chat::get_messages(c, &tid, 10)).unwrap().is_empty());
    }

    #[tokio::test]
    async fn editing_a_message_that_is_gone_is_refused() {
        let state = test_state();
        let tid = state.db.with_conn(|c| hq_db::chat::create_thread(c, "t", "user", "user")).unwrap().thread_id;
        let mut rx = state.tx.subscribe();

        handle_chat(&state, request(&tid, "edited", Some("no-such-message"))).await;

        let ev = next_event(&mut rx);
        assert_eq!(ev["type"], "chat_rejected");
        assert_eq!(ev["running"], false);
    }

    #[tokio::test]
    async fn stop_keeps_what_streamed_and_ends_the_turn_with_its_id() {
        let state = test_state();
        let tid = state.db.with_conn(|c| hq_db::chat::create_thread(c, "t", "user", "user")).unwrap().thread_id;
        let record = plant_turn(&state, &tid, "half an answer").await;
        let mut rx = state.tx.subscribe();

        stop_chat_turn(&state, &tid).await;

        let ev = next_event(&mut rx);
        assert_eq!(ev["type"], "turn_end");
        assert_eq!(ev["stopped"], true);
        let saved = state.db.with_conn(|c| hq_db::chat::get_messages_page(c, &tid, 10, None)).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].message.content, "half an answer");
        assert_eq!(saved[0].meta.as_ref().unwrap()["stopped"], true);
        assert_eq!(ev["message_id"], saved[0].message.message_id);
        assert!(state.active_chat_turns.read().await.is_empty());
        // The run finishing after the stop must not save a second copy.
        assert!(save_reply(&state.db, &tid, &record, Some("full"), false).is_none());
    }

    #[tokio::test]
    async fn a_client_that_fell_behind_is_told_instead_of_left_guessing() {
        let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(2);
        for i in 0..5 {
            tx.send(format!("delta {i}")).unwrap();
        }
        let frame: serde_json::Value = serde_json::from_str(&next_frame(&mut rx).await.unwrap()).unwrap();
        assert_eq!(frame["type"], "stream_lag");
        assert_eq!(frame["skipped"], 3);
        assert_eq!(next_frame(&mut rx).await.as_deref(), Some("delta 3"));
        drop(tx);
        assert_eq!(next_frame(&mut rx).await.as_deref(), Some("delta 4"));
        assert!(next_frame(&mut rx).await.is_none());
    }

    #[tokio::test]
    async fn a_panicking_turn_keeps_its_partial_ends_for_every_tab_and_frees_the_chat() {
        let state = test_state();
        let tid = state.db.with_conn(|c| hq_db::chat::create_thread(c, "t", "user", "user")).unwrap().thread_id;
        let record: SharedRecord = Arc::new(std::sync::Mutex::new(TurnRecord::default()));
        record.lock().unwrap().observe(&SessionEvent::TextDelta("half".into()));
        let turn = ChatTurn {
            db: state.db.clone(),
            tx: state.tx.clone(),
            turns: state.active_chat_turns.clone(),
            tid: Some(tid.clone()),
            record: record.clone(),
            ask: None,
        };
        let task = tokio::spawn(async { panic!("boom") });
        state
            .active_chat_turns
            .write()
            .await
            .insert(tid.clone(), ChatTurnSlot { abort: task.abort_handle(), record, ask_id: None });
        let mut rx = state.tx.subscribe();

        turn.supervise(task).await;

        assert_eq!(next_event(&mut rx)["type"], "error");
        let end = next_event(&mut rx);
        assert_eq!(end["type"], "turn_end");
        let saved = state.db.with_conn(|c| hq_db::chat::get_messages_page(c, &tid, 10, None)).unwrap();
        assert_eq!(saved[0].message.content, "half");
        assert_eq!(end["message_id"], saved[0].message.message_id);
        assert!(state.active_chat_turns.read().await.is_empty());
    }

    #[tokio::test]
    async fn a_stopped_turn_is_not_treated_as_a_crash() {
        let state = test_state();
        let turn = ChatTurn {
            db: state.db.clone(),
            tx: state.tx.clone(),
            turns: state.active_chat_turns.clone(),
            tid: Some("t".into()),
            record: Arc::new(std::sync::Mutex::new(TurnRecord::default())),
            ask: None,
        };
        let task = tokio::spawn(std::future::pending::<()>());
        task.abort();
        let mut rx = state.tx.subscribe();
        turn.supervise(task).await;
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn thread_title_is_the_first_line_cut_to_size() {
        assert_eq!(thread_title("\n  Plan the launch  \nmore"), "Plan the launch");
        let long = "é".repeat(80);
        assert_eq!(thread_title(&long), format!("{}...", "é".repeat(60)));
        assert_eq!(thread_title("   "), "");
    }
}
