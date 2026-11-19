//! Every tool that changes something has been looked at and given a zone. Zone B and C tools are
//! named in `governance::owner_approval`; a tool that changes things and is in neither list must
//! be on `zone_a_tools.txt`, which someone adds to on purpose. A new tool that is in none of the
//! lists fails here, so it cannot ship without a decision about whether it needs the owner.

use std::collections::BTreeSet;
use std::sync::Arc;

use hq_agent::governance::{Zone, zone_of};
use hq_mcp::registry::create_default_registry;

const REVIEWED_ZONE_A: &str = include_str!("zone_a_tools.txt");

fn registry() -> hq_tools::registry::ToolRegistry {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(hq_db::Database::open_memory().unwrap());
    let vault = Arc::new(hq_vault::VaultClient::new(dir.path().to_path_buf()).unwrap());
    create_default_registry(
        vault,
        db,
        dir.path().join("skills"),
        dir.path().join("Agents"),
        None,
    )
}

fn reviewed() -> BTreeSet<&'static str> {
    REVIEWED_ZONE_A
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect()
}

#[test]
fn every_tool_that_changes_things_has_a_zone() {
    let registry = registry();
    let known = reviewed();
    let unclassified: Vec<String> = registry
        .list()
        .into_iter()
        .map(|t| t.name)
        .filter(|name| registry.get(name).is_some_and(|t| !t.is_read_only()))
        .filter(|name| zone_of(name) == Zone::A && !known.contains(name.as_str()))
        .collect();
    assert!(
        unclassified.is_empty(),
        "these tools change things and have no zone. Decide whether the owner must approve them: add them to ZONE_B_TOOLS or ZONE_C_TOOLS, or to crates/hq-cli/tests/zone_a_tools.txt: {unclassified:?}"
    );
}

#[test]
fn the_reviewed_list_names_only_tools_that_exist_and_none_that_ask() {
    let registry = registry();
    let stale: Vec<&str> = reviewed()
        .into_iter()
        .filter(|n| registry.get(n).is_none() && !n.starts_with('#'))
        .collect();
    let asking: Vec<&str> = reviewed()
        .into_iter()
        .filter(|n| zone_of(n) != Zone::A)
        .collect();
    // The suite and gws tools exist only with their features, so a missing name is not an error
    // there; anything else on the list must be registered.
    let optional =
        |n: &&str| n.starts_with("suite_") || n.starts_with("office") || n.starts_with("gws");
    let stale: Vec<&str> = stale.into_iter().filter(|n| !optional(n)).collect();
    assert!(
        stale.is_empty(),
        "on the list but not registered: {stale:?}"
    );
    assert!(
        asking.is_empty(),
        "on the zone A list but asked about: {asking:?}"
    );
}

#[test]
#[ignore = "prints the current candidates; run with --ignored --nocapture to regenerate the list"]
fn print_the_unclassified_tools() {
    let registry = registry();
    let mut names: Vec<String> = registry
        .list()
        .into_iter()
        .map(|t| t.name)
        .filter(|n| registry.get(n).is_some_and(|t| !t.is_read_only()) && zone_of(n) == Zone::A)
        .collect();
    names.sort();
    println!("ZONE_A_START\n{}\nZONE_A_END", names.join("\n"));
}
