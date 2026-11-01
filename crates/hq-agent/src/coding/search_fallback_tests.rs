use super::*;
use std::time::Duration;

const TIME: Duration = Duration::from_secs(10);

fn opts(pattern: &str, path: &Path) -> Search {
    Search {
        pattern: pattern.to_string(),
        path: path.to_path_buf(),
        file_type: None,
        context: 0,
        case_insensitive: false,
        max_output: usize::MAX,
        deadline: Instant::now() + TIME,
    }
}

fn tree() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let p = root.path();
    for dir in ["src", ".git", "node_modules/dep", "build"] {
        fs::create_dir_all(p.join(dir)).unwrap();
    }
    fs::write(p.join("src/main.rs"), "fn main() {}\nfn helper() {}\n").unwrap();
    fs::write(p.join("src/notes.md"), "fn in prose\n").unwrap();
    fs::write(p.join(".git/config"), "fn hidden\n").unwrap();
    fs::write(p.join("node_modules/dep/index.js"), "fn vendored\n").unwrap();
    fs::write(p.join("build/out.rs"), "fn generated\n").unwrap();
    root
}

fn shown(root: &Path, rel: &str) -> String {
    root.join(rel).display().to_string()
}

#[test]
fn matches_print_path_line_and_text_and_skip_ignored_places() {
    let root = tree();
    let out = search(&opts("^fn", root.path())).unwrap();
    let main = shown(root.path(), "src/main.rs");
    let notes = shown(root.path(), "src/notes.md");
    let expected =
        format!("{main}:1:fn main() {{}}\n{main}:2:fn helper() {{}}\n{notes}:1:fn in prose\n");
    assert_eq!(out, expected);
}

#[test]
fn file_types_follow_ripgrep_names_and_fall_back_to_an_extension() {
    let root = tempfile::tempdir().unwrap();
    for name in ["a.rs", "b.tsx", "c.h", "d.py", "e.zig"] {
        fs::write(root.path().join(name), "needle\n").unwrap();
    }
    let found = |file_type: &str| {
        let mut o = opts("needle", root.path());
        o.file_type = Some(file_type.to_string());
        search(&o).unwrap().lines().count()
    };
    assert_eq!(found("rust"), 1);
    assert_eq!(found("rs"), 1);
    assert_eq!(found("ts"), 1, "ripgrep's ts includes tsx");
    assert_eq!(found("c"), 1, "ripgrep's c includes headers");
    assert_eq!(found("python"), 1);
    assert_eq!(found("zig"), 1, "an unknown name is an extension");
}

#[test]
fn case_insensitivity_widens_the_match() {
    let root = tree();
    let mut o = opts("FN HELPER", root.path());
    assert!(search(&o).unwrap().is_empty());
    o.case_insensitive = true;
    assert!(search(&o).unwrap().ends_with(":2:fn helper() {}\n"));
}

#[test]
fn context_lines_use_dashes_and_groups_are_separated() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("a.txt");
    fs::write(&file, "x\nhit\ny\nz\nw\nhit\nv\n").unwrap();
    let mut o = opts("hit", &file);
    o.context = 1;
    let f = file.display();
    let expected = format!("{f}-1-x\n{f}:2:hit\n{f}-3-y\n--\n{f}-5-w\n{f}:6:hit\n{f}-7-v\n");
    assert_eq!(search(&o).unwrap(), expected);
}

#[test]
fn adjacent_and_overlapping_context_prints_each_line_once_without_a_separator() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("a.txt");
    fs::write(&file, "hit\nmid\nhit\nx\ny\nhit\n").unwrap();
    let mut o = opts("hit", &file);
    o.context = 1;
    let f = file.display();
    let expected = format!("{f}:1:hit\n{f}-2-mid\n{f}:3:hit\n{f}-4-x\n{f}-5-y\n{f}:6:hit\n");
    assert_eq!(search(&o).unwrap(), expected);
}

#[test]
fn context_groups_in_different_files_are_separated_as_ripgrep_does() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), "a\nhit\n").unwrap();
    fs::write(root.path().join("b.txt"), "hit\nb\n").unwrap();
    let mut o = opts("hit", root.path());
    o.context = 1;
    let (a, b) = (shown(root.path(), "a.txt"), shown(root.path(), "b.txt"));
    let expected = format!("{a}-1-a\n{a}:2:hit\n--\n{b}:1:hit\n{b}-2-b\n");
    assert_eq!(search(&o).unwrap(), expected);
}

#[test]
fn a_huge_context_neither_panics_nor_loses_lines() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("a.txt");
    fs::write(&file, "a\nhit\nb").unwrap();
    let mut o = opts("hit", &file);
    o.context = usize::MAX;
    assert_eq!(search(&o).unwrap().lines().count(), 3);
}

#[test]
fn skipped_files_are_counted_in_a_note() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("blob.bin"), b"needle\0binary").unwrap();
    fs::write(root.path().join("latin1.txt"), b"needle \xe9\n").unwrap();
    let big = vec![b'x'; MAX_FILE_BYTES as usize + 1];
    fs::write(root.path().join("big.txt"), big).unwrap();
    let out = search(&opts("needle", root.path())).unwrap();
    assert!(out.contains("skipped 3 file(s)"), "{out}");
}

#[test]
fn a_search_past_its_deadline_says_its_results_are_incomplete() {
    let root = tree();
    let mut o = opts("fn", root.path());
    o.deadline = Instant::now() - Duration::from_secs(1);
    let out = search(&o).unwrap();
    assert!(out.contains("stopped at the time limit"), "{out}");
}

#[test]
fn the_output_limit_stops_the_search_and_says_so() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), "hit\n".repeat(1000)).unwrap();
    let mut o = opts("hit", root.path());
    o.max_output = 100;
    let out = search(&o).unwrap();
    assert!(out.lines().count() < 10, "{out}");
    assert!(out.contains("stopped at the output limit"));
}

#[test]
fn a_bad_pattern_and_a_missing_path_are_errors() {
    let root = tempfile::tempdir().unwrap();
    assert!(search(&opts("(", root.path())).is_err());
    assert!(search(&opts("x", &root.path().join("missing"))).is_err());
}

#[cfg(unix)]
#[test]
fn links_inside_are_not_followed_but_a_linked_start_path_is() {
    let root = tree();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.txt"), "fn outside\n").unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("src/link")).unwrap();
    assert!(search(&opts("outside", root.path())).unwrap().is_empty());

    let linked_file = root.path().join("file-link.rs");
    std::os::unix::fs::symlink(root.path().join("src/main.rs"), &linked_file).unwrap();
    assert!(
        search(&opts("helper", &linked_file))
            .unwrap()
            .contains(":2:fn helper")
    );
    let linked_dir = root.path().join("dir-link");
    std::os::unix::fs::symlink(root.path().join("src"), &linked_dir).unwrap();
    assert!(
        search(&opts("helper", &linked_dir))
            .unwrap()
            .contains(":2:fn helper")
    );
}

/// Runs through `GrepTool` itself, so it covers whichever engine this machine has. Run the test
/// binary with a `PATH` that lacks `rg` to exercise the fallback end to end.
#[tokio::test]
async fn grep_finds_a_match_with_or_without_rg() {
    use crate::tools::AgentTool;
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.rs"), "fn needle() {}\n").unwrap();
    let args = serde_json::json!({"pattern": "needle", "path": root.path().display().to_string(), "file_type": "rust"});
    let result = crate::coding::GrepTool.execute("t", args).await.unwrap();
    let text = &result.content[0].text;
    assert!(text.contains("a.rs:1:fn needle() {}"), "{text}");
}
