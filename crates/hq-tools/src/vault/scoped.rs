//! Disclosure-scoped wrapper for vault tools (FR-072).

use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_core::privacy::{DENIED_MESSAGE, DisclosureScope, Label};
use hq_vault::VaultClient;
use serde_json::Value;
use std::sync::Arc;

use crate::registry::{HqTool, ToolPolicy};

/// Tools that take one `path` and touch only that note.
const PATH_READ_TOOLS: &[&str] = &[
    "vault_read",
    "vault_outline",
    "vault_read_section",
    "vault_links",
];
/// Same shape, but they change the note, so shared or public labels are not enough.
const PATH_WRITE_TOOLS: &[&str] = &[
    "vault_write_note",
    "vault_append_note",
    "vault_patch_section",
    "vault_frontmatter_update",
    "vault_delete",
];

/// Wraps every vault or memory tool when the audience is restricted. Tools
/// this file does not know how to filter (graph, tags, reorg, reindex) are
/// refused outright rather than allowed to leak through aggregate results.
struct ScopedVaultTool {
    inner: Box<dyn HqTool>,
    vault: Arc<VaultClient>,
    scope: DisclosureScope,
}

pub fn scope_vault_tools(
    tools: Vec<Box<dyn HqTool>>,
    vault: Arc<VaultClient>,
    scope: &DisclosureScope,
) -> Vec<Box<dyn HqTool>> {
    if scope.is_unrestricted() {
        return tools;
    }
    tools
        .into_iter()
        .map(|inner| -> Box<dyn HqTool> {
            if inner.name().starts_with("vault_") || inner.name().starts_with("memory_") {
                Box::new(ScopedVaultTool {
                    inner,
                    vault: vault.clone(),
                    scope: scope.clone(),
                })
            } else {
                inner
            }
        })
        .collect()
}

impl ScopedVaultTool {
    fn label_of(&self, path: &str) -> Label {
        match self.vault.read_note(path) {
            Ok(note) => label_from_note(&note),
            Err(_) if !self.vault.vault_path().join(path).exists() => {
                Label::classify(path, None, &[])
            }
            Err(_) => Label::Unmarked,
        }
    }

    fn allows_path(&self, path: &str) -> bool {
        self.scope.allows(&self.label_of(path))
    }

    fn allows_write(&self, path: &str) -> bool {
        matches!(self.label_of(path), Label::Private(_)) && self.allows_path(path)
    }

    fn keep_paths(&self, result: &mut Value, key: &str, path_of: fn(&Value) -> Option<&str>) {
        if let Some(items) = result.get_mut(key).and_then(Value::as_array_mut) {
            items.retain(|item| path_of(item).is_some_and(|p| self.allows_path(p)));
        }
        let count = result.get(key).and_then(Value::as_array).map(Vec::len);
        if let Some(n) = count {
            result["count"] = n.into();
        }
    }
}

fn label_from_note(note: &hq_core::types::Note) -> Label {
    let visibility = note.frontmatter.get("visibility").and_then(|v| v.as_str());
    let people = match note.frontmatter.get("people") {
        Some(serde_yaml::Value::Sequence(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(serde_yaml::Value::String(one)) => vec![one.clone()],
        _ => Vec::new(),
    };
    Label::classify(&note.path, visibility, &people)
}

fn item_path(item: &Value) -> Option<&str> {
    item.get("path").and_then(Value::as_str)
}

fn plain_path(item: &Value) -> Option<&str> {
    item.as_str()
}

#[async_trait]
impl HqTool for ScopedVaultTool {
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
        let path = args.get("path").and_then(Value::as_str).unwrap_or_default();
        if PATH_READ_TOOLS.contains(&name) {
            if !self.allows_path(path) {
                bail!(DENIED_MESSAGE);
            }
            return self.inner.execute(args).await;
        }
        if PATH_WRITE_TOOLS.contains(&name) {
            if !self.allows_write(path) {
                bail!(DENIED_MESSAGE);
            }
            return self.inner.execute(args).await;
        }
        match name {
            "vault_batch_read" => {
                let paths = args.get("paths").and_then(Value::as_array);
                let all_ok = paths.is_some_and(|ps| {
                    ps.iter()
                        .all(|p| p.as_str().is_some_and(|p| self.allows_path(p)))
                });
                if !all_ok {
                    bail!(DENIED_MESSAGE);
                }
                self.inner.execute(args).await
            }
            "vault_person_record_fact" => {
                let mut args = args;
                let wants_create = args.get(super::person::CREATE_PERSON_ARG);
                if wants_create.and_then(Value::as_bool) == Some(true) {
                    bail!(super::person::CREATE_PERSON_OWNER_ONLY);
                }
                let person = args
                    .get("person")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let allowed = super::person::person_probe_path(person)
                    .is_some_and(|probe| self.allows_write(&probe));
                if !allowed {
                    bail!(DENIED_MESSAGE);
                }
                // The write gate proved the caller is this verified, registered person.
                args[super::person::PERSON_REGISTERED_ARG] = true.into();
                args[super::person::CREATE_PERSON_ARG] = false.into();
                self.inner.execute(args).await
            }
            "vault_search" => {
                let mut result = self.inner.execute(args).await?;
                self.keep_paths(&mut result, "results", item_path);
                Ok(result)
            }
            "vault_list" => {
                let mut result = self.inner.execute(args).await?;
                self.keep_paths(&mut result, "paths", plain_path);
                Ok(result)
            }
            _ => bail!(DENIED_MESSAGE),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::privacy::{IdentityClaim, PersonRegistry, Resolution, resolve};
    use serde_json::json;

    fn vault_with_notes() -> (tempfile::TempDir, Arc<VaultClient>) {
        let dir = tempfile::tempdir().unwrap();
        let write = |rel: &str, body: &str| {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write("Notebooks/People/bob/goals.md", "# Goals\ndon secret");
        write("Notebooks/People/carol/goals.md", "# Goals\nashley secret");
        write("Notebooks/Business/pricing.md", "# Pricing\nowner business");
        write(
            "Notebooks/Shared/trip.md",
            "---\nvisibility: shared\n---\n# Trip\nfamily trip",
        );
        write(
            "Notebooks/Joint/plan.md",
            "---\npeople: [Bob, Carol]\n---\n# Plan\njoint",
        );
        let vault = Arc::new(VaultClient::new(dir.path().to_path_buf()).unwrap());
        (dir, vault)
    }

    fn scoped(vault: &Arc<VaultClient>, account: &str) -> Vec<Box<dyn HqTool>> {
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
        let reg = PersonRegistry::from_discord(&[1], &family);
        let res: Resolution = resolve(&reg, &IdentityClaim::discord(account));
        let scope = DisclosureScope::for_person(&res);
        let tools = crate::vault::create_vault_tools(vault.vault_path().to_path_buf(), None);
        scope_vault_tools(tools, vault.clone(), &scope)
    }

    async fn call(tools: &[Box<dyn HqTool>], name: &str, args: Value) -> Result<Value> {
        let tool = tools.iter().find(|t| t.name() == name).unwrap();
        tool.execute(args).await
    }

    #[tokio::test]
    async fn member_reads_own_and_shared_but_not_other_people_or_business() {
        let (_d, vault) = vault_with_notes();
        let t = scoped(&vault, "2");
        for ok in [
            "Notebooks/People/bob/goals.md",
            "Notebooks/Shared/trip.md",
            "Notebooks/Joint/plan.md",
        ] {
            assert!(
                call(&t, "vault_read", json!({ "path": ok })).await.is_ok(),
                "{ok}"
            );
        }
        for denied in [
            "Notebooks/People/carol/goals.md",
            "Notebooks/Business/pricing.md",
        ] {
            let err = call(&t, "vault_read", json!({ "path": denied }))
                .await
                .unwrap_err();
            assert_eq!(err.to_string(), DENIED_MESSAGE);
        }
    }

    #[tokio::test]
    async fn listing_batch_and_graph_tools_do_not_leak() {
        let (_d, vault) = vault_with_notes();
        let t = scoped(&vault, "2");
        let list = call(
            &t,
            "vault_list",
            json!({ "directory": "Notebooks", "recursive": true }),
        )
        .await
        .unwrap();
        let paths: Vec<&str> = list["paths"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(paths.iter().any(|p| p.contains("People/bob")));
        assert!(
            !paths
                .iter()
                .any(|p| p.contains("carol") || p.contains("Business"))
        );
        assert_eq!(list["count"], paths.len());

        let batch =
            json!({ "paths": ["Notebooks/People/bob/goals.md", "Notebooks/Business/pricing.md"] });
        assert!(call(&t, "vault_batch_read", batch).await.is_err());
        assert!(call(&t, "vault_context", json!({})).await.is_err());
    }

    #[tokio::test]
    async fn members_write_only_inside_their_own_private_space() {
        let (_d, vault) = vault_with_notes();
        let t = scoped(&vault, "2");
        let write = |path: &str| json!({ "path": path, "content": "x", "title": "x" });
        assert!(
            call(&t, "vault_write_note", write("Notebooks/Shared/trip.md"))
                .await
                .is_err()
        );
        assert!(
            call(
                &t,
                "vault_write_note",
                write("Notebooks/People/carol/new.md")
            )
            .await
            .is_err()
        );
        assert!(
            call(&t, "vault_write_note", write("Notebooks/People/bob/new.md"))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn owner_is_unwrapped_and_unknown_gets_public_only() {
        let (_d, vault) = vault_with_notes();
        let owner = scoped(&vault, "1");
        let p = "Notebooks/People/carol/goals.md";
        assert!(
            call(&owner, "vault_read", json!({ "path": p }))
                .await
                .is_ok()
        );
        let stranger = scoped(&vault, "999");
        assert!(
            call(
                &stranger,
                "vault_read",
                json!({ "path": "Notebooks/Shared/trip.md" })
            )
            .await
            .is_err()
        );
    }
}
