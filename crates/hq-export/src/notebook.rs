//! A Jupyter notebook: prose becomes Markdown cells, Python blocks code cells.

use serde_json::{Value, json};

use crate::doc::{Block, Document};
use crate::markdown::blocks_to_markdown;

fn is_python(lang: Option<&str>) -> bool {
    matches!(
        lang.map(str::to_ascii_lowercase).as_deref(),
        Some("python" | "python3" | "py" | "ipython")
    )
}

/// nbformat stores a cell's source as lines that keep their newline, except
/// the last.
fn source_lines(text: &str) -> Vec<Value> {
    let lines: Vec<&str> = text.split('\n').collect();
    let last = lines.len().saturating_sub(1);
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            if i < last {
                json!(format!("{l}\n"))
            } else {
                json!(l)
            }
        })
        .filter(|v| v != "")
        .collect()
}

fn markdown_cell(text: &str) -> Value {
    json!({ "cell_type": "markdown", "metadata": {}, "source": source_lines(text.trim_end()) })
}

fn code_cell(text: &str) -> Value {
    json!({
        "cell_type": "code",
        "execution_count": null,
        "metadata": {},
        "outputs": [],
        "source": source_lines(text),
    })
}

/// nbformat 4.4, the last minor version that does not require cell ids.
pub fn to_notebook(doc: &Document) -> String {
    let mut cells: Vec<Value> = Vec::new();
    let mut pending: Vec<Block> = vec![Block::Heading {
        level: 1,
        content: vec![crate::doc::Inline::Text(doc.title.clone())],
    }];
    let mut has_code = false;

    let flush = |pending: &mut Vec<Block>, cells: &mut Vec<Value>| {
        if !pending.is_empty() {
            cells.push(markdown_cell(&blocks_to_markdown(pending)));
            pending.clear();
        }
    };
    for block in &doc.blocks {
        match block {
            Block::Code { lang, text } if is_python(lang.as_deref()) => {
                flush(&mut pending, &mut cells);
                cells.push(code_cell(text));
                has_code = true;
            }
            other => pending.push(other.clone()),
        }
    }
    flush(&mut pending, &mut cells);

    let mut metadata = json!({ "title": doc.title });
    if has_code {
        metadata["kernelspec"] =
            json!({ "display_name": "Python 3", "language": "python", "name": "python3" });
        metadata["language_info"] = json!({ "name": "python" });
    }
    let nb = json!({ "nbformat": 4, "nbformat_minor": 4, "metadata": metadata, "cells": cells });
    let mut text = serde_json::to_string_pretty(&nb).unwrap_or_default();
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nb(md: &str) -> Value {
        serde_json::from_str(&to_notebook(&Document::from_markdown("My Note", md))).unwrap()
    }

    #[test]
    fn python_blocks_become_code_cells_between_markdown_cells() {
        let v = nb("Intro text.\n\n```python\nx = 1\nprint(x)\n```\n\nOutro.\n");
        let cells = v["cells"].as_array().unwrap();
        let kinds: Vec<&str> = cells
            .iter()
            .map(|c| c["cell_type"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["markdown", "code", "markdown"]);
        assert_eq!(cells[1]["source"], json!(["x = 1\n", "print(x)"]));
        assert!(
            cells[0]["source"][0]
                .as_str()
                .unwrap()
                .starts_with("# My Note")
        );
        assert_eq!(v["metadata"]["kernelspec"]["name"], "python3");
    }

    #[test]
    fn other_languages_stay_inside_markdown() {
        let v = nb("```rust\nfn main() {}\n```\n");
        let cells = v["cells"].as_array().unwrap();
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0]["cell_type"], "markdown");
        assert!(cells[0]["source"].to_string().contains("```rust"));
        assert!(v["metadata"].get("kernelspec").is_none());
    }

    #[test]
    fn notebook_has_the_required_top_level_fields() {
        let v = nb("text\n");
        assert_eq!(v["nbformat"], 4);
        assert_eq!(v["nbformat_minor"], 4);
        assert!(v["cells"].is_array() && v["metadata"].is_object());
    }

    #[test]
    fn code_cells_carry_empty_outputs() {
        let v = nb("```py\n1+1\n```\n");
        let code = &v["cells"][1];
        assert_eq!(code["outputs"], json!([]));
        assert!(code["execution_count"].is_null());
    }
}
