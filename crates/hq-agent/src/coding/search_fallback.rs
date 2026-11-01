//! Content search without ripgrep, for machines that have none (a phone).

use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use regex::{Regex, RegexBuilder};

/// Files larger than this are skipped, as generated or data files rarely worth a line search.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// A NUL byte this early marks a binary file, the same test ripgrep uses.
const BINARY_SNIFF_BYTES: usize = 8 * 1024;
/// Folders a project ignores almost everywhere; without `.gitignore` parsing these are the bulk.
const SKIPPED_DIRS: [&str; 2] = ["node_modules", "target"];
/// What ripgrep prints between context groups.
const GROUP_SEPARATOR: &str = "--";

pub(super) struct Search<'a> {
    pub pattern: &'a str,
    pub path: &'a Path,
    pub file_type: Option<&'a str>,
    pub context: usize,
    pub case_insensitive: bool,
    /// Stop collecting once the output is this long; the caller truncates and says so.
    pub max_output: usize,
    pub deadline: Instant,
}

/// Lines matching the pattern as `path:line:text`, with `path-line-text` context lines and `--`
/// between groups, the shape `rg --no-heading --line-number` prints. Empty when nothing matches.
pub(super) fn search(opts: &Search<'_>) -> Result<String> {
    let regex = RegexBuilder::new(opts.pattern)
        .case_insensitive(opts.case_insensitive)
        .build()
        .context("the pattern is not a valid regular expression")?;
    let extensions = opts.file_type.map(extensions_for);
    let mut files = Vec::new();
    collect_files(opts.path, extensions.as_deref(), &mut files, opts.deadline)?;
    files.sort();
    let mut out = String::new();
    for file in files {
        if out.len() >= opts.max_output {
            break;
        }
        if Instant::now() > opts.deadline {
            anyhow::bail!("the search ran out of time");
        }
        search_file(&file, &regex, opts.context, &mut out);
    }
    Ok(out)
}

/// The extensions ripgrep's common type names cover; any other name is taken as an extension.
fn extensions_for(file_type: &str) -> Vec<String> {
    let known: &[&str] = match file_type {
        "js" => &["js", "mjs", "cjs", "jsx"],
        "ts" => &["ts", "mts", "cts"],
        "py" => &["py", "pyi"],
        "md" | "markdown" => &["md", "markdown"],
        "yaml" | "yml" => &["yaml", "yml"],
        "cpp" => &["cpp", "cc", "cxx", "hpp", "hh", "h"],
        _ => return vec![file_type.to_string()],
    };
    known.iter().map(|e| e.to_string()).collect()
}

/// Walks `path` without following links, skipping hidden and commonly ignored folders.
fn collect_files(
    path: &Path,
    extensions: Option<&[String]>,
    out: &mut Vec<std::path::PathBuf>,
    deadline: Instant,
) -> Result<()> {
    let meta =
        fs::symlink_metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
    if meta.is_file() {
        out.push(path.to_path_buf());
        return Ok(());
    }
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        if Instant::now() > deadline {
            anyhow::bail!("the search ran out of time");
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if kind.is_dir() && !is_skipped_dir(&name) {
                pending.push(entry.path());
            } else if kind.is_file() && has_extension(&name, extensions) {
                out.push(entry.path());
            }
        }
    }
    Ok(())
}

fn is_skipped_dir(name: &str) -> bool {
    name.starts_with('.') || SKIPPED_DIRS.contains(&name)
}

fn has_extension(name: &str, extensions: Option<&[String]>) -> bool {
    let Some(extensions) = extensions else {
        return !name.starts_with('.');
    };
    let ext = Path::new(name).extension().and_then(|e| e.to_str());
    ext.is_some_and(|ext| extensions.iter().any(|e| e == ext))
}

/// The file's text, or `None` when it is too large, binary or not UTF-8.
fn read_text(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_FILE_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let sniff = &bytes[..bytes.len().min(BINARY_SNIFF_BYTES)];
    if sniff.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn search_file(path: &Path, regex: &Regex, context: usize, out: &mut String) {
    let Some(text) = read_text(path) else {
        return;
    };
    let lines: Vec<&str> = text.lines().collect();
    let hits: Vec<usize> = (0..lines.len())
        .filter(|i| regex.is_match(lines[*i]))
        .collect();
    let shown = path.display();
    let mut last_printed: Option<usize> = None;
    for &hit in &hits {
        let start = hit.saturating_sub(context);
        let end = (hit + context).min(lines.len() - 1);
        let from = last_printed.map_or(start, |last| start.max(last + 1));
        if context > 0 && last_printed.is_some_and(|last| from > last + 1) {
            out.push_str(GROUP_SEPARATOR);
            out.push('\n');
        }
        for (i, line) in lines.iter().enumerate().take(end + 1).skip(from) {
            let mark = if regex.is_match(line) { ':' } else { '-' };
            out.push_str(&format!("{shown}{mark}{}{mark}{line}\n", i + 1));
        }
        last_printed = Some(end.max(last_printed.unwrap_or(0)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const TIME: Duration = Duration::from_secs(10);

    fn opts<'a>(pattern: &'a str, path: &'a Path) -> Search<'a> {
        Search {
            pattern,
            path,
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
        fs::create_dir_all(p.join("src")).unwrap();
        fs::create_dir_all(p.join(".git")).unwrap();
        fs::create_dir_all(p.join("node_modules/dep")).unwrap();
        fs::write(p.join("src/main.rs"), "fn main() {}\nfn helper() {}\n").unwrap();
        fs::write(p.join("src/notes.md"), "fn in prose\n").unwrap();
        fs::write(p.join(".git/config"), "fn hidden\n").unwrap();
        fs::write(p.join("node_modules/dep/index.js"), "fn vendored\n").unwrap();
        fs::write(p.join("src/blob.bin"), b"fn \0binary").unwrap();
        root
    }

    #[test]
    fn matches_print_path_line_and_text_and_skip_ignored_places() {
        let root = tree();
        let out = search(&opts("^fn", root.path())).unwrap();
        let main = root.path().join("src/main.rs").display().to_string();
        let notes = root.path().join("src/notes.md").display().to_string();
        let expected =
            format!("{main}:1:fn main() {{}}\n{main}:2:fn helper() {{}}\n{notes}:1:fn in prose\n");
        assert_eq!(out, expected);
    }

    #[test]
    fn a_file_type_and_case_insensitivity_narrow_and_widen() {
        let root = tree();
        let mut o = opts("FN HELPER", root.path());
        o.case_insensitive = true;
        o.file_type = Some("rs");
        let out = search(&o).unwrap();
        assert!(out.ends_with(":2:fn helper() {}\n"), "{out}");
        o.file_type = Some("md");
        assert!(search(&o).unwrap().is_empty());
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
    fn overlapping_context_prints_each_line_once() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("a.txt");
        fs::write(&file, "hit\nmid\nhit\n").unwrap();
        let mut o = opts("hit", &file);
        o.context = 1;
        let f = file.display();
        assert_eq!(
            search(&o).unwrap(),
            format!("{f}:1:hit\n{f}-2-mid\n{f}:3:hit\n")
        );
    }

    #[test]
    fn a_bad_pattern_and_a_missing_path_are_errors() {
        let root = tempfile::tempdir().unwrap();
        assert!(search(&opts("(", root.path())).is_err());
        assert!(search(&opts("x", &root.path().join("missing"))).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn links_are_not_followed() {
        let root = tree();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "fn outside\n").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("src/link")).unwrap();
        let out = search(&opts("outside", root.path())).unwrap();
        assert!(out.is_empty(), "{out}");
    }
}

/// Runs through `GrepTool` itself, so it covers whichever engine this machine has. Run the test
/// binary with a `PATH` that lacks `rg` to exercise the fallback end to end.
#[cfg(test)]
mod through_the_tool {
    use crate::coding::GrepTool;
    use crate::tools::AgentTool;

    #[tokio::test]
    async fn grep_finds_a_match_with_or_without_rg() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.rs"), "fn needle() {}\n").unwrap();
        let args =
            serde_json::json!({"pattern": "needle", "path": root.path().display().to_string()});
        let result = GrepTool.execute("t", args).await.unwrap();
        let text = &result.content[0].text;
        assert!(text.contains("a.rs:1:fn needle() {}"), "{text}");
    }
}
