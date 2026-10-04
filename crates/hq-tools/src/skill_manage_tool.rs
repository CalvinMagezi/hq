//! Agent-callable skill lifecycle tool — create, edit, and delete skills.
//!
//! Writes are gated by `governance.skills_write_approval`. When approval is
//! required, all mutations stage under `<skills_dir>/_proposed/<name>/` instead
//! of taking effect immediately.

use std::path::PathBuf;

use anyhow::{Result, bail};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::registry::HqTool;
use crate::skill_audit::{Disposition, archive_current, audit_live, delete_live_skill, keep_if_valid};
use crate::skills::{SkillDefinition, SkillProvenance, render_skill};

const ALLOWED_FILES: &[&str] = &["SKILL.md", "SUMMARY.md", "README.md"];

/// Create, edit, or delete skills under the configured skills directory.
pub struct SkillManageTool {
    skills_dir: PathBuf,
    write_approval: bool,
}

impl SkillManageTool {
    pub fn new(skills_dir: PathBuf, write_approval: bool) -> Self {
        Self {
            skills_dir,
            write_approval,
        }
    }

    fn validate_name(name: &str) -> Result<()> {
        if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
            bail!("invalid skill name: must be non-empty and cannot contain /, \\, or ..");
        }
        Ok(())
    }

    fn validate_file(file: &str) -> Result<()> {
        if file.contains('/') || file.contains('\\') || file.contains("..") {
            bail!("invalid file path: must be a basename");
        }
        if !ALLOWED_FILES.contains(&file) {
            bail!("invalid file: allowed values are {:?}", ALLOWED_FILES);
        }
        Ok(())
    }

    fn skill_dir(&self, name: &str) -> PathBuf {
        if self.write_approval {
            self.skills_dir.join("_proposed").join(name)
        } else {
            self.skills_dir.join(name)
        }
    }

    /// Live writes archive the prior SKILL.md so `hq skills revert` can undo them.
    fn archive_if_live(&self, name: &str) -> Result<Option<PathBuf>> {
        if self.write_approval {
            return Ok(None);
        }
        archive_current(&self.skills_dir, name)
    }

    /// A live SKILL.md that fails validation is rolled back before it is audited.
    fn keep_and_audit(&self, name: &str, archived: Option<PathBuf>, action: &str) -> Result<()> {
        if self.write_approval {
            return audit_live(&self.skills_dir, name, Disposition::Held, &format!("skill_manage {action}"));
        }
        keep_if_valid(&self.skills_dir, name, archived)?;
        audit_live(&self.skills_dir, name, Disposition::Adopted, &format!("skill_manage {action}"))
    }

    async fn create(&self, name: &str, content: &str) -> Result<Value> {
        Self::validate_name(name)?;
        if content.is_empty() {
            bail!("content is required for create");
        }

        let dir = self.skill_dir(name);
        tokio::fs::create_dir_all(&dir).await?;
        let path = dir.join("SKILL.md");

        let body = if content.starts_with("---") {
            content.to_string()
        } else {
            let description = content
                .lines()
                .next()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .unwrap_or(name)
                .to_string();
            let skill = SkillDefinition {
                name: name.to_string(),
                description,
                auto_load: false,
                load_full: false,
                hints: Vec::new(),
                next_skills: Vec::new(),
                bundle_only: false,
                requires_bins: Vec::new(),
                context_need: None,
                provenance: SkillProvenance::default(),
                content: content.to_string(),
            };
            render_skill(&skill)
        };

        let archived = self.archive_if_live(name)?;
        tokio::fs::write(&path, body).await?;
        self.keep_and_audit(name, archived, "create")?;
        Ok(json!({
            "status": "created",
            "path": path.to_string_lossy()
        }))
    }

    async fn write_file(&self, name: &str, file: &str, content: &str) -> Result<Value> {
        Self::validate_name(name)?;
        Self::validate_file(file)?;
        if content.is_empty() {
            bail!("content is required for write_file");
        }

        let dir = self.skill_dir(name);
        tokio::fs::create_dir_all(&dir).await?;
        let path = dir.join(file);
        if file != "SKILL.md" {
            tokio::fs::write(&path, content).await?;
            return Ok(json!({ "status": "written", "path": path.to_string_lossy() }));
        }
        let archived = self.archive_if_live(name)?;
        tokio::fs::write(&path, content).await?;
        self.keep_and_audit(name, archived, "write_file")?;
        Ok(json!({
            "status": "written",
            "path": path.to_string_lossy()
        }))
    }

    async fn remove_file(&self, name: &str, file: &str) -> Result<Value> {
        Self::validate_name(name)?;
        Self::validate_file(file)?;

        let dir = self.skill_dir(name);
        let path = dir.join(file);
        if !path.exists() {
            bail!("file does not exist: {}", path.to_string_lossy());
        }
        // Removing SKILL.md deletes the skill, so it gets the same archive and audit as `delete`.
        let removes_live_skill = file == "SKILL.md" && !self.write_approval;
        if removes_live_skill {
            archive_current(&self.skills_dir, name)?;
        }
        tokio::fs::remove_file(&path).await?;
        if removes_live_skill {
            audit_live(&self.skills_dir, name, Disposition::Deleted, "skill_manage remove_file")?;
        }
        Ok(json!({
            "status": "removed",
            "path": path.to_string_lossy()
        }))
    }

    async fn delete(&self, name: &str) -> Result<Value> {
        Self::validate_name(name)?;

        let dir = self.skill_dir(name);
        if !self.write_approval {
            delete_live_skill(&self.skills_dir, name, "skill_manage delete")?;
        } else if dir.exists() {
            tokio::fs::remove_dir_all(&dir).await?;
        } else {
            bail!("skill does not exist: {}", dir.to_string_lossy());
        }
        Ok(json!({
            "status": "deleted",
            "path": dir.to_string_lossy()
        }))
    }
}

#[async_trait]
impl HqTool for SkillManageTool {
    fn name(&self) -> &str {
        "skill_manage"
    }

    fn description(&self) -> &str {
        "Create, edit, or delete skills under the skills directory. \
         When governance.skills_write_approval is true, writes stage to `_proposed/` for review."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "write_file", "remove_file", "delete"],
                    "description": "Operation to perform"
                },
                "name": {
                    "type": "string",
                    "description": "Sanitized skill directory name"
                },
                "content": {
                    "type": "string",
                    "description": "Full file content (required for create and write_file)"
                },
                "file": {
                    "type": "string",
                    "enum": ["SKILL.md", "SUMMARY.md", "README.md"],
                    "default": "SKILL.md",
                    "description": "Target file inside the skill directory (default: SKILL.md)"
                }
            },
            "required": ["action", "name"]
        })
    }

    fn category(&self) -> &str {
        "skills"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let file = args
            .get("file")
            .and_then(|v| v.as_str())
            .unwrap_or("SKILL.md");

        match action {
            "create" => self.create(name, content).await,
            "write_file" => self.write_file(name, file, content).await,
            "remove_file" => self.remove_file(name, file).await,
            "delete" => self.delete(name).await,
            _ => bail!("invalid action: {}", action),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(tmp: &std::path::Path, approval: bool) -> SkillManageTool {
        SkillManageTool::new(tmp.to_path_buf(), approval)
    }

    #[tokio::test]
    async fn creates_skill_directly_when_approval_off() {
        let tmp = tempfile::tempdir().unwrap();
        let t = tool(tmp.path(), false);
        let res = t
            .execute(json!({
                "action": "create",
                "name": "test-skill",
                "content": "# Test Skill\n\nDo the thing."
            }))
            .await
            .unwrap();
        assert_eq!(res["status"], "created");
        let path = tmp.path().join("test-skill").join("SKILL.md");
        assert!(path.exists());
        let body = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(body.contains("---"));
        assert!(body.contains("description: \"# Test Skill\""));
        assert!(body.contains("autoLoad: false"));
        assert!(body.contains("# Test Skill\n\nDo the thing."));
    }

    #[tokio::test]
    async fn stages_skill_when_approval_on() {
        let tmp = tempfile::tempdir().unwrap();
        let t = tool(tmp.path(), true);
        let res = t
            .execute(json!({
                "action": "create",
                "name": "pending-skill",
                "content": "A new skill idea."
            }))
            .await
            .unwrap();
        assert_eq!(res["status"], "created");
        let path = tmp
            .path()
            .join("_proposed")
            .join("pending-skill")
            .join("SKILL.md");
        assert!(path.exists());
        assert!(!tmp.path().join("pending-skill").exists());
    }

    #[tokio::test]
    async fn write_file_and_remove_file() {
        let tmp = tempfile::tempdir().unwrap();
        let t = tool(tmp.path(), false);
        t.execute(json!({
            "action": "create",
            "name": "wf-skill",
            "content": "# WF"
        }))
        .await
        .unwrap();

        let written = t
            .execute(json!({
                "action": "write_file",
                "name": "wf-skill",
                "file": "SUMMARY.md",
                "content": "Short summary."
            }))
            .await
            .unwrap();
        assert_eq!(written["status"], "written");
        let summary_path = tmp.path().join("wf-skill").join("SUMMARY.md");
        assert!(summary_path.exists());
        assert_eq!(
            tokio::fs::read_to_string(&summary_path).await.unwrap(),
            "Short summary."
        );

        let removed = t
            .execute(json!({
                "action": "remove_file",
                "name": "wf-skill",
                "file": "SUMMARY.md"
            }))
            .await
            .unwrap();
        assert_eq!(removed["status"], "removed");
        assert!(!summary_path.exists());
    }

    #[tokio::test]
    async fn delete_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let t = tool(tmp.path(), false);
        t.execute(json!({
            "action": "create",
            "name": "dead-skill",
            "content": "# Dead"
        }))
        .await
        .unwrap();

        let deleted = t
            .execute(json!({
                "action": "delete",
                "name": "dead-skill"
            }))
            .await
            .unwrap();
        assert_eq!(deleted["status"], "deleted");
        assert!(!tmp.path().join("dead-skill/SKILL.md").exists());
        assert!(tmp.path().join("dead-skill/archive/SKILL-v1.md").exists());
    }

    #[tokio::test]
    async fn removing_skill_md_archives_and_audits_it() {
        let tmp = tempfile::tempdir().unwrap();
        let t = tool(tmp.path(), false);
        t.execute(json!({"action": "create", "name": "gone", "content": "# Gone"}))
            .await
            .unwrap();
        t.execute(json!({"action": "remove_file", "name": "gone", "file": "SKILL.md"}))
            .await
            .unwrap();
        assert!(tmp.path().join("gone/archive/SKILL-v1.md").exists());
        let last = crate::skill_audit::audit_history(tmp.path(), "gone").pop().unwrap();
        assert_eq!(last.disposition, Disposition::Deleted);
    }

    #[tokio::test]
    async fn an_invalid_live_write_is_refused_and_rolled_back() {
        let tmp = tempfile::tempdir().unwrap();
        let t = tool(tmp.path(), false);
        let broken = "---\ndescription: \"x\"\nautoLoad: true\n---\n# Never fires";

        assert!(t.execute(json!({"action": "create", "name": "bad", "content": broken})).await.is_err());
        assert!(!tmp.path().join("bad").exists());

        t.execute(json!({"action": "create", "name": "ok", "content": "# Fine"}))
            .await
            .unwrap();
        let res = t
            .execute(json!({"action": "write_file", "name": "ok", "content": broken}))
            .await;
        assert!(res.is_err());
        let live = std::fs::read_to_string(tmp.path().join("ok/SKILL.md")).unwrap();
        assert!(live.contains("# Fine"), "{live}");
        assert!(!tmp.path().join("ok/archive/SKILL-v1.md").exists(), "the archive went back into place");
    }

    #[tokio::test]
    async fn live_edits_archive_the_prior_version_and_are_audited() {
        let tmp = tempfile::tempdir().unwrap();
        let t = tool(tmp.path(), false);
        t.execute(json!({"action": "create", "name": "ed", "content": "# First"}))
            .await
            .unwrap();
        t.execute(json!({"action": "write_file", "name": "ed", "content": "---\ndescription: \"x\"\n---\n# Second"}))
            .await
            .unwrap();

        let archived = std::fs::read_to_string(tmp.path().join("ed/archive/SKILL-v1.md")).unwrap();
        assert!(archived.contains("# First"));
        let dispositions: Vec<Disposition> = crate::skill_audit::audit_history(tmp.path(), "ed")
            .into_iter()
            .map(|e| e.disposition)
            .collect();
        assert_eq!(dispositions, vec![Disposition::Adopted, Disposition::Adopted]);
    }

    #[tokio::test]
    async fn rejects_invalid_name() {
        let tmp = tempfile::tempdir().unwrap();
        let t = tool(tmp.path(), false);
        let res = t
            .execute(json!({
                "action": "create",
                "name": "foo/bar",
                "content": "x"
            }))
            .await;
        assert!(res.is_err());
    }
}
