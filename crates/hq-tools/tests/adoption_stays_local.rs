//! Adoption data (what the person uses HQ for, how often it had to be corrected, whether they
//! come back) stays on the device. The code that reads or reports it must have no way out: no
//! HTTP client, no model, no relay, no web push, no sockets, no child processes. This test reads
//! those files and fails if one of them reaches for any of it, so a later change that wants to
//! send this data somewhere has to delete the guard first and say why in review.
//! Policy: `docs/security/ADOPTION_DATA.md`.

const FILES: &[(&str, &str)] = &[
    (
        "hq-db/src/adoption.rs",
        include_str!("../../hq-db/src/adoption.rs"),
    ),
    (
        "hq-tools/src/capabilities/mod.rs",
        include_str!("../src/capabilities/mod.rs"),
    ),
    (
        "hq-tools/src/capabilities/adoption.rs",
        include_str!("../src/capabilities/adoption.rs"),
    ),
    (
        "hq-tools/src/capabilities/offers.rs",
        include_str!("../src/capabilities/offers.rs"),
    ),
    (
        "hq-cli/src/commands/adoption.rs",
        include_str!("../../hq-cli/src/commands/adoption.rs"),
    ),
    (
        "hq-cli/.../adoption_digest.rs",
        include_str!("../../hq-cli/src/commands/start/daemon/tasks_periodic/adoption_digest.rs"),
    ),
    (
        "hq-web/src/capabilities_api.rs",
        include_str!("../../hq-web/src/capabilities_api.rs"),
    ),
];

/// Ways out of the device, as they would appear in Rust source.
const WAYS_OUT: &[&str] = &[
    "reqwest",
    "hyper::",
    "hq_llm",
    "hq_relay",
    "hq_memory",
    "web_push",
    "notify_owner",
    "std::net",
    "tokio::net",
    "Command::new",
    "remote_mcp",
];

#[test]
fn adoption_code_has_no_way_off_the_device() {
    let mut found = Vec::new();
    for (name, src) in FILES {
        for way in WAYS_OUT {
            if src.contains(way) {
                found.push(format!("{name}: {way}"));
            }
        }
    }
    assert!(found.is_empty(), "adoption code reaches out: {found:?}");
}
