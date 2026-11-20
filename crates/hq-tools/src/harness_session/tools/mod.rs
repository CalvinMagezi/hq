//! MCP tools over the harness session manager.
//!
//! One file per concern: `spawn` starts a session (and holds the folder checks it runs),
//! `handoff` starts one with a task, `inspect` reads sessions, `control` drives them. This file
//! holds what they share and builds the tool list.

use hq_db::Database;
use std::sync::Arc;

use crate::harness_session::WatchingChat;
use crate::registry::HqTool;

mod control;
mod handoff;
mod inspect;
mod spawn;

pub use control::{
    HarnessSessionAnswerTool, HarnessSessionAttachTool, HarnessSessionGoalTool,
    HarnessSessionLinkTool, HarnessSessionModeTool, HarnessSessionResumeTool,
    HarnessSessionSendTool, HarnessSessionStopTool, HarnessSessionWatchTool,
};
pub use handoff::HarnessSessionHandoffTool;
#[cfg(test)]
pub(in crate::harness_session) use handoff::handoff_drives_new;
pub use inspect::{
    HarnessSessionListTool, HarnessSessionLogsTool, HarnessSessionPendingTool,
    HarnessSessionStatusTool, HarnessSessionWaitTool,
};
pub use spawn::HarnessSessionSpawnTool;

/// Most sessions one wait may cover.
const MAX_FAN_IN: usize = 10;
use hq_db::harness_sessions_registry as registry;

/// `chat` is the web chat of the calling session, if any.
pub fn create_harness_session_tools(
    db: Arc<Database>,
    chat: Option<WatchingChat>,
    family_guest: Option<crate::family_guest::FamilyGuestContext>,
) -> Vec<Box<dyn HqTool>> {
    let family_guest_for_attach = family_guest.clone();
    let family_guest_for_handoff = family_guest.clone();
    vec![
        Box::new(HarnessSessionHandoffTool {
            db: db.clone(),
            chat: chat.clone(),
            family_guest: family_guest_for_handoff,
        }),
        Box::new(HarnessSessionSpawnTool {
            db: db.clone(),
            chat: chat.clone(),
            family_guest,
        }),
        Box::new(HarnessSessionListTool { db: db.clone() }),
        Box::new(HarnessSessionStatusTool { db: db.clone() }),
        Box::new(crate::harness_session::fleet::HarnessFleetStatusTool { db: db.clone() }),
        Box::new(HarnessSessionLogsTool { db: db.clone() }),
        Box::new(HarnessSessionSendTool {
            db: db.clone(),
            chat: chat.clone(),
        }),
        Box::new(HarnessSessionWaitTool { db: db.clone() }),
        Box::new(HarnessSessionPendingTool { db: db.clone() }),
        Box::new(crate::harness_session::events::HarnessSessionEventsTool {
            db: db.clone(),
            feed: crate::harness_session::events::Feed::new(db.clone()),
        }),
        Box::new(crate::harness_session::decisions::TaskDecisionsTool { db: db.clone() }),
        Box::new(crate::harness_session::stats::HarnessSessionStatsTool { db: db.clone() }),
        Box::new(crate::harness_session::subscribe::HarnessSessionSubscribeTool { db: db.clone() }),
        Box::new(
            crate::harness_session::subscribe::HarnessSessionUnsubscribeTool { db: db.clone() },
        ),
        Box::new(HarnessSessionAnswerTool {
            db: db.clone(),
            chat: chat.clone(),
        }),
        Box::new(HarnessSessionStopTool { db: db.clone() }),
        Box::new(crate::harness_session::publish::HarnessSessionPublishTool { db: db.clone() }),
        Box::new(crate::harness_session::sandbox_request::SandboxRequestTool { db: db.clone() }),
        Box::new(crate::harness_session::merge::HarnessSessionRequestMergeTool { db: db.clone() }),
        Box::new(HarnessSessionLinkTool {
            db: db.clone(),
            chat: chat.clone(),
        }),
        Box::new(HarnessSessionWatchTool {
            db: db.clone(),
            chat: chat.clone(),
        }),
        Box::new(HarnessSessionGoalTool {
            db: db.clone(),
            chat: chat.clone(),
        }),
        Box::new(HarnessSessionModeTool {
            db: db.clone(),
            chat: chat.clone(),
        }),
        Box::new(HarnessSessionAttachTool {
            db: db.clone(),
            chat: chat.clone(),
            family_guest: family_guest_for_attach,
        }),
        Box::new(HarnessSessionResumeTool { db, chat }),
    ]
}
