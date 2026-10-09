use anyhow::{Context, Result};
use super::cursor_mcp_config::ServeScope;
use hq_core::config::HqConfig;
use hq_db::Database;
use hq_vault::VaultClient;
use std::sync::Arc;

/// Start the MCP stdio server (used by Claude Desktop / editors). A scope other
/// than `full` narrows it to that scope's tools, the same lists the HTTP keys use.
pub async fn run(config: &HqConfig, scope: ServeScope) -> Result<()> {
    let vault =
        Arc::new(VaultClient::new(config.vault_path.clone()).context("failed to open vault")?);

    let db_path = config.db_path();
    let db = Arc::new(Database::open(&db_path).context("failed to open database")?);

    let skills_dir = hq_core::skills_dir(&config.vault_path);
    let agents_dir = config.vault_path.join("Agents");

    let registry = hq_mcp::registry::create_default_registry(
        vault,
        db.clone(),
        skills_dir,
        agents_dir,
        Some(config),
    );
    let mut server = hq_mcp::server::HqMcpServer::new(Arc::new(registry), (*db).clone());
    if let Some(allowed) = scope.allowlist() {
        server = server.with_allowlist(allowed);
    }
    server.serve_stdio().await
}
