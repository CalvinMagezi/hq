//! `vault_health`: ask whether the notes are in good order, or one note just written.

use anyhow::Result;
use async_trait::async_trait;
use hq_db::Database;
use hq_vault::VaultClient;
use serde_json::{Value, json};
use std::sync::Arc;

use super::validate_path_in_vault;
use crate::registry::HqTool;

/// Reads every note and reports links that reach nothing, links that fit more than one note,
/// links to a heading that is not there, notes with the same text and properties that do not
/// read. It changes nothing: repairs are the owner's to approve, from the app or `hq vault
/// health --apply`, not a tool an agent can call.
pub struct VaultHealthTool {
    vault: Arc<VaultClient>,
    db: Option<Arc<Database>>,
}

impl VaultHealthTool {
    /// `db` is only for what the owner chose to ignore; without it nothing is ignored.
    pub fn new(vault: Arc<VaultClient>, db: Option<Arc<Database>>) -> Self {
        Self { vault, db }
    }
}

#[async_trait]
impl HqTool for VaultHealthTool {
    fn is_read_only(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "vault_health"
    }

    fn description(&self) -> &str {
        "Check the notes for broken links, links that fit several notes, links to a missing heading, \
         duplicate notes and unreadable properties. Give a note path to check only that note, for \
         example one you just wrote. Reports only; it changes nothing."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "A note path to check on its own. Leave out to check all notes."
                }
            }
        })
    }

    fn category(&self) -> &str {
        "vault"
    }

    fn checks_arguments(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string);
        if let Some(p) = &path {
            validate_path_in_vault(self.vault.vault_path(), p)?;
        }
        let vault_path = self.vault.vault_path().to_path_buf();
        let db = self.db.clone();

        let report = tokio::task::spawn_blocking(move || -> Result<_> {
            let ignored = match &db {
                Some(db) => crate::vault_health::load_ignored(db)?,
                None => Default::default(),
            };
            let mut report = crate::vault_health::scan_full_now(&vault_path, &ignored);
            match path {
                Some(p) => {
                    report.only_note(&hq_vault::rel_path::RelPath::parse(&p)?.normalized());
                }
                None => report.cap(),
            }
            Ok(report)
        })
        .await??;

        let healthy = report.counts.problems() == 0;
        let mut out = serde_json::to_value(&report)?;
        out["healthy"] = json!(healthy);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> (tempfile::TempDir, Arc<VaultClient>) {
        let dir = tempfile::tempdir().unwrap();
        for (rel, text) in [
            (
                "Notebooks/Quarterly Review.md",
                "the review, with enough words to be a real note",
            ),
            (
                "Notebooks/A.md",
                "[[Quarterly Reveiw]] and a real [[Quarterly Review]], long enough",
            ),
            (
                "Notebooks/B.md",
                "[[Nowhere]] is not a note, and this one is long enough to count",
            ),
        ] {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let vault = Arc::new(VaultClient::new(dir.path().to_path_buf()).unwrap());
        (dir, vault)
    }

    #[tokio::test]
    async fn the_whole_vault_is_reported_with_where_each_problem_is() {
        let (_dir, vault) = vault();
        let out = VaultHealthTool::new(vault, None)
            .execute(json!({}))
            .await
            .unwrap();
        assert_eq!(out["healthy"], false);
        assert_eq!(out["counts"]["links"]["dead"], 2);
        assert_eq!(out["dead_links"][0]["note"], "Notebooks/A.md");
        assert_eq!(out["dead_links"][0]["did_you_mean"], "Quarterly Review");
    }

    #[tokio::test]
    async fn one_note_can_be_checked_on_its_own() {
        let (_dir, vault) = vault();
        let tool = VaultHealthTool::new(vault, None);
        let out = tool
            .execute(json!({ "path": "Notebooks/B.md" }))
            .await
            .unwrap();
        assert_eq!(out["counts"]["links"]["dead"], 1);
        assert_eq!(out["dead_links"][0]["link"], "[[Nowhere]]");
        let clean = tool
            .execute(json!({ "path": "Notebooks/Quarterly Review.md" }))
            .await
            .unwrap();
        assert_eq!(clean["healthy"], true);
    }

    #[tokio::test]
    async fn a_path_outside_the_vault_is_refused() {
        let (_dir, vault) = vault();
        let err = VaultHealthTool::new(vault, None)
            .execute(json!({ "path": "../secret.md" }))
            .await;
        assert!(err.is_err());
    }

    #[test]
    fn it_is_read_only_and_checks_its_arguments() {
        let (_dir, vault) = vault();
        let tool = VaultHealthTool::new(vault, None);
        assert!(tool.is_read_only());
        assert!(tool.checks_arguments());
    }
}
