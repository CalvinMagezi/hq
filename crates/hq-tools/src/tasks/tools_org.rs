//! Space, folder, and initiative tools.

use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_db::Database;
use hq_db::tasks as t;
use serde_json::{Value, json};
use std::sync::Arc;

use super::placement::*;
use crate::registry::HqTool;
use crate::util::{arg_str, generate_id};

pub(super) struct SpaceListTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for SpaceListTool {
    fn name(&self) -> &str {
        "space_list"
    }
    fn description(&self) -> &str {
        "List all Spaces (top-level task categories, e.g. Personal/Professional)."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, _args: Value) -> Result<Value> {
        let spaces = self.db.with_conn(t::list_spaces)?;
        Ok(json!({
            "spaces": spaces.iter().map(|s| json!({
                "id": s.id, "name": s.name, "slug": s.slug,
            })).collect::<Vec<_>>()
        }))
    }
}

pub(super) struct SpaceCreateTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for SpaceCreateTool {
    fn name(&self) -> &str {
        "space_create"
    }
    fn description(&self) -> &str {
        "Create a new Space (a top-level task category, e.g. a new area of life or business)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "name": { "type": "string" } },
            "required": ["name"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let name = arg_str(&args, "name");
        if name.is_empty() {
            bail!("name is required");
        }
        let id = generate_id("sp");
        let space_slug = slug(&name);
        let space = self
            .db
            .with_conn(move |c| t::create_space(c, &id, &name, &space_slug))?;
        Ok(json!({ "id": space.id, "name": space.name, "slug": space.slug }))
    }
}

pub(super) struct SpaceUpdateTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for SpaceUpdateTool {
    fn name(&self) -> &str {
        "space_update"
    }
    fn description(&self) -> &str {
        "Rename a Space."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "name": { "type": "string" }
            },
            "required": ["id", "name"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let id = arg_str(&args, "id");
        let name = arg_str(&args, "name");
        if id.is_empty() || name.is_empty() {
            bail!("id and name are required");
        }
        let space = self.db.with_conn(move |c| t::update_space(c, &id, &name))?;
        Ok(json!({ "id": space.id, "name": space.name, "slug": space.slug }))
    }
}

// ─── folders ────────────────────────────────────────────────────────────

pub(super) struct FolderListTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for FolderListTool {
    fn name(&self) -> &str {
        "folder_list"
    }
    fn description(&self) -> &str {
        "List folders (Space > Folder > Initiative), optionally filtered to one Space."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "space_id": { "type": "string" } }
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let space_id = args
            .get("space_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        let folders = self
            .db
            .with_conn(move |c| t::list_folders(c, space_id.as_deref()))?;
        Ok(json!({
            "folders": folders.iter().map(|f| json!({
                "id": f.id, "space_id": f.space_id, "name": f.name, "slug": f.slug,
            })).collect::<Vec<_>>()
        }))
    }
}

pub(super) struct FolderCreateTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for FolderCreateTool {
    fn name(&self) -> &str {
        "folder_create"
    }
    fn description(&self) -> &str {
        "Create a folder within a Space. Initiatives can optionally be filed under one."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "space_id": { "type": "string" },
                "name": { "type": "string" }
            },
            "required": ["space_id", "name"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let space_id = arg_str(&args, "space_id");
        let name = arg_str(&args, "name");
        if space_id.is_empty() || name.is_empty() {
            bail!("space_id and name are required");
        }
        let folder = self
            .db
            .with_conn(move |c| find_or_create_folder(c, &space_id, &name))?;
        Ok(
            json!({ "id": folder.id, "space_id": folder.space_id, "name": folder.name, "slug": folder.slug }),
        )
    }
}

// ─── initiatives ────────────────────────────────────────────────────────

pub(super) struct InitiativeListTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for InitiativeListTool {
    fn name(&self) -> &str {
        "initiative_list"
    }
    fn description(&self) -> &str {
        "List initiatives (projects/work areas), optionally filtered to one Space and/or Folder."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "space_id": { "type": "string" },
                "folder_id": { "type": "string", "description": "Filter to one folder's initiatives" }
            }
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let space_id = args
            .get("space_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        let folder_id = args
            .get("folder_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        let initiatives = self.db.with_conn(move |c| {
            t::list_initiatives(c, space_id.as_deref(), folder_id.as_deref().map(Some))
        })?;
        Ok(json!({
            "initiatives": initiatives.iter().map(|i| json!({
                "id": i.id, "space_id": i.space_id, "folder_id": i.folder_id, "name": i.name,
                "slug": i.slug, "id_prefix": i.id_prefix,
            })).collect::<Vec<_>>()
        }))
    }
}

pub(super) struct InitiativeCreateTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for InitiativeCreateTool {
    fn name(&self) -> &str {
        "initiative_create"
    }
    fn description(&self) -> &str {
        "Create an initiative (a project/work area) within a Space, optionally under a Folder. Tasks \
         created without an explicit initiative_id auto-resolve to one of these by name, so this is \
         only needed to set an explicit id_prefix up front or to place it in a specific folder."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "space_id": { "type": "string" },
                "folder": { "type": "string", "description": "Optional folder name to find-or-create this initiative under" },
                "name": { "type": "string" },
                "id_prefix": { "type": "string", "description": "Display-id prefix, e.g. 'AGENT-HQ'. Derived from space+name if omitted." }
            },
            "required": ["space_id", "name"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let space_id = arg_str(&args, "space_id");
        let name = arg_str(&args, "name");
        if space_id.is_empty() || name.is_empty() {
            bail!("space_id and name are required");
        }
        let folder_name = args
            .get("folder")
            .and_then(|v| v.as_str())
            .map(String::from);
        let explicit_prefix = args
            .get("id_prefix")
            .and_then(|v| v.as_str())
            .map(String::from);
        let id = generate_id("in");
        let initiative_slug = slug(&name);
        let initiative = self.db.with_conn(move |c| {
            let space_id = resolve_space_id(c, &space_id)?;
            let space = t::get_space(c, &space_id)?
                .ok_or_else(|| anyhow::anyhow!("space '{space_id}' does not exist"))?;
            let folder_id = match &folder_name {
                Some(fname) if !fname.trim().is_empty() => {
                    Some(find_or_create_folder(c, &space_id, fname)?.id)
                }
                _ => None,
            };
            let id_prefix = match explicit_prefix {
                Some(p) => p,
                None => derive_id_prefix(c, &space.slug, &name)?,
            };
            t::create_initiative(
                c,
                &id,
                &space_id,
                folder_id.as_deref(),
                &name,
                &initiative_slug,
                &id_prefix,
            )
        })?;
        Ok(json!({
            "id": initiative.id, "space_id": initiative.space_id, "folder_id": initiative.folder_id,
            "name": initiative.name, "id_prefix": initiative.id_prefix,
        }))
    }
}
