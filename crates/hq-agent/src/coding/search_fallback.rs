//! Content search without ripgrep, for machines that have none (a phone).

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use regex::{Regex, RegexBuilder};

/// Files larger than this are skipped, as generated or data files rarely worth a line search.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// A NUL byte this early marks a binary file, the same test ripgrep uses.
const BINARY_SNIFF_BYTES: usize = 8 * 1024;
/// Build output most projects ignore; without `.gitignore` parsing these are the bulk.
const SKIPPED_DIRS: [&str; 4] = ["node_modules", "target", "build", "dist"];
/// What ripgrep prints between context groups, within a file and between files.
const GROUP_SEPARATOR: &str = "--";

/// ripgrep's file types for the languages agents ask about, as `(names, extensions)`.
const FILE_TYPES: &[(&[&str], &[&str])] = &[
    (&["rust", "rs"], &["rs"]),
    (&["py", "python"], &["py", "pyi"]),
    (&["js", "javascript"], &["js", "mjs", "cjs", "jsx", "vue"]),
    (&["ts", "typescript"], &["ts", "mts", "cts", "tsx"]),
    (&["c"], &["c", "h", "H"]),
    (
        &["cpp"],
        &["cpp", "cc", "cxx", "hpp", "hh", "hxx", "h", "H", "C", "inl"],
    ),
    (&["go"], &["go"]),
    (&["java"], &["java", "jsp", "properties"]),
    (&["kotlin", "kt"], &["kt", "kts"]),
    (&["swift"], &["swift"]),
    (&["dart"], &["dart"]),
    (&["json"], &["json"]),
    (&["toml"], &["toml"]),
    (&["yaml", "yml"], &["yaml", "yml"]),
    (&["md", "markdown"], &["md", "markdown", "mdx"]),
    (&["html"], &["html", "htm"]),
    (&["css"], &["css", "scss"]),
    (&["sh"], &["sh", "bash", "zsh"]),
];

pub(super) struct Search {
    pub pattern: String,
    pub path: PathBuf,
    pub file_type: Option<String>,
    pub context: usize,
    pub case_insensitive: bool,
    /// Stop collecting matches once they are this long; the caller truncates and says so.
    pub max_output: usize,
    pub deadline: Instant,
}

/// What a search found, and what the agent must know about how far it got.
#[derive(Debug, Default)]
pub(super) struct Found {
    /// Lines as `rg --no-heading --line-number` prints them: `path:line:text` for matches,
    /// `path-line-text` for context, `--` between groups.
    pub matches: String,
    /// Plain sentences about files left out or a search cut short; empty when it covered all.
    pub notes: Vec<String>,
}

pub(super) fn search(opts: &Search) -> Result<Found> {
    let regex = RegexBuilder::new(&opts.pattern)
        .case_insensitive(opts.case_insensitive)
        .build()
        .context("the pattern is not a valid regular expression")?;
    let mut run = Run {
        opts,
        regex,
        extensions: opts.file_type.as_deref().map(extensions_for),
        out: String::new(),
        skipped: Skipped::default(),
        stopped: None,
    };
    // The start path may be a link (a symlinked checkout); links inside it are never followed.
    let meta =
        fs::metadata(&opts.path).with_context(|| format!("cannot read {}", opts.path.display()))?;
    if meta.is_file() {
        run.search_file(&opts.path);
    } else {
        fs::read_dir(&opts.path).with_context(|| format!("cannot read {}", opts.path.display()))?;
        run.walk(&opts.path);
    }
    Ok(run.finish())
}

/// Extensions for a ripgrep type name; any other name is taken as an extension itself.
fn extensions_for(file_type: &str) -> Vec<String> {
    let known = FILE_TYPES
        .iter()
        .find(|(names, _)| names.contains(&file_type));
    match known {
        Some((_, exts)) => exts.iter().map(|e| e.to_string()).collect(),
        None => vec![file_type.to_string()],
    }
}

/// Why a file was not searched. Binary files are left out silently, as ripgrep does.
enum Skip {
    Binary,
    TooLarge,
    NotText,
    Unreadable,
}

/// Files that might have held a match but were not searched, by cause.
#[derive(Default)]
struct Skipped {
    too_large: usize,
    not_text: usize,
    unreadable: usize,
}

impl Skipped {
    fn count(&mut self, why: Skip) {
        match why {
            Skip::Binary => {}
            Skip::TooLarge => self.too_large += 1,
            Skip::NotText => self.not_text += 1,
            Skip::Unreadable => self.unreadable += 1,
        }
    }

    fn notes(&self) -> Vec<String> {
        let mb = MAX_FILE_BYTES / (1024 * 1024);
        [
            (self.too_large, format!("larger than {mb} MB")),
            (self.not_text, "not UTF-8 text".to_string()),
            (self.unreadable, "unreadable".to_string()),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, why)| format!("{n} file(s) were not searched because they are {why}."))
        .collect()
    }
}

struct Run<'a> {
    opts: &'a Search,
    regex: Regex,
    extensions: Option<Vec<String>>,
    out: String,
    skipped: Skipped,
    /// Why the search ended before covering everything.
    stopped: Option<&'static str>,
}

impl Run<'_> {
    /// Asked only before more work, so a search that finished is never called incomplete.
    fn should_stop(&mut self) -> bool {
        if self.stopped.is_some() {
            return true;
        }
        if Instant::now() > self.opts.deadline {
            self.stopped = Some("the time limit");
        } else if self.out.len() >= self.opts.max_output {
            self.stopped = Some("the output limit");
        }
        self.stopped.is_some()
    }

    /// Depth first, in name order, never following links, searching each file as it is reached.
    fn walk(&mut self, root: &Path) {
        let mut pending = vec![root.to_path_buf()];
        while let Some(dir) = pending.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                self.skipped.count(Skip::Unreadable);
                continue;
            };
            let mut entries: Vec<_> = entries.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            let mut subdirs = Vec::new();
            for entry in entries {
                if self.should_stop() {
                    return;
                }
                self.visit(&entry, &mut subdirs);
            }
            pending.extend(subdirs.into_iter().rev());
        }
    }

    fn visit(&mut self, entry: &fs::DirEntry, subdirs: &mut Vec<PathBuf>) {
        let Ok(kind) = entry.file_type() else { return };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            return;
        }
        if kind.is_dir() && !SKIPPED_DIRS.contains(&&*name) {
            subdirs.push(entry.path());
        } else if kind.is_file() && has_extension(&name, self.extensions.as_deref()) {
            self.search_file(&entry.path());
        }
    }

    fn search_file(&mut self, path: &Path) {
        let text = match read_text(path) {
            Ok(text) => text,
            Err(why) => return self.skipped.count(why),
        };
        let lines: Vec<&str> = text.lines().collect();
        let matched: Vec<bool> = lines.iter().map(|l| self.regex.is_match(l)).collect();
        let context = self.opts.context;
        let shown = path.display().to_string();
        let mut last_printed: Option<usize> = None;
        for hit in (0..lines.len()).filter(|i| matched[*i]) {
            let start = hit.saturating_sub(context);
            let end = hit.saturating_add(context).min(lines.len() - 1);
            let from = last_printed.map_or(start, |last| start.max(last + 1));
            if from > end {
                continue;
            }
            if self.should_stop() {
                return;
            }
            let joins_previous = last_printed.is_some_and(|last| from == last + 1);
            if context > 0 && !self.out.is_empty() && !joins_previous {
                self.out.push_str(GROUP_SEPARATOR);
                self.out.push('\n');
            }
            for (i, line) in lines.iter().enumerate().take(end + 1).skip(from) {
                let mark = if matched[i] { ':' } else { '-' };
                self.out
                    .push_str(&format!("{shown}{mark}{}{mark}{line}\n", i + 1));
            }
            last_printed = Some(end);
        }
    }

    fn finish(self) -> Found {
        let mut notes = self.skipped.notes();
        if let Some(why) = self.stopped {
            notes.push(format!(
                "The search stopped at {why}, so these results are incomplete."
            ));
        }
        Found {
            matches: self.out,
            notes,
        }
    }
}

fn has_extension(name: &str, extensions: Option<&[String]>) -> bool {
    let Some(extensions) = extensions else {
        return true;
    };
    let ext = Path::new(name).extension().and_then(|e| e.to_str());
    ext.is_some_and(|ext| extensions.iter().any(|e| e == ext))
}

fn read_text(path: &Path) -> std::result::Result<String, Skip> {
    let mut file = fs::File::open(path).map_err(|_| Skip::Unreadable)?;
    let len = file.metadata().map_err(|_| Skip::Unreadable)?.len();
    if len > MAX_FILE_BYTES {
        return Err(Skip::TooLarge);
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|_| Skip::Unreadable)?;
    let sniff = &bytes[..bytes.len().min(BINARY_SNIFF_BYTES)];
    if sniff.contains(&0) {
        return Err(Skip::Binary);
    }
    String::from_utf8(bytes).map_err(|_| Skip::NotText)
}

#[cfg(test)]
#[path = "search_fallback_tests.rs"]
mod tests;
