//! Native task management tools: replaces ClickUp. Spaces > initiatives >
//! tasks > comments, tags as the filter/routing dimension. Unlike Missions
//! (creation is chat-flow only), task creation is a direct tool in both
//! directions — agents and humans both create/update tasks. See
//! docs/plans/native-tasks for the design.

use hq_db::Database;
use hq_vault::VaultClient;
use std::path::PathBuf;
use std::sync::Arc;

use crate::registry::HqTool;

mod from_note;
mod json;
mod placement;
mod scoped;
mod tools_graph;
mod tools_org;
mod tools_task;

use from_note::TaskCreateFromNoteTool;
pub use json::{
    added_tags, apply_dependency_changes, notify_ready_for_review, notify_tags, notify_unblocked,
    task_json, task_json_with_warnings, unblocked_by_transition,
};
pub use placement::{
    Placement, create_task_in, derive_id_prefix, find_or_create_folder, find_or_create_initiative,
    resolve_space_id, slug,
};
pub use scoped::{ensure_person_space, scope_task_tools};
use tools_graph::TaskRelatedTool;
use tools_org::*;
use tools_task::*;

pub fn create_task_tools(
    vault_path: PathBuf,
    vault: Arc<VaultClient>,
    db: Arc<Database>,
) -> Vec<Box<dyn HqTool>> {
    vec![
        Box::new(TaskCreateTool {
            vault_path: vault_path.clone(),
            db: db.clone(),
        }),
        Box::new(TaskListTool { db: db.clone() }),
        Box::new(TaskGetTool { db: db.clone() }),
        Box::new(TaskUpdateTool {
            vault_path,
            db: db.clone(),
        }),
        Box::new(TaskRelatedTool { db: db.clone() }),
        Box::new(TaskDeleteTool { db: db.clone() }),
        Box::new(TaskCommentAddTool { db: db.clone() }),
        Box::new(TaskCommentListTool { db: db.clone() }),
        Box::new(SpaceListTool { db: db.clone() }),
        Box::new(SpaceCreateTool { db: db.clone() }),
        Box::new(SpaceUpdateTool { db: db.clone() }),
        Box::new(FolderListTool { db: db.clone() }),
        Box::new(FolderCreateTool { db: db.clone() }),
        Box::new(InitiativeListTool { db: db.clone() }),
        Box::new(InitiativeCreateTool { db: db.clone() }),
        Box::new(TaskCreateFromNoteTool { vault, db }),
    ]
}

#[cfg(test)]
mod tests;
