use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::Path;

use crate::file_edit::{FileHistory, FileStateCache};
use crate::registry::HqTool;

pub struct BatchEditTool {
    state_cache: FileStateCache,
    history: FileHistory,
}

impl BatchEditTool {
    pub fn new(state_cache: FileStateCache, history: FileHistory) -> Self {
        Self {
            state_cache,
            history,
        }
    }
}

#[async_trait]
impl HqTool for BatchEditTool {
    fn name(&self) -> &str {
        "file_edit_batch"
    }

    fn description(&self) -> &str {
        "Apply multiple file edits in one call. Each edit is a string replacement. \
         If any edit fails, all changes are rolled back. Use this for multi-file refactors."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["edits"],
            "properties": {
                "edits": {
                    "type": "array",
                    "description": "Array of edits to apply",
                    "items": {
                        "type": "object",
                        "required": ["file_path", "old_string", "new_string"],
                        "properties": {
                            "file_path": {
                                "type": "string",
                                "description": "Absolute path to the file"
                            },
                            "old_string": {
                                "type": "string",
                                "description": "Text to find"
                            },
                            "new_string": {
                                "type": "string",
                                "description": "Replacement text"
                            },
                            "replace_all": {
                                "type": "boolean",
                                "description": "Replace all occurrences (default: false)"
                            }
                        }
                    }
                }
            }
        })
    }

    fn category(&self) -> &str {
        "coding"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let edits = args["edits"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing edits array"))?;

        if edits.is_empty() {
            return Ok(json!({"error": "edits array is empty"}));
        }

        // Validate all edits first (fail-fast). `content` here is only used
        // to check occurrences up front; the apply phase below re-reads each
        // file immediately before writing it, so a stale copy here can never
        // silently overwrite a change made since validation.
        let mut validated = Vec::new();
        for (i, edit) in edits.iter().enumerate() {
            let file_path = edit["file_path"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("edit[{}]: missing file_path", i))?;
            let old_string = edit["old_string"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("edit[{}]: missing old_string", i))?;
            let new_string = edit["new_string"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("edit[{}]: missing new_string", i))?;
            let replace_all = edit
                .get("replace_all")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let path = Path::new(file_path);
            if !path.exists() {
                return Ok(json!({"error": format!("edit[{}]: file not found: {}", i, file_path)}));
            }

            if old_string == new_string {
                return Ok(
                    json!({"error": format!("edit[{}]: old_string == new_string (no-op)", i)}),
                );
            }

            let content = tokio::fs::read_to_string(path).await?;
            let occurrences = content.matches(old_string).count();

            if occurrences == 0 {
                return Ok(json!({
                    "error": format!("edit[{}]: old_string not found in {}", i, file_path)
                }));
            }

            if !replace_all && occurrences > 1 {
                return Ok(json!({
                    "error": format!("edit[{}]: old_string found {} times in {} — ambiguous", i, occurrences, file_path)
                }));
            }

            validated.push((
                file_path.to_string(),
                old_string.to_string(),
                new_string.to_string(),
                replace_all,
            ));
        }

        // Snapshot all files for rollback
        for (file_path, _, _, _) in &validated {
            self.history.snapshot(Path::new(file_path));
        }

        // Apply each edit, re-reading right before the write so a file
        // touched twice in one batch (or by something else mid-batch) is
        // always edited against its current content, not a stale copy
        // captured during validation.
        let mut results = Vec::new();
        let mut applied_paths: Vec<String> = Vec::new();
        let mut failure: Option<String> = None;

        for (file_path, old_string, new_string, replace_all) in &validated {
            let path = Path::new(file_path);

            if failure.is_some() {
                results.push(json!({
                    "file_path": file_path,
                    "status": "failed",
                    "reason": "skipped: an earlier edit in this batch failed",
                }));
                continue;
            }

            let fresh = match tokio::fs::read_to_string(path).await {
                Ok(c) => c,
                Err(e) => {
                    let reason = format!("re-read before write failed: {e}");
                    failure = Some(format!("{file_path}: {reason}"));
                    results.push(json!({"file_path": file_path, "status": "failed", "reason": reason}));
                    continue;
                }
            };
            let occurrences = fresh.matches(old_string.as_str()).count();
            if occurrences == 0 {
                let reason = "old_string no longer present (file changed since validation)";
                failure = Some(format!("{file_path}: {reason}"));
                results.push(json!({"file_path": file_path, "status": "failed", "reason": reason}));
                continue;
            }
            // Same uniqueness guarantee validation enforced against the
            // pre-batch content: a prior edit in this same batch touching
            // this file can turn a single pre-batch occurrence into several
            // (e.g. two edits that each introduce the other's old_string),
            // and replacen(1) would then silently rewrite whichever
            // occurrence happens to be first, not the one this edit meant.
            if !*replace_all && occurrences > 1 {
                let reason = format!(
                    "old_string now matches {occurrences} times (an earlier edit in this batch made it ambiguous) — ambiguous"
                );
                failure = Some(format!("{file_path}: {reason}"));
                results.push(json!({"file_path": file_path, "status": "failed", "reason": reason}));
                continue;
            }

            let new_content = if *replace_all {
                fresh.replace(old_string.as_str(), new_string.as_str())
            } else {
                fresh.replacen(old_string.as_str(), new_string.as_str(), 1)
            };

            if let Err(e) = tokio::fs::write(path, &new_content).await {
                let reason = format!("write failed: {e}");
                failure = Some(format!("{file_path}: {reason}"));
                results.push(json!({"file_path": file_path, "status": "failed", "reason": reason}));
                continue;
            }

            // Read back and verify the write actually landed before counting
            // it as applied — a `tokio::fs::write` that returns `Ok` is not
            // proof the bytes on disk match what was requested.
            match tokio::fs::read_to_string(path).await {
                Ok(verify) if verify == new_content => {
                    self.state_cache.record_write(path);
                    applied_paths.push(file_path.clone());
                    results.push(json!({
                        "file_path": file_path,
                        "status": "applied",
                        "replacements": if *replace_all { occurrences } else { 1 },
                    }));
                }
                Ok(_) => {
                    let reason = "read-back mismatch after write";
                    failure = Some(format!("{file_path}: {reason}"));
                    results.push(json!({"file_path": file_path, "status": "failed", "reason": reason}));
                }
                Err(e) => {
                    let reason = format!("read-back failed: {e}");
                    failure = Some(format!("{file_path}: {reason}"));
                    results.push(json!({"file_path": file_path, "status": "failed", "reason": reason}));
                }
            }
        }

        // If anything failed, roll every already-applied file in this batch
        // back to its pre-edit snapshot — making the "all changes are rolled
        // back" claim in this tool's description actually true. `history`'s
        // snapshot store is capacity-bounded (shared session-wide with
        // `file_edit`), so a big batch or a long session can evict an early
        // snapshot before rollback needs it — that must surface as a
        // distinct status, never silently leave the entry as "applied"
        // while the file is, in fact, still sitting in its edited state.
        if let Some(ref err) = failure {
            for path_str in &applied_paths {
                let path = Path::new(path_str);
                let Some(entry) = results
                    .iter_mut()
                    .find(|r| r["file_path"] == *path_str && r["status"] == "applied")
                else {
                    continue;
                };
                match self.history.rollback(path) {
                    Some(prev) => match tokio::fs::write(path, &prev).await {
                        Ok(()) => {
                            // Same reasoning as the forward write's own
                            // read-back a few lines up: `Ok(())` from
                            // `tokio::fs::write` isn't proof the restored
                            // bytes are actually on disk. The rollback claim
                            // this whole branch exists to make true deserves
                            // the same verification the write it's undoing got.
                            match tokio::fs::read_to_string(path).await {
                                Ok(verify) if verify == prev => {
                                    self.state_cache.record_write(path);
                                    entry["status"] = json!("rolled_back");
                                }
                                Ok(_) => {
                                    entry["status"] = json!("rollback_failed");
                                    entry["reason"] = json!(
                                        "rollback read-back mismatch; file left in an unknown state"
                                    );
                                }
                                Err(e) => {
                                    entry["status"] = json!("rollback_failed");
                                    entry["reason"] = json!(format!(
                                        "rollback read-back failed: {e}; file left in an unknown state"
                                    ));
                                }
                            }
                        }
                        Err(e) => {
                            entry["status"] = json!("rollback_failed");
                            entry["reason"] =
                                json!(format!("rollback write failed: {e}; file left in its edited state"));
                        }
                    },
                    None => {
                        entry["status"] = json!("rollback_failed");
                        entry["reason"] = json!(
                            "no snapshot available to roll back to (evicted by this batch's own \
                             capacity limit, or by other edits earlier in the session); file left \
                             in its edited state"
                        );
                    }
                }
            }
            return Ok(json!({
                "success": false,
                "edits_applied": 0,
                "results": results,
                "error": err,
            }));
        }

        Ok(json!({
            "success": true,
            "edits_applied": results.len(),
            "results": results
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> BatchEditTool {
        BatchEditTool::new(FileStateCache::new(), FileHistory::new(10, 1024 * 1024))
    }

    #[tokio::test]
    async fn a_failed_edit_rolls_back_edits_already_applied_in_the_batch() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, "hello world").unwrap();
        std::fs::write(&b, "irrelevant content").unwrap();
        // Passes validation (file exists, old_string present) but fails at
        // write time, which is the apply-phase failure this test targets.
        std::fs::set_permissions(&b, std::fs::Permissions::from_mode(0o444)).unwrap();

        let result = tool()
            .execute(json!({
                "edits": [
                    {"file_path": a.to_str().unwrap(), "old_string": "hello", "new_string": "goodbye"},
                    {"file_path": b.to_str().unwrap(), "old_string": "irrelevant", "new_string": "x"},
                ]
            }))
            .await
            .unwrap();

        std::fs::set_permissions(&b, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(result["success"], json!(false), "{result}");
        assert_eq!(
            std::fs::read_to_string(&a).unwrap(),
            "hello world",
            "edit applied before the later failure must be rolled back on disk"
        );
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "irrelevant content");
    }

    #[tokio::test]
    async fn evicted_snapshot_reports_rollback_failed_instead_of_silently_staying_applied() {
        use std::os::unix::fs::PermissionsExt;

        // FileHistory holds only 1 snapshot at a time — a batch touching 3
        // files evicts the earliest snapshots before the apply loop even
        // starts, so rollback for those files has nothing to restore from.
        let tool = BatchEditTool::new(FileStateCache::new(), FileHistory::new(1, 1024 * 1024));
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        let c = dir.path().join("c.txt");
        std::fs::write(&a, "hello").unwrap();
        std::fs::write(&b, "hello").unwrap();
        std::fs::write(&c, "hello").unwrap();
        std::fs::set_permissions(&c, std::fs::Permissions::from_mode(0o444)).unwrap();

        let result = tool
            .execute(json!({
                "edits": [
                    {"file_path": a.to_str().unwrap(), "old_string": "hello", "new_string": "A2"},
                    {"file_path": b.to_str().unwrap(), "old_string": "hello", "new_string": "B2"},
                    {"file_path": c.to_str().unwrap(), "old_string": "hello", "new_string": "C2"},
                ]
            }))
            .await
            .unwrap();

        std::fs::set_permissions(&c, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(result["success"], json!(false), "{result}");
        let a_status = result["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["file_path"] == json!(a.to_str().unwrap()))
            .unwrap()["status"]
            .clone();
        assert_eq!(
            a_status,
            json!("rollback_failed"),
            "must not report 'applied' for a file whose snapshot was evicted and so was never actually rolled back"
        );
        assert_eq!(
            std::fs::read_to_string(&a).unwrap(),
            "A2",
            "the response's status must match reality: this file genuinely was not restored"
        );
    }

    #[tokio::test]
    async fn same_file_edited_twice_in_one_batch_applies_both_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f.txt");
        std::fs::write(&f, "one two").unwrap();

        let result = tool()
            .execute(json!({
                "edits": [
                    {"file_path": f.to_str().unwrap(), "old_string": "one", "new_string": "ONE"},
                    {"file_path": f.to_str().unwrap(), "old_string": "two", "new_string": "TWO"},
                ]
            }))
            .await
            .unwrap();

        assert_eq!(result["success"], json!(true));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "ONE TWO");
    }

    #[tokio::test]
    async fn an_earlier_edit_creating_ambiguity_for_a_later_one_is_rejected() {
        // Regression: validation rejects `!replace_all && occurrences > 1`
        // against the pre-batch content, but the apply loop's re-read
        // (added for the stale-content fix) skipped that same check against
        // the *current* content. Two edits that are each individually
        // unambiguous pre-batch can still collide: applying "alpha"->"beta"
        // first turns the second edit's single "beta" into two, and
        // replacen(1) would then silently rewrite the wrong one.
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f.txt");
        std::fs::write(&f, "alpha\nbeta\n").unwrap();

        let result = tool()
            .execute(json!({
                "edits": [
                    {"file_path": f.to_str().unwrap(), "old_string": "alpha", "new_string": "beta"},
                    {"file_path": f.to_str().unwrap(), "old_string": "beta", "new_string": "gamma"},
                ]
            }))
            .await
            .unwrap();

        assert_eq!(result["success"], json!(false), "{result}");
        // The batch must not silently land a wrong-occurrence rewrite —
        // either the second edit is reported failed/ambiguous, or the
        // whole batch (including the first edit) is rolled back. Either
        // way, the file must not end up in the "gamma\nbeta\n" or
        // "beta\ngamma\n" shape a naive replacen(1) would produce.
        let content = std::fs::read_to_string(&f).unwrap();
        assert_ne!(content, "gamma\nbeta\n", "{result}");
        assert_ne!(content, "beta\ngamma\n", "{result}");
    }

    #[tokio::test]
    async fn successful_batch_reports_applied_status_per_edit() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f.txt");
        std::fs::write(&f, "hello").unwrap();

        let result = tool()
            .execute(json!({
                "edits": [
                    {"file_path": f.to_str().unwrap(), "old_string": "hello", "new_string": "bye"},
                ]
            }))
            .await
            .unwrap();

        assert_eq!(result["results"][0]["status"], json!("applied"));
    }
}
