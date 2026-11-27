//! One grammar for links between notes.
//!
//! Wikilinks (`[[Note]]`, `[[Note#Heading|label]]`, `[[Folder/Note]]`, `![[embed]]`) and
//! markdown links (`[label](Note.md#heading)`) are read by [`extract`], which skips fenced
//! and inline code and reports where each link sits in the text. [`NoteSet`] says what a link
//! points at: one note, several notes (never guessed between), an attachment, an outside
//! address, the note itself, or nothing. [`retarget`] and [`apply_edits`] change only the
//! target part of a link, leaving the label, heading and alias alone.
//!
//! Indented code blocks are not treated as code: a list continuation looks the same, and a
//! link wrongly skipped is worse than one wrongly counted.
//!
//! Everything here is pure: no disk, no database, no clock. The health report, the move
//! tools and the fix planner all read links through this module so they cannot disagree.

use std::collections::HashMap;
use std::ops::Range;
use unicode_normalization::UnicodeNormalization;

/// How a link was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Syntax {
    /// `[[Target]]` or `![[Target]]`.
    Wiki,
    /// `[label](target)` or `![alt](target)`.
    Markdown,
}

/// One link found in a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRef {
    pub syntax: Syntax,
    /// `![[x]]` and `![x](y)`: the target is shown in place, not followed.
    pub embed: bool,
    /// The whole link, including a leading `!`.
    pub span: Range<usize>,
    /// Only the path part of the target, as written (before any `#heading`).
    pub target_span: Range<usize>,
    /// The path part, with `%20`-style escapes decoded. Empty for a link to the same note.
    pub target: String,
    /// The text after `#`, when it names a heading.
    pub heading: Option<String>,
    /// The id after `#^`, when it names a block.
    pub block: Option<String>,
    /// The label after `|` in a wikilink.
    pub alias: Option<String>,
    /// 1-based line of the link's first character.
    pub line: usize,
    /// The link sits in the leading `---` block.
    pub in_frontmatter: bool,
    /// Markdown only: the target was written inside `<...>`.
    pub angle: bool,
}

const MAX_WIKI_INNER: usize = 400;
const MAX_MD_LABEL: usize = 1000;
const MAX_MD_DEST: usize = 2000;

/// Every link in `content`, in order, outside code. Links in the frontmatter are included and
/// flagged, because a property such as `related: "[[Note]]"` is a real link.
pub fn extract(content: &str) -> Vec<LinkRef> {
    let body_start = content.len()
        - hq_core::frontmatter_utils::split_frontmatter(content)
            .1
            .len();
    let code = code_ranges(content, body_start);
    let newlines: Vec<usize> = content
        .bytes()
        .enumerate()
        .filter(|(_, b)| *b == b'\n')
        .map(|(i, _)| i)
        .collect();
    let line_of = |pos: usize| newlines.partition_point(|&n| n < pos) + 1;

    let bytes = content.as_bytes();
    let mut out = Vec::new();
    let mut code_idx = 0;
    let mut i = 0;
    while i < bytes.len() {
        while code_idx < code.len() && code[code_idx].end <= i {
            code_idx += 1;
        }
        if code_idx < code.len() && code[code_idx].start <= i {
            i = code[code_idx].end;
            continue;
        }
        let parsed = match bytes[i] {
            b'\\' => {
                // An escaped character is not link syntax.
                i += 1 + content[i + 1..].chars().next().map_or(0, char::len_utf8);
                continue;
            }
            b'!' if bytes.get(i + 1) == Some(&b'[') && bytes.get(i + 2) == Some(&b'[') => {
                wiki_at(content, i + 1, true).map(|mut l| {
                    l.span.start = i;
                    l
                })
            }
            b'!' if bytes.get(i + 1) == Some(&b'[') => {
                markdown_at(content, i + 1, true).map(|mut l| {
                    l.span.start = i;
                    l
                })
            }
            b'[' if bytes.get(i + 1) == Some(&b'[') => wiki_at(content, i, false),
            b'[' => markdown_at(content, i, false),
            _ => None,
        };
        match parsed {
            Some(mut link) => {
                link.line = line_of(link.span.start);
                link.in_frontmatter = link.span.start < body_start;
                i = link.span.end;
                out.push(link);
            }
            None => i += 1,
        }
    }
    out
}

/// `[[inner]]` starting at `p`, which points at the first `[`.
fn wiki_at(content: &str, p: usize, embed: bool) -> Option<LinkRef> {
    let rest = &content[p + 2..];
    let limit = rest.len().min(MAX_WIKI_INNER);
    let mut end = None;
    let rb = rest.as_bytes();
    let mut j = 0;
    while j < limit {
        match rb[j] {
            b'\n' | b'\r' | b'[' => return None,
            b']' if rb.get(j + 1) == Some(&b']') => {
                end = Some(j);
                break;
            }
            _ => j += 1,
        }
    }
    let end = end?;
    let inner_start = p + 2;
    let inner = &content[inner_start..inner_start + end];

    // The alias starts at the first `|`. Inside a table a pipe is written `\|`.
    let (left, alias) = match inner.find('|') {
        Some(k) => {
            let alias = inner[k + 1..].trim();
            (&inner[..k], (!alias.is_empty()).then(|| alias.to_string()))
        }
        None => (inner, None),
    };
    let left_len = left.trim_end_matches('\\').len();
    let left = &left[..left_len];

    let (path_part, frag) = match left.find('#') {
        Some(h) => (&left[..h], Some(&left[h + 1..])),
        None => (left, None),
    };
    let (ts, te) = trimmed_range(content, inner_start, inner_start + path_part.len());
    let target = content[ts..te].to_string();
    let (heading, block) = split_fragment(frag);
    if target.is_empty() && heading.is_none() && block.is_none() {
        return None;
    }
    Some(LinkRef {
        syntax: Syntax::Wiki,
        embed,
        span: p..p + 2 + end + 2,
        target_span: ts..te,
        target,
        heading,
        block,
        alias,
        line: 0,
        in_frontmatter: false,
        angle: false,
    })
}

fn split_fragment(frag: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(frag) = frag.map(str::trim).filter(|f| !f.is_empty()) else {
        return (None, None);
    };
    match frag.strip_prefix('^') {
        Some(id) if !id.is_empty() => (None, Some(id.to_string())),
        _ => (Some(frag.to_string()), None),
    }
}

/// `[label](dest)` starting at `p`, which points at the `[`.
fn markdown_at(content: &str, p: usize, embed: bool) -> Option<LinkRef> {
    let b = content.as_bytes();
    // The label: balanced brackets on one line.
    let mut depth = 0usize;
    let mut j = p;
    let label_end = loop {
        if j >= b.len() || j - p > MAX_MD_LABEL {
            return None;
        }
        match b[j] {
            b'\n' => return None,
            b'\\' => j += 1,
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break j;
                }
            }
            _ => {}
        }
        j += 1;
    };
    if b.get(label_end + 1) != Some(&b'(') {
        return None;
    }
    let mut k = label_end + 2;
    while matches!(b.get(k), Some(b' ' | b'\t')) {
        k += 1;
    }
    let (dest_start, dest_end, angle) = if b.get(k) == Some(&b'<') {
        let start = k + 1;
        let close = content[start..]
            .find(['>', '\n'])
            .filter(|&c| b[start + c] == b'>')?;
        (start, start + close, true)
    } else {
        let start = k;
        let mut parens = 0usize;
        let mut e = start;
        while e < b.len() && e - start < MAX_MD_DEST {
            match b[e] {
                b' ' | b'\t' | b'\n' => break,
                b'(' => parens += 1,
                b')' if parens == 0 => break,
                b')' => parens -= 1,
                b'\\' => e += 1,
                _ => {}
            }
            e += 1;
        }
        (start, e.min(b.len()), false)
    };
    if dest_end <= dest_start {
        return None;
    }
    // After the destination: an optional "title", then `)`.
    let mut m = dest_end + usize::from(angle);
    while matches!(b.get(m), Some(b' ' | b'\t')) {
        m += 1;
    }
    if let Some(&q) = b.get(m).filter(|c| matches!(**c, b'"' | b'\'' | b'(')) {
        let close_q = if q == b'(' { b')' } else { q };
        let t = content[m + 1..].find(close_q as char)?;
        if content[m + 1..m + 1 + t].contains('\n') {
            return None;
        }
        m += t + 2;
        while matches!(b.get(m), Some(b' ' | b'\t')) {
            m += 1;
        }
    }
    if b.get(m) != Some(&b')') {
        return None;
    }
    let span_end = m + 1;

    let raw_dest = &content[dest_start..dest_end];
    let (raw_path, frag) = match raw_dest.find('#') {
        Some(h) => (&raw_dest[..h], Some(&raw_dest[h + 1..])),
        None => (raw_dest, None),
    };
    let target = percent_decode(raw_path);
    let (heading, block) = split_fragment(frag.map(percent_decode).as_deref());
    Some(LinkRef {
        syntax: Syntax::Markdown,
        embed,
        span: p..span_end,
        target_span: dest_start..dest_start + raw_path.len(),
        target,
        heading,
        block,
        alias: None,
        line: 0,
        in_frontmatter: false,
        angle,
    })
}

/// `start..end` shrunk past surrounding whitespace.
fn trimmed_range(content: &str, start: usize, end: usize) -> (usize, usize) {
    let s = &content[start..end];
    let lead = s.len() - s.trim_start().len();
    let trail = s.len() - s.trim_end().len();
    if lead == s.len() {
        return (start, start);
    }
    (start + lead, end - trail)
}

fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

// ─── Code ────────────────────────────────────────────────────────────────────

/// Byte ranges of fenced code blocks and inline code spans in the body, ascending and
/// disjoint. A link that starts inside one is not a link.
fn code_ranges(content: &str, body_start: usize) -> Vec<Range<usize>> {
    let mut ranges: Vec<Range<usize>> = Vec::new();
    let mut fence: Option<(u8, usize, usize)> = None; // (char, length, start)
    let mut open: Option<(usize, usize)> = None; // inline code: (run length, start)
    let mut offset = body_start;
    for raw_line in content[body_start..].split_inclusive('\n') {
        let line_start = offset;
        offset += raw_line.len();
        let line = raw_line.trim_end_matches(['\n', '\r']);

        if let Some((ch, len, start)) = fence {
            if is_closing_fence(line, ch, len) {
                ranges.push(start..offset);
                fence = None;
            }
            continue;
        }
        if let Some((ch, len)) = opening_fence(line) {
            open = None;
            fence = Some((ch, len, line_start));
            continue;
        }
        if line.trim().is_empty() {
            open = None;
            continue;
        }
        let lb = line.as_bytes();
        let mut j = 0;
        while j < lb.len() {
            match lb[j] {
                b'\\' if open.is_none() => j += 2,
                b'`' => {
                    let run_start = j;
                    while j < lb.len() && lb[j] == b'`' {
                        j += 1;
                    }
                    let run = j - run_start;
                    match open {
                        Some((n, start)) if n == run => {
                            ranges.push(start..line_start + j);
                            open = None;
                        }
                        None => open = Some((run, line_start + run_start)),
                        Some(_) => {}
                    }
                }
                _ => j += 1,
            }
        }
    }
    if let Some((_, _, start)) = fence {
        ranges.push(start..content.len());
    }
    ranges
}

/// A fence-opening line: up to three spaces, then three or more backticks or tildes. A
/// backtick fence's info string may not contain a backtick (that is inline code).
fn opening_fence(line: &str) -> Option<(u8, usize)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let ch = *trimmed
        .as_bytes()
        .first()
        .filter(|c| matches!(**c, b'`' | b'~'))?;
    let len = trimmed.bytes().take_while(|b| *b == ch).count();
    if len < 3 || (ch == b'`' && trimmed[len..].contains('`')) {
        return None;
    }
    Some((ch, len))
}

fn is_closing_fence(line: &str, ch: u8, len: usize) -> bool {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return false;
    }
    let run = trimmed.bytes().take_while(|b| *b == ch).count();
    run >= len && trimmed[run..].trim().is_empty()
}

// ─── Resolution ──────────────────────────────────────────────────────────────

/// What a link points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Exactly one note, by its vault-relative path.
    Unique(String),
    /// More than one note fits. Nothing here picks one.
    Ambiguous(Vec<String>),
    /// A file that is not a note (an image, a PDF).
    Attachment(String),
    /// A web address, `mailto:` and the like.
    External,
    /// The link has no target path: `[[#Heading]]`, `[x](#heading)`.
    SameNote,
    /// Nothing by that name.
    Dangling,
}

/// A note as the resolver needs it: its path and the names it also answers to.
#[derive(Debug, Clone)]
pub struct NoteInput {
    /// Vault-relative, `/`-separated, with the `.md` ending.
    pub path: String,
    /// `aliases:` from the frontmatter.
    pub aliases: Vec<String>,
}

/// Every note and attachment a link may name, indexed for lookup.
#[derive(Debug, Default)]
pub struct NoteSet {
    notes: Vec<Note>,
    by_full: HashMap<String, Vec<usize>>,
    by_stem: HashMap<String, Vec<usize>>,
    by_alias: HashMap<String, Vec<usize>>,
    attachments: Vec<(String, String)>,
    by_att_name: HashMap<String, Vec<usize>>,
    by_att_full: HashMap<String, usize>,
}

#[derive(Debug)]
struct Note {
    path: String,
    /// Folded path without `.md`.
    full: String,
}

impl NoteSet {
    pub fn new(
        notes: impl IntoIterator<Item = NoteInput>,
        attachments: impl IntoIterator<Item = String>,
    ) -> Self {
        let mut set = NoteSet::default();
        let mut inputs: Vec<NoteInput> = notes.into_iter().collect();
        inputs.sort_by(|a, b| a.path.cmp(&b.path));
        for input in inputs {
            let idx = set.notes.len();
            let full = fold(strip_md(&input.path));
            let stem = full.rsplit('/').next().unwrap_or(&full).to_string();
            set.by_full.entry(full.clone()).or_default().push(idx);
            set.by_stem.entry(stem).or_default().push(idx);
            for alias in &input.aliases {
                let key = fold(alias);
                if !key.is_empty() {
                    set.by_alias.entry(key).or_default().push(idx);
                }
            }
            set.notes.push(Note {
                path: input.path,
                full,
            });
        }
        let mut atts: Vec<String> = attachments.into_iter().collect();
        atts.sort();
        for path in atts {
            let idx = set.attachments.len();
            let folded = fold(&path);
            let name = folded.rsplit('/').next().unwrap_or(&folded).to_string();
            set.by_att_name.entry(name).or_default().push(idx);
            set.by_att_full.entry(folded.clone()).or_insert(idx);
            set.attachments.push((folded, path));
        }
        set
    }

    pub fn len(&self) -> usize {
        self.notes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// Whether `path` (vault-relative, with `.md`) is a note in the set.
    pub fn contains(&self, path: &str) -> bool {
        self.by_full
            .get(&fold(strip_md(path)))
            .is_some_and(|v| v.iter().any(|&i| self.notes[i].path == path))
    }

    /// What `link`, written in the note at `from`, points at.
    pub fn resolve(&self, from: &str, link: &LinkRef) -> Resolution {
        self.resolve_target(from, link.syntax, &link.target)
    }

    /// What a target written as `target`, in a link of the given syntax in the note at `from`,
    /// points at. For trying a spelling before it is written into a note.
    pub fn resolve_target(&self, from: &str, syntax: Syntax, target: &str) -> Resolution {
        let target = target.trim();
        if target.is_empty() {
            return Resolution::SameNote;
        }
        if syntax == Syntax::Markdown && is_external(target) {
            return Resolution::External;
        }
        let target = target.trim_start_matches("./");
        if syntax == Syntax::Markdown
            && let Some(found) = self.resolve_path_form(from, target)
        {
            return found;
        }
        self.resolve_name(target.trim_start_matches('/'))
    }

    /// Whether `path` is exactly a note (with or without `.md`) or an attachment in the set.
    pub fn knows_path(&self, path: &str) -> bool {
        let path = path.trim_start_matches('/');
        self.by_full.contains_key(&fold(strip_md(path)))
            || self.by_att_full.contains_key(&fold(path))
    }

    /// A markdown link is a path: relative to the note it sits in, or from the vault root.
    fn resolve_path_form(&self, from: &str, target: &str) -> Option<Resolution> {
        let dir = from.rsplit_once('/').map_or("", |(d, _)| d);
        let joined = if target.starts_with('/') {
            target.trim_start_matches('/').to_string()
        } else if dir.is_empty() {
            target.to_string()
        } else {
            format!("{dir}/{target}")
        };
        // A candidate that climbs out of the vault is skipped; the other may still fit.
        for candidate in [normalize_dots(&joined), normalize_dots(target)]
            .into_iter()
            .flatten()
        {
            if let Some(hits) = self.by_full.get(&fold(strip_md(&candidate)))
                && let [only] = hits.as_slice()
            {
                return Some(Resolution::Unique(self.notes[*only].path.clone()));
            }
            let folded = fold(&candidate);
            if let Some(&idx) = self.by_att_full.get(&folded) {
                return Some(Resolution::Attachment(self.attachments[idx].1.clone()));
            }
        }
        None
    }

    /// A wikilink is a name: a bare file name, or the end of a path, or an alias.
    fn resolve_name(&self, target: &str) -> Resolution {
        let key = fold(strip_md(target));
        let last = key.rsplit('/').next().unwrap_or(&key);
        let qualified = key.contains('/');

        let mut hits: Vec<usize> = self
            .by_stem
            .get(last)
            .map(|v| {
                v.iter()
                    .copied()
                    .filter(|&i| {
                        !qualified
                            || self.notes[i].full == key
                            || self.notes[i].full.ends_with(&format!("/{key}"))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if qualified && hits.len() > 1 {
            // An exact path beats the notes that merely end the same way.
            let exact: Vec<usize> = hits
                .iter()
                .copied()
                .filter(|&i| self.notes[i].full == key)
                .collect();
            if !exact.is_empty() {
                hits = exact;
            }
        }
        if hits.is_empty() && !qualified {
            hits = self.by_alias.get(&key).cloned().unwrap_or_default();
        }
        match hits.as_slice() {
            [] => self.resolve_attachment(&fold(target)),
            [only] => Resolution::Unique(self.notes[*only].path.clone()),
            many => {
                Resolution::Ambiguous(many.iter().map(|&i| self.notes[i].path.clone()).collect())
            }
        }
    }

    fn resolve_attachment(&self, key: &str) -> Resolution {
        let name = key.rsplit('/').next().unwrap_or(key);
        let hits: Vec<&(String, String)> = self
            .by_att_name
            .get(name)
            .into_iter()
            .flatten()
            .map(|&i| &self.attachments[i])
            .filter(|(f, _)| !key.contains('/') || f == key || f.ends_with(&format!("/{key}")))
            .collect();
        // Several files may share a name. Any of them is a file that exists, so the link is
        // not broken, and nothing here needs to say which one.
        match hits.first() {
            Some((_, path)) => Resolution::Attachment(path.clone()),
            None => Resolution::Dangling,
        }
    }
}

/// Among notes a link could mean, the one in the same folder as `from`, if exactly one is.
/// For proposing a fix; [`NoteSet::resolve`] never does this on its own.
pub fn nearest<'a>(from: &str, candidates: &'a [String]) -> Option<&'a str> {
    let dir = from.rsplit_once('/').map_or("", |(d, _)| d);
    let mut same = candidates
        .iter()
        .filter(|c| c.rsplit_once('/').map_or("", |(d, _)| d) == dir);
    match (same.next(), same.next()) {
        (Some(only), None) => Some(only.as_str()),
        _ => None,
    }
}

/// The notes and files `content` links to, as the writer named them: each target once, in
/// order, without `.md`, heading or alias. Outside addresses and links to the same note are
/// left out.
pub fn target_names(content: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for link in extract(content) {
        if link.target.is_empty() || (link.syntax == Syntax::Markdown && is_external(&link.target))
        {
            continue;
        }
        let name = strip_md(&link.target).to_string();
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// A name as links compare it: Unicode-composed and lowercase.
pub fn fold(s: &str) -> String {
    s.trim().nfc().collect::<String>().to_lowercase()
}

fn strip_md(s: &str) -> &str {
    let t = s.trim();
    if t.len() > 3 && t[t.len() - 3..].eq_ignore_ascii_case(".md") {
        &t[..t.len() - 3]
    } else {
        t
    }
}

/// `a/b/../c` as `a/c`. `None` when the path climbs out of the vault.
fn normalize_dots(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

/// A target with a scheme (`https:`, `mailto:`) or a `//` start leaves the vault. A one-letter
/// scheme is a Windows drive, which is not a web address.
pub fn is_external(target: &str) -> bool {
    if target.starts_with("//") {
        return true;
    }
    match target.split_once(':') {
        Some((scheme, _)) => {
            scheme.len() >= 2
                && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        }
        None => false,
    }
}

/// The `aliases:` (or `alias:`) names in a note's frontmatter. A single string counts as one
/// alias. Unparseable frontmatter has none.
pub fn aliases_of(content: &str) -> Vec<String> {
    let (Some(fm), _) = hq_core::frontmatter_utils::split_frontmatter(content) else {
        return Vec::new();
    };
    let Ok(serde_yaml::Value::Mapping(map)) = serde_yaml::from_str::<serde_yaml::Value>(fm) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for key in ["aliases", "alias"] {
        match map.get(key) {
            Some(serde_yaml::Value::Sequence(items)) => {
                out.extend(items.iter().filter_map(|v| v.as_str()).map(str::to_string));
            }
            Some(serde_yaml::Value::String(s)) => out.push(s.clone()),
            _ => {}
        }
    }
    out.retain(|a| !a.trim().is_empty());
    out
}

// ─── Editing ─────────────────────────────────────────────────────────────────

/// A change to the text: replace `range` with `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub text: String,
}

/// The edit that points `link` at `new_target`, keeping its heading, block, alias and label.
/// For a markdown link not written in `<...>`, spaces and parentheses are escaped.
pub fn retarget(link: &LinkRef, new_target: &str) -> Edit {
    let text = if link.syntax == Syntax::Markdown && !link.angle {
        new_target
            .replace(' ', "%20")
            .replace('(', "%28")
            .replace(')', "%29")
    } else {
        new_target.to_string()
    };
    Edit {
        range: link.target_span.clone(),
        text,
    }
}

/// `content` with every edit applied. `None` if two edits overlap, one runs past the text or
/// lands inside a character: a plan that is wrong is refused whole, never half applied.
pub fn apply_edits(content: &str, edits: &[Edit]) -> Option<String> {
    let mut sorted: Vec<&Edit> = edits.iter().collect();
    sorted.sort_by_key(|e| (e.range.start, e.range.end));
    let mut out = String::with_capacity(content.len());
    let mut at = 0;
    for e in sorted {
        if e.range.start < at
            || e.range.end > content.len()
            || e.range.start > e.range.end
            || !content.is_char_boundary(e.range.start)
            || !content.is_char_boundary(e.range.end)
        {
            return None;
        }
        out.push_str(&content[at..e.range.start]);
        out.push_str(&e.text);
        at = e.range.end;
    }
    out.push_str(&content[at..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets(content: &str) -> Vec<String> {
        extract(content).into_iter().map(|l| l.target).collect()
    }

    fn set(paths: &[&str]) -> NoteSet {
        NoteSet::new(
            paths.iter().map(|p| NoteInput {
                path: p.to_string(),
                aliases: vec![],
            }),
            [],
        )
    }

    fn resolve_one(set: &NoteSet, from: &str, text: &str) -> Resolution {
        let links = extract(text);
        assert_eq!(links.len(), 1, "expected one link in {text:?}");
        set.resolve(from, &links[0])
    }

    #[test]
    fn a_wikilink_splits_into_target_heading_block_and_alias() {
        let l = &extract("see [[Folder/Note#Goals|the goals]] now")[0];
        assert_eq!(l.target, "Folder/Note");
        assert_eq!(l.heading.as_deref(), Some("Goals"));
        assert_eq!(l.alias.as_deref(), Some("the goals"));
        assert_eq!(l.syntax, Syntax::Wiki);
        assert!(!l.embed);

        let b = &extract("[[Note#^abc123]]")[0];
        assert_eq!(b.block.as_deref(), Some("abc123"));
        assert_eq!(b.heading, None);
    }

    #[test]
    fn spans_cover_the_text_that_was_read() {
        let text = "a ![[Pic.png]] b [[ Spaced Name |x]] c";
        let links = extract(text);
        assert_eq!(&text[links[0].span.clone()], "![[Pic.png]]");
        assert!(links[0].embed);
        assert_eq!(&text[links[0].target_span.clone()], "Pic.png");
        assert_eq!(&text[links[1].span.clone()], "[[ Spaced Name |x]]");
        assert_eq!(&text[links[1].target_span.clone()], "Spaced Name");
    }

    #[test]
    fn same_note_links_have_no_target() {
        let l = &extract("[[#Goals]] and [x](#top)")[..];
        assert_eq!(l[0].target, "");
        assert_eq!(l[0].heading.as_deref(), Some("Goals"));
        assert_eq!(l[1].target, "");
        let s = set(&["a.md"]);
        assert_eq!(s.resolve("a.md", &l[0]), Resolution::SameNote);
        assert_eq!(s.resolve("a.md", &l[1]), Resolution::SameNote);
    }

    #[test]
    fn markdown_links_decode_escapes_and_drop_titles() {
        let l = &extract(r#"[a](My%20Note.md#Part "title") and [b](<Has Space.md>)"#);
        assert_eq!(l[0].target, "My Note.md");
        assert_eq!(l[0].heading.as_deref(), Some("Part"));
        assert_eq!(l[1].target, "Has Space.md");
        assert!(l[1].angle);
    }

    #[test]
    fn image_syntax_marks_an_embed() {
        let l = &extract("![alt](pics/a.png) [t](n.md)");
        assert!(l[0].embed);
        assert!(!l[1].embed);
        assert_eq!(l[0].span.start, 0);
    }

    #[test]
    fn links_in_fenced_and_inline_code_are_not_links() {
        let text = "real [[A]]\n```\n[[Fenced]]\n```\n~~~md\n[[Tilde]]\n~~~\ninline `[[Inline]]` and ``a ` [[Double]] ``\nafter [[B]]";
        assert_eq!(targets(text), ["A", "B"]);
    }

    #[test]
    fn an_unclosed_fence_hides_the_rest_and_an_unclosed_tick_hides_nothing() {
        assert_eq!(targets("[[A]]\n```\n[[Hidden]]"), ["A"]);
        assert_eq!(targets("a ` b [[Shown]]"), ["Shown"]);
    }

    #[test]
    fn a_tick_run_closes_only_with_a_run_of_the_same_length() {
        assert_eq!(targets("`` `[[X]]` `` then [[Y]]"), ["Y"]);
    }

    #[test]
    fn escaped_brackets_and_newlines_are_not_links() {
        assert_eq!(targets("\\[[A]] and [[B\n]] and [[C]]"), ["C"]);
        assert!(extract("[[Open\nbroken]]").is_empty());
        assert!(extract("[[]]").is_empty());
    }

    #[test]
    fn frontmatter_links_are_found_and_flagged() {
        let text = "---\nrelated: \"[[Other]]\"\n---\n\nbody [[Body]]\n";
        let l = extract(text);
        assert_eq!(l.len(), 2);
        assert!(l[0].in_frontmatter);
        assert_eq!(l[0].line, 2);
        assert!(!l[1].in_frontmatter);
        assert_eq!(l[1].line, 5);
    }

    #[test]
    fn a_bare_name_a_path_an_extension_and_a_heading_all_find_the_note() {
        let s = set(&["Notebooks/Projects/Alpha/Plan.md", "Notebooks/Other.md"]);
        let want = Resolution::Unique("Notebooks/Projects/Alpha/Plan.md".into());
        for text in [
            "[[Plan]]",
            "[[plan]]",
            "[[Plan.md]]",
            "[[Plan#Goals]]",
            "[[Plan|label]]",
            "[[Alpha/Plan]]",
            "[[Projects/Alpha/Plan]]",
            "[[Notebooks/Projects/Alpha/Plan.md]]",
            "![[Plan]]",
        ] {
            assert_eq!(resolve_one(&s, "Notebooks/Other.md", text), want, "{text}");
        }
    }

    #[test]
    fn a_similar_name_is_dangling_not_a_match() {
        let s = set(&["Notebooks/Plan.md"]);
        for text in ["[[Plans]]", "[[Master Plan]]", "[[Beta/Plan]]"] {
            assert_eq!(
                resolve_one(&s, "Notebooks/x.md", text),
                Resolution::Dangling,
                "{text}"
            );
        }
    }

    #[test]
    fn one_name_on_two_notes_is_ambiguous_until_a_path_picks_one() {
        let s = set(&["Notebooks/A/Plan.md", "Notebooks/B/Plan.md"]);
        match resolve_one(&s, "Notebooks/x.md", "[[Plan]]") {
            Resolution::Ambiguous(paths) => assert_eq!(paths.len(), 2),
            other => panic!("expected ambiguous, got {other:?}"),
        }
        assert_eq!(
            resolve_one(&s, "Notebooks/x.md", "[[A/Plan]]"),
            Resolution::Unique("Notebooks/A/Plan.md".into())
        );
        let cands = vec![
            "Notebooks/A/Plan.md".to_string(),
            "Notebooks/B/Plan.md".to_string(),
        ];
        assert_eq!(
            nearest("Notebooks/B/Other.md", &cands),
            Some("Notebooks/B/Plan.md")
        );
        assert_eq!(nearest("Notebooks/C/Other.md", &cands), None);
    }

    #[test]
    fn case_and_unicode_form_do_not_split_a_name() {
        let composed = "Caf\u{e9}";
        let decomposed = "Cafe\u{301}";
        let s = set(&[&format!("Notebooks/{composed}.md")]);
        for name in [composed, decomposed, "CAF\u{c9}"] {
            assert!(
                matches!(
                    resolve_one(&s, "Notebooks/x.md", &format!("[[{name}]]")),
                    Resolution::Unique(_)
                ),
                "{name:?}"
            );
        }
    }

    #[test]
    fn an_alias_answers_when_no_file_has_the_name() {
        let s = NoteSet::new(
            [NoteInput {
                path: "Notebooks/Person.md".into(),
                aliases: vec!["Jo".into()],
            }],
            [],
        );
        assert_eq!(
            resolve_one(&s, "Notebooks/x.md", "[[Jo]]"),
            Resolution::Unique("Notebooks/Person.md".into())
        );
    }

    #[test]
    fn a_file_name_beats_an_alias() {
        let s = NoteSet::new(
            [
                NoteInput {
                    path: "Notebooks/Jo.md".into(),
                    aliases: vec![],
                },
                NoteInput {
                    path: "Notebooks/Person.md".into(),
                    aliases: vec!["Jo".into()],
                },
            ],
            [],
        );
        assert_eq!(
            resolve_one(&s, "Notebooks/x.md", "[[Jo]]"),
            Resolution::Unique("Notebooks/Jo.md".into())
        );
    }

    #[test]
    fn attachments_resolve_by_name_and_are_not_dangling() {
        let s = NoteSet::new(
            [NoteInput {
                path: "Notebooks/A.md".into(),
                aliases: vec![],
            }],
            ["_attachments/pic.PNG".to_string()],
        );
        assert_eq!(
            resolve_one(&s, "Notebooks/A.md", "![[pic.png]]"),
            Resolution::Attachment("_attachments/pic.PNG".into())
        );
        assert_eq!(
            resolve_one(&s, "Notebooks/A.md", "![[gone.png]]"),
            Resolution::Dangling
        );
    }

    #[test]
    fn markdown_links_resolve_relative_to_the_note_then_from_the_root() {
        let s = set(&[
            "Notebooks/Projects/Plan.md",
            "Notebooks/Projects/Sub/Deep.md",
            "Notebooks/Top.md",
        ]);
        let from = "Notebooks/Projects/Sub/Deep.md";
        assert_eq!(
            resolve_one(&s, from, "[x](../Plan.md)"),
            Resolution::Unique("Notebooks/Projects/Plan.md".into())
        );
        assert_eq!(
            resolve_one(&s, from, "[x](Notebooks/Top.md)"),
            Resolution::Unique("Notebooks/Top.md".into())
        );
        assert_eq!(
            resolve_one(&s, from, "[x](Top.md)"),
            Resolution::Unique("Notebooks/Top.md".into())
        );
        assert_eq!(
            resolve_one(&s, from, "[x](../../../../escape.md)"),
            Resolution::Dangling
        );
    }

    #[test]
    fn outside_addresses_are_external_and_a_windows_drive_is_not() {
        let s = set(&["a.md"]);
        for text in [
            "[x](https://example.com/a)",
            "[x](mailto:a@example.com)",
            "[x](//cdn.example.com/a)",
        ] {
            assert_eq!(
                resolve_one(&s, "a.md", text),
                Resolution::External,
                "{text}"
            );
        }
        assert_eq!(
            resolve_one(&s, "a.md", "[x](C:/notes/a.md)"),
            Resolution::Dangling
        );
    }

    #[test]
    fn aliases_read_from_a_list_or_a_single_string() {
        assert_eq!(aliases_of("---\naliases: [A, B]\n---\nx"), ["A", "B"]);
        assert_eq!(aliases_of("---\nalias: Solo\n---\nx"), ["Solo"]);
        assert!(aliases_of("no frontmatter").is_empty());
        assert!(aliases_of("---\naliases: [unclosed\n---\nx").is_empty());
    }

    #[test]
    fn retarget_changes_only_the_target_and_keeps_the_rest() {
        let text = "x [[Old#Goals|label]] y ![[Old]] z [t](Old.md#Part) w [t](<Old.md>)";
        let edits: Vec<Edit> = extract(text)
            .iter()
            .map(|l| {
                retarget(
                    l,
                    if l.syntax == Syntax::Wiki {
                        "New Name"
                    } else {
                        "New Name.md"
                    },
                )
            })
            .collect();
        assert_eq!(
            apply_edits(text, &edits).unwrap(),
            "x [[New Name#Goals|label]] y ![[New Name]] z [t](New%20Name.md#Part) w [t](<New Name.md>)"
        );
    }

    #[test]
    fn a_wrong_plan_is_refused_whole() {
        let text = "[[A]] [[B]]";
        let overlap = [
            Edit {
                range: 2..3,
                text: "x".into(),
            },
            Edit {
                range: 2..4,
                text: "y".into(),
            },
        ];
        assert_eq!(apply_edits(text, &overlap), None);
        let past = [Edit {
            range: 5..99,
            text: "x".into(),
        }];
        assert_eq!(apply_edits(text, &past), None);
        assert_eq!(
            apply_edits(
                "caf\u{e9}",
                &[Edit {
                    range: 4..5,
                    text: "x".into()
                }]
            ),
            None
        );
    }

    #[test]
    fn target_names_lists_each_destination_once_without_decoration() {
        let text = "[[A#h]] [[A|again]] [[Folder/B.md]] [x](c/d.md#s) [w](https://e.com) [[#same]] ![[pic.png]]";
        assert_eq!(target_names(text), ["A", "Folder/B", "c/d", "pic.png"]);
    }

    #[test]
    fn applying_nothing_returns_the_text_unchanged() {
        assert_eq!(apply_edits("same", &[]).as_deref(), Some("same"));
    }
}
