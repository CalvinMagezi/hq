//! The extraction rules of `crates/suite/CLAUDE.md`, as a test.
//!
//! Every suite crate except this one must be free of HQ: no dependency on a host crate, no read
//! of the user's configuration or environment, no network, no child process. That is what lets
//! the suite be lifted into its own repository by moving a folder. The scanner below is itself
//! tested against planted breaches, so the guard cannot quietly stop guarding.
//!
//! A change that really must cross a line deletes the rule here first and says why in review.

use std::fs;
use std::path::{Path, PathBuf};

/// HQ crates only the adapter may use. Each is also written with underscores for source scans.
const HOST_CRATES: &[&str] = &[
    "hq-agent",
    "hq-relay",
    "hq-llm",
    "hq-web",
    "hq-mcp",
    "hq-tools",
    "hq-cli",
    "hq-daemon",
    "hq-memory",
];

/// Source text no suite crate but the adapter may contain, with the rule it protects.
const FORBIDDEN_SOURCE: &[(&str, &str)] = &[
    // Rule 3: nothing of the user's machine, found on its own.
    ("env::var", "reads the environment (rule 3)"),
    ("env::args", "reads the command line (rule 3)"),
    ("env::current_dir", "reads the working directory (rule 3)"),
    ("env::home_dir", "finds the home folder (rule 3)"),
    ("env::temp_dir", "finds a temp folder (rule 3)"),
    ("dirs::", "finds the user's folders (rule 3)"),
    ("~/.hq", "names the HQ config folder (rule 3)"),
    (".hq/", "names the HQ config folder (rule 3)"),
    ("SystemTime::now", "reads the clock (rule 3)"),
    ("Utc::now", "reads the clock (rule 3)"),
    ("Local::now", "reads the clock (rule 3)"),
    // Rule 4: no network.
    ("reqwest", "uses an HTTP client (rule 4)"),
    ("hyper", "uses an HTTP stack (rule 4)"),
    ("std::net", "opens a socket (rule 4)"),
    ("tokio::net", "opens a socket (rule 4)"),
    ("TcpListener", "listens on a port (rule 4)"),
    ("TcpStream", "opens a connection (rule 4)"),
    ("UdpSocket", "opens a socket (rule 4)"),
    ("iroh", "uses the pairing transport directly (rule 4)"),
    // Only the adapter speaks to the outside world.
    ("Command::new", "starts a program (rule 2)"),
    ("tokio::process", "starts a program (rule 2)"),
];

fn suite_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Every suite crate folder except the adapter.
fn free_crates() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(suite_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.join("Cargo.toml").is_file())
        .filter(|p| p.file_name().unwrap() != "hq-suite-adapter")
        .collect();
    dirs.sort();
    assert!(
        !dirs.is_empty(),
        "no suite crates found under {}",
        suite_dir().display()
    );
    dirs
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n != "target") {
                rust_files(&p, out);
            }
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// Lines of code with `//` comments and string-free doc lines removed, so a comment that talks
/// about a rule is not mistaken for a breach of it.
fn code_of(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// What is wrong with one source file, as `rule` descriptions.
fn source_breaches(src: &str) -> Vec<String> {
    let code = code_of(src);
    let mut found = Vec::new();
    for (needle, why) in FORBIDDEN_SOURCE {
        if code.contains(needle) {
            found.push(format!("`{needle}` {why}"));
        }
    }
    for host in HOST_CRATES {
        let ident = host.replace('-', "_");
        if code.contains(&format!("{ident}::")) || code.contains(&format!("use {ident}")) {
            found.push(format!("`{ident}` is a host crate (rule 1)"));
        }
    }
    found
}

/// Dependency names declared in a manifest, across every dependency table.
fn manifest_deps(text: &str) -> Vec<String> {
    let doc: toml::Table = text.parse().expect("a suite manifest parses");
    let mut names = Vec::new();
    let mut collect = |t: Option<&toml::Value>| {
        if let Some(table) = t.and_then(|v| v.as_table()) {
            names.extend(table.keys().cloned());
        }
    };
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        collect(doc.get(section));
    }
    if let Some(targets) = doc.get("target").and_then(|t| t.as_table()) {
        for cfg in targets.values() {
            for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                collect(cfg.get(section));
            }
        }
    }
    names
}

fn manifest_breaches(text: &str) -> Vec<String> {
    manifest_deps(text)
        .into_iter()
        .filter(|d| HOST_CRATES.contains(&d.as_str()) || d == "hq-suite-adapter")
        .map(|d| format!("depends on `{d}`"))
        .collect()
}

#[test]
fn no_suite_crate_but_the_adapter_depends_on_a_host_crate() {
    for dir in free_crates() {
        let text = fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        let breaches = manifest_breaches(&text);
        assert!(
            breaches.is_empty(),
            "{}: {breaches:?} (crates/suite/CLAUDE.md, rule 1)",
            dir.display()
        );
    }
}

#[test]
fn no_suite_crate_but_the_adapter_touches_the_machine_the_network_or_other_programs() {
    for dir in free_crates() {
        // Shipped code only. A test may read an environment variable to regenerate a golden
        // file; it is not part of what the suite does on a person's phone.
        let mut files = Vec::new();
        rust_files(&dir.join("src"), &mut files);
        for file in files {
            let src = fs::read_to_string(&file).unwrap();
            let breaches = source_breaches(&src);
            assert!(
                breaches.is_empty(),
                "{}: {breaches:?} (crates/suite/CLAUDE.md)",
                file.display()
            );
        }
    }
}

#[test]
fn every_suite_crate_is_listed_in_the_rules_file() {
    let rules = fs::read_to_string(suite_dir().join("CLAUDE.md")).unwrap();
    let mut all: Vec<PathBuf> = free_crates();
    all.push(suite_dir().join("hq-suite-adapter"));
    for dir in all {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            rules.contains(&format!("`{name}`")),
            "{name} is not in the crate table of crates/suite/CLAUDE.md"
        );
    }
}

#[test]
fn every_suite_crate_is_a_workspace_member() {
    let root = suite_dir()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("Cargo.toml");
    let manifest = fs::read_to_string(root).unwrap();
    let mut all: Vec<PathBuf> = free_crates();
    all.push(suite_dir().join("hq-suite-adapter"));
    for dir in all {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            manifest.contains(&format!("\"crates/suite/{name}\"")),
            "{name} is not a member of the workspace"
        );
    }
}

// The other direction: HQ may reach the suite only through a short, named list of places.

/// Crates outside `crates/suite/` that may depend on a suite crate. Everything else (`hq-core`,
/// `hq-db`, `hq-tool-api`, `hq-tools` and the rest) must stay free of it, so a change in the
/// suite never rebuilds them. Add a crate here only with a reason in review.
const MAY_DEPEND_ON_SUITE: &[&str] = &["hq-cli", "hq-tools-office", "hq-web"];

/// Source files outside `crates/suite/` that may name a suite crate, relative to the workspace.
const MAY_NAME_SUITE: &[&str] = &[
    "crates/hq-cli/src/commands/start/daemon/tasks_periodic/suite_governor.rs",
    "crates/hq-cli/src/commands/suite.rs",
    "crates/hq-tools-office/src/lib.rs",
    "crates/hq-web/src/suite_api.rs",
];

fn workspace_root() -> PathBuf {
    suite_dir()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Folders of every crate outside the suite, relative paths sorted.
fn host_crate_dirs() -> Vec<PathBuf> {
    let crates = workspace_root().join("crates");
    let mut dirs: Vec<PathBuf> = fs::read_dir(&crates)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("Cargo.toml").is_file())
        .collect();
    dirs.sort();
    dirs
}

fn suite_dependencies(manifest: &str) -> Vec<String> {
    manifest_deps(manifest)
        .into_iter()
        .filter(|d| d.starts_with("hq-suite-"))
        .collect()
}

fn unlisted_suite_users(crates: &[(String, String)], allowed: &[&str]) -> Vec<String> {
    crates
        .iter()
        .filter(|(name, text)| {
            !allowed.contains(&name.as_str()) && !suite_dependencies(text).is_empty()
        })
        .map(|(name, _)| name.clone())
        .collect()
}

#[test]
fn only_listed_crates_depend_on_the_suite() {
    let crates: Vec<(String, String)> = host_crate_dirs()
        .into_iter()
        .map(|d| {
            (
                d.file_name().unwrap().to_string_lossy().into_owned(),
                fs::read_to_string(d.join("Cargo.toml")).unwrap(),
            )
        })
        .collect();
    let bad = unlisted_suite_users(&crates, MAY_DEPEND_ON_SUITE);
    assert!(
        bad.is_empty(),
        "{bad:?} depend on a suite crate but are not in MAY_DEPEND_ON_SUITE. A foundation crate \
         that reaches the suite makes every suite change rebuild it."
    );
    // A stale entry is as bad as a missing one: the list must not grow past what is true.
    for name in MAY_DEPEND_ON_SUITE {
        let (_, text) = crates
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("{name} in MAY_DEPEND_ON_SUITE is not a crate"));
        assert!(
            !suite_dependencies(text).is_empty(),
            "{name} no longer depends on the suite: remove it from MAY_DEPEND_ON_SUITE"
        );
    }
}

/// Rust files outside `crates/suite/` that name a suite crate in code, relative paths.
fn files_naming_the_suite() -> Vec<String> {
    let mut found = Vec::new();
    for dir in host_crate_dirs() {
        let mut files = Vec::new();
        rust_files(&dir, &mut files);
        for file in files {
            let src = fs::read_to_string(&file).unwrap();
            let code = code_of(&src);
            if code.contains("hq_suite") || code.contains("hq-suite") {
                let rel = file.strip_prefix(workspace_root()).unwrap();
                found.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    found.sort();
    found
}

#[test]
fn only_listed_files_name_a_suite_crate() {
    let found = files_naming_the_suite();
    let bad: Vec<&String> = found
        .iter()
        .filter(|f| !MAY_NAME_SUITE.contains(&f.as_str()))
        .collect();
    assert!(
        bad.is_empty(),
        "{bad:?} name a suite crate but are not in MAY_NAME_SUITE. Reach the suite through \
         hq-tools-office, hq-web or hq-cli, or add the file here with a reason."
    );
    for allowed in MAY_NAME_SUITE {
        assert!(
            found.iter().any(|f| f == allowed),
            "{allowed} no longer names the suite: remove it from MAY_NAME_SUITE"
        );
    }
}

#[test]
fn the_reverse_check_sees_a_planted_dependency() {
    let planted = vec![(
        "hq-tools".to_string(),
        "[package]\nname='x'\n[dependencies]\nhq-suite-adapter = { workspace = true, optional = true }\n"
            .to_string(),
    )];
    assert_eq!(
        unlisted_suite_users(&planted, MAY_DEPEND_ON_SUITE),
        ["hq-tools"]
    );
    assert!(unlisted_suite_users(&planted, &["hq-tools"]).is_empty());
}

// The guard guarding itself: a planted breach of every kind must be seen.

#[test]
fn the_scanner_sees_planted_breaches() {
    for (src, expect) in [
        ("let k = std::env::var(\"HOME\");", "env::var"),
        ("let t = std::time::SystemTime::now();", "SystemTime::now"),
        ("use std::net::TcpStream;", "std::net"),
        ("let c = reqwest::Client::new();", "reqwest"),
        ("std::process::Command::new(\"ls\")", "Command::new"),
        ("use hq_tools::Tool;", "hq_tools"),
        ("let p = hq_agent::session::x();", "hq_agent"),
        ("let p = \"~/.hq/config.yaml\";", "~/.hq"),
    ] {
        let found = source_breaches(src);
        assert!(
            found.iter().any(|f| f.contains(expect)),
            "missed {expect}: {found:?}"
        );
    }
}

#[test]
fn the_scanner_sees_planted_manifest_breaches() {
    let bad = "[package]\nname='x'\n[dependencies]\nhq-tools = { workspace = true }\n[target.'cfg(unix)'.dependencies]\nhq-web = '1'\n[dev-dependencies]\nhq-suite-adapter = { path = '..' }\n";
    let found = manifest_breaches(bad);
    assert_eq!(found.len(), 3, "{found:?}");
    let fine =
        "[package]\nname='x'\n[dependencies]\nhq-convert = { workspace = true }\nserde = '1'\n";
    assert!(manifest_breaches(fine).is_empty());
}

#[test]
fn comments_and_look_alikes_are_not_breaches() {
    assert!(source_breaches("// uses env::var in a comment\nlet x = 1;").is_empty());
    assert!(source_breaches("/// reads ~/.hq nowhere\nfn f() {}").is_empty());
    assert!(source_breaches("let os = std::env::consts::OS;").is_empty());
    assert!(source_breaches("let hq_toolbox = 1;").is_empty());
}
