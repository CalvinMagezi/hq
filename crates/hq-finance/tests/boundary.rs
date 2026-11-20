//! Finance data must have no code path to a model, an index or a sync layer. The cheapest proof
//! is that this crate cannot name them: it depends on none of the crates that can.

const FORBIDDEN: &[&str] = &[
    "hq-llm",
    "hq-memory",
    "hq-vault",
    "hq-db",
    "hq-agent",
    "hq-tools",
    "hq-web",
    "hq-relay",
    "hq-bridge",
    "hq-mcp",
    "reqwest",
    "hyper",
    "ureq",
    "tokio",
    "async-openai",
];

#[test]
fn manifest_has_no_path_to_a_model_or_the_network() {
    let manifest = include_str!("../Cargo.toml");
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .expect("has dependencies");
    for name in FORBIDDEN {
        let named = deps.lines().any(|l| l.trim_start().starts_with(name));
        assert!(!named, "{name} must not be a dependency");
    }
}
