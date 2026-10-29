//! A blocked agent's dialog as data: what it asks, the command or path it is
//! about, the options it offers and how to pick one.
//!
//! A rule file describes a dialog's shape in a `[rules.dialog]` table on the
//! blocked rule that recognises it. The rule decides the state from the screen;
//! the table says where the dialog starts and ends and which lines are its
//! question, its command and its options. The dialog is read from the
//! scrollback as well as the screen, so one taller than the screen (a long
//! heredoc) arrives whole, up to `MAX_DIALOG_BYTES`.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

/// Longest dialog text kept, in bytes. Anything longer is cut and marked truncated.
pub const MAX_DIALOG_BYTES: usize = 32 * 1024;
/// Lines of output searched for the top of a dialog, counting up from the bottom.
pub const MAX_DIALOG_LINES: usize = 2_000;

/// Characters an agent draws its boxes' sides with; stripped from both ends of a line.
const BORDERS: &[char] = &['│', '┃', '║'];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DialogKind {
    /// Run a shell command.
    Command,
    /// Create, change or delete a file.
    Edit,
    /// Reach a path outside what the session may touch.
    Path,
    /// The agent asked the person a question.
    Question,
    /// Trust a folder, its hooks or its config.
    Trust,
    Other,
}

impl DialogKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DialogKind::Command => "command",
            DialogKind::Edit => "edit",
            DialogKind::Path => "path",
            DialogKind::Question => "question",
            DialogKind::Trust => "trust",
            DialogKind::Other => "other",
        }
    }
}

/// How an option is picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerMethod {
    /// Move the cursor to the option with the arrow keys, check it is there, then press Enter.
    #[default]
    Arrows,
    /// Press the option's number.
    Digit,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKindWhen {
    kind: DialogKind,
    line: String,
}

/// The `[rules.dialog]` table as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawDialog {
    kind: DialogKind,
    /// A more specific kind when one of the dialog's lines matches.
    #[serde(default)]
    kind_when: Vec<RawKindWhen>,
    /// The dialog's first line, found by searching up from the footer.
    top: String,
    /// The dialog's key hints, found by searching up from the bottom. It is not part of the dialog.
    footer: String,
    question: Option<String>,
    /// `[from, to]`: the command is the lines strictly between the first line matching `from`
    /// and the next one matching `to`.
    #[serde(default)]
    command_block: Vec<String>,
    /// A line whose first capture is the command (or path), when there is no block.
    command_line: Option<String>,
    /// An option line: a `label` capture, and a `num` capture when options are numbered.
    option: String,
    /// The option line the cursor is on.
    selected: Option<String>,
    /// Options that widen what the agent may do from now on ("don't ask again").
    remember: Option<String>,
    /// Options that open a text field instead of answering.
    typed: Option<String>,
    #[serde(default)]
    answer: AnswerMethod,
    /// Keys that dismiss the dialog without picking an option.
    #[serde(default = "default_decline")]
    decline: Vec<String>,
}

fn default_decline() -> Vec<String> {
    vec!["esc".to_string()]
}

/// A compiled `[rules.dialog]` table.
#[derive(Debug)]
pub struct DialogSpec {
    kind: DialogKind,
    kind_when: Vec<(DialogKind, Regex)>,
    top: Regex,
    footer: Regex,
    question: Option<Regex>,
    command_block: Option<(Regex, Regex)>,
    command_line: Option<Regex>,
    option: Regex,
    selected: Option<Regex>,
    remember: Option<Regex>,
    typed: Option<Regex>,
    pub answer: AnswerMethod,
    pub decline: Vec<String>,
}

fn pattern(p: &str) -> Result<Regex, String> {
    Regex::new(p).map_err(|e| format!("pattern {p:?}: {e}"))
}

impl DialogSpec {
    pub(super) fn compile(raw: &RawDialog) -> Result<Self, String> {
        let option = pattern(&raw.option)?;
        if !option.capture_names().any(|n| n == Some("label")) {
            return Err(format!(
                "dialog option pattern {:?} needs a `label` capture",
                raw.option
            ));
        }
        let command_block = match raw.command_block.as_slice() {
            [] => None,
            [from, to] => Some((pattern(from)?, pattern(to)?)),
            _ => return Err("dialog command_block takes exactly [from, to]".to_string()),
        };
        if raw.decline.is_empty() {
            return Err("dialog decline needs at least one key".to_string());
        }
        for key in &raw.decline {
            crate::keys::encode_key(key).map_err(|e| format!("dialog decline: {e}"))?;
        }
        let optional = |p: &Option<String>| p.as_deref().map(pattern).transpose();
        Ok(Self {
            kind: raw.kind,
            kind_when: raw
                .kind_when
                .iter()
                .map(|w| Ok((w.kind, pattern(&w.line)?)))
                .collect::<Result<_, String>>()?,
            top: pattern(&raw.top)?,
            footer: pattern(&raw.footer)?,
            question: optional(&raw.question)?,
            command_block,
            command_line: optional(&raw.command_line)?,
            option,
            selected: optional(&raw.selected)?,
            remember: optional(&raw.remember)?,
            typed: optional(&raw.typed)?,
            answer: raw.answer,
            decline: raw.decline.clone(),
        })
    }
}

/// One choice a dialog offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DialogOption {
    /// The number the agent shows, or the option's place in the list (from 1) when it shows none.
    pub id: u32,
    pub label: String,
    /// Picking it widens what the agent may do later without asking.
    pub remember: bool,
    /// Picking it opens a text field rather than answering.
    pub typed: bool,
    /// The cursor is on it, so Enter would pick it.
    pub selected: bool,
}

/// A dialog read off an agent's screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Dialog {
    pub kind: DialogKind,
    pub question: Option<String>,
    /// The command, or the file or path, the dialog is about.
    pub command: Option<String>,
    pub options: Vec<DialogOption>,
    /// Every line of the dialog with box borders removed.
    pub text: String,
    /// The dialog was longer than `MAX_DIALOG_BYTES` (or its top was out of reach) and was cut.
    pub truncated: bool,
    pub answer: AnswerMethod,
    pub decline: Vec<String>,
}

impl Dialog {
    /// Identifies the decision this dialog asks for: the same question about the same command
    /// with the same choices is the same dialog wherever the cursor is.
    pub fn identity(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.kind.hash(&mut h);
        self.question.hash(&mut h);
        self.command.hash(&mut h);
        for o in &self.options {
            o.id.hash(&mut h);
            o.label.hash(&mut h);
        }
        h.finish()
    }

    pub fn selected(&self) -> Option<&DialogOption> {
        self.options.iter().find(|o| o.selected)
    }

    pub fn option(&self, id: u32) -> Option<&DialogOption> {
        self.options.iter().find(|o| o.id == id)
    }
}

/// `line` with box sides removed from both ends, leading space kept and trailing space dropped,
/// so `│ │ cmd │ │` reads ` cmd`.
fn unbox(line: &str) -> &str {
    let mut s = line.trim_end();
    loop {
        let start = s.trim_start();
        let Some(inner) = start.strip_prefix(BORDERS) else {
            return s;
        };
        s = inner.trim_end();
        if let Some(inner) = s.strip_suffix(BORDERS) {
            s = inner.trim_end();
        }
    }
}

/// A line of nothing but box corners, edges and rules.
fn is_rule(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty() && t.chars().all(|c| "─━═╌┄╭╮╰╯┌┐└┘├┤┬┴┼".contains(c))
}

/// `lines` with their common indentation removed and blank lines at either end dropped.
fn dedent(lines: &[&str]) -> String {
    let first = lines.iter().position(|l| !l.trim().is_empty());
    let last = lines.iter().rposition(|l| !l.trim().is_empty());
    let (Some(first), Some(last)) = (first, last) else {
        return String::new();
    };
    let body = &lines[first..=last];
    let indent = body
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    body.iter()
        .map(|l| l.get(indent..).unwrap_or_else(|| l.trim_start()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `text` cut to at most `max` bytes at a character boundary; whether it was cut.
fn cap(text: &mut String, max: usize) -> bool {
    if text.len() <= max {
        return false;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    true
}

/// The dialog `spec` describes, read from `lines` (oldest first, the screen last). `None` when its
/// footer is not on the last lines or its top cannot be told apart from it.
pub fn extract(spec: &DialogSpec, lines: &[String]) -> Option<Dialog> {
    let footer = lines.iter().rposition(|l| spec.footer.is_match(l))?;
    // The footer sits at the bottom: one further up is a dialog from earlier.
    if lines[footer + 1..]
        .iter()
        .filter(|l| !unbox(l).is_empty())
        .count()
        > 2
    {
        return None;
    }
    let reach = footer.saturating_sub(MAX_DIALOG_LINES);
    let (top, mut truncated) = match lines[reach..footer]
        .iter()
        .rposition(|l| spec.top.is_match(l))
    {
        Some(i) => (reach + i, false),
        None => (reach, true),
    };
    let body: Vec<&str> = lines[top..footer].iter().map(|l| unbox(l)).collect();
    let kind = spec
        .kind_when
        .iter()
        .find(|(_, re)| body.iter().any(|l| re.is_match(l)))
        .map_or(spec.kind, |(k, _)| *k);
    let question_at = spec
        .question
        .as_ref()
        .and_then(|re| body.iter().position(|l| re.is_match(l)));
    let question = question_at.map(|i| body[i].trim().to_string());
    let mut command = command_of(spec, &body);
    // Numbered lines inside a command are not options: look only below the question.
    let options_from = question_at.map_or(0, |i| i + 1);
    let mut found: Vec<(Option<u32>, String, bool)> = Vec::new();
    for line in &body[options_from..] {
        let Some(caps) = spec.option.captures(line) else {
            continue;
        };
        let label = caps.name("label").map_or("", |m| m.as_str()).trim();
        if !label.is_empty() {
            let num = caps.name("num").and_then(|m| m.as_str().parse().ok());
            let selected = spec.selected.as_ref().is_some_and(|r| r.is_match(line));
            found.push((num, label.to_string(), selected));
        }
    }
    // In a numbered list, an unnumbered line is an option's description, not an option.
    if found.iter().any(|(num, ..)| num.is_some()) {
        found.retain(|(num, ..)| num.is_some());
    }
    let hit = |re: &Option<Regex>, text: &str| re.as_ref().is_some_and(|r| r.is_match(text));
    let options = found
        .into_iter()
        .zip(1..)
        .map(|((num, label, selected), position)| DialogOption {
            id: num.unwrap_or(position),
            remember: hit(&spec.remember, &label),
            typed: hit(&spec.typed, &label),
            selected,
            label,
        })
        .collect();
    // Box edges and rules carry nothing a reader needs.
    let readable: Vec<&str> = body.iter().copied().filter(|l| !is_rule(l)).collect();
    let mut text = dedent(&readable);
    truncated |= cap(&mut text, MAX_DIALOG_BYTES);
    if let Some(c) = command.as_mut() {
        truncated |= cap(c, MAX_DIALOG_BYTES);
    }
    Some(Dialog {
        kind,
        question,
        command,
        options,
        text,
        truncated,
        answer: spec.answer,
        decline: spec.decline.clone(),
    })
}

fn command_of(spec: &DialogSpec, body: &[&str]) -> Option<String> {
    if let Some((from, to)) = &spec.command_block
        && let Some(start) = body.iter().position(|l| from.is_match(l))
    {
        let end = body[start + 1..]
            .iter()
            .position(|l| to.is_match(l))
            .map_or(body.len(), |i| start + 1 + i);
        let block = dedent(&body[start + 1..end]);
        if !block.is_empty() {
            return Some(block);
        }
    }
    let re = spec.command_line.as_ref()?;
    body.iter().find_map(|l| {
        re.captures(l)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(toml_text: &str) -> DialogSpec {
        let raw: RawDialog = toml::from_str(toml_text).unwrap();
        DialogSpec::compile(&raw).unwrap()
    }

    const BOXED: &str = r#"
kind = "other"
kind_when = [{ kind = "command", line = '(?i)run this command' }]
top = '^╭'
footer = 'enter to select'
question = '(?i)^\s*do you want to\b'
command_block = ['^\s+╭', '^\s+╰']
option = '^\s*(?:❯\s*)?(?P<num>\d+)\.\s+(?P<label>\S.*)$'
selected = '^\s*❯'
remember = "(?i)don't ask again"
typed = '(?i)what to do differently'
"#;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    #[test]
    fn a_boxed_dialog_gives_its_command_question_and_options() {
        let screen = lines(
            "earlier output\n\
             ╭──────────────╮\n\
             │ Run it       │\n\
             │ ╭──────────╮ │\n\
             │ │ cat <<EOF│ │\n\
             │ │   1. hi  │ │\n\
             │ │ EOF      │ │\n\
             │ ╰──────────╯ │\n\
             │ Do you want to run this command? │\n\
             │ ❯ 1. Yes     │\n\
             │   2. Yes, and don't ask again for `cat` │\n\
             │   3. No, and tell it what to do differently │\n\
             │ enter to select │\n\
             ╰──────────────╯",
        );
        let d = extract(&spec(BOXED), &screen).unwrap();
        assert_eq!(d.kind, DialogKind::Command);
        assert_eq!(d.command.as_deref(), Some("cat <<EOF\n  1. hi\nEOF"));
        assert_eq!(
            d.question.as_deref(),
            Some("Do you want to run this command?")
        );
        let ids: Vec<u32> = d.options.iter().map(|o| o.id).collect();
        assert_eq!(
            ids,
            [1, 2, 3],
            "numbered lines in the command are not options"
        );
        assert!(d.options[0].selected && !d.options[1].selected);
        assert!(d.options[1].remember && !d.options[0].remember);
        assert!(d.options[2].typed);
        assert_eq!(d.selected().unwrap().label, "Yes");
        assert!(!d.truncated);
        assert!(!d.text.contains('│'), "{}", d.text);
    }

    #[test]
    fn a_dialog_whose_footer_is_not_at_the_bottom_is_not_read() {
        let mut screen = lines(
            "╭─╮\n│ Do you want to run this command? │\n│ ❯ 1. Yes │\n│ enter to select │\n╰─╯",
        );
        screen.extend(lines("later\nmore\nstill more"));
        assert!(extract(&spec(BOXED), &screen).is_none());
    }

    #[test]
    fn a_dialog_taller_than_the_reach_is_cut_and_says_so() {
        let mut screen = vec!["╭─╮".to_string(), "│ ╭─╮ │".to_string()];
        screen.extend((0..MAX_DIALOG_LINES + 10).map(|i| format!("│ │ line {i} │ │")));
        screen.extend(lines(
            "│ ╰─╯ │\n│ Do you want to run this command? │\n│ ❯ 1. Yes │\n│ enter to select │\n╰─╯",
        ));
        let d = extract(&spec(BOXED), &screen).unwrap();
        assert!(d.truncated);
        assert!(d.text.len() <= MAX_DIALOG_BYTES);
        assert_eq!(d.options.len(), 1);
    }

    #[test]
    fn unnumbered_options_count_from_one_and_the_identity_ignores_the_cursor() {
        let s = spec(
            r#"
kind = "trust"
top = '^─{5,}'
footer = '(?i)enter to confirm'
question = '\?\s*$'
option = '^ (?:❯ |  )\s*(?P<label>\S.*)$'
selected = '^ ❯'
"#,
        );
        let first =
            lines("─────\n Trust this folder?\n ❯ No, exit\n   Yes, I trust it\n Enter to confirm");
        let moved =
            lines("─────\n Trust this folder?\n   No, exit\n ❯ Yes, I trust it\n Enter to confirm");
        let a = extract(&s, &first).unwrap();
        let b = extract(&s, &moved).unwrap();
        assert_eq!(
            a.options
                .iter()
                .map(|o| (o.id, o.label.as_str()))
                .collect::<Vec<_>>(),
            [(1, "No, exit"), (2, "Yes, I trust it")]
        );
        assert_eq!(a.selected().unwrap().id, 1);
        assert_eq!(b.selected().unwrap().id, 2);
        assert_eq!(a.identity(), b.identity());
        assert_eq!(a.decline, ["esc"]);
    }

    #[test]
    fn bad_dialog_tables_are_refused() {
        let compile = |t: &str| DialogSpec::compile(&toml::from_str::<RawDialog>(t).unwrap());
        assert!(
            compile("kind = 'other'\ntop = 'a'\nfooter = 'b'\noption = '(?P<num>\\d)'")
                .unwrap_err()
                .contains("label")
        );
        assert!(
            compile(
                "kind = 'other'\ntop = 'a'\nfooter = 'b'\noption = '(?P<label>.)'\ncommand_block = ['x']"
            )
            .is_err()
        );
        assert!(
            compile(
                "kind = 'other'\ntop = 'a'\nfooter = 'b'\noption = '(?P<label>.)'\ndecline = ['--x']"
            )
            .is_err()
        );
    }

    #[test]
    fn unbox_strips_nested_sides_and_keeps_indentation() {
        assert_eq!(unbox("│ │ cmd   │ │"), " cmd");
        assert_eq!(unbox("│   2. Yes   │"), "   2. Yes");
        assert_eq!(unbox("plain  "), "plain");
        assert_eq!(unbox("│      │"), "");
    }
}
