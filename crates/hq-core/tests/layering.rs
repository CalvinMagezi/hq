//! The layering guard.
//!
//! `layering.yaml` at the repository root says which crate may depend on which, and which files
//! belong to a module that still lives inside a base crate. This test asks cargo for the
//! workspace's crates and their dependencies, reads every file under `src/` of the crates below
//! the app tier, and fails when the code and that file disagree, in either direction: a new
//! dependency the file does not allow fails, and so does a debt entry the code no longer needs.
//! The base therefore cannot start depending on a module without an edit to `layering.yaml`,
//! and the list of known violations can only get shorter.
//!
//! The source check reads paths as they are written: full paths (`hq_tools::github::dispatch`,
//! `crate::harness_session`), every path a `use` statement imports, `self::` and `super::`
//! paths, and a child module named bare by its parent (`dispatch::kick()` in `github/mod.rs`).
//! It is still a tripwire and not a proof. It does not see a name used after it was imported
//! (the import is what counts), a crate or module renamed with `as`, a file mounted with
//! `#[path]`, or `tests/` folders, and it counts a path inside a string. Unit tests under
//! `src/` are counted. The proof arrives when a module sits behind a cargo feature and the
//! build without it compiles, or becomes its own crate and rule 1 covers it.
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
    /// Module id, then the module crates its files may name.
    #[serde(default)]
    module_uses: BTreeMap<String, Vec<String>>,
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
    /// `"from -> module"` and the number of places outside the module that name it.
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

/// `src/github/dispatch.rs` and `src/harness_session` as the segments of a Rust path inside
/// their crate; a crate root (`src/lib.rs`) is empty.
fn module_path(path: &str) -> Vec<String> {
    let inner = path
        .strip_prefix("src/")
        .unwrap_or_else(|| panic!("{path} must start with src/"));
    let inner = inner.strip_suffix(".rs").unwrap_or(inner);
    let mut segments: Vec<String> = inner.split('/').map(str::to_string).collect();
    if matches!(
        segments.last().map(String::as_str),
        Some("mod" | "lib" | "main")
    ) {
        segments.pop();
    }
    segments
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

/// A `use` statement's paths with its groups written out: `a::{b, c::{d, e as f}, self}`
/// becomes `a::b`, `a::c::d`, `a::c::e` and `a`.
fn expand_use(tree: &str) -> Vec<String> {
    fn walk(prefix: &str, tree: &str, out: &mut Vec<String>) {
        let tree = tree.trim();
        let Some(open) = tree.find('{') else {
            let leaf = tree.split(" as ").next().unwrap_or(tree).trim();
            let leaf = leaf.trim_end_matches("::*").trim_end_matches('*');
            let joined = match leaf {
                "" | "self" => prefix.trim_end_matches("::").to_string(),
                leaf => format!("{prefix}{leaf}"),
            };
            if !joined.is_empty() {
                out.push(joined);
            }
            return;
        };
        let close = tree.rfind('}').unwrap_or(tree.len());
        let prefix = format!("{prefix}{}", tree[..open].trim());
        let body = &tree[open + 1..close];
        let (mut depth, mut start) = (0usize, 0usize);
        for (at, c) in body.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    walk(&prefix, &body[start..at], out);
                    start = at + 1;
                }
                _ => {}
            }
        }
        walk(&prefix, &body[start..], out);
    }
    let mut out = Vec::new();
    walk("", tree, &mut out);
    out
}

/// A line of code with a trailing `//` comment cut off.
fn code_of(line: &str) -> &str {
    let line = line.trim();
    if line.starts_with("//") {
        return "";
    }
    line.split(" //").next().unwrap_or(line)
}

/// Every path written in a file, as `(line, path, text shown in a failure)`. A `use` statement
/// gives one entry per path it imports, whatever lines it spans; other lines give every
/// `a::b` they contain.
fn written_paths(text: &str) -> Vec<(usize, String, String)> {
    let path =
        Regex::new(r"(?:^|[^:A-Za-z0-9_])([A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)+)")
            .expect("path pattern");
    let use_start = Regex::new(r"^(?:pub(?:\([^)]*\))?\s+)?use\s+(.*)$").expect("use pattern");
    let mut found = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut at = 0;
    while at < lines.len() {
        let code = code_of(lines[at]);
        if let Some(start) = use_start.captures(code) {
            let first = at;
            let mut statement = start[1].to_string();
            while !statement.contains(';') && at + 1 < lines.len() {
                at += 1;
                statement.push(' ');
                statement.push_str(code_of(lines[at]));
            }
            let tree = statement.split(';').next().unwrap_or("");
            for imported in expand_use(tree) {
                if imported.contains("::") {
                    found.push((first + 1, imported.clone(), format!("use {imported}")));
                }
            }
        } else {
            for hit in path.captures_iter(code) {
                found.push((at + 1, hit[1].to_string(), code.to_string()));
            }
        }
        at += 1;
    }
    found
}

/// Where a written path can lead: the crate, and the module path inside it. `super` is read
/// both from the file's module and from a module declared inside the file (`mod tests`), since
/// the text alone does not say which one the line is in.
fn resolve(
    written: &str,
    krate: &str,
    here: &[String],
    idents: &BTreeMap<String, String>,
) -> Vec<(String, Vec<String>)> {
    let segments: Vec<String> = written.split("::").map(str::to_string).collect();
    let first = segments[0].as_str();
    let below = |base: &[String], rest: &[String]| {
        let mut full = base.to_vec();
        full.extend_from_slice(rest);
        (krate.to_string(), full)
    };
    if let Some(other) = idents.get(first) {
        return vec![(other.clone(), segments[1..].to_vec())];
    }
    match first {
        "crate" => vec![below(&[], &segments[1..])],
        "self" => vec![below(here, &segments[1..])],
        "super" => {
            let supers = segments.iter().take_while(|s| *s == "super").count();
            let rest = &segments[supers..];
            let mut leads = Vec::new();
            for up in [supers, supers - 1] {
                if up <= here.len() {
                    leads.push(below(&here[..here.len() - up], rest));
                }
            }
            leads
        }
        // A child module of this file, named bare by its parent.
        _ => vec![below(here, &segments)],
    }
}

/// Lines that name a module from outside it, grouped as `"from -> module"`.
///
/// `from` is the in-crate module the line's file belongs to, or its crate when it belongs to
/// none. `module` is an in-crate module whose files the line names, or a module crate that a
/// base crate names without being a module allowed to use it (`module_uses`).
fn crossings(layering: &Layering, sources: &[Source]) -> BTreeMap<String, Vec<String>> {
    let places = layering.places();
    let idents: BTreeMap<String, String> = places
        .keys()
        .map(|name| (name.replace('-', "_"), name.to_string()))
        .collect();
    // In-crate walls: crate, then (module, module path of a file or folder it owns).
    let mut walls: BTreeMap<&str, Vec<(&str, Vec<String>)>> = BTreeMap::new();
    for (module, by_crate) in &layering.in_crate {
        for (krate, paths) in by_crate {
            for path in paths {
                walls
                    .entry(krate.as_str())
                    .or_default()
                    .push((module.as_str(), module_path(path)));
            }
        }
    }
    let is_base = |krate: &str| {
        matches!(
            places.get(krate).map(|place| place.0),
            Some(Tier::Contract | Tier::Kernel)
        )
    };
    let is_module_crate =
        |krate: &str| places.get(krate).map(|place| place.0) == Some(Tier::Module);

    let mut found: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for source in sources {
        let from = owner(layering, &source.krate, &source.path);
        let here = module_path(&source.path);
        let may_use = |module_crate: &str| {
            from.and_then(|module| layering.module_uses.get(module))
                .is_some_and(|allowed| allowed.iter().any(|name| name == module_crate))
        };
        let mut counted: BTreeSet<(usize, String, String)> = BTreeSet::new();
        for (line, written, shown) in written_paths(&source.text) {
            let mut named: BTreeSet<String> = BTreeSet::new();
            for (krate, path) in resolve(&written, &source.krate, &here, &idents) {
                for (module, wall) in walls.get(krate.as_str()).into_iter().flatten() {
                    if from != Some(*module) && path.starts_with(wall) {
                        named.insert((*module).to_string());
                    }
                }
                if krate != source.krate
                    && is_module_crate(&krate)
                    && is_base(&source.krate)
                    && !may_use(&krate)
                {
                    named.insert(krate.clone());
                }
            }
            for module in named {
                // One count per line and module, or per imported path for a `use`.
                let unit = if shown.starts_with("use ") {
                    shown.clone()
                } else {
                    String::new()
                };
                if counted.insert((line, module.clone(), unit)) {
                    let key = format!("{} -> {module}", from.unwrap_or(&source.krate));
                    found.entry(key).or_default().push(format!(
                        "crates/{}/{}:{line}: {shown}",
                        source.krate, source.path
                    ));
                }
            }
        }
    }
    found
}

/// The counts must equal `debt.refs` exactly, so a number can fall only in the change that
/// fixes the code and a rise is always a visible edit to `layering.yaml`.
fn check_refs(layering: &Layering, found: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    let mut problems = Vec::new();
    for (key, lines) in found {
        let allowed = layering.debt.refs.get(key).copied().unwrap_or(0);
        if lines.len() > allowed {
            problems.push(format!(
                "{key}: {} places name the module from outside it, layering.yaml allows \
                 {allowed}. Code outside a module must not name it; reach it through a \
                 contract in the base. The places today:\n    {}",
                lines.len(),
                lines.join("\n    ")
            ));
        } else if lines.len() < allowed {
            problems.push(format!(
                "{key}: {} places, layering.yaml still says {allowed}. Lower debt.refs to {}",
                lines.len(),
                lines.len()
            ));
        }
    }
    for (key, allowed) in &layering.debt.refs {
        if !found.contains_key(key) {
            problems.push(format!(
                "{key}: no places left, layering.yaml still says {allowed}. Remove the entry"
            ));
        }
    }
    problems
}

/// `in_crate` and `module_uses` must name things that exist.
fn check_walls(layering: &Layering, crates: &BTreeMap<String, PathBuf>) -> Vec<String> {
    let mut problems = Vec::new();
    let places = layering.places();
    for (module, by_crate) in &layering.in_crate {
        if places.contains_key(module.as_str()) || module.starts_with("hq-") {
            problems.push(format!("module id {module} reads as a crate name"));
        }
        if by_crate.is_empty() {
            problems.push(format!("in_crate.{module} is empty: remove the key"));
        }
        for (krate, paths) in by_crate {
            let Some(dir) = crates.get(krate) else {
                problems.push(format!("in_crate.{module} names {krate}, not a crate"));
                continue;
            };
            if paths.is_empty() {
                problems.push(format!(
                    "in_crate.{module}.{krate} is empty: remove the key"
                ));
            }
            for path in paths {
                if !dir.join(path).exists() {
                    problems.push(format!(
                        "in_crate.{module}.{krate} lists {path}, which is gone: remove it"
                    ));
                }
            }
        }
    }
    for (module, uses) in &layering.module_uses {
        if !layering.in_crate.contains_key(module) {
            problems.push(format!(
                "module_uses names {module}, not a module in in_crate"
            ));
        }
        for krate in uses {
            if places.get(krate.as_str()).map(|place| place.0) != Some(Tier::Module) {
                problems.push(format!(
                    "module_uses.{module} lists {krate}, which is not a module crate"
                ));
            }
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

/// Workspace crates with their folders, and the dependencies between them that a normal build
/// links, on any target and whether or not optional. Cargo is asked, so a renamed dependency,
/// one inherited from the workspace table and a path crate missing from `members` are all
/// seen as cargo sees them. Dev and build dependencies do not count: they are not linked
/// into `hq`.
fn workspace(root: &Path) -> (BTreeMap<String, PathBuf>, BTreeSet<(String, String)>) {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let output = std::process::Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata prints JSON");
    let packages = metadata["packages"].as_array().expect("packages");
    let crates: BTreeMap<String, PathBuf> = packages
        .iter()
        .map(|package| {
            let name = package["name"].as_str().expect("name").to_string();
            let manifest = Path::new(package["manifest_path"].as_str().expect("manifest_path"));
            (name, manifest.parent().expect("crate folder").to_path_buf())
        })
        .collect();
    let mut edges = BTreeSet::new();
    for package in packages {
        let from = package["name"].as_str().expect("name");
        for dep in package["dependencies"].as_array().into_iter().flatten() {
            let to = dep["name"].as_str().expect("dependency name");
            if dep["kind"].is_null() && crates.contains_key(to) {
                edges.insert((from.to_string(), to.to_string()));
            }
        }
    }
    (crates, edges)
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

fn load() -> (
    Layering,
    BTreeMap<String, PathBuf>,
    BTreeSet<(String, String)>,
) {
    let root = repo_root();
    let text = fs::read_to_string(root.join("layering.yaml")).expect("layering.yaml at the root");
    let (crates, edges) = workspace(&root);
    (Layering::parse(&text), crates, edges)
}

#[test]
fn the_crate_graph_follows_layering_yaml() {
    let (layering, crates, edges) = load();
    let names: BTreeSet<String> = crates.keys().cloned().collect();
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
    let (layering, crates, _) = load();
    let mut problems = check_walls(&layering, &crates);

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
module_uses:
  helpers: [host]
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

fn counts(layering: &Layering, sources: &[Source]) -> Vec<(String, usize)> {
    crossings(layering, sources)
        .into_iter()
        .map(|(key, places)| (key, places.len()))
        .collect()
}

fn pairs(list: &[(&str, usize)]) -> Vec<(String, usize)> {
    list.iter().map(|(key, n)| (key.to_string(), *n)).collect()
}

#[test]
fn full_paths_are_counted_by_where_they_come_from() {
    let layering = Layering::parse(SAMPLE);
    let sources = [
        // Outside the module, same crate: counts.
        source("tools", "src/tasks.rs", "let a = crate::sessions::list();"),
        // Outside the module, another crate: counts once, and the comment does not.
        source(
            "agent",
            "src/loop.rs",
            "tools::a2a::send(); // and tools::sessions::x\n// tools::a2a in a comment",
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
    assert_eq!(
        counts(&layering, &sources),
        pairs(&[("agent -> helpers", 1), ("tools -> helpers", 1)])
    );
    assert!(found["agent -> helpers"][0].starts_with("crates/agent/src/loop.rs:1:"));
}

#[test]
fn a_use_statement_counts_once_for_each_module_path_it_imports() {
    let layering = Layering::parse(SAMPLE);
    let sources = [
        source(
            "agent",
            "src/build.rs",
            "use tools::{\n    a2a,\n    sessions::{self, launch as go},\n    vault::read,\n};\nuse std::fmt;",
        ),
        source("tools", "src/tasks.rs", "pub(crate) use crate::a2a::*;"),
    ];
    assert_eq!(
        counts(&layering, &sources),
        pairs(&[("agent -> helpers", 3), ("tools -> helpers", 1)])
    );
    assert_eq!(
        expand_use("a::{b, c::{d, e as f}, self}"),
        ["a::b", "a::c::d", "a::c::e", "a"]
    );
}

#[test]
fn a_parent_naming_its_child_and_a_relative_path_are_counted() {
    let layering = Layering::parse(SAMPLE);
    let sources = [
        // The crate root names its child module bare. The `mod` line itself is no path.
        source(
            "tools",
            "src/lib.rs",
            "pub mod a2a;\npub use a2a::send;\nfn f() { sessions::list() }",
        ),
        // A sibling reaches it through its parent.
        source("tools", "src/tasks/lease.rs", "super::super::a2a::send()"),
        // Inside `mod tests` of a root file, `super` is the file itself.
        source(
            "tools",
            "src/main.rs",
            "mod tests { fn t() { super::sessions::x() } }",
        ),
        // The same words that lead somewhere else are not counted.
        source(
            "tools",
            "src/tasks/mod.rs",
            "a2a::send(); self::sessions::x(); super::vault::y()",
        ),
    ];
    assert_eq!(
        counts(&layering, &sources),
        pairs(&[("tools -> helpers", 4)])
    );
}

#[test]
fn a_base_file_naming_a_module_crate_is_counted_unless_its_module_may_use_it() {
    let layering = Layering::parse(SAMPLE);
    let sources = [
        // A base file of a base crate: counts, for each module crate it names.
        source("tools", "src/git.rs", "host::pins();\nuse export::render;"),
        // A file of the module that is allowed to use the host: does not count.
        source("tools", "src/sessions/client.rs", "host::connect()"),
        // That module is not allowed the export crate.
        source("tools", "src/a2a.rs", "export::render()"),
        // A module crate naming another is the crate graph's business, not this check's.
        source("host", "src/lib.rs", "sandbox::wrap()"),
    ];
    assert_eq!(
        counts(&layering, &sources),
        pairs(&[
            ("helpers -> export", 1),
            ("tools -> export", 1),
            ("tools -> host", 1),
        ])
    );
}

#[test]
fn walls_that_name_nothing_are_reported() {
    let mut layering = Layering::parse(SAMPLE);
    let crates: BTreeMap<String, PathBuf> = SAMPLE_CRATES
        .iter()
        .map(|name| (name.to_string(), PathBuf::from("/nowhere").join(name)))
        .collect();
    layering
        .in_crate
        .insert("empty".into(), BTreeMap::from([("tools".into(), vec![])]));
    layering
        .module_uses
        .insert("ghost".into(), vec!["core".into()]);
    let problems = check_walls(&layering, &crates);
    assert!(
        problems
            .iter()
            .any(|p| p == "in_crate.empty.tools is empty: remove the key")
    );
    assert!(
        problems
            .iter()
            .any(|p| p.contains("lists src/sessions, which is gone"))
    );
    assert!(
        problems
            .iter()
            .any(|p| p == "module_uses names ghost, not a module in in_crate")
    );
    assert!(
        problems
            .iter()
            .any(|p| p.contains("lists core, which is not a module crate"))
    );
    // An empty wall matches nothing: it must not turn every line into a crossing.
    let sources = [source(
        "tools",
        "src/vault.rs",
        "std::fmt::write(); crate::x::y()",
    )];
    assert!(crossings(&layering, &sources).is_empty());
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
    assert!(check_refs(&layering, &new_key)[0].contains("agent -> helpers: 1 places"));

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
    assert_eq!(
        module_path("src/github/dispatch.rs"),
        ["github", "dispatch"]
    );
    assert_eq!(module_path("src/sessions"), ["sessions"]);
    assert_eq!(module_path("src/tasks/mod.rs"), ["tasks"]);
    assert_eq!(module_path("src/chmod.rs"), ["chmod"]);
    assert!(module_path("src/lib.rs").is_empty());
}
