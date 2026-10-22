//! A deterministic scan of a diff for edits that weaken the checks themselves: a worker who makes
//! the tests pass by deleting or skipping them, or by changing the CI that runs them.
//!
//! A flag does not fail a task. It raises the tier and obliges the reviewer to address it by
//! name, because some of these edits are right (a flaky test is skipped with a reason, a snapshot
//! is regenerated on purpose). What the scan must never do is miss one silently, so a diff that
//! was cut short is itself reported as a flag.

use serde::Serialize;
use std::sync::LazyLock;

use regex::Regex;

/// Most flags kept; more only repeats the point.
const MAX_FLAGS: usize = 50;
/// Longest line sample kept on a flag.
const MAX_SAMPLE_CHARS: usize = 120;

pub const KIND_DELETED_TEST_FILE: &str = "deleted_test_file";
pub const KIND_REMOVED_TESTS: &str = "removed_tests";
pub const KIND_SKIPPED_TEST: &str = "skipped_test";
pub const KIND_WEAKENED_ASSERTIONS: &str = "weakened_assertions";
pub const KIND_LINT_RELAXED: &str = "lint_relaxed";
pub const KIND_LINT_CONFIG: &str = "lint_config_changed";
pub const KIND_WORKFLOW: &str = "workflow_changed";
pub const KIND_SNAPSHOT: &str = "snapshot_updated";
pub const KIND_TRUNCATED: &str = "diff_truncated";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TamperFlag {
    pub kind: String,
    pub file: String,
    /// The line that raised it, when one did.
    pub sample: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct TamperReport {
    pub files_scanned: usize,
    pub flags: Vec<TamperFlag>,
    /// More flags were raised than are listed.
    pub more_flags: bool,
}

impl TamperReport {
    pub fn is_clean(&self) -> bool {
        self.flags.is_empty()
    }

    pub fn has(&self, kind: &str) -> bool {
        self.flags.iter().any(|f| f.kind == kind)
    }
}

static TEST_MARKER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"#\[(tokio::)?test\b|#\[test_case|\b(it|test|describe)\(|\bdef test_|\bfunc Test[A-Z_]|@Test\b",
    )
    .unwrap()
});
static ASSERTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bassert\w*|\bexpect\(|\.should\b|\bt\.(Error|Fatal)\w*\(|\bassertEquals").unwrap()
});
static SKIP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"#\[ignore\b|\.skip\(|\bxit\(|\bxdescribe\(|\bxtest\(|\b(it|test|describe)\.skip\b|@pytest\.mark\.skip|@unittest\.skip|@Disabled\b|@Ignore\b|\bt\.Skip\w*\(|\bpytest\.skip\(",
    )
    .unwrap()
});
static LINT_RELAXED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"#!?\[allow\(|eslint-disable|@ts-ignore|@ts-nocheck|#\s*noqa\b|#\s*type:\s*ignore|//\s*nolint\b|#pragma warning disable",
    )
    .unwrap()
});

/// Whether a path is a test file, by the conventions the languages here use.
pub fn is_test_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let name = p.rsplit('/').next().unwrap_or(&p);
    p.starts_with("tests/")
        || p.starts_with("test/")
        || p.contains("/tests/")
        || p.contains("/test/")
        || p.contains("/__tests__/")
        || p.contains("/e2e/")
        || name.ends_with("_test.go")
        || name.ends_with("_test.rs")
        || name.ends_with("_tests.rs")
        || name == "tests.rs"
        || name.ends_with("_test.py")
        || name.starts_with("test_")
        || name == "conftest.py"
        || [".test.", ".spec."].iter().any(|m| name.contains(m))
}

fn is_workflow_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.starts_with(".github/workflows/")
        || p.starts_with(".circleci/")
        || p.ends_with(".gitlab-ci.yml")
        || p == "jenkinsfile"
        || p == "azure-pipelines.yml"
        || p == ".travis.yml"
}

fn is_lint_config_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    matches!(
        name.as_str(),
        "clippy.toml"
            | ".clippy.toml"
            | "rustfmt.toml"
            | ".rustfmt.toml"
            | "deny.toml"
            | "ruff.toml"
            | ".flake8"
            | ".pylintrc"
            | "eslint.config.js"
            | "eslint.config.mjs"
            | "eslint.config.ts"
            | ".eslintrc"
            | ".eslintrc.js"
            | ".eslintrc.json"
            | ".eslintrc.cjs"
            | "biome.json"
            | ".golangci.yml"
            | ".golangci.yaml"
    )
}

fn is_snapshot_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.contains("__snapshots__/") || p.ends_with(".snap") || p.ends_with(".snap.new")
}

fn sample(line: &str) -> Option<String> {
    let t = line.trim();
    (!t.is_empty()).then(|| t.chars().take(MAX_SAMPLE_CHARS).collect())
}

/// What one file's section of the diff added and removed, for the per-file comparisons.
#[derive(Default)]
struct FileCounts {
    added_tests: usize,
    removed_tests: usize,
    added_asserts: usize,
    removed_asserts: usize,
}

/// Scans a unified diff. `truncated` says the diff was cut short, which is flagged because a
/// scan of part of a change proves nothing about the rest.
pub fn scan_diff(diff: &str, truncated: bool) -> TamperReport {
    let mut report = TamperReport::default();
    let mut flags: Vec<TamperFlag> = Vec::new();
    let mut file = String::new();
    let mut deleted = false;
    let mut counts = FileCounts::default();
    let mut test_file = false;

    // Closes the section of the file being read: the comparisons that need the whole file.
    let close = |file: &str,
                 deleted: bool,
                 test_file: bool,
                 c: &FileCounts,
                 flags: &mut Vec<TamperFlag>| {
        if file.is_empty() {
            return;
        }
        if test_file {
            if deleted {
                flags.push(TamperFlag {
                    kind: KIND_DELETED_TEST_FILE.into(),
                    file: file.into(),
                    sample: None,
                });
            } else if c.removed_tests > c.added_tests {
                flags.push(TamperFlag {
                    kind: KIND_REMOVED_TESTS.into(),
                    file: file.into(),
                    sample: Some(format!(
                        "{} tests removed, {} added",
                        c.removed_tests, c.added_tests
                    )),
                });
            }
            if !deleted && c.removed_asserts > c.added_asserts {
                flags.push(TamperFlag {
                    kind: KIND_WEAKENED_ASSERTIONS.into(),
                    file: file.into(),
                    sample: Some(format!(
                        "{} assertions removed, {} added",
                        c.removed_asserts, c.added_asserts
                    )),
                });
            }
        }
    };

    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            close(&file, deleted, test_file, &counts, &mut flags);
            counts = FileCounts::default();
            deleted = false;
            // "a/x b/y": the new name is what the change is about.
            file = rest
                .rsplit_once(" b/")
                .map(|(_, new)| new.to_string())
                .unwrap_or_else(|| rest.to_string());
            test_file = is_test_path(&file);
            report.files_scanned += 1;
            if is_workflow_path(&file) {
                flags.push(TamperFlag {
                    kind: KIND_WORKFLOW.into(),
                    file: file.clone(),
                    sample: None,
                });
            }
            if is_lint_config_path(&file) {
                flags.push(TamperFlag {
                    kind: KIND_LINT_CONFIG.into(),
                    file: file.clone(),
                    sample: None,
                });
            }
            if is_snapshot_path(&file) {
                flags.push(TamperFlag {
                    kind: KIND_SNAPSHOT.into(),
                    file: file.clone(),
                    sample: None,
                });
            }
            continue;
        }
        if line.starts_with("deleted file mode") {
            deleted = true;
            continue;
        }
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if let Some(added) = line.strip_prefix('+') {
            if SKIP.is_match(added) {
                flags.push(TamperFlag {
                    kind: KIND_SKIPPED_TEST.into(),
                    file: file.clone(),
                    sample: sample(added),
                });
            }
            if LINT_RELAXED.is_match(added) {
                flags.push(TamperFlag {
                    kind: KIND_LINT_RELAXED.into(),
                    file: file.clone(),
                    sample: sample(added),
                });
            }
            if test_file {
                counts.added_tests += usize::from(TEST_MARKER.is_match(added));
                counts.added_asserts += usize::from(ASSERTION.is_match(added));
            }
        } else if let Some(removed) = line.strip_prefix('-')
            && test_file
        {
            counts.removed_tests += usize::from(TEST_MARKER.is_match(removed));
            counts.removed_asserts += usize::from(ASSERTION.is_match(removed));
        }
    }
    close(&file, deleted, test_file, &counts, &mut flags);

    if truncated {
        flags.push(TamperFlag {
            kind: KIND_TRUNCATED.into(),
            file: String::new(),
            sample: Some("the diff was cut short, so part of the change was not scanned".into()),
        });
    }
    // The same flag on the same file is one point, however many lines raised it.
    let mut seen = std::collections::HashSet::new();
    flags.retain(|f| seen.insert((f.kind.clone(), f.file.clone(), f.sample.clone())));
    if flags.len() > MAX_FLAGS {
        flags.truncate(MAX_FLAGS);
        report.more_flags = true;
    }
    report.flags = flags;
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, body: &str) -> String {
        format!(
            "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1,3 +1,3 @@\n{body}\n"
        )
    }

    #[test]
    fn an_ordinary_change_with_a_new_test_is_clean() {
        let diff = [
            file("src/lib.rs", "-fn old() {}\n+fn new() {}"),
            file(
                "src/lib_tests.rs",
                "+#[test]\n+fn it_works() {\n+    assert_eq!(new(), 1);\n+}",
            ),
        ]
        .concat();
        let r = scan_diff(&diff, false);
        assert!(r.is_clean(), "{:?}", r.flags);
        assert_eq!(r.files_scanned, 2);
    }

    #[test]
    fn a_deleted_test_file_is_flagged() {
        let diff = "diff --git a/tests/api.rs b/tests/api.rs\ndeleted file mode 100644\n--- a/tests/api.rs\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-#[test]\n-fn a() {}\n";
        let r = scan_diff(diff, false);
        assert!(r.has(KIND_DELETED_TEST_FILE));
        assert!(
            !r.has(KIND_REMOVED_TESTS),
            "a whole file is one flag, not two"
        );
    }

    #[test]
    fn fewer_tests_than_before_in_a_test_file_is_flagged_but_a_rewrite_is_not() {
        let fewer = file(
            "tests/api.rs",
            "-#[test]\n-fn a() {}\n-#[test]\n-fn b() {}\n+#[test]\n+fn a() {}",
        );
        assert!(scan_diff(&fewer, false).has(KIND_REMOVED_TESTS));
        let rewrite = file(
            "tests/api.rs",
            "-#[test]\n-fn a() {}\n+#[test]\n+fn a_renamed() {}",
        );
        assert!(!scan_diff(&rewrite, false).has(KIND_REMOVED_TESTS));
    }

    #[test]
    fn skipping_a_test_is_flagged_in_each_language() {
        for line in [
            "+#[ignore]",
            "+    it.skip('works', () => {})",
            "+xit('works', () => {})",
            "+@pytest.mark.skip(reason='later')",
            "+    t.Skip(\"flaky\")",
            "+@Disabled",
        ] {
            let r = scan_diff(&file("src/x.any", line), false);
            assert!(r.has(KIND_SKIPPED_TEST), "{line}");
        }
    }

    #[test]
    fn removing_more_assertions_than_are_added_in_a_test_file_is_flagged() {
        let weaker = file(
            "src/service.test.ts",
            "-    expect(a).toBe(1);\n-    expect(b).toBe(2);\n+    expect(a).toBe(1);",
        );
        let r = scan_diff(&weaker, false);
        assert!(r.has(KIND_WEAKENED_ASSERTIONS));
        // The same removal in production code is not a test being weakened.
        let prod = file("src/service.ts", "-    expect(a).toBe(1);");
        assert!(!scan_diff(&prod, false).has(KIND_WEAKENED_ASSERTIONS));
    }

    #[test]
    fn relaxed_lints_and_changed_lint_config_are_flagged() {
        assert!(
            scan_diff(&file("src/a.rs", "+#[allow(clippy::all)]"), false).has(KIND_LINT_RELAXED)
        );
        assert!(
            scan_diff(&file("src/a.ts", "+// eslint-disable-next-line"), false)
                .has(KIND_LINT_RELAXED)
        );
        assert!(scan_diff(&file("src/a.ts", "+// @ts-ignore"), false).has(KIND_LINT_RELAXED));
        assert!(scan_diff(&file("clippy.toml", "+x = 1"), false).has(KIND_LINT_CONFIG));
        assert!(scan_diff(&file("web/eslint.config.js", "+x"), false).has(KIND_LINT_CONFIG));
    }

    #[test]
    fn ci_workflow_and_snapshot_changes_are_flagged() {
        assert!(
            scan_diff(
                &file(".github/workflows/ci.yml", "-      - run: cargo test"),
                false
            )
            .has(KIND_WORKFLOW)
        );
        assert!(scan_diff(&file("src/__snapshots__/a.snap", "+x"), false).has(KIND_SNAPSHOT));
    }

    #[test]
    fn a_diff_that_was_cut_short_is_itself_a_flag() {
        let r = scan_diff(&file("src/a.rs", "+fn a() {}"), true);
        assert!(r.has(KIND_TRUNCATED) && !r.is_clean());
    }

    #[test]
    fn the_same_flag_is_listed_once_and_the_list_is_bounded() {
        let many: String = (0..20)
            .map(|i| format!("+#[ignore]\n+fn t{i}() {{}}\n"))
            .collect();
        let r = scan_diff(&file("src/a.rs", &many), false);
        assert_eq!(r.flags.len(), 1, "one point however many lines raise it");

        let files: String = (0..80)
            .map(|i| file(&format!("src/f{i}.rs"), "+#[ignore]"))
            .collect();
        let r = scan_diff(&files, false);
        assert_eq!(r.flags.len(), MAX_FLAGS);
        assert!(r.more_flags);
    }

    #[test]
    fn test_paths_follow_the_usual_conventions() {
        for p in [
            "tests/api.rs",
            "crates/x/tests/it.rs",
            "src/tasks/tests.rs",
            "src/a_test.go",
            "web/src/a.test.tsx",
            "web/src/__tests__/a.ts",
            "pkg/test_models.py",
            "conftest.py",
        ] {
            assert!(is_test_path(p), "{p}");
        }
        for p in [
            "src/latest.rs",
            "src/contest.rs",
            "docs/testing.md",
            "src/attest.ts",
        ] {
            assert!(!is_test_path(p), "{p}");
        }
    }
}
