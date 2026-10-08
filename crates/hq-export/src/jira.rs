//! Jira / Confluence wiki markup.

use crate::doc::{Block, Document, Inline, ListItem, Table};
use crate::util::{capitalise, safe_url};

pub fn to_jira(doc: &Document) -> String {
    let mut out = format!("h1. {}\n\n", line(&doc.title));
    out.push_str(&blocks(&doc.blocks, ""));
    out
}

fn blocks(bs: &[Block], marks: &str) -> String {
    let parts: Vec<String> = bs.iter().map(|b| block(b, marks)).collect();
    let mut out = parts.join("\n\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn block(b: &Block, marks: &str) -> String {
    match b {
        Block::Heading { level, content } => {
            format!("h{}. {}", (*level).clamp(1, 6), inlines(content))
        }
        Block::Paragraph(c) => inlines(c),
        Block::Quote(body) => format!("{{quote}}\n{}{{quote}}", blocks(body, marks)),
        Block::Callout { kind, title, body } => {
            let label = title.clone().unwrap_or_else(|| capitalise(kind));
            format!(
                "{{panel:title={}}}\n{}{{panel}}",
                label.replace([':', '|', '}', '{', '\n'], " "),
                blocks(body, marks)
            )
        }
        Block::List { ordered, items, .. } => list(*ordered, items, marks),
        Block::Code { lang, text } => {
            let lang = lang
                .as_deref()
                .filter(|l| {
                    l.chars()
                        .all(|c| c.is_ascii_alphanumeric() || "+-#_".contains(c))
                })
                .map(|l| format!(":{l}"))
                .unwrap_or_default();
            format!("{{code{lang}}}\n{text}\n{{code}}")
        }
        Block::Table(t) => table(t),
        Block::Rule => "----".to_owned(),
    }
}

fn list(ordered: bool, items: &[ListItem], marks: &str) -> String {
    let mark = if ordered { '#' } else { '*' };
    let here = format!("{marks}{mark}");
    let mut out = Vec::new();
    for item in items {
        let task = match item.checked {
            Some(true) => "(/) ",
            Some(false) => "(x) ",
            None => "",
        };
        let mut first = true;
        for b in &item.blocks {
            match b {
                Block::List { ordered, items, .. } => out.push(list(*ordered, items, &here)),
                other if first => out.push(format!(
                    "{here} {task}{}",
                    block(other, &here).replace('\n', " ")
                )),
                other => out.push(format!("{here} {}", block(other, &here).replace('\n', " "))),
            }
            first = false;
        }
    }
    out.join("\n")
}

fn table(t: &Table) -> String {
    let n = t.columns();
    if n == 0 {
        return String::new();
    }
    let cell = |c: Option<&Vec<Inline>>| {
        let s = c.map(|c| inlines(c)).unwrap_or_default();
        let s = s
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace('|', "\\|");
        if s.is_empty() { " ".to_owned() } else { s }
    };
    let head: Vec<String> = (0..n).map(|i| cell(t.header.get(i))).collect();
    let mut lines = vec![format!("||{}||", head.join("||"))];
    for r in &t.rows {
        let cols: Vec<String> = (0..n).map(|i| cell(r.get(i))).collect();
        lines.push(format!("|{}|", cols.join("|")));
    }
    lines.join("\n")
}

fn inlines(content: &[Inline]) -> String {
    let mut out = String::new();
    for i in content {
        match i {
            Inline::Text(t) => out.push_str(&escape(t)),
            Inline::Emph(c) => out.push_str(&format!("_{}_", inlines(c))),
            Inline::Strong(c) => out.push_str(&format!("*{}*", inlines(c))),
            Inline::Strike(c) => out.push_str(&format!("-{}-", inlines(c))),
            Inline::Code(c) => out.push_str(&format!("{{{{{}}}}}", c.replace(['{', '}'], ""))),
            Inline::Link { url, content } => {
                let text = inlines(content);
                if safe_url(url) {
                    out.push_str(&format!("[{}|{}]", text, url.replace(['|', ']'], "")));
                } else {
                    out.push_str(&text);
                }
            }
            // Jira cannot reach a file on this machine, so only the caption goes.
            Inline::Image { alt, .. } => {
                out.push_str(&escape(if alt.is_empty() { "image" } else { alt }))
            }
            Inline::SoftBreak => out.push(' '),
            Inline::HardBreak => out.push_str("\\\\ "),
        }
    }
    out
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '{' | '}' | '[' | ']' | '*' | '_' | '|' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jira(md: &str) -> String {
        to_jira(&Document::from_markdown("Title", md))
    }

    #[test]
    fn headings_emphasis_links_and_code() {
        let j =
            jira("## Sec\n\nSome **bold**, *it*, ~~gone~~, `code` and [a](https://example.com).\n");
        assert!(j.starts_with("h1. Title\n\nh2. Sec\n"));
        assert!(j.contains("*bold*") && j.contains("_it_") && j.contains("-gone-"));
        assert!(j.contains("{{code}}") && j.contains("[a|https://example.com]"));
    }

    #[test]
    fn nested_lists_use_repeated_markers() {
        let j = jira("- a\n  - b\n\n1. x\n2. y\n");
        assert!(j.contains("* a\n** b"), "{j}");
        assert!(j.contains("# x\n# y"), "{j}");
    }

    #[test]
    fn tables_use_header_pipes() {
        let j = jira("| a | b |\n|---|---|\n| 1 | 2 |\n");
        assert_eq!(j.lines().nth(2).unwrap(), "||a||b||");
        assert_eq!(j.lines().nth(3).unwrap(), "|1|2|");
    }

    #[test]
    fn code_blocks_callouts_and_quotes() {
        let j = jira("```rust\nfn x() {}\n```\n\n> [!tip] Hi\n> body\n\n> quoted\n");
        assert!(j.contains("{code:rust}\nfn x() {}\n{code}"), "{j}");
        assert!(j.contains("{panel:title=Hi}\nbody\n{panel}"), "{j}");
        assert!(j.contains("{quote}\nquoted\n{quote}"), "{j}");
    }

    #[test]
    fn markup_characters_in_text_are_escaped() {
        let j = jira("a {macro} and [b] and \\*x\\* and a|pipe\n");
        assert!(
            j.contains("a \\{macro\\} and \\[b\\] and \\*x\\* and a\\|pipe"),
            "{j}"
        );
    }

    #[test]
    fn unsafe_links_lose_their_target() {
        let j = jira("[x](javascript:alert(1))\n");
        assert!(!j.contains("javascript"), "{j}");
    }
}
