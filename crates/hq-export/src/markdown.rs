//! Document model back to Markdown.
//!
//! Used for the `md` format and for the Markdown cells of a notebook. Text is
//! escaped so that what the parser read as plain text reads the same way again.

use crate::doc::{Align, Block, Document, Inline, ListItem, Table};

pub fn to_markdown(doc: &Document) -> String {
    let mut out = format!("# {}\n\n", flatten_line(&doc.title));
    out.push_str(&blocks_to_markdown(&doc.blocks));
    out
}

pub fn blocks_to_markdown(blocks: &[Block]) -> String {
    let rendered: Vec<String> = blocks.iter().map(block).collect();
    let mut out = rendered.join("\n\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn block(b: &Block) -> String {
    match b {
        Block::Heading { level, content } => {
            format!(
                "{} {}",
                "#".repeat((*level).clamp(1, 6) as usize),
                inlines(content)
            )
        }
        Block::Paragraph(content) => inlines(content),
        Block::Quote(body) => quote(&blocks_to_markdown(body)),
        Block::Callout { kind, title, body } => {
            let head = match title {
                Some(t) => format!("[!{kind}] {}", flatten_line(t)),
                None => format!("[!{kind}]"),
            };
            let mut text = head;
            let body = blocks_to_markdown(body);
            if !body.trim().is_empty() {
                text.push('\n');
                text.push_str(&body);
            }
            quote(&text)
        }
        Block::List {
            ordered,
            start,
            items,
        } => list(*ordered, *start, items),
        Block::Code { lang, text } => {
            let longest = text
                .lines()
                .map(|l| l.chars().take_while(|c| *c == '`').count())
                .max()
                .unwrap_or(0);
            let fence = "`".repeat((longest + 1).max(3));
            format!("{fence}{}\n{text}\n{fence}", lang.as_deref().unwrap_or(""))
        }
        Block::Table(t) => table(t),
        Block::Rule => "---".to_owned(),
    }
}

fn quote(text: &str) -> String {
    text.trim_end()
        .lines()
        .map(|l| {
            if l.is_empty() {
                ">".to_owned()
            } else {
                format!("> {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn list(ordered: bool, start: u64, items: &[ListItem]) -> String {
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let marker = if ordered {
            format!("{}. ", start + i as u64)
        } else {
            "- ".to_owned()
        };
        let task = match item.checked {
            Some(true) => "[x] ",
            Some(false) => "[ ] ",
            None => "",
        };
        let mut body = String::new();
        for (j, b) in item.blocks.iter().enumerate() {
            if j > 0 {
                body.push_str(if matches!(b, Block::List { .. }) {
                    "\n"
                } else {
                    "\n\n"
                });
            }
            body.push_str(&block(b));
        }
        let pad = " ".repeat(marker.len());
        let mut lines = body.lines();
        let mut text = format!("{marker}{task}{}", lines.next().unwrap_or(""));
        for line in lines {
            text.push('\n');
            if !line.is_empty() {
                text.push_str(&pad);
                text.push_str(line);
            }
        }
        out.push(text);
    }
    out.join("\n")
}

fn table(t: &Table) -> String {
    let n = t.columns();
    if n == 0 {
        return String::new();
    }
    let cells = |row: &[Vec<Inline>]| -> String {
        let cols: Vec<String> = (0..n)
            .map(|i| match row.get(i) {
                Some(c) => flatten_line(&inlines(c)).replace('|', "\\|"),
                None => String::new(),
            })
            .collect();
        format!("| {} |", cols.join(" | "))
    };
    let rule: Vec<&str> = (0..n)
        .map(|i| match t.aligns.get(i) {
            Some(Align::Left) => ":---",
            Some(Align::Center) => ":---:",
            Some(Align::Right) => "---:",
            None => "---",
        })
        .collect();
    let mut lines = vec![cells(&t.header), format!("| {} |", rule.join(" | "))];
    lines.extend(t.rows.iter().map(|r| cells(r)));
    lines.join("\n")
}

fn inlines(content: &[Inline]) -> String {
    let mut out = String::new();
    for i in content {
        match i {
            Inline::Text(t) => out.push_str(&escape(t)),
            Inline::Emph(c) => out.push_str(&format!("*{}*", inlines(c))),
            Inline::Strong(c) => out.push_str(&format!("**{}**", inlines(c))),
            Inline::Strike(c) => out.push_str(&format!("~~{}~~", inlines(c))),
            Inline::Code(c) => out.push_str(&code_span(c)),
            Inline::Link { url, content } => out.push_str(&format!(
                "[{}](<{}>)",
                inlines(content),
                url.replace('>', "%3E")
            )),
            Inline::Image { alt, src } => out.push_str(&format!(
                "![{}](<{}>)",
                escape(alt),
                src.replace('>', "%3E")
            )),
            Inline::SoftBreak => out.push('\n'),
            Inline::HardBreak => out.push_str("\\\n"),
        }
    }
    out
}

fn code_span(code: &str) -> String {
    let mut run = 0;
    let mut longest = 0;
    for c in code.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat(longest + 1);
    let pad = if code.starts_with('`') || code.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{code}{pad}{fence}")
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    for (i, c) in text.char_indices() {
        let special = matches!(c, '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '~')
            || (i == 0 && matches!(c, '#' | '-' | '+'));
        if special {
            out.push('\\');
        }
        out.push(c);
    }
    // "1. text" at the start of a line would become a list.
    let digits = out.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && matches!(out[digits..].chars().next(), Some('.' | ')')) {
        out.insert(digits, '\\');
    }
    out
}

fn flatten_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(md: &str) -> Vec<Block> {
        let first = Document::from_markdown("t", md);
        let again = Document::from_markdown("t", &blocks_to_markdown(&first.blocks));
        assert_eq!(
            first.blocks,
            again.blocks,
            "markdown was:\n{}",
            blocks_to_markdown(&first.blocks)
        );
        again.blocks
    }

    #[test]
    fn a_rich_note_survives_a_round_trip() {
        roundtrip(
            "## Heading\n\nSome **bold**, *it*, `code`, ~~gone~~ and [a link](https://example.com).\n\n\
             > [!tip] Remember\n> Bring the forms.\n>\n> - and a charger\n\n\
             | a | b |\n|:--|--:|\n| 1 | 2 |\n\n\
             1. one\n2. two\n   - nested\n\n- [x] done\n- [ ] todo\n\n\
             ```rust\nfn main() {}\n```\n\n---\n\n> plain quote\n",
        );
    }

    #[test]
    fn special_characters_in_text_do_not_become_markup() {
        roundtrip(
            "a \\* star and \\_ under and \\[brackets\\] and \\<angle\\> and 1\\. not a list\n",
        );
    }

    #[test]
    fn code_containing_backticks_and_fences_survives() {
        roundtrip("```\nplain\n````\ninner fence\n````\n```\n");
        roundtrip("use ``a ` b`` here\n");
    }

    #[test]
    fn document_markdown_starts_with_the_title() {
        let d = Document::from_markdown("My Title", "body\n");
        assert_eq!(to_markdown(&d), "# My Title\n\nbody\n");
    }
}
