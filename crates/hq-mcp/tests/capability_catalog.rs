//! Every tool the capability catalog names is one the default registry registers, so an entry
//! is never hidden on a full install because of a typo or a renamed tool.

use std::sync::Arc;

use hq_mcp::registry::create_default_registry;

#[test]
fn every_catalog_tool_is_registered() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path();
    let db = Arc::new(hq_db::Database::open_memory().unwrap());
    let vault_client = Arc::new(hq_vault::VaultClient::new(vault.to_path_buf()).unwrap());
    let registry = create_default_registry(
        vault_client,
        db,
        vault.join("skills"),
        vault.join("Agents"),
        None,
    );
    let missing: Vec<String> = hq_tools::capabilities::catalog()
        .iter()
        .flat_map(|c| c.tools.iter().map(move |t| format!("{}: {t}", c.id)))
        .filter(|entry| {
            let tool = entry.split(": ").nth(1).unwrap_or_default();
            registry.get(tool).is_none()
        })
        .collect();
    assert!(missing.is_empty(), "not registered: {missing:?}");
}
