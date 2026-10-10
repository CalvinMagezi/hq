//! Disclosure-scoped wrapper for task tools (FR-073).
//!
//! A person's tasks live in the space `people-<slug>`. For a restricted
//! audience, reads see only spaces labelled for everyone present, writes go
//! only into the sole verified person's own space, and every tool this file
//! cannot filter is refused.

use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_core::privacy::{
    DENIED_MESSAGE, DisclosureScope, Label, PersonId, person_space_slug, space_label,
};
use hq_db::Database;
use hq_db::tasks as t;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

use super::placement::resolve_space_id;
use crate::registry::{HqTool, ToolPolicy};
use crate::util::generate_id;

/// Tools whose only input is one task id under `id` or `task_id`, read-only.
const TASK_READ_TOOLS: &[&str] = &["task_get", "task_comment_list"];
/// Tools that change one task, so the task must be in the caller's own space.
const TASK_WRITE_TOOLS: &[&str] = &["task_update", "task_delete", "task_comment_add"];

struct ScopedTaskTool {
    inner: Box<dyn HqTool>,
    db: Arc<Database>,
    scope: DisclosureScope,
}

pub fn scope_task_tools(
    tools: Vec<Box<dyn HqTool>>,
    db: Arc<Database>,
    scope: &DisclosureScope,
) -> Vec<Box<dyn HqTool>> {
    if scope.is_unrestricted() {
        return tools;
    }
    tools
        .into_iter()
        .map(|inner| -> Box<dyn HqTool> {
            Box::new(ScopedTaskTool {
                inner,
                db: db.clone(),
                scope: scope.clone(),
            })
        })
        .collect()
}

/// Finds the person's task space or creates it. Safe to call repeatedly: the slug is
/// unique, and a lost creation race falls back to the row the winner made.
pub fn ensure_person_space(db: &Database, person: &PersonId) -> Result<t::Space> {
    let Some(slug) = person_space_slug(person) else {
        bail!("person has no usable slug");
    };
    db.with_conn(move |c| {
        if let Some(found) = space_by_slug(c, &slug)? {
            return Ok(found);
        }
        let name = format!("Person {}", slug.trim_start_matches("people-"));
        match t::create_space(c, &generate_id("sp"), &name, &slug) {
            Ok(space) => Ok(space),
            Err(e) => space_by_slug(c, &slug)?.ok_or(e),
        }
    })
}

fn space_by_slug(c: &rusqlite::Connection, slug: &str) -> Result<Option<t::Space>> {
    Ok(t::list_spaces(c)?.into_iter().find(|s| s.slug == slug))
}

fn label_of_space_id(c: &rusqlite::Connection, space_id: &str) -> Label {
    match t::get_space(c, space_id) {
        Ok(Some(space)) => space_label(&space.slug),
        _ => Label::Unmarked,
    }
}

fn label_of_initiative(c: &rusqlite::Connection, initiative_id: &str) -> Label {
    match t::get_initiative(c, initiative_id) {
        Ok(Some(i)) => label_of_space_id(c, &i.space_id),
        _ => Label::Unmarked,
    }
}

fn label_of_task(c: &rusqlite::Connection, id: &str) -> Label {
    match t::get_task(c, id) {
        Ok(Some(task)) => label_of_initiative(c, &task.initiative_id),
        _ => Label::Unmarked,
    }
}

fn str_list(args: &Value, key: &str) -> Vec<String> {
    match args.get(key) {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

impl ScopedTaskTool {
    fn writable(&self, label: &Label) -> bool {
        matches!(label, Label::Private(_))
            && self.scope.sole_person().is_some()
            && self.scope.allows(label)
    }

    fn check_tasks(&self, ids: &[String], write: bool) -> Result<()> {
        let labels: Vec<Label> = self
            .db
            .with_conn(|c| Ok(ids.iter().map(|id| label_of_task(c, id)).collect()))?;
        let ok = labels.iter().all(|l| {
            if write {
                self.writable(l)
            } else {
                self.scope.allows(l)
            }
        });
        if ids.is_empty() || !ok {
            bail!(DENIED_MESSAGE);
        }
        Ok(())
    }

    fn own_space(&self) -> Result<t::Space> {
        let Some(person) = self.scope.sole_person() else {
            bail!(DENIED_MESSAGE);
        };
        ensure_person_space(&self.db, person).map_err(|e| {
            tracing::warn!(error = %e, "task scope: could not prepare person space");
            anyhow::anyhow!(DENIED_MESSAGE)
        })
    }

    fn update_write_targets(args: &Value) -> Vec<String> {
        let mut ids = str_list(args, "id");
        for key in ["parent_id", "add_depends_on", "remove_depends_on"] {
            ids.extend(str_list(args, key));
        }
        ids
    }

    fn create_write_targets(args: &Value) -> Vec<String> {
        let mut ids = str_list(args, "parent_id");
        ids.extend(str_list(args, "depends_on"));
        ids
    }

    async fn create_task(&self, mut args: Value) -> Result<Value> {
        let space = self.own_space()?;
        let targets = Self::create_write_targets(&args);
        if !targets.is_empty() {
            self.check_tasks(&targets, true)?;
        }
        if let Some(iid) = args.get("initiative_id").and_then(Value::as_str) {
            let label = self.db.with_conn(|c| Ok(label_of_initiative(c, iid)))?;
            if !self.writable(&label) {
                bail!(DENIED_MESSAGE);
            }
        }
        args["space_id"] = json!(space.slug);
        // A restricted audience cannot link, and the advice block names other
        // people's tasks, so neither goes in or comes out.
        if let Some(obj) = args.as_object_mut() {
            obj.remove("links");
        }
        let mut created = self.inner.execute(args).await?;
        if let Some(obj) = created.as_object_mut() {
            for key in CREATE_EXTRAS {
                obj.remove(*key);
            }
        }
        Ok(created)
    }

    async fn create_in_space(&self, mut args: Value) -> Result<Value> {
        let space = self.own_space()?;
        if let Some(given) = args.get("space_id").and_then(Value::as_str) {
            let resolved = self.db.with_conn(|c| resolve_space_id(c, given));
            if resolved.as_deref().ok() != Some(space.id.as_str()) {
                bail!(DENIED_MESSAGE);
            }
        }
        args["space_id"] = json!(space.slug);
        if let Some(obj) = args.as_object_mut() {
            obj.remove("id_prefix");
        }
        self.inner.execute(args).await
    }

    /// Drops the rows whose space this audience may not see.
    fn retain_visible(&self, rows: &mut Vec<Value>, space_of: SpaceOf) {
        let mut cache: HashMap<String, bool> = HashMap::new();
        let db = &self.db;
        rows.retain(|row| {
            let Some(anchor) = space_of.anchor(row) else {
                return false;
            };
            *cache.entry(anchor.clone()).or_insert_with(|| {
                let label = db
                    .with_conn(|c| Ok(space_of.label(c, &anchor, row)))
                    .unwrap_or(Label::Unmarked);
                self.scope.allows(&label)
            })
        });
    }

    async fn filtered_list(&self, args: Value, key: &str, space_of: SpaceOf) -> Result<Value> {
        let mut result = self.inner.execute(args).await?;
        let Some(rows) = result.get_mut(key).and_then(Value::as_array_mut) else {
            bail!(DENIED_MESSAGE);
        };
        self.retain_visible(rows, space_of);
        let n = rows.len();
        if result.get("count").is_some() {
            result["count"] = n.into();
        }
        Ok(result)
    }

    /// `task_list` is paged, so the audience filter runs over every page first and the
    /// caller's `limit` and `offset` apply to what it may see. Otherwise `total` and
    /// `has_more` would describe tasks in spaces it cannot read.
    async fn scoped_task_list(&self, args: Value) -> Result<Value> {
        let page_size = args.get("limit").and_then(Value::as_u64).unwrap_or(DEFAULT_SCOPED_PAGE) as usize;
        let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let mut visible: Vec<Value> = Vec::new();
        let mut inner_args = args;
        inner_args["limit"] = json!(t::MAX_LIST_LIMIT);
        let mut read = 0;
        loop {
            inner_args["offset"] = json!(read);
            let mut page = self.inner.execute(inner_args.clone()).await?;
            let more = page["has_more"].as_bool().unwrap_or(false);
            let Some(rows) = page.get_mut("tasks").and_then(Value::as_array_mut) else {
                bail!(DENIED_MESSAGE);
            };
            read += rows.len();
            let fetched = rows.len();
            self.retain_visible(rows, SpaceOf::Initiative);
            visible.append(rows);
            if !more || fetched == 0 {
                break;
            }
        }
        let total = visible.len();
        let rows: Vec<Value> = visible.into_iter().skip(offset).take(page_size.max(1)).collect();
        Ok(json!({
            "count": rows.len(),
            "total": total,
            "offset": offset,
            "has_more": offset + rows.len() < total,
            "tasks": rows,
        }))
    }
}

/// Parts of a `task_create` reply that name tasks, chats or notes outside the caller's own space.
const CREATE_EXTRAS: &[&str] = &["links", "similar_open_tasks", "similar_note", "suggested_estimate"];

/// Which machine, directory and branch an agent worked in, and under which lease,
/// is for the owner. A restricted audience sees what happened and when, not where.
fn strip_work_details(task: &mut Value) {
    if let Some(obj) = task.as_object_mut() {
        obj.remove("work_sessions");
        obj.remove("held_by");
        obj.remove("links");
    }
    let events = task.get_mut("lifecycle_events").and_then(Value::as_array_mut);
    for event in events.into_iter().flatten().filter_map(Value::as_object_mut) {
        event.remove("actor");
        event.remove("work_session_id");
    }
}

/// Page size when a restricted audience names none; matches `task_list`'s own default.
const DEFAULT_SCOPED_PAGE: u64 = 100;

/// Which field of a listed row ties it to a space, and how to label it.
#[derive(Clone, Copy)]
enum SpaceOf {
    Initiative,
    SpaceId,
    SpaceSlug,
}

impl SpaceOf {
    fn anchor(self, row: &Value) -> Option<String> {
        let field = match self {
            Self::Initiative => "initiative_id",
            Self::SpaceId => "space_id",
            Self::SpaceSlug => "slug",
        };
        row.get(field).and_then(Value::as_str).map(str::to_string)
    }

    fn label(self, c: &rusqlite::Connection, anchor: &str, _row: &Value) -> Label {
        match self {
            Self::Initiative => label_of_initiative(c, anchor),
            Self::SpaceId => label_of_space_id(c, anchor),
            Self::SpaceSlug => space_label(anchor),
        }
    }
}

#[async_trait]
impl HqTool for ScopedTaskTool {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn parameters(&self) -> Value {
        self.inner.parameters()
    }
    async fn validate(&self, args: &Value) -> hq_core::types::ValidationResult {
        self.inner.validate(args).await
    }
    fn category(&self) -> &str {
        self.inner.category()
    }
    fn search_hint(&self) -> Option<&str> {
        self.inner.search_hint()
    }
    fn is_read_only(&self) -> bool {
        self.inner.is_read_only()
    }
    fn is_destructive(&self) -> bool {
        self.inner.is_destructive()
    }
    fn requires_live_user_turn(&self) -> bool {
        self.inner.requires_live_user_turn()
    }
    fn tool_policy(&self) -> ToolPolicy {
        self.inner.tool_policy()
    }
    fn timeout_ms(&self) -> Option<u64> {
        self.inner.timeout_ms()
    }
    fn behavioral_prompt(&self) -> Option<&str> {
        self.inner.behavioral_prompt()
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let name = self.inner.name();
        if TASK_READ_TOOLS.contains(&name) {
            let ids = [str_list(&args, "id"), str_list(&args, "task_id")].concat();
            self.check_tasks(&ids, false)?;
            let mut result = self.inner.execute(args).await?;
            if name == "task_get" {
                strip_work_details(&mut result);
            }
            return Ok(result);
        }
        if TASK_WRITE_TOOLS.contains(&name) {
            let mut ids = Self::update_write_targets(&args);
            ids.extend(str_list(&args, "task_id"));
            self.check_tasks(&ids, true)?;
            return self.inner.execute(args).await;
        }
        match name {
            "task_create" => self.create_task(args).await,
            "folder_create" | "initiative_create" => self.create_in_space(args).await,
            "task_list" => self.scoped_task_list(args).await,
            "initiative_list" | "folder_list" => {
                let key = if name == "folder_list" {
                    "folders"
                } else {
                    "initiatives"
                };
                self.filtered_list(args, key, SpaceOf::SpaceId).await
            }
            "space_list" => self.filtered_list(args, "spaces", SpaceOf::SpaceSlug).await,
            _ => bail!(DENIED_MESSAGE),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::privacy::{IdentityClaim, PersonRegistry, resolve};
    use hq_vault::VaultClient;
    use std::path::PathBuf;

    struct Fixture {
        db: Arc<Database>,
        path: PathBuf,
        vault: Arc<VaultClient>,
    }

    fn fixture() -> Fixture {
        let path = PathBuf::from("/tmp/test-vault");
        Fixture {
            db: Arc::new(Database::open_memory().unwrap()),
            vault: Arc::new(VaultClient::new(path.clone()).unwrap()),
            path,
        }
    }

    fn registry() -> PersonRegistry {
        let family = vec![
            hq_core::config::DiscordFamilyUser {
                user_id: 2,
                name: "Bob".into(),
            },
            hq_core::config::DiscordFamilyUser {
                user_id: 3,
                name: "Carol".into(),
            },
        ];
        PersonRegistry::from_discord(&[1], &family)
    }

    fn scope_for(account: &str) -> DisclosureScope {
        DisclosureScope::for_person(&resolve(&registry(), &IdentityClaim::discord(account)))
    }

    fn tools_for(f: &Fixture, scope: &DisclosureScope) -> Vec<Box<dyn HqTool>> {
        let all = super::super::create_task_tools(f.path.clone(), f.vault.clone(), f.db.clone());
        scope_task_tools(all, f.db.clone(), scope)
    }

    async fn call(tools: &[Box<dyn HqTool>], name: &str, args: Value) -> Result<Value> {
        tools
            .iter()
            .find(|t| t.name() == name)
            .unwrap()
            .execute(args)
            .await
    }

    fn denied(r: &Result<Value>) -> bool {
        r.as_ref().is_err_and(|e| e.to_string() == DENIED_MESSAGE)
    }

    async fn add(tools: &[Box<dyn HqTool>], title: &str) -> String {
        let v = call(tools, "task_create", json!({ "title": title }))
            .await
            .unwrap();
        v["id"].as_str().unwrap().to_string()
    }

    fn space_count(f: &Fixture, slug: &str) -> usize {
        f.db.with_conn(|c| Ok(t::list_spaces(c)?.iter().filter(|s| s.slug == slug).count()))
            .unwrap()
    }

    #[tokio::test]
    async fn create_routes_to_own_space_and_ignores_requested_space() {
        let f = fixture();
        let bob = tools_for(&f, &scope_for("2"));
        let v = call(
            &bob,
            "task_create",
            json!({ "title": "buy paint", "space_id": "personal" }),
        )
        .await
        .unwrap();
        let iid = v["initiative_id"].as_str().unwrap().to_string();
        let slug =
            f.db.with_conn(|c| {
                let i = t::get_initiative(c, &iid)?.unwrap();
                Ok(t::get_space(c, &i.space_id)?.unwrap().slug)
            })
            .unwrap();
        assert_eq!(slug, "people-bob");
    }

    #[tokio::test]
    async fn space_is_reused_and_not_duplicated() {
        let f = fixture();
        let bob = tools_for(&f, &scope_for("2"));
        add(&bob, "one").await;
        add(&bob, "two").await;
        assert_eq!(space_count(&f, "people-bob"), 1);
        let person = PersonId::from_name("bob");
        let again = ensure_person_space(&f.db, &person).unwrap();
        assert_eq!(again.slug, "people-bob");
        assert_eq!(space_count(&f, "people-bob"), 1);
    }

    #[tokio::test]
    async fn member_cannot_read_or_change_another_persons_task() {
        let f = fixture();
        let carol = tools_for(&f, &scope_for("3"));
        let secret = add(&carol, "carol private").await;
        let bob = tools_for(&f, &scope_for("2"));
        for tool in [
            "task_get",
            "task_update",
            "task_delete",
            "task_comment_list",
        ] {
            let args = json!({ "id": secret, "task_id": secret, "title": "x" });
            assert!(denied(&call(&bob, tool, args).await), "{tool}");
        }
        let comment = json!({ "task_id": secret, "body": "hi" });
        assert!(denied(&call(&bob, "task_comment_add", comment).await));
        assert!(
            call(&carol, "task_get", json!({ "id": secret }))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn lists_are_filtered_per_person() {
        let f = fixture();
        let carol = tools_for(&f, &scope_for("3"));
        add(&carol, "carol private").await;
        let bob = tools_for(&f, &scope_for("2"));
        add(&bob, "bob private").await;
        let owner = tools_for(&f, &scope_for("1"));
        f.db.with_conn(|c| {
            t::create_space(c, "sp-biz", "Business", "business")?;
            Ok(())
        })
        .unwrap();

        let tasks = call(&bob, "task_list", json!({})).await.unwrap();
        assert_eq!(tasks["count"], 1);
        assert_eq!(tasks["tasks"][0]["title"], "bob private");
        let spaces = call(&bob, "space_list", json!({})).await.unwrap();
        let slugs: Vec<&str> = spaces["spaces"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|s| s["slug"].as_str())
            .collect();
        assert!(slugs.contains(&"people-bob"));
        assert!(!slugs.contains(&"people-carol") && !slugs.contains(&"business"));
        let inits = call(&bob, "initiative_list", json!({})).await.unwrap();
        assert_eq!(inits["initiatives"].as_array().unwrap().len(), 1);
        let all = call(&owner, "task_list", json!({})).await.unwrap();
        assert_eq!(all["count"], 2);
    }

    #[tokio::test]
    async fn creating_as_a_restricted_audience_reveals_no_other_tasks_and_takes_no_links() {
        let f = fixture();
        let owner = super::super::create_task_tools(f.path.clone(), f.vault.clone(), f.db.clone());
        let secret = call(
            &owner,
            "task_create",
            json!({ "title": "Payroll review for the whole company", "description": "salary payroll review confidential" }),
        )
        .await
        .unwrap();
        let bob = tools_for(&f, &scope_for("2"));
        let made = call(
            &bob,
            "task_create",
            json!({
                "title": "Payroll review for the whole company",
                "description": "salary payroll review confidential",
                "links": [{ "kind": "task", "ref": secret["display_id"] }],
            }),
        )
        .await
        .unwrap();
        let text = made.to_string();
        assert!(!text.contains(secret["display_id"].as_str().unwrap()), "{text}");
        for key in CREATE_EXTRAS {
            assert!(made.get(*key).is_none(), "{key} leaked: {text}");
        }
        let links = f.db.with_conn(|c| t::list_task_links(c, made["id"].as_str().unwrap())).unwrap();
        assert!(links.is_empty(), "the link a restricted caller asked for was not recorded");
    }

    #[tokio::test]
    async fn a_restricted_audience_does_not_see_where_or_how_work_was_done() {
        let f = fixture();
        let bob = tools_for(&f, &scope_for("2"));
        let id = add(&bob, "bob private").await;
        let open = super::super::create_task_tools(f.path.clone(), f.vault.clone(), f.db.clone());
        let claim = call(&open, "task_claim", json!({ "task_id": id, "actor": "builder", "cwd": "/srv/app/secret", "host": "laptop" }))
            .await
            .unwrap();
        assert!(claim["lease"].is_string());
        let seen = call(&bob, "task_get", json!({ "id": id })).await.unwrap();
        assert!(seen.get("work_sessions").is_none() && seen.get("held_by").is_none(), "{seen}");
        assert!(seen.get("links").is_none(), "links name chats, sessions and notes the audience may not see");
        let events = seen["lifecycle_events"].as_array().unwrap();
        assert!(!events.is_empty());
        assert!(events.iter().all(|e| e.get("actor").is_none() && e.get("work_session_id").is_none()));
        let owner = call(&open, "task_get", json!({ "id": id })).await.unwrap();
        assert_eq!(owner["held_by"]["cwd"], "/srv/app/secret", "the owner still sees it");
    }

    #[tokio::test]
    async fn a_restricted_audience_cannot_take_work_leases() {
        let f = fixture();
        let bob = tools_for(&f, &scope_for("2"));
        let id = add(&bob, "bob private").await;
        for tool in ["task_claim", "task_heartbeat", "task_release"] {
            let args = json!({ "task_id": id, "actor": "x", "lease": "hql_x" });
            assert!(denied(&call(&bob, tool, args).await), "{tool}");
        }
    }

    #[tokio::test]
    async fn a_scoped_list_pages_and_totals_only_what_the_audience_may_see() {
        let f = fixture();
        let carol = tools_for(&f, &scope_for("3"));
        for n in 0..4 {
            add(&carol, &format!("carol {n}")).await;
        }
        let bob = tools_for(&f, &scope_for("2"));
        for n in 0..3 {
            add(&bob, &format!("bob {n}")).await;
        }

        let first = call(&bob, "task_list", json!({ "limit": 2 })).await.unwrap();
        assert_eq!((first["count"].as_i64(), first["total"].as_i64()), (Some(2), Some(3)));
        assert_eq!(first["has_more"], json!(true));
        let rest = call(&bob, "task_list", json!({ "limit": 2, "offset": 2 })).await.unwrap();
        assert_eq!((rest["count"].as_i64(), rest["has_more"].clone()), (Some(1), json!(false)));
        let titles: Vec<&str> = first["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .chain(rest["tasks"].as_array().unwrap())
            .filter_map(|t| t["title"].as_str())
            .collect();
        assert_eq!(titles.len(), 3);
        assert!(titles.iter().all(|t| t.starts_with("bob")), "{titles:?}");
    }

    #[tokio::test]
    async fn unresolved_and_shared_audiences_cannot_create_or_read() {
        let f = fixture();
        let bob = tools_for(&f, &scope_for("2"));
        let id = add(&bob, "bob private").await;
        let stranger = tools_for(&f, &scope_for("999"));
        let denied_create = call(&stranger, "task_create", json!({ "title": "x" })).await;
        assert!(denied(&denied_create));
        assert!(denied(
            &call(&stranger, "task_get", json!({ "id": id })).await
        ));
        assert_eq!(space_count(&f, "people-999"), 0);

        let audience = [
            resolve(&registry(), &IdentityClaim::discord("2")),
            resolve(&registry(), &IdentityClaim::discord("3")),
        ];
        let shared = tools_for(&f, &DisclosureScope::for_audience(&audience));
        assert!(denied(
            &call(&shared, "task_create", json!({ "title": "x" })).await
        ));
        assert!(denied(
            &call(&shared, "task_get", json!({ "id": id })).await
        ));
        let deny_all = tools_for(&f, &DisclosureScope::deny_all());
        assert!(denied(
            &call(&deny_all, "task_create", json!({ "title": "x" })).await
        ));
    }

    #[tokio::test]
    async fn cross_person_links_and_unfilterable_tools_are_refused() {
        let f = fixture();
        let carol = tools_for(&f, &scope_for("3"));
        let theirs = add(&carol, "carol private").await;
        let bob = tools_for(&f, &scope_for("2"));
        let sub = json!({ "title": "x", "parent_id": theirs });
        assert!(denied(&call(&bob, "task_create", sub).await));
        let dep = json!({ "title": "x", "depends_on": [theirs] });
        assert!(denied(&call(&bob, "task_create", dep).await));
        let foreign =
            f.db.with_conn(|c| Ok(t::list_initiatives(c, None, None)?[0].id.clone()))
                .unwrap();
        let filed = json!({ "title": "x", "initiative_id": foreign });
        assert!(denied(&call(&bob, "task_create", filed).await));
        for tool in [
            "space_create",
            "space_update",
            "task_related",
            "task_create_from_note",
        ] {
            assert!(
                denied(&call(&bob, tool, json!({ "id": theirs, "name": "n" })).await),
                "{tool}"
            );
        }
    }

    #[tokio::test]
    async fn folders_and_initiatives_stay_in_own_space() {
        let f = fixture();
        let bob = tools_for(&f, &scope_for("2"));
        add(&bob, "seed").await;
        let other = json!({ "space_id": "people-carol", "name": "Plans" });
        assert!(denied(&call(&bob, "folder_create", other).await));
        let own = json!({ "name": "Plans" });
        assert!(call(&bob, "folder_create", own.clone()).await.is_ok());
        assert!(call(&bob, "folder_create", own).await.is_ok());
        let folders = f.db.with_conn(|c| t::list_folders(c, None)).unwrap();
        assert_eq!(folders.len(), 1);
        let init = call(&bob, "initiative_create", json!({ "name": "Garden" })).await;
        assert!(init.is_ok());
    }

    #[tokio::test]
    async fn owner_is_unwrapped() {
        let f = fixture();
        let owner = tools_for(&f, &scope_for("1"));
        let v = call(
            &owner,
            "task_create",
            json!({ "title": "biz", "space_id": "professional" }),
        )
        .await;
        assert!(v.is_ok());
        assert_eq!(space_count(&f, "people-owner"), 0);
    }
}
