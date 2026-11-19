//! The layering rule for the top-level modules of `hq-tools`, as a test.
//!
//! `layers.toml` gives every module a layer. A module may use modules on its own layer or a lower
//! one. A reference from a lower layer up to a higher one (`crate::<module>` in non-test source) is
//! a back edge, and the only back edges allowed are the ones listed in `allowed_back_edges`, which
//! can only shrink: an entry that is no longer a back edge fails the test, so the change that
//! removes an edge deletes its line in the same PR. A module that is not in `layers.toml` fails
//! too, so a new module has to be given a layer. The scanner is itself tested on a copy of the
//! source with breaches planted in it, so the guard cannot quietly stop guarding.
//!
//! Measure the current shape with `python3 -I scripts/ci/module-graph.py`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

struct Layers {
    layer: BTreeMap<String, u32>,
    allowed: BTreeSet<(String, String)>,
}

fn parse_layers(text: &str) -> Layers {
    let doc: toml::Table = text.parse().expect("layers.toml parses");
    let table = doc
        .get("layers")
        .and_then(|v| v.as_table())
        .expect("layers.toml has a [layers] table");
    let layer = table
        .iter()
        .map(|(name, v)| {
            let n = v.as_integer().expect("a layer is an integer");
            (
                name.clone(),
                u32::try_from(n).expect("a layer is not negative"),
            )
        })
        .collect();
    let allowed = doc
        .get("allowed_back_edges")
        .and_then(|v| v.as_array())
        .expect("layers.toml has allowed_back_edges")
        .iter()
        .map(|v| {
            let s = v.as_str().expect("an allowed edge is a string");
            let (from, to) = s
                .split_once(" -> ")
                .unwrap_or_else(|| panic!("allowed edge `{s}` is not `from -> to`"));
            (from.trim().to_string(), to.trim().to_string())
        })
        .collect();
    Layers { layer, allowed }
}

/// Every `.rs` file under `dir`, sorted.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(dir).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let p = entry.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// Top-level modules of the crate (`foo.rs`, `foo/`, or both) and the files that make each one up.
/// `lib.rs` is the root that sees everything and is not a module.
fn modules(src: &Path) -> BTreeMap<String, Vec<PathBuf>> {
    let mut mods: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for entry in fs::read_dir(src).unwrap().flatten() {
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            let files = mods.entry(name).or_default();
            rust_files(&p, files);
        } else if let Some(stem) = name.strip_suffix(".rs")
            && stem != "lib"
        {
            mods.entry(stem.to_string()).or_default().push(p);
        }
    }
    for files in mods.values_mut() {
        files.sort();
    }
    mods
}

/// Test code is allowed to reach anywhere: `tests*.rs`, `*_tests.rs` and anything under a `tests/`
/// folder. Inline `#[cfg(test)]` modules inside a production file count as production here, the
/// same way `scripts/ci/module-graph.py` counts them.
fn is_test_file(src: &Path, file: &Path) -> bool {
    let rel = file.strip_prefix(src).unwrap_or(file);
    let name = file.file_name().unwrap().to_string_lossy();
    name.starts_with("tests")
        || name.ends_with("_tests.rs")
        || rel.components().any(|c| c.as_os_str() == "tests")
}

/// Code with `//` comment lines (including doc comments) removed, so prose that names a module is
/// not mistaken for a use of it.
fn code_of(text: &str) -> String {
    text.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The modules named as `crate::<module>` in `code`, one entry per occurrence.
fn crate_refs<'a>(code: &'a str, known: &BTreeMap<String, Vec<PathBuf>>) -> Vec<&'a str> {
    let mut found = Vec::new();
    let mut rest = code;
    let mut offset = 0;
    while let Some(i) = rest.find("crate::") {
        let at = offset + i;
        let before = code[..at].chars().next_back();
        let start = at + "crate::".len();
        let name_len = code[start..]
            .find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
            .unwrap_or(code.len() - start);
        let name = &code[start..start + name_len];
        // `subcrate::x` is not `crate::x`; `$crate::x` in a macro is.
        let word_before = before.is_some_and(|c| c.is_alphanumeric() || c == '_');
        if !word_before && known.contains_key(name) {
            found.push(name);
        }
        offset = start;
        rest = &code[start..];
    }
    found
}

#[derive(Default)]
struct Scan {
    /// (from, to) -> (file relative to src, references in it), non-test source only.
    refs: BTreeMap<(String, String), BTreeMap<String, usize>>,
    modules: BTreeSet<String>,
}

fn scan(src: &Path) -> Scan {
    let mods = modules(src);
    let mut out = Scan {
        modules: mods.keys().cloned().collect(),
        ..Scan::default()
    };
    for (module, files) in &mods {
        for file in files {
            if is_test_file(src, file) {
                continue;
            }
            let text = fs::read_to_string(file).unwrap_or_default();
            let code = code_of(&text);
            let rel = file.strip_prefix(src).unwrap().display().to_string();
            for to in crate_refs(&code, &mods) {
                if to != module {
                    *out.refs
                        .entry((module.clone(), to.to_string()))
                        .or_default()
                        .entry(rel.clone())
                        .or_default() += 1;
                }
            }
        }
    }
    out
}

/// What is wrong, in words a person can act on. Empty means the layering holds.
fn problems(scan: &Scan, layers: &Layers) -> Vec<String> {
    let mut out = Vec::new();

    for m in scan
        .modules
        .difference(&layers.layer.keys().cloned().collect())
    {
        out.push(format!(
            "module `{m}` is not in layers.toml: give it a layer (lowest it can be)"
        ));
    }
    for k in layers.layer.keys() {
        if !scan.modules.contains(k) {
            out.push(format!(
                "layers.toml lists `{k}`, which is not a module of src/: delete the line"
            ));
        }
    }

    let mut back_edges = BTreeSet::new();
    for ((from, to), files) in &scan.refs {
        let (Some(a), Some(b)) = (layers.layer.get(from), layers.layer.get(to)) else {
            continue; // already reported as a missing module
        };
        if a < b {
            back_edges.insert((from.clone(), to.clone()));
            if !layers.allowed.contains(&(from.clone(), to.clone())) {
                let at: Vec<String> = files.iter().map(|(f, n)| format!("{f} ({n})")).collect();
                out.push(format!(
                    "`{from}` (layer {a}) uses `{to}` (layer {b}), a higher layer: move what it \
                     needs down, or invert the dependency. In: {}",
                    at.join(", ")
                ));
            }
        }
    }
    for (from, to) in &layers.allowed {
        if !back_edges.contains(&(from.clone(), to.clone())) {
            out.push(format!(
                "allowed_back_edges lists `{from} -> {to}`, which is no longer a back edge: \
                 delete the line (the list only shrinks)"
            ));
        }
    }
    out
}

fn real_layers() -> Layers {
    parse_layers(&fs::read_to_string(crate_dir().join("layers.toml")).expect("layers.toml exists"))
}

#[test]
fn hq_tools_layering_holds() {
    let found = problems(&scan(&crate_dir().join("src")), &real_layers());
    assert!(
        found.is_empty(),
        "hq-tools layering:\n  {}\n\nSee crates/hq-tools/layers.toml.",
        found.join("\n  ")
    );
}

#[test]
fn the_allowlist_is_the_back_edges_that_exist() {
    // The same fact from the other side, so a bad edit to layers.toml is named plainly: every
    // allowed edge is between two known modules and really does point up.
    let layers = real_layers();
    for (from, to) in &layers.allowed {
        let (a, b) = (layers.layer.get(from), layers.layer.get(to));
        assert!(
            a.is_some() && b.is_some(),
            "allowed edge `{from} -> {to}` names a module that is not in [layers]"
        );
        assert!(a < b, "allowed edge `{from} -> {to}` does not point up");
    }
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap().flatten() {
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if src.is_dir() {
            copy_tree(&src, &dst);
        } else {
            fs::copy(&src, &dst).unwrap();
        }
    }
}

#[test]
fn the_scanner_catches_planted_breaches() {
    let layers = real_layers();
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    copy_tree(&crate_dir().join("src"), &src);

    // A faithful copy is clean, so any problem below was planted.
    assert_eq!(problems(&scan(&src), &layers), Vec::<String>::new());

    // 1. A module free of the tangle (layer 1) reaching up into it.
    let agent_comm = src.join("agent_comm.rs");
    let mut text = fs::read_to_string(&agent_comm).unwrap();
    text.push_str("\nuse crate::tasks::PlantedBreach;\n");
    fs::write(&agent_comm, text).unwrap();

    // 2. A new module nobody gave a layer.
    fs::write(src.join("zz_planted_module.rs"), "pub fn planted() {}\n").unwrap();

    // 3. An upward reference in a test file must NOT be reported.
    fs::write(
        src.join("tasks").join("tests_planted.rs"),
        "use crate::shortcuts::Anything;\n",
    )
    .unwrap();

    // 4. A reference that only appears in a comment must NOT be reported.
    let usage = src.join("harness_usage.rs");
    let mut text = fs::read_to_string(&usage).unwrap();
    text.push_str("\n// see crate::vault for how this is used\n");
    fs::write(&usage, text).unwrap();

    let found = problems(&scan(&src), &layers);
    let has = |needle: &str| found.iter().any(|p| p.contains(needle));
    assert!(has("`agent_comm` (layer 1) uses `tasks`"), "{found:#?}");
    assert!(
        has("module `zz_planted_module` is not in layers.toml"),
        "{found:#?}"
    );
    assert!(!has("tests_planted"), "a test file was scanned: {found:#?}");
    assert!(
        !has("`harness_usage` (layer"),
        "a comment was scanned: {found:#?}"
    );
    assert_eq!(found.len(), 2, "{found:#?}");
}

#[test]
fn the_allowlist_can_only_shrink_and_a_stale_layer_is_named() {
    let mut layers = real_layers();
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    copy_tree(&crate_dir().join("src"), &src);

    // An allowed edge that is not a back edge (a down-pointing one) is stale, as is one that does
    // not exist at all.
    layers.allowed.insert(("tasks".into(), "agent_comm".into()));
    // A layer for a module that is not there.
    layers.layer.insert("zz_gone".into(), 1);

    let found = problems(&scan(&src), &layers);
    assert!(
        found
            .iter()
            .any(|p| p.contains("`tasks -> agent_comm`") && p.contains("no longer a back edge")),
        "{found:#?}"
    );
    assert!(
        found
            .iter()
            .any(|p| p.contains("`zz_gone`") && p.contains("not a module")),
        "{found:#?}"
    );
    assert_eq!(found.len(), 2, "{found:#?}");
}
