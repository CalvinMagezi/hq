//! The small document model every native reader fills in, and the one place
//! that turns it into Markdown.
//!
//! Readers (docx, pptx, odf, rtf, html, xml) describe what they found as
//! [`Block`]s of [`Inline`] runs. Rendering lives here so that escaping,
//! emphasis spacing, list indentation and table layout are the same for every
//! format, and a fix lands everywhere at once.

use std::fmt::Write as _;

use super::report::Losses;

/// What a reader hands back: the blocks it found, a title if the file names
/// one, and what it had to leave out.
#[derive(Debug, Default)]
pub struct Parsed {
    pub blocks: Vec<Block>,
    pub title: Option<String>,
    pub losses: Losses,
}

/// Whether a font name is a fixed-width one, so its text reads as code.
pub fn is_mono_font(name: &str) -> bool {
    let n = name.to_lowercase();
    [
        "consolas",
        "courier",
        "monospace",
        "menlo",
        "monaco",
        "lucida console",
        "inconsolata",
        "fira code",
        "fira mono",
        "cascadia",
        "andale mono",
        "source code",
        "dejavu sans mono",
        "roboto mono",
        "sf mono",
        "jetbrains mono",
        "liberation mono",
    ]
    .iter()
    .any(|k| n.contains(k))
}

/// A run of text with one set of character formatting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    /// Monospace text, rendered as an inline code span.
    pub code: bool,
    pub link: Option<String>,
}

impl Run {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    fn same_format(&self, other: &Run) -> bool {
        self.bold == other.bold
            && self.italic == other.italic
            && self.strike == other.strike
            && self.code == other.code
            && self.link == other.link
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Run(Run),
    /// A line break inside a paragraph.
    Break,
    /// A picture that cannot be carried into Markdown; the alt text, if any.
    Image(String),
    /// A footnote reference, by the id the reader uses for its definition.
    FootnoteRef(String),
}

impl Inline {
    pub fn text(text: impl Into<String>) -> Self {
        Inline::Run(Run::plain(text))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
    /// Nesting depth, 0 for the outermost list.
    pub level: usize,
    pub ordered: bool,
    pub content: Vec<Inline>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Table {
    /// Every row is a list of cells; a cell is a list of inline runs.
    pub rows: Vec<Vec<Vec<Inline>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// Level 0 is a document title; 1 to 6 are ordinary headings.
    Heading(u8, Vec<Inline>),
    Para(Vec<Inline>),
    List(Vec<ListItem>),
    Table(Table),
    Code {
        lang: Option<String>,
        text: String,
    },
    Quote(Vec<Block>),
    Rule,
    /// Markdown that is already final, such as footnote definitions.
    Raw(String),
}

/// Collapses every run of whitespace to one space, as HTML and ODF do for text.
pub fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            space = true;
        } else {
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
    }
    if space {
        out.push(' ');
    }
    out
}

/// Alt text worth keeping: not an empty string, a file name or a generated label.
pub fn clean_alt(alt: &str) -> String {
    let a = alt.trim();
    let lower = a.to_lowercase();
    let file_like = [
        ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".tif", ".tiff", ".webp", ".svg", ".emf", ".wmf",
    ]
    .iter()
    .any(|e| lower.ends_with(e));
    let generated = [
        "picture ",
        "image ",
        "graphic ",
        "figure ",
        "object ",
        "content placeholder ",
    ]
    .iter()
    .any(|p| lower.starts_with(p) && lower[p.len()..].chars().all(|c| c.is_ascii_digit()));
    if file_like || generated {
        String::new()
    } else {
        a.to_string()
    }
}

/// Plain text of some inline runs, for titles and alt text.
pub fn plain_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for i in inlines {
        match i {
            Inline::Run(r) => out.push_str(&r.text),
            Inline::Break => out.push(' '),
            Inline::Image(_) | Inline::FootnoteRef(_) => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Paragraph,
    Heading,
    ListItem,
    TableCell,
}

/// Renders blocks to Markdown.
///
/// A document with a title block gets that title as the only `#` heading and
/// every other heading moves down one level.
pub fn render(mut blocks: Vec<Block>) -> String {
    shift_headings(&mut blocks);
    let blocks = merge_adjacent(blocks);
    let mut parts: Vec<String> = Vec::new();
    for block in &blocks {
        let text = render_block(block);
        if !text.trim().is_empty() {
            parts.push(text);
        }
    }
    let mut out = parts.join("\n\n");
    out = tidy(&out);
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn shift_headings(blocks: &mut [Block]) {
    let has_title = blocks.iter().any(|b| matches!(b, Block::Heading(0, _)));
    for b in blocks.iter_mut() {
        if let Block::Heading(level, _) = b {
            *level = if has_title {
                (*level + 1).min(6)
            } else {
                (*level).clamp(1, 6)
            };
        }
    }
}

/// Joins neighbouring code blocks and quotes that a reader emitted line by line.
fn merge_adjacent(blocks: Vec<Block>) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::with_capacity(blocks.len());
    for block in blocks {
        match (out.last_mut(), block) {
            (
                Some(Block::Code { lang, text }),
                Block::Code {
                    lang: next_lang,
                    text: next_text,
                },
            ) if *lang == next_lang || next_lang.is_none() => {
                text.push('\n');
                text.push_str(&next_text);
            }
            (Some(Block::Quote(prev)), Block::Quote(next)) => prev.extend(next),
            // Bullet lists that a producer split in two read as one list.
            (Some(Block::List(prev)), Block::List(next))
                if prev.last().is_some_and(|i| !i.ordered)
                    && next.first().is_some_and(|i| !i.ordered) =>
            {
                prev.extend(next)
            }
            (_, other) => out.push(other),
        }
    }
    out
}

fn render_block(block: &Block) -> String {
    match block {
        Block::Heading(level, inlines) => {
            // Headings are bold by nature; markers inside them are noise.
            let plain: Vec<Inline> = inlines
                .iter()
                .map(|i| match i {
                    Inline::Run(r) => Inline::Run(Run {
                        bold: false,
                        ..r.clone()
                    }),
                    other => other.clone(),
                })
                .collect();
            let text = render_inlines(&plain, Mode::Heading);
            if text.trim().is_empty() {
                return String::new();
            }
            format!(
                "{} {}",
                "#".repeat((*level).clamp(1, 6) as usize),
                text.trim()
            )
        }
        Block::Para(inlines) => {
            let text = render_inlines(inlines, Mode::Paragraph);
            if text.trim().is_empty() {
                return String::new();
            }
            escape_line_starts(text.trim())
        }
        Block::List(items) => render_list(items),
        Block::Table(table) => render_table(table),
        Block::Code { lang, text } => {
            let text = text.trim_matches('\n');
            if text.trim().is_empty() {
                return String::new();
            }
            let fence = fence_for(text, '`', 3);
            format!("{fence}{}\n{text}\n{fence}", lang.as_deref().unwrap_or(""))
        }
        Block::Quote(inner) => {
            let mut parts = Vec::new();
            for b in inner {
                let t = render_block(b);
                if !t.trim().is_empty() {
                    parts.push(t);
                }
            }
            parts
                .join("\n\n")
                .lines()
                .map(|l| {
                    if l.is_empty() {
                        ">".to_string()
                    } else {
                        format!("> {l}")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        Block::Rule => "---".to_string(),
        Block::Raw(text) => text.clone(),
    }
}

fn render_list(items: &[ListItem]) -> String {
    let mut lines: Vec<String> = Vec::new();
    // Next number to print at each depth; deeper counters restart when a shallower item appears.
    let mut counters: Vec<u64> = Vec::new();
    for item in items {
        let text = render_inlines(&item.content, Mode::ListItem);
        if text.trim().is_empty() {
            continue;
        }
        counters.truncate(item.level + 1);
        while counters.len() <= item.level {
            counters.push(1);
        }
        let indent = "    ".repeat(item.level);
        let marker = if item.ordered {
            let n = counters[item.level];
            counters[item.level] += 1;
            format!("{n}. ")
        } else {
            counters[item.level] = 1;
            "- ".to_string()
        };
        let cont = format!("{indent}{}", " ".repeat(marker.len()));
        let body = text.trim().replace('\n', &format!("\n{cont}"));
        lines.push(format!("{indent}{marker}{body}"));
    }
    lines.join("\n")
}

fn render_table(table: &Table) -> String {
    let cols = table.rows.iter().map(Vec::len).max().unwrap_or(0);
    if cols == 0 {
        return String::new();
    }
    let rows: Vec<Vec<String>> = table
        .rows
        .iter()
        .map(|row| {
            let mut cells: Vec<String> = row
                .iter()
                .map(|c| {
                    render_inlines(c, Mode::TableCell)
                        .trim()
                        .replace('\n', "<br>")
                })
                .collect();
            cells.resize(cols, String::new());
            cells
        })
        .collect();
    if rows.iter().all(|r| r.iter().all(|c| c.is_empty())) {
        return String::new();
    }
    let mut out = String::new();
    for (i, row) in rows.iter().enumerate() {
        let _ = writeln!(out, "| {} |", row.join(" | "));
        if i == 0 {
            let _ = writeln!(out, "|{}|", vec![" --- "; cols].join("|"));
        }
    }
    out.trim_end().to_string()
}

/// Renders inline runs: merges runs with the same formatting, moves the
/// whitespace outside emphasis markers, and groups runs under one link.
fn render_inlines(inlines: &[Inline], mode: Mode) -> String {
    let merged = merge_runs(inlines);
    let mut out = String::new();
    let mut i = 0;
    while i < merged.len() {
        match &merged[i] {
            Inline::Run(run) if run.link.is_some() => {
                let url = run.link.clone().unwrap_or_default();
                if run.text.starts_with(char::is_whitespace)
                    && !out.is_empty()
                    && !out.ends_with(' ')
                    && !out.ends_with('\n')
                {
                    out.push(' ');
                }
                let mut j = i;
                let mut inner = String::new();
                while j < merged.len() {
                    match &merged[j] {
                        Inline::Run(r) if r.link.as_deref() == Some(url.as_str()) => {
                            let mut plain = r.clone();
                            plain.link = None;
                            push_run(&mut inner, &plain, mode);
                            j += 1;
                        }
                        _ => break,
                    }
                }
                push_link(&mut out, &inner, &url);
                i = j;
            }
            Inline::Run(run) => {
                push_run(&mut out, run, mode);
                i += 1;
            }
            Inline::Break => {
                match mode {
                    Mode::Paragraph => out.push_str("\\\n"),
                    Mode::ListItem | Mode::TableCell => out.push('\n'),
                    Mode::Heading => out.push(' '),
                }
                i += 1;
            }
            Inline::Image(alt) => {
                if !out.is_empty() && !out.ends_with([' ', '\n']) {
                    out.push(' ');
                }
                if alt.trim().is_empty() {
                    out.push_str("*[image]*");
                } else {
                    let _ = write!(out, "*[image: {}]*", escape_inline(alt.trim(), false));
                }
                i += 1;
            }
            Inline::FootnoteRef(id) => {
                let _ = write!(out, "[^{id}]");
                i += 1;
            }
        }
    }
    out
}

fn merge_runs(inlines: &[Inline]) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(inlines.len());
    for inline in inlines {
        match (out.last_mut(), inline) {
            (Some(Inline::Run(prev)), Inline::Run(next)) if prev.same_format(next) => {
                prev.text.push_str(&next.text);
            }
            _ => out.push(inline.clone()),
        }
    }
    out
}

fn push_link(out: &mut String, inner: &str, url: &str) {
    let trimmed = inner.trim();
    if trimmed.is_empty() {
        out.push_str(inner);
        return;
    }
    let lead = &inner[..inner.len() - inner.trim_start().len()];
    let trail = &inner[inner.trim_end().len()..];
    let dest = if url.contains([' ', '(', ')', '<', '>']) {
        format!("<{}>", url.replace('<', "%3C").replace('>', "%3E"))
    } else {
        url.to_string()
    };
    let _ = write!(out, "{lead}[{trimmed}]({dest}){trail}");
}

fn push_run(out: &mut String, run: &Run, mode: Mode) {
    let text = run.text.replace(['\r', '\t', '\u{a0}'], " ");
    let text = text.replace('\n', " ");
    // Markdown collapses runs of spaces, so text that is not code does too.
    let text = if run.code { text } else { collapse_ws(&text) };
    if text.is_empty() {
        return;
    }
    let core = text.trim();
    if core.is_empty() {
        if !out.ends_with(' ') && !out.is_empty() {
            out.push(' ');
        }
        return;
    }
    let lead = &text[..text.len() - text.trim_start().len()];
    let trail = &text[text.trim_end().len()..];
    let mut body = if run.code {
        code_span(core)
    } else {
        escape_inline(core, mode == Mode::TableCell)
    };
    if run.strike {
        body = format!("~~{body}~~");
    }
    let after_star = out.ends_with('*') && lead.is_empty();
    if !run.code {
        let (bold_mark, italic_mark) = if after_star { ("__", "_") } else { ("**", "*") };
        if run.bold && run.italic {
            body = format!("{bold_mark}{italic_mark}{body}{italic_mark}{bold_mark}");
        } else if run.bold {
            body = format!("{bold_mark}{body}{bold_mark}");
        } else if run.italic {
            body = format!("{italic_mark}{body}{italic_mark}");
        }
    }
    if !lead.is_empty() && !out.ends_with(' ') && !out.ends_with('\n') && !out.is_empty() {
        out.push(' ');
    }
    out.push_str(&body);
    if !trail.is_empty() {
        out.push(' ');
    }
}

fn code_span(text: &str) -> String {
    let fence = fence_for(text, '`', 1);
    if text.starts_with('`') || text.ends_with('`') {
        format!("{fence} {text} {fence}")
    } else {
        format!("{fence}{text}{fence}")
    }
}

/// A fence of `ch` one longer than any run of it inside `text`, and at least `min` long.
fn fence_for(text: &str, ch: char, min: usize) -> String {
    let mut longest = 0;
    let mut current = 0;
    for c in text.chars() {
        if c == ch {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    ch.to_string().repeat((longest + 1).max(min))
}

fn escape_inline(s: &str, in_table: bool) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        let prev = i.checked_sub(1).map(|j| chars[j]);
        let next = chars.get(i + 1).copied();
        match c {
            '\\' | '`' | '*' | '[' | ']' => {
                out.push('\\');
                out.push(c);
            }
            '<' if next
                .is_some_and(|n| n.is_ascii_alphabetic() || n == '/' || n == '!' || n == '?') =>
            {
                out.push('\\');
                out.push(c);
            }
            '_' => {
                let intraword = prev.is_some_and(char::is_alphanumeric)
                    && next.is_some_and(char::is_alphanumeric);
                if !intraword {
                    out.push('\\');
                }
                out.push('_');
            }
            '~' if next == Some('~') || prev == Some('~') => {
                out.push('\\');
                out.push(c);
            }
            '|' if in_table => out.push_str("\\|"),
            _ => out.push(c),
        }
    }
    out
}

/// Stops a paragraph that happens to begin like Markdown syntax from turning
/// into a heading, list or quote.
fn escape_line_starts(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            let t = line.trim_start();
            let numbered = {
                let digits = t.chars().take_while(char::is_ascii_digit).count();
                digits > 0
                    && digits <= 9
                    && matches!(t[digits..].chars().next(), Some('.' | ')'))
                    && t[digits + 1..].starts_with(' ')
            };
            let bullet =
                matches!(t.chars().next(), Some('-' | '+' | '*')) && t[1..].starts_with(' ');
            let heading = t.starts_with('#');
            let quote = t.starts_with('>');
            let rule = t.len() >= 3 && t.chars().all(|c| c == '-' || c == '=' || c == '_');
            if numbered {
                let digits = t.chars().take_while(char::is_ascii_digit).count();
                format!(
                    "{}{}\\{}",
                    &line[..line.len() - t.len()],
                    &t[..digits],
                    &t[digits..]
                )
            } else if bullet || heading || quote || rule {
                format!("{}\\{}", &line[..line.len() - t.len()], t)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Collapses runs of blank lines and trims trailing spaces.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim_matches('\n').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> Inline {
        Inline::text(text)
    }

    fn bold(text: &str) -> Inline {
        Inline::Run(Run {
            text: text.into(),
            bold: true,
            ..Run::default()
        })
    }

    #[test]
    fn adjacent_runs_with_the_same_format_merge_and_whitespace_leaves_the_markers() {
        let md = render(vec![Block::Para(vec![
            run("We saw "),
            bold("three "),
            bold("sites"),
            run(" today."),
        ])]);
        assert_eq!(md, "We saw **three sites** today.\n");
    }

    #[test]
    fn whitespace_collapses_and_generated_alt_text_is_dropped() {
        assert_eq!(collapse_ws("a \n  b\t c"), "a b c");
        assert_eq!(clean_alt("Picture 12"), "");
        assert_eq!(clean_alt("_pic.png"), "");
        assert_eq!(clean_alt("Field photo"), "Field photo");
    }

    #[test]
    fn headings_drop_bold_markers() {
        let md = render(vec![Block::Heading(1, vec![bold("Payment details")])]);
        assert_eq!(md, "# Payment details\n");
    }

    #[test]
    fn title_block_becomes_the_only_h1() {
        let md = render(vec![
            Block::Heading(0, vec![run("Report")]),
            Block::Heading(1, vec![run("Intro")]),
            Block::Heading(2, vec![run("Detail")]),
        ]);
        assert_eq!(md, "# Report\n\n## Intro\n\n### Detail\n");
    }

    #[test]
    fn without_a_title_the_first_level_is_h1() {
        let md = render(vec![Block::Heading(1, vec![run("Intro")])]);
        assert_eq!(md, "# Intro\n");
    }

    #[test]
    fn links_wrap_emphasis_and_keep_outer_spaces() {
        let md = render(vec![Block::Para(vec![
            run("See "),
            Inline::Run(Run {
                text: "the plan ".into(),
                link: Some("https://example.com/plan".into()),
                ..Run::default()
            }),
            run("now"),
        ])]);
        assert_eq!(md, "See [the plan](https://example.com/plan) now\n");
    }

    #[test]
    fn nested_lists_indent_and_number_per_level() {
        let md = render(vec![Block::List(vec![
            ListItem {
                level: 0,
                ordered: true,
                content: vec![run("one")],
            },
            ListItem {
                level: 1,
                ordered: false,
                content: vec![run("inner")],
            },
            ListItem {
                level: 0,
                ordered: true,
                content: vec![run("two")],
            },
        ])]);
        assert_eq!(md, "1. one\n    - inner\n2. two\n");
    }

    #[test]
    fn tables_pad_short_rows_and_escape_pipes_and_breaks() {
        let t = Table {
            rows: vec![
                vec![vec![run("Site")], vec![run("Note")]],
                vec![vec![run("A|B")], vec![run("x"), Inline::Break, run("y")]],
                vec![vec![run("only")]],
            ],
        };
        let md = render(vec![Block::Table(t)]);
        assert_eq!(
            md,
            "| Site | Note |\n| --- | --- |\n| A\\|B | x<br>y |\n| only |  |\n"
        );
    }

    #[test]
    fn adjacent_code_lines_merge_into_one_fenced_block() {
        let md = render(vec![
            Block::Code {
                lang: None,
                text: "a = 1".into(),
            },
            Block::Code {
                lang: None,
                text: "b = 2".into(),
            },
        ]);
        assert_eq!(md, "```\na = 1\nb = 2\n```\n");
    }

    #[test]
    fn text_that_looks_like_markdown_is_escaped() {
        let md = render(vec![
            Block::Para(vec![run("# not a heading")]),
            Block::Para(vec![run("1. not a list")]),
            Block::Para(vec![run("snake_case and 2 * 3")]),
        ]);
        assert_eq!(
            md,
            "\\# not a heading\n\n1\\. not a list\n\nsnake_case and 2 \\* 3\n"
        );
    }

    #[test]
    fn bold_directly_followed_by_italic_does_not_collide() {
        let md = render(vec![Block::Para(vec![
            bold("a"),
            Inline::Run(Run {
                text: "b".into(),
                italic: true,
                ..Run::default()
            }),
        ])]);
        assert_eq!(md, "**a**_b_\n");
    }
}
