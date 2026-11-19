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
            // The HQ Suite's tool exists only in a build with the `suite` feature.
            let suite_tool = [
                hq_tool_api::office::OFFICE_TOOL,
                "office_create",
                "office_export",
            ];
            let optional = suite_tool.contains(&tool) && !cfg!(feature = "suite");
            !optional && registry.get(tool).is_none()
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

fn default_registry() -> hq_tools::registry::ToolRegistry {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(hq_db::Database::open_memory().unwrap());
    let vault_client = Arc::new(hq_vault::VaultClient::new(dir.path().to_path_buf()).unwrap());
    create_default_registry(
        vault_client,
        db,
        dir.path().join("skills"),
        dir.path().join("Agents"),
        None,
    )
}

/// Tool categories nobody lists in the catalog on purpose: plumbing, agent-only and owner-only
/// tools. A category leaves this list when a catalog entry covers it; it joins only with a reason
/// in review.
const NOT_IN_THE_CATALOG: &str = include_str!("catalog_exceptions.txt");

/// Every tool category in the default registry is either offered in the catalog (an entry names a
/// tool of that category) or on the reviewed exceptions list, so a new feature cannot ship that
/// the empty chat never mentions without someone deciding that is right. The list can only shrink.
#[test]
fn every_tool_category_is_in_the_catalog_or_a_reviewed_exception() {
    let registry = default_registry();
    let catalogued: std::collections::BTreeSet<String> = hq_tools::capabilities::catalog()
        .iter()
        .flat_map(|c| c.tools.iter())
        .filter_map(|t| registry.get(t).map(|tool| tool.category().to_string()))
        .collect();
    let excepted: std::collections::BTreeSet<&str> = NOT_IN_THE_CATALOG
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    let categories: std::collections::BTreeSet<String> =
        registry.categories().into_iter().collect();
    let missing: Vec<&String> = categories
        .iter()
        .filter(|c| !catalogued.contains(*c) && !excepted.contains(c.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "these tool categories have no catalog entry and no reviewed exception (add an entry to crates/hq-tools/assets/capabilities.yaml, or the category to crates/hq-mcp/tests/catalog_exceptions.txt with a reason in the PR): {missing:?}"
    );
    let stale: Vec<&&str> = excepted
        .iter()
        .filter(|c| catalogued.contains(**c) || !categories.contains(**c))
        .collect();
    assert!(
        stale.is_empty(),
        "on the exceptions list but covered or gone, remove them: {stale:?}"
    );
}

#[test]
#[ignore = "prints the categories to put on the exceptions list; run with --ignored --nocapture"]
fn print_the_uncatalogued_categories() {
    let registry = default_registry();
    let catalogued: std::collections::BTreeSet<String> = hq_tools::capabilities::catalog()
        .iter()
        .flat_map(|c| c.tools.iter())
        .filter_map(|t| registry.get(t).map(|tool| tool.category().to_string()))
        .collect();
    for c in registry
        .categories()
        .into_iter()
        .filter(|c| !catalogued.contains(c))
    {
        println!("EXC {c}");
    }
}
