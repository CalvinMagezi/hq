//! Native task management tools: replaces ClickUp. Spaces > initiatives >
//! tasks > comments, tags as the filter/routing dimension. Unlike Missions
//! (creation is chat-flow only), task creation is a direct tool in both
//! directions — agents and humans both create/update tasks. See
//! docs/plans/native-tasks for the design.

use hq_core::config::TasksConfig;
use hq_db::Database;
use hq_vault::VaultClient;
use std::path::PathBuf;
use std::sync::Arc;

use crate::registry::HqTool;

mod from_note;
mod json;
mod placement;
mod scoped;
mod tools_bulk;
mod tools_graph;
mod tools_links;
mod tools_lease;
mod tools_ops;
mod tools_org;
mod tools_task;
mod tools_time;

use from_note::TaskCreateFromNoteTool;
pub use json::{
    added_tags, apply_dependency_changes, notify_ready_for_review, notify_recipients, notify_tags,
    notify_unblocked,
    task_json, task_json_with_warnings, unblocked_by_transition,
};
pub use placement::{
    Placement, create_task_in, derive_id_prefix, find_or_create_folder, find_or_create_initiative,
    resolve_space_id, slug,
};
pub use scoped::{ensure_person_space, scope_task_tools};
pub use tools_lease::{lease_ttl_secs, task_settings};
use tools_bulk::{TaskCreateManyTool, TaskUpdateManyTool};
use tools_graph::TaskRelatedTool;
use tools_links::*;
use tools_lease::*;
use tools_ops::{InitiativeProgressTool, TaskRoutingAuditTool, TaskStaleTool};
use tools_org::*;
use tools_task::*;
use tools_time::TaskTimeReportTool;

pub fn create_task_tools(
    vault_path: PathBuf,
    vault: Arc<VaultClient>,
    db: Arc<Database>,
) -> Vec<Box<dyn HqTool>> {
    create_task_tools_with(vault_path, vault, db, task_settings())
}

/// `create_task_tools` with explicit task settings, for tests and callers that
/// already hold the loaded config.
pub fn create_task_tools_with(
    vault_path: PathBuf,
    vault: Arc<VaultClient>,
    db: Arc<Database>,
    settings: TasksConfig,
) -> Vec<Box<dyn HqTool>> {
    create_task_tools_for_chat(vault_path, vault, db, settings, None)
}

/// The tools for one web chat. A task it creates gets that thread as its origin
/// link, with nothing for the agent to remember.
pub fn create_task_tools_for_chat(
    vault_path: PathBuf,
    vault: Arc<VaultClient>,
    db: Arc<Database>,
    settings: TasksConfig,
    origin_thread: Option<String>,
) -> Vec<Box<dyn HqTool>> {
    let create = || TaskCreateTool {
        settings: settings.clone(),
        origin_thread: origin_thread.clone(),
        vault_path: vault_path.clone(),
        db: db.clone(),
    };
    let update = || TaskUpdateTool {
        settings: settings.clone(),
        vault_path: vault_path.clone(),
        db: db.clone(),
    };
    vec![
        Box::new(create()),
        Box::new(TaskCreateManyTool { inner: create() }),
        Box::new(TaskUpdateManyTool { inner: update() }),
        Box::new(TaskListTool { settings: settings.clone(), db: db.clone() }),
        Box::new(TaskGetTool { settings: settings.clone(), db: db.clone() }),
        Box::new(update()),
        Box::new(TaskRelatedTool { db: db.clone() }),
        Box::new(TaskDeleteTool { settings: settings.clone(), db: db.clone() }),
        Box::new(TaskRestoreTool { settings: settings.clone(), db: db.clone() }),
        Box::new(TaskCommentAddTool { settings: settings.clone(), db: db.clone() }),
        Box::new(TaskCommentListTool { db: db.clone() }),
        Box::new(TaskClaimTool { settings: settings.clone(), db: db.clone() }),
        Box::new(TaskNextTool { settings: settings.clone(), db: db.clone() }),
        Box::new(TaskHeartbeatTool { settings: settings.clone(), db: db.clone() }),
        Box::new(TaskReleaseTool { settings: settings.clone(), db: db.clone() }),
        Box::new(TaskTimeReportTool { settings: settings.clone(), db: db.clone() }),
        Box::new(TaskStaleTool { settings: settings.clone(), db: db.clone() }),
        Box::new(InitiativeProgressTool { settings: settings.clone(), db: db.clone() }),
        Box::new(TaskRoutingAuditTool { settings: settings.clone(), vault_path: vault_path.clone(), db: db.clone() }),
        Box::new(TaskLinkAddTool { settings: settings.clone(), db: db.clone() }),
        Box::new(TaskLinkRemoveTool { db: db.clone() }),
        Box::new(TaskLinkListTool { db: db.clone() }),
        Box::new(SpaceListTool { db: db.clone() }),
        Box::new(SpaceCreateTool { db: db.clone() }),
        Box::new(SpaceUpdateTool { db: db.clone() }),
        Box::new(FolderListTool { db: db.clone() }),
        Box::new(FolderCreateTool { db: db.clone() }),
        Box::new(InitiativeListTool { db: db.clone() }),
        Box::new(InitiativeCreateTool { db: db.clone() }),
        Box::new(TaskCreateFromNoteTool { route_tags: settings.route_tags, vault, db }),
    ]
}

#[cfg(test)]
mod tests;
