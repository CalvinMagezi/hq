//! The layering guard.
//!
//! `layering.yaml` at the repository root says which crate may depend on which, and which files
//! belong to a module that still lives inside a base crate. This test reads every `Cargo.toml`
//! and source file under `crates/` and fails when the code and that file disagree, in either
//! direction: a new dependency the file does not allow fails, and so does a debt entry the code
//! no longer needs. The base therefore cannot start depending on a module without a change to
//! `layering.yaml` that a reviewer sees, and the list of known violations can only get shorter.
//!
//! The source check counts written paths (`crate::harness_session`, `hq_tools::github::dispatch`).
//! It misses a path reached through a grouped `use` or a re-export, so it is a tripwire and not
//! a proof. The proof arrives when a module becomes its own crate and rule 1 covers it.
//!
//! Design and the order the debt is paid in: `docs/plans/modular-hq.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Layering {
    tiers: Tiers,
    #[serde(default)]
    module_edges: BTreeMap<String, Vec<String>>,
    /// Module id, then crate, then paths inside that crate.
    #[serde(default)]
    in_crate: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    #[serde(default)]
    debt: Debt,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tiers {
    contract: Vec<String>,
    kernel: Vec<String>,
    module: Vec<String>,
    app: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Debt {
    /// `"from -> to"` crate edges that break a rule today.
    #[serde(default)]
    cargo_edges: Vec<String>,
    /// `"from -> module"` and the number of source lines outside the module that name it.
    #[serde(default)]
    refs: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    Contract,
    Kernel,
    Module,
    App,
}

impl Layering {
    fn parse(text: &str) -> Self {
        serde_yaml::from_str(text).expect("layering.yaml parses")
    }

    /// Each crate's tier and its position in that tier's list.
    fn places(&self) -> BTreeMap<&str, (Tier, usize)> {
        let lists = [
            (Tier::Contract, &self.tiers.contract),
            (Tier::Kernel, &self.tiers.kernel),
            (Tier::Module, &self.tiers.module),
            (Tier::App, &self.tiers.app),
        ];
        let mut places = BTreeMap::new();
        for (tier, list) in lists {
            for (index, name) in list.iter().enumerate() {
                places.insert(name.as_str(), (tier, index));
            }
        }
        places
    }

    /// Whether `from` may depend on `to` under the rules, ignoring debt.
    fn allows(&self, from: &str, to: &str) -> bool {
        let places = self.places();
        let (Some(&(from_tier, from_at)), Some(&(to_tier, to_at))) =
            (places.get(from), places.get(to))
        else {
            return false;
        };
        match (from_tier, to_tier) {
            (Tier::Contract, Tier::Contract) | (Tier::Kernel, Tier::Kernel) => to_at < from_at,
            (Tier::Contract, _) => false,
            (Tier::Kernel, Tier::Contract) => true,
            (Tier::Kernel, _) => false,
            (Tier::Module, Tier::Contract | Tier::Kernel) => true,
            (Tier::Module, Tier::Module) => self
                .module_edges
                .get(from)
                .is_some_and(|allowed| allowed.iter().any(|name| name == to)),
            (Tier::Module, Tier::App) => false,
            (Tier::App, Tier::App) => false,
            (Tier::App, _) => true,
        }
    }
}

/// Every crate must be in exactly one tier, and every name in a tier must be a crate.
fn check_tiers(layering: &Layering, crates: &BTreeSet<String>) -> Vec<String> {
    let mut problems = Vec::new();
    let tiers = &layering.tiers;
    let listed: Vec<&String> = tiers
        .contract
        .iter()
        .chain(&tiers.kernel)
        .chain(&tiers.module)
        .chain(&tiers.app)
        .collect();
    let mut seen = BTreeSet::new();
    for name in &listed {
        if !seen.insert(name.as_str()) {
            problems.push(format!(
                "{name} is listed in more than one place under tiers"
            ));
        }
        if !crates.contains(name.as_str()) {
            problems.push(format!(
                "{name} is listed under tiers but is not a workspace crate"
            ));
        }
    }
    for name in crates {
        if !seen.contains(name.as_str()) {
            problems.push(format!(
                "{name} is a workspace crate with no tier: add it to layering.yaml (a new crate \
                 is a module unless every flavour of hq must link it)"
            ));
        }
    }
    for (from, allowed) in &layering.module_edges {
        for to in allowed {
            let places = layering.places();
            let both_modules = [from, to]
                .iter()
                .all(|name| places.get(name.as_str()).map(|p| p.0) == Some(Tier::Module));
            if !both_modules {
                problems.push(format!(
                    "module_edges lists {from} -> {to}, but both must be module crates"
                ));
            }
        }
    }
    problems
}

/// Every dependency between workspace crates must be allowed by the rules or listed as debt,
/// and every allowance that is no longer used must be removed.
fn check_edges(layering: &Layering, edges: &BTreeSet<(String, String)>) -> Vec<String> {
    let mut problems = Vec::new();
    let debt: BTreeSet<&str> = layering
        .debt
        .cargo_edges
        .iter()
        .map(String::as_str)
        .collect();
    for (from, to) in edges {
        let key = format!("{from} -> {to}");
        let allowed = layering.allows(from, to);
        if !allowed && !debt.contains(key.as_str()) {
            problems.push(format!(
                "{key} is not allowed: a crate depends only on crates below it. Invert the \
                 dependency (a trait or a registration in the lower crate) instead of adding it"
            ));
        }
        if allowed && debt.contains(key.as_str()) {
            problems.push(format!(
                "{key} is listed under debt.cargo_edges but the rules allow it: remove the entry"
            ));
        }
    }
    for key in &debt {
        let exists = edges
            .iter()
            .any(|(from, to)| format!("{from} -> {to}") == *key);
        if !exists {
            problems.push(format!(
                "{key} is listed under debt.cargo_edges but the dependency is gone: remove the \
                 entry"
            ));
        }
    }
    for (from, allowed) in &layering.module_edges {
        for to in allowed {
            if !edges.contains(&(from.clone(), to.clone())) {
                problems.push(format!(
                    "module_edges allows {from} -> {to} but the dependency is gone: remove it"
                ));
            }
        }
    }
    problems
}

/// One source file: the crate it is in, its path inside the crate, and its text.
struct Source {
    krate: String,
    path: String,
    text: String,
}

/// `src/github/dispatch.rs` and `src/harness_session` as Rust paths inside their crate.
fn rust_path(path: &str) -> String {
    let inner = path
        .strip_prefix("src/")
        .unwrap_or_else(|| panic!("in_crate path {path} must start with src/"));
    let inner = inner.strip_suffix(".rs").unwrap_or(inner);
    assert!(
        !inner.is_empty() && !inner.ends_with("mod") && inner != "lib" && inner != "main",
        "in_crate path {path} must name a module file or folder"
    );
    inner.replace('/', "::")
}

/// The module a file belongs to, if any.
fn owner<'a>(layering: &'a Layering, krate: &str, path: &str) -> Option<&'a str> {
    layering.in_crate.iter().find_map(|(module, by_crate)| {
        let paths = by_crate.get(krate)?;
        paths
            .iter()
            .any(|owned| path == owned || path.starts_with(&format!("{owned}/")))
            .then_some(module.as_str())
    })
}

/// Lines that name a module's files from outside the module, grouped as `"from -> module"`.
/// `from` is the module the line's file belongs to, or its crate when it belongs to none.
fn crossings(layering: &Layering, sources: &[Source]) -> BTreeMap<String, Vec<String>> {
    struct Wall {
        module: String,
        /// Paths as written from another crate: `hq_tools::harness_session`.
        outside: Regex,
        /// Paths as written inside the owning crate: `crate::harness_session`.
        inside: BTreeMap<String, Regex>,
    }
    let any_of = |paths: Vec<String>| {
        Regex::new(&format!(r"\b(?:{})\b", paths.join("|"))).expect("module path pattern")
    };
    let walls: Vec<Wall> = layering
        .in_crate
        .iter()
        .map(|(module, by_crate)| {
            let mut outside = Vec::new();
            let mut inside = BTreeMap::new();
            for (krate, paths) in by_crate {
                let ident = krate.replace('-', "_");
                let inner: Vec<String> = paths.iter().map(|p| rust_path(p)).collect();
                outside.extend(inner.iter().map(|p| format!("{ident}::{p}")));
                inside.insert(
                    krate.clone(),
                    any_of(inner.iter().map(|p| format!("crate::{p}")).collect()),
                );
            }
            Wall {
                module: module.clone(),
                outside: any_of(outside),
                inside,
            }
        })
        .collect();

    let mut found: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for source in sources {
        let from = owner(layering, &source.krate, &source.path);
        for (number, line) in source.text.lines().enumerate() {
            if !line.contains("::") || line.trim_start().starts_with("//") {
                continue;
            }
            for wall in &walls {
                if from == Some(wall.module.as_str()) {
                    continue;
                }
                let hit = wall.outside.is_match(line)
                    || wall
                        .inside
                        .get(&source.krate)
                        .is_some_and(|inside| inside.is_match(line));
                if hit {
                    let key = format!("{} -> {}", from.unwrap_or(&source.krate), wall.module);
                    found.entry(key).or_default().push(format!(
                        "crates/{}/{}:{}: {}",
                        source.krate,
                        source.path,
                        number + 1,
                        line.trim()
                    ));
                }
            }
        }
    }
    found
}

/// The counts must equal `debt.refs` exactly, so a number can fall but never rise unseen.
fn check_refs(layering: &Layering, found: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    let mut problems = Vec::new();
    for (key, lines) in found {
        let allowed = layering.debt.refs.get(key).copied().unwrap_or(0);
        if lines.len() > allowed {
            problems.push(format!(
                "{key}: {} lines name the module from outside it, layering.yaml allows \
                 {allowed}. Code outside a module must not name its files; reach it through a \
                 contract in the base. The lines today:\n    {}",
                lines.len(),
                lines.join("\n    ")
            ));
        } else if lines.len() < allowed {
            problems.push(format!(
                "{key}: {} lines, layering.yaml still says {allowed}. Lower debt.refs to {}",
                lines.len(),
                lines.len()
            ));
        }
    }
    for (key, allowed) in &layering.debt.refs {
        if !found.contains_key(key) {
            problems.push(format!(
                "{key}: no lines left, layering.yaml still says {allowed}. Remove the entry"
            ));
        }
    }
    problems
}

// --- reading the repository -------------------------------------------------------------------

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

fn read_toml(path: &Path) -> toml::Table {
    fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .parse()
        .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// Workspace crates by name, with the folder each lives in.
fn workspace(root: &Path) -> BTreeMap<String, PathBuf> {
    let manifest = read_toml(&root.join("Cargo.toml"));
    manifest["workspace"]["members"]
        .as_array()
        .expect("workspace.members")
        .iter()
        .map(|member| {
            let dir = root.join(member.as_str().expect("member path"));
            let name = read_toml(&dir.join("Cargo.toml"))["package"]["name"]
                .as_str()
                .expect("package.name")
                .to_string();
            (name, dir)
        })
        .collect()
}

/// The workspace crates a manifest depends on in a normal build, on any target and whether or
/// not the dependency is optional. Dev and build dependencies do not count: they are not linked
/// into `hq`.
fn workspace_deps(manifest: &toml::Table, crates: &BTreeMap<String, PathBuf>) -> BTreeSet<String> {
    let mut tables: Vec<&toml::Table> = Vec::new();
    if let Some(deps) = manifest.get("dependencies").and_then(|d| d.as_table()) {
        tables.push(deps);
    }
    if let Some(targets) = manifest.get("target").and_then(|t| t.as_table()) {
        for target in targets.values() {
            if let Some(deps) = target.get("dependencies").and_then(|d| d.as_table()) {
                tables.push(deps);
            }
        }
    }
    let mut found = BTreeSet::new();
    for table in tables {
        for (key, spec) in table {
            let package = spec.get("package").and_then(|p| p.as_str()).unwrap_or(key);
            if crates.contains_key(package) {
                found.insert(package.to_string());
            }
        }
    }
    found
}

fn rust_files(dir: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, into);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            into.push(path);
        }
    }
}

fn load() -> (Layering, BTreeMap<String, PathBuf>) {
    let root = repo_root();
    let text = fs::read_to_string(root.join("layering.yaml")).expect("layering.yaml at the root");
    (Layering::parse(&text), workspace(&root))
}

#[test]
fn the_crate_graph_follows_layering_yaml() {
    let (layering, crates) = load();
    let names: BTreeSet<String> = crates.keys().cloned().collect();
    let mut edges = BTreeSet::new();
    for (name, dir) in &crates {
        let manifest = read_toml(&dir.join("Cargo.toml"));
        for dep in workspace_deps(&manifest, &crates) {
            edges.insert((name.clone(), dep));
        }
    }
    let mut problems = check_tiers(&layering, &names);
    problems.extend(check_edges(&layering, &edges));
    assert!(
        problems.is_empty(),
        "layering.yaml and the crate graph disagree:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn code_outside_a_module_does_not_name_it() {
    let (layering, crates) = load();
    let mut problems = Vec::new();
    for (module, by_crate) in &layering.in_crate {
        assert!(
            !module.starts_with("hq-"),
            "module id {module} would read as a crate name"
        );
        for (krate, paths) in by_crate {
            let Some(dir) = crates.get(krate) else {
                problems.push(format!("in_crate.{module} names {krate}, not a crate"));
                continue;
            };
            for path in paths {
                if !dir.join(path).exists() {
                    problems.push(format!(
                        "in_crate.{module}.{krate} lists {path}, which is gone: remove it"
                    ));
                }
            }
        }
    }

    // The app tier is where modules are put together, so it may name them.
    let places = layering.places();
    let mut sources = Vec::new();
    for (name, dir) in &crates {
        if places.get(name.as_str()).map(|p| p.0) == Some(Tier::App) {
            continue;
        }
        let mut files = Vec::new();
        rust_files(&dir.join("src"), &mut files);
        files.sort();
        for file in files {
            let path = file
                .strip_prefix(dir)
                .expect("file under its crate")
                .to_string_lossy()
                .replace('\\', "/");
            let text = fs::read_to_string(&file).expect("read source");
            sources.push(Source {
                krate: name.clone(),
                path,
                text,
            });
        }
    }
    problems.extend(check_refs(&layering, &crossings(&layering, &sources)));
    assert!(
        problems.is_empty(),
        "layering.yaml and the source disagree:\n  {}",
        problems.join("\n  ")
    );
}

// --- the checks themselves, on a made-up workspace -------------------------------------------

const SAMPLE: &str = r#"
tiers:
  contract: [core]
  kernel: [db, tools, agent]
  module: [host, sandbox, export]
  app: [cli]
module_edges:
  host: [sandbox]
in_crate:
  helpers:
    tools: [src/sessions, src/a2a.rs]
debt:
  cargo_edges:
    - tools -> export
  refs:
    tools -> helpers: 1
"#;

fn edges(pairs: &[(&str, &str)]) -> BTreeSet<(String, String)> {
    pairs
        .iter()
        .map(|(from, to)| (from.to_string(), to.to_string()))
        .collect()
}

fn names(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|name| name.to_string()).collect()
}

const SAMPLE_CRATES: &[&str] = &[
    "core", "db", "tools", "agent", "host", "sandbox", "export", "cli",
];

/// The sample's own edges, which the rules and its debt accept.
const SAMPLE_EDGES: &[(&str, &str)] = &[
    ("db", "core"),
    ("tools", "db"),
    ("tools", "export"),
    ("agent", "tools"),
    ("host", "sandbox"),
    ("host", "core"),
    ("cli", "agent"),
    ("cli", "host"),
];

#[test]
fn a_graph_that_follows_the_rules_passes() {
    let layering = Layering::parse(SAMPLE);
    assert_eq!(
        check_tiers(&layering, &names(SAMPLE_CRATES)),
        Vec::<String>::new()
    );
    assert_eq!(
        check_edges(&layering, &edges(SAMPLE_EDGES)),
        Vec::<String>::new()
    );
}

#[test]
fn a_base_crate_may_not_depend_on_a_module_crate() {
    let layering = Layering::parse(SAMPLE);
    let mut graph = edges(SAMPLE_EDGES);
    graph.insert(("agent".into(), "host".into()));
    let problems = check_edges(&layering, &graph);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].starts_with("agent -> host is not allowed"));
}

#[test]
fn a_base_crate_may_not_depend_on_one_listed_after_it() {
    let layering = Layering::parse(SAMPLE);
    let mut graph = edges(SAMPLE_EDGES);
    graph.insert(("db".into(), "tools".into()));
    let problems = check_edges(&layering, &graph);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].starts_with("db -> tools is not allowed"));
}

#[test]
fn the_contract_depends_on_nothing_and_nothing_depends_on_the_app() {
    let layering = Layering::parse(SAMPLE);
    assert!(!layering.allows("core", "db"));
    assert!(!layering.allows("host", "cli"));
    assert!(!layering.allows("agent", "cli"));
    assert!(layering.allows("cli", "export"));
    assert!(layering.allows("host", "agent"));
}

#[test]
fn a_module_needs_a_declared_edge_to_another_module() {
    let layering = Layering::parse(SAMPLE);
    assert!(layering.allows("host", "sandbox"));
    assert!(!layering.allows("sandbox", "host"));
    let mut graph = edges(SAMPLE_EDGES);
    graph.insert(("export".into(), "host".into()));
    let problems = check_edges(&layering, &graph);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].starts_with("export -> host is not allowed"));
}

#[test]
fn paid_debt_and_unused_allowances_must_be_removed() {
    let layering = Layering::parse(SAMPLE);
    let graph: BTreeSet<_> = edges(SAMPLE_EDGES)
        .into_iter()
        .filter(|edge| edge != &("tools".to_string(), "export".to_string()))
        .filter(|edge| edge != &("host".to_string(), "sandbox".to_string()))
        .collect();
    let problems = check_edges(&layering, &graph);
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems[0].contains("tools -> export is listed under debt.cargo_edges"));
    assert!(problems[1].contains("module_edges allows host -> sandbox"));
}

#[test]
fn a_crate_with_no_tier_fails_and_so_does_a_tier_with_no_crate() {
    let layering = Layering::parse(SAMPLE);
    let mut crates = names(SAMPLE_CRATES);
    crates.insert("newcomer".into());
    crates.remove("export");
    let problems = check_tiers(&layering, &crates);
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems[0].contains("export is listed under tiers but is not a workspace crate"));
    assert!(problems[1].contains("newcomer is a workspace crate with no tier"));
}

fn source(krate: &str, path: &str, text: &str) -> Source {
    Source {
        krate: krate.into(),
        path: path.into(),
        text: text.into(),
    }
}

#[test]
fn lines_that_name_a_module_from_outside_are_counted_by_where_they_come_from() {
    let layering = Layering::parse(SAMPLE);
    let sources = [
        // Outside the module, same crate: counts.
        source("tools", "src/tasks.rs", "let a = crate::sessions::list();"),
        // Outside the module, another crate: counts.
        source(
            "agent",
            "src/loop.rs",
            "use tools::a2a::send;\n// tools::a2a in a comment",
        ),
        // Inside the module: its own business.
        source(
            "tools",
            "src/sessions/launch.rs",
            "crate::a2a::send(crate::sessions::x())",
        ),
        // A longer name is another module, and `crate::` in another crate is that crate's own.
        source("tools", "src/vault.rs", "crate::sessions_log::write()"),
        source("agent", "src/x.rs", "crate::sessions::mine()"),
    ];
    let found = crossings(&layering, &sources);
    let counts: Vec<(&str, usize)> = found.iter().map(|(k, v)| (k.as_str(), v.len())).collect();
    assert_eq!(counts, [("agent -> helpers", 1), ("tools -> helpers", 1)]);
    assert!(found["agent -> helpers"][0].starts_with("crates/agent/src/loop.rs:1:"));
}

#[test]
fn a_count_may_fall_only_together_with_its_number_and_may_never_rise() {
    let layering = Layering::parse(SAMPLE);
    let line = |n: usize| -> Vec<String> { (0..n).map(|i| format!("line {i}")).collect() };

    let same = BTreeMap::from([("tools -> helpers".to_string(), line(1))]);
    assert_eq!(check_refs(&layering, &same), Vec::<String>::new());

    let more = BTreeMap::from([("tools -> helpers".to_string(), line(2))]);
    assert!(check_refs(&layering, &more)[0].contains("layering.yaml allows 1"));

    let new_key = BTreeMap::from([
        ("tools -> helpers".to_string(), line(1)),
        ("agent -> helpers".to_string(), line(1)),
    ]);
    assert!(check_refs(&layering, &new_key)[0].contains("agent -> helpers: 1 lines"));

    let none = BTreeMap::new();
    assert!(check_refs(&layering, &none)[0].contains("Remove the entry"));
}

#[test]
fn a_file_belongs_to_the_module_that_lists_it_or_its_folder() {
    let layering = Layering::parse(SAMPLE);
    assert_eq!(owner(&layering, "tools", "src/a2a.rs"), Some("helpers"));
    assert_eq!(
        owner(&layering, "tools", "src/sessions/launch.rs"),
        Some("helpers")
    );
    assert_eq!(owner(&layering, "tools", "src/sessions_log.rs"), None);
    assert_eq!(owner(&layering, "agent", "src/a2a.rs"), None);
    assert_eq!(rust_path("src/github/dispatch.rs"), "github::dispatch");
    assert_eq!(rust_path("src/sessions"), "sessions");
}
