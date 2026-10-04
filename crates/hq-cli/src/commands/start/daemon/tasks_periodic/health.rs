use anyhow::Result;
use hq_db::Database;
use hq_llm::LlmProvider;
use std::path::Path;
use std::sync::Arc;

use super::super::helpers::*;

const HEARTBEAT_PENDING_HEADING: &str = "## Pending";

/// Everything from `## Pending` onward, preserved verbatim across
/// regenerations. Without this, an agent has nowhere durable to record "awaiting a reply"
/// or similar: this file is otherwise blind-overwritten every 5 minutes.
fn preserved_pending(existing: &str) -> String {
    let Some(idx) = existing.find(HEARTBEAT_PENDING_HEADING) else {
        return format!("{HEARTBEAT_PENDING_HEADING}\n\n(nothing pending)\n");
    };
    existing[idx..].trim_end().to_string() + "\n"
}

/// Update HEARTBEAT.md with daemon status.
pub async fn run_heartbeat(vault_path: &Path) -> Result<()> {
    let sys_dir = vault_path.join("_system");
    ensure_dir(&sys_dir);
    let heartbeat_path = sys_dir.join("HEARTBEAT.md");
    let existing = std::fs::read_to_string(&heartbeat_path).unwrap_or_default();
    let pending = preserved_pending(&existing);
    let now = chrono::Utc::now().to_rfc3339();
    let uptime_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let content = format!(
        "---\nstatus: alive\nlast_heartbeat: {now}\nruntime: rust\n---\n\n\
         # Heartbeat\n\nDaemon is alive.\n\n\
         - **Last heartbeat**: {now}\n\
         - **System uptime**: {}h {}m\n\
         - **Components**: daemon, agent-worker\n\n\
         {pending}",
        uptime_secs / 3600,
        (uptime_secs % 3600) / 60
    );
    std::fs::write(heartbeat_path, content)?;
    Ok(())
}

/// Run the gated memory consolidation cycle.
/// Delegates to the real implementation in hq-daemon which uses hq-memory internally.
pub async fn run_memory_consolidation(vault_path: &Path, db: &Database) -> Result<()> {
    let vault_path_buf = vault_path.to_path_buf();
    let router = Arc::new(hq_llm::router::LlmRouter::from_env()) as Arc<dyn LlmProvider>;
    match hq_daemon::run_memory_cycle(db, &vault_path_buf, Some(router)).await {
        Ok(()) => {
            tracing::debug!("memory-consolidation: cycle complete");
        }
        Err(e) => {
            // An unconfigured or unreachable LLM provider is routine here, so it only logs.
            tracing::debug!(error = %e, "memory-consolidation: cycle skipped or failed");
        }
    }

    Ok(())
}

#[cfg(test)]
mod heartbeat_tests {
    use super::*;

    #[test]
    fn preserves_pending_section_across_regeneration() {
        let existing = "# Heartbeat\n\nDaemon is alive.\n\n## Pending\n- waiting on Alex's reply about FR-011\n";
        let pending = preserved_pending(existing);
        assert!(pending.starts_with(HEARTBEAT_PENDING_HEADING));
        assert!(pending.contains("FR-011"));
    }

    #[test]
    fn missing_pending_section_gets_placeholder() {
        let pending = preserved_pending("# Heartbeat\n\nDaemon is alive.\n");
        assert!(pending.contains(HEARTBEAT_PENDING_HEADING));
        assert!(pending.contains("nothing pending"));
    }
}
