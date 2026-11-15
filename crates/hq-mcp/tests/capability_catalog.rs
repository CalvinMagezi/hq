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

/// A connector asks approval only for tools in its confirm list and never writes through a
/// read-only tool, so the context tools must exist, say they only read, and stay off that list.
#[test]
fn the_context_tools_are_registered_read_only_and_need_no_approval() {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(hq_db::Database::open_memory().unwrap());
    let vault_client = Arc::new(hq_vault::VaultClient::new(dir.path().to_path_buf()).unwrap());
    let registry = create_default_registry(
        vault_client,
        db,
        dir.path().join("skills"),
        dir.path().join("Agents"),
        None,
    );
    for name in [
        "context_orient",
        "context_search",
        "context_read",
        "context_expand",
        "context_related",
    ] {
        let tool = registry
            .get(name)
            .unwrap_or_else(|| panic!("{name} missing"));
        assert!(tool.is_read_only(), "{name}");
        assert!(
            !hq_mcp::gateway::needs_owner_confirmation(&registry, name, &serde_json::json!({})),
            "{name}"
        );
    }
}
