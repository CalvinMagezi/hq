//! Generic XML to Markdown.
//!
//! A file whose root holds a list of like records becomes a table, with the
//! attributes and child elements as columns. Anything else becomes an outline
//! of element names, attributes and text. A file that is not well-formed is
//! kept verbatim in a code block instead of being refused.
//!
//! External entities are never loaded, and entity expansion and node counts
//! are capped by the parser.

use roxmltree::{Document, Node, ParsingOptions};

use super::ir::{Block, Inline, ListItem, Parsed, Run, Table};
use super::report::{LossKind, Losses};
use super::text::decode;
use crate::types::ConvertError;

const MAX_OUTLINE_ITEMS: usize = 5000;
const MAX_RAW_CHARS: usize = 64 * 1024;
const MAX_COLUMNS: usize = 60;

pub fn read(bytes: &[u8], max_nodes: u32, max_rows: usize) -> Result<Parsed, ConvertError> {
    let (text, _) = decode(bytes, declared_encoding(bytes).as_deref());
    let mut losses = Losses::default();
    let parsed = Document::parse_with_options(
        &text,
        ParsingOptions {
            allow_dtd: true,
            nodes_limit: max_nodes,
            ..ParsingOptions::default()
        },
    );
    let doc = match parsed {
        Ok(d) => d,
        Err(e) => {
            losses.note(
                LossKind::NotWellFormed,
                format!("not well-formed XML ({e}); kept as written"),
            );
            let mut raw: String = text.chars().take(MAX_RAW_CHARS).collect();
            if text.chars().count() > MAX_RAW_CHARS {
                raw.push_str("\n...");
                losses.note(
                    LossKind::Truncated,
                    format!("raw XML cut at {MAX_RAW_CHARS} characters"),
                );
            }
            return Ok(Parsed {
                blocks: vec![Block::Code {
                    lang: Some("xml".into()),
                    text: raw,
                }],
                title: None,
                losses,
            });
        }
    };
    let root = doc.root_element();
    if let Some(table) = records(root, max_rows, &mut losses) {
        return Ok(Parsed {
            blocks: vec![
                Block::Para(vec![Inline::Run(Run {
                    text: root.tag_name().name().to_string(),
                    bold: true,
                    ..Run::default()
                })]),
                Block::Table(table),
            ],
            title: None,
            losses,
        });
    }
    let mut items = Vec::new();
    let mut budget = MAX_OUTLINE_ITEMS;
    outline(root, 0, &mut items, &mut budget);
    if budget == 0 {
        losses.note(
            LossKind::Truncated,
            format!("outline cut at {MAX_OUTLINE_ITEMS} elements"),
        );
    }
    Ok(Parsed {
        blocks: vec![Block::List(items)],
        title: None,
        losses,
    })
}

fn declared_encoding(bytes: &[u8]) -> Option<String> {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(200)]).to_lowercase();
    let at = head.find("encoding")?;
    let rest = head[at + "encoding".len()..].trim_start_matches([' ', '=', '"', '\'']);
    let label: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    (!label.is_empty()).then_some(label)
}

fn element_children<'a, 'i>(n: Node<'a, 'i>) -> Vec<Node<'a, 'i>> {
    n.children().filter(Node::is_element).collect()
}

fn text_of(n: Node<'_, '_>) -> String {
    let t: String = n
        .descendants()
        .filter(|d| d.is_text())
        .filter_map(|d| d.text())
        .collect();
    super::ir::collapse_ws(&t).trim().to_string()
}

fn is_leaf(n: Node<'_, '_>) -> bool {
    element_children(n).is_empty()
}

/// A root that holds two or more records of one kind, each flat enough for a row.
fn records(root: Node<'_, '_>, max_rows: usize, losses: &mut Losses) -> Option<Table> {
    let kids = element_children(root);
    let name = kids.first()?.tag_name().name().to_string();
    if kids.len() < 2 || !kids.iter().all(|k| k.tag_name().name() == name) {
        return None;
    }
    if !kids
        .iter()
        .all(|k| element_children(*k).iter().all(|c| is_leaf(*c)))
    {
        return None;
    }
    let mut columns: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<(String, String)>> = Vec::new();
    for k in &kids {
        let mut cells: Vec<(String, String)> = Vec::new();
        for a in k.attributes() {
            cells.push((a.name().to_string(), a.value().to_string()));
        }
        for c in element_children(*k) {
            let n = c.tag_name().name().to_string();
            let v = text_of(c);
            match cells.iter_mut().find(|(cn, _)| *cn == n) {
                Some((_, existing)) => {
                    existing.push_str("; ");
                    existing.push_str(&v);
                }
                None => cells.push((n, v)),
            }
        }
        // A record that is only text has one unnamed column.
        let own = text_of(*k);
        if cells.is_empty() && !own.is_empty() {
            cells.push((name.clone(), own));
        }
        for (n, _) in &cells {
            if !columns.contains(n) {
                columns.push(n.clone());
            }
        }
        rows.push(cells);
    }
    if columns.is_empty() || columns.len() > MAX_COLUMNS {
        return None;
    }
    let total = rows.len();
    if total > max_rows {
        losses.note(
            LossKind::Truncated,
            format!("showing the first {max_rows} of {total} records"),
        );
        rows.truncate(max_rows);
    }
    let mut table = Table::default();
    table.rows.push(
        columns
            .iter()
            .map(|c| vec![Inline::text(c.clone())])
            .collect(),
    );
    for r in rows {
        table.rows.push(
            columns
                .iter()
                .map(|c| {
                    let v = r
                        .iter()
                        .find(|(n, _)| n == c)
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default();
                    vec![Inline::text(v)]
                })
                .collect(),
        );
    }
    Some(table)
}

fn outline(n: Node<'_, '_>, depth: usize, items: &mut Vec<ListItem>, budget: &mut usize) {
    if *budget == 0 {
        return;
    }
    *budget -= 1;
    let kids = element_children(n);
    // Elements whose children are all leaves read as one line, with mixed content joined.
    let inline_text = kids.iter().all(|k| is_leaf(*k));
    let mut content = vec![Inline::Run(Run {
        text: n.tag_name().name().to_string(),
        bold: true,
        ..Run::default()
    })];
    let attrs: Vec<String> = n
        .attributes()
        .map(|a| format!("{}={}", a.name(), a.value()))
        .collect();
    if !attrs.is_empty() {
        content.push(Inline::text(format!(" ({})", attrs.join(", "))));
    }
    let text = if inline_text {
        text_of(n)
    } else {
        String::new()
    };
    if !text.is_empty() {
        content.push(Inline::text(format!(": {text}")));
    }
    items.push(ListItem {
        level: depth.min(8),
        ordered: false,
        content,
    });
    if !inline_text {
        // Text mixed in with child elements, such as a heading followed by paragraphs.
        let own: String = n
            .children()
            .filter(|c| c.is_text())
            .filter_map(|c| c.text())
            .collect();
        let own = super::ir::collapse_ws(&own).trim().to_string();
        if !own.is_empty() {
            items.push(ListItem {
                level: (depth + 1).min(8),
                ordered: false,
                content: vec![Inline::text(own)],
            });
        }
        for k in kids {
            outline(k, depth + 1, items, budget);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(xml: &str) -> String {
        super::super::ir::render(read(xml.as_bytes(), 1_000_000, 100).unwrap().blocks)
    }

    #[test]
    fn like_records_become_a_table_with_attribute_and_child_columns() {
        assert_eq!(
            md(
                "<visits><visit site=\"Mukono\" farmers=\"42\"><note>On time</note></visit><visit site=\"Wakiso\" farmers=\"17\"><note>Rain</note></visit></visits>"
            ),
            "**visits**\n\n| site | farmers | note |\n| --- | --- | --- |\n| Mukono | 42 | On time |\n| Wakiso | 17 | Rain |\n"
        );
    }

    #[test]
    fn other_documents_become_an_outline() {
        assert_eq!(
            md(
                "<report><title>Field report</title><section><heading>Findings</heading><para>Loss fell to <b>12%</b>.</para></section></report>"
            ),
            "- **report**\n    - **title**: Field report\n    - **section**\n        - **heading**: Findings\n        - **para**: Loss fell to 12%.\n"
        );
    }

    #[test]
    fn malformed_xml_is_kept_verbatim_not_refused() {
        let p = read(b"<a><b>unclosed</a>", 1000, 10).unwrap();
        let warnings = p.losses.into_warnings();
        assert_eq!(warnings[0].kind, LossKind::NotWellFormed);
    }

    #[test]
    fn external_entities_are_never_loaded() {
        // A classic XXE payload: the file must not be read.
        let xml = "<?xml version=\"1.0\"?><!DOCTYPE d [<!ENTITY x SYSTEM \"file:///etc/hostname\">]><d><v>&x;</v></d>";
        let out = md(xml);
        let host = std::fs::read_to_string("/etc/hostname").unwrap_or_default();
        assert!(
            host.trim().is_empty() || !out.contains(host.trim()),
            "{out}"
        );
    }

    #[test]
    fn entity_expansion_bombs_fail_fast_instead_of_exhausting_memory() {
        let mut dtd = String::from("<!ENTITY a0 \"lol\">");
        for i in 1..10 {
            let prev = format!("&a{};", i - 1).repeat(10);
            dtd.push_str(&format!("<!ENTITY a{i} \"{prev}\">"));
        }
        let xml = format!("<?xml version=\"1.0\"?><!DOCTYPE d [{dtd}]><d>&a9;</d>");
        let started = std::time::Instant::now();
        let r = read(xml.as_bytes(), 1_000_000, 10);
        assert!(started.elapsed().as_secs() < 5);
        // Either refused (kept raw) or expanded within limits; never hangs or aborts.
        assert!(r.is_ok());
    }

    #[test]
    fn declared_encodings_are_honoured() {
        let xml = b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><d><v>caf\xe9</v></d>";
        assert!(
            super::super::ir::render(read(xml, 1000, 10).unwrap().blocks).contains("caf\u{e9}")
        );
    }
}
