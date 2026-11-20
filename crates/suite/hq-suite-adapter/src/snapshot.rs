//! A note as it should read in a shared snapshot (a PDF or a PNG): the suite's typed blocks
//! written out in words and tables, because the document exporter knows nothing of hqmd and would
//! draw a `tile` or a `plot` as a code block.
//!
//! - a `tile` becomes a labelled number (`**Stock value:** UGX 213,000`);
//! - a `plot` or a `chart` becomes a table of the values it would draw (there is no drawn chart
//!   in the exporter yet, `docs/techdebt/dashboard-snapshot-charts.md`);
//! - a `sheet` becomes a table of its calculated values;
//! - a `comment` is left out, so a shared file carries no review notes.
//!
//! Everything else in the note is returned as written, so the exporter's own preparation
//! (wiki links, callouts, pictures) still applies to the result.

use std::path::Path;

use anyhow::{Context, Result};
use hq_suite_md::{BlockType, Document, NodeKind};
use hq_suite_store::SuitePaths;

/// Most blocks of one kind that are calculated for a snapshot, so a hostile note cannot make a
/// share do unbounded work.
const MAX_BLOCKS_PER_KIND: usize = 100;
/// Most rows and columns of a sheet written into a snapshot.
const MAX_SHEET_ROWS: u32 = 200;
const MAX_SHEET_COLS: u32 = 20;

/// The note's text for a snapshot, and one line saying what the snapshot kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    /// The whole note, frontmatter included, with its typed blocks written out.
    pub text: String,
    pub fidelity: String,
}

/// Escapes one table cell: no pipes, no line breaks.
fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

fn table(header: &[String], rows: &[Vec<String>]) -> String {
    let width = header.len().max(1);
    let mut out = format!(
        "| {} |\n|{}\n",
        header
            .iter()
            .map(|h| cell(h))
            .collect::<Vec<_>>()
            .join(" | "),
        "---|".repeat(width)
    );
    for r in rows {
        let mut cells: Vec<String> = r.iter().map(|c| cell(c)).collect();
        cells.resize(width, String::new());
        out.push_str(&format!("| {} |\n", cells.join(" | ")));
    }
    out.trim_end().to_string()
}

fn number(v: &serde_json::Value) -> String {
    match v.as_f64() {
        Some(n) if n.fract() == 0.0 && n.abs() < 1e15 => format!("{}", n as i64),
        Some(n) => format!("{n:.2}"),
        None => v.to_string(),
    }
}

/// A chart in the format of `docs/CHART_BLOCKS.md` as a bold title and a table of its values.
fn chart_table(chart: &serde_json::Value) -> Option<String> {
    let labels = chart.get("labels")?.as_array()?;
    let series = chart.get("series")?.as_array()?;
    let x = chart
        .get("xLabel")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Label");
    let mut header = vec![x.to_string()];
    header.extend(series.iter().map(|s| {
        s.get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("Value")
            .to_string()
    }));
    let rows: Vec<Vec<String>> = labels
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let mut row = vec![l.as_str().map_or_else(|| l.to_string(), str::to_string)];
            row.extend(series.iter().map(|s| {
                s.get("values")
                    .and_then(|v| v.get(i))
                    .map(number)
                    .unwrap_or_default()
            }));
            row
        })
        .collect();
    let mut out = String::new();
    if let Some(t) = chart
        .get("title")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
    {
        out.push_str(&format!("**{}**\n\n", t.replace('*', "")));
    }
    out.push_str(&table(&header, &rows));
    Some(out)
}

/// A note that could not be drawn says why, in one sentence, instead of failing the share.
fn unavailable(what: &str, e: &anyhow::Error) -> String {
    format!("*{what} could not be shown: {e}*")
}

/// The sheet at `index` as a table of calculated values, in windows of the sheet view.
fn sheet_table(vault: &Path, note: &str, index: usize) -> Result<String> {
    let first = crate::sheets::view(vault, note, index, 0, 0)?;
    let rows_wanted = first.row_count.min(MAX_SHEET_ROWS);
    let cols_wanted = first.col_count.min(MAX_SHEET_COLS);
    let mut grid: Vec<Vec<String>> = vec![Vec::new(); rows_wanted as usize];
    let mut col0 = 0;
    while col0 < cols_wanted {
        let mut row0 = 0;
        while row0 < rows_wanted {
            let w = crate::sheets::view(vault, note, index, row0, col0)?;
            for (r, row) in w.rows.iter().enumerate() {
                let at = row0 as usize + r;
                if at < grid.len() {
                    grid[at].extend(row.iter().map(|c| c.text.clone()));
                }
            }
            row0 += crate::sheets::MAX_ROWS;
        }
        col0 += crate::sheets::MAX_COLS;
    }
    let mut rows = grid.into_iter();
    let header = rows.next().unwrap_or_default();
    let body: Vec<Vec<String>> = rows.collect();
    let mut out = format!(
        "**{}**\n\n{}",
        first.name.replace('*', ""),
        table(&header, &body)
    );
    if first.row_count > MAX_SHEET_ROWS || first.col_count > MAX_SHEET_COLS {
        out.push_str(&format!(
            "\n\n*Showing the first {rows_wanted} rows and {cols_wanted} columns of {} by {}.*",
            first.row_count, first.col_count
        ));
    }
    Ok(out)
}

/// Writes the note at `note` (vault-relative) as snapshot text. See the module comment.
pub fn prepare(vault: &Path, note: &str) -> Result<Prepared> {
    let file = SuitePaths::new(vault).resolve(note)?;
    let text = std::fs::read_to_string(&file).with_context(|| format!("{note} cannot be read"))?;
    let doc = Document::parse(&text);
    let (mut tiles, mut plots, mut sheets, mut charts, mut comments) = (0, 0, 0, 0, 0);
    let (mut missing, mut capped) = (0usize, false);
    let mut out = String::from(doc.frontmatter());
    let mut wrote = false;
    let mut first_lead: Option<&str> = None;
    for (idx, node) in doc.nodes().iter().enumerate() {
        first_lead.get_or_insert(node.lead());
        let replacement: Option<String> = match node.kind() {
            NodeKind::Typed(BlockType::Comment) => {
                comments += 1;
                continue;
            }
            NodeKind::Typed(BlockType::Tile) => {
                let i = tiles;
                tiles += 1;
                if i >= MAX_BLOCKS_PER_KIND {
                    capped = true;
                    continue;
                }
                Some(match crate::sheets::tile(vault, note, i) {
                    Ok(t) => format!("**{}:** {}", t.label.replace('*', ""), t.text),
                    Err(e) => {
                        missing += 1;
                        unavailable("A number", &e)
                    }
                })
            }
            NodeKind::Typed(BlockType::Plot) => {
                let i = plots;
                plots += 1;
                if i >= MAX_BLOCKS_PER_KIND {
                    capped = true;
                    continue;
                }
                Some(
                    match crate::sheets::plot(vault, note, i).and_then(|j| {
                        let v: serde_json::Value = serde_json::from_str(&j)?;
                        chart_table(&v).context("the chart has no data")
                    }) {
                        Ok(t) => t,
                        Err(e) => {
                            missing += 1;
                            unavailable("A chart", &e)
                        }
                    },
                )
            }
            NodeKind::Typed(BlockType::Chart) => {
                charts += 1;
                let body = doc
                    .typed(idx)
                    .and_then(Result::ok)
                    .map(|b| b.body)
                    .unwrap_or_default();
                Some(
                    match serde_json::from_str::<serde_json::Value>(&body)
                        .ok()
                        .and_then(|v| chart_table(&v))
                    {
                        Some(t) => t,
                        None => {
                            missing += 1;
                            "*A chart could not be shown: its data is not readable.*".to_string()
                        }
                    },
                )
            }
            NodeKind::Typed(BlockType::Sheet) => {
                let i = sheets;
                sheets += 1;
                if i >= MAX_BLOCKS_PER_KIND {
                    capped = true;
                    continue;
                }
                Some(match sheet_table(vault, note, i) {
                    Ok(t) => t,
                    Err(e) => {
                        missing += 1;
                        unavailable("A sheet", &e)
                    }
                })
            }
            _ => None,
        };
        out.push_str(if wrote {
            node.lead()
        } else {
            first_lead.unwrap_or("")
        });
        out.push_str(replacement.as_deref().unwrap_or(node.src()));
        wrote = true;
    }
    // The exporter ignores trailing blank space, so one newline is enough.
    out.push('\n');

    let mut kept = Vec::new();
    if tiles > 0 {
        kept.push("numbers are shown with their labels".to_string());
    }
    if plots + charts > 0 {
        kept.push("charts are shown as tables of their values, not drawn".to_string());
    }
    if sheets > 0 {
        kept.push("sheets are shown as tables of calculated values".to_string());
    }
    let mut fidelity = String::from("A fixed-layout copy of the note's text, tables and pictures.");
    if !kept.is_empty() {
        let joined = kept.join("; ");
        let mut chars = joined.chars();
        let head: String = chars
            .next()
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_default();
        fidelity.push_str(&format!(" {head}{}.", chars.as_str()));
    }
    if comments > 0 {
        fidelity.push_str(" Review comments are left out.");
    }
    if missing > 0 {
        fidelity.push_str(&format!(
            " {missing} item(s) could not be calculated and say so in the file."
        ));
    }
    if capped {
        fidelity.push_str(&format!(
            " Only the first {MAX_BLOCKS_PER_KIND} blocks of each kind are included."
        ));
    }
    Ok(Prepared {
        text: out,
        fidelity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "---\ntitle: Shop dashboard\nsuite_kind: dashboard\n---\n# Shop dashboard\n\n```sheet name=Stock types=\"text,int,ugx,ugx\"\n| item | qty | price | total |\n|---|---|---|---|\n| Sugar | 40 | 4200 | =B2*C2 |\n| Salt | 25 | 1800 | =B3*C3 |\n```\n\n```tile from=\"D2:D3\" fn=sum label=\"Stock value\" format=ugx\n```\n\n```plot type=bar from=\"A1:D3\" x=item y=total title=\"Value by item\"\n```\n\n```comment on=0123456789ab by=Sam\nPrivate review note.\n```\n\n```tile from=\"Z1:Z2\" fn=sum label=\"Broken\"\n```\n";

    fn vault() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("Notebooks")).unwrap();
        std::fs::write(d.path().join("Notebooks/shop.md"), NOTE).unwrap();
        d
    }

    #[test]
    fn tiles_become_labelled_numbers_plots_and_sheets_become_tables_and_comments_go() {
        let d = vault();
        let p = prepare(d.path(), "Notebooks/shop.md").unwrap();
        assert!(
            p.text.starts_with("---\ntitle: Shop dashboard"),
            "{}",
            p.text
        );
        assert!(
            p.text.contains("**Stock value:** UGX 213,000"),
            "{}",
            p.text
        );
        assert!(p.text.contains("**Value by item**"), "{}", p.text);
        assert!(p.text.contains("| item | total |"), "{}", p.text);
        assert!(p.text.contains("| Sugar | 168000 |"), "{}", p.text);
        assert!(
            p.text.contains("| Sugar | 40 | UGX 4,200 | UGX 168,000 |"),
            "{}",
            p.text
        );
        assert!(!p.text.contains("Private review note"));
        assert!(
            !p.text.contains("```"),
            "no typed block is left as code: {}",
            p.text
        );
        assert!(p.text.contains("A number could not be shown"), "{}", p.text);
        assert!(p.fidelity.contains("not drawn") && p.fidelity.contains("comments"));
        assert!(p.fidelity.contains("1 item(s) could not be calculated"));
    }

    #[test]
    fn a_note_with_no_typed_blocks_is_returned_as_written() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("Notebooks")).unwrap();
        let plain =
            "---\ntitle: Letter\n---\n\n# Letter\n\nDear Sam,\n\n```rust\nfn main() {}\n```\n";
        std::fs::write(d.path().join("Notebooks/l.md"), plain).unwrap();
        let p = prepare(d.path(), "Notebooks/l.md").unwrap();
        assert_eq!(p.text.trim_end(), plain.trim_end());
        assert!(!p.fidelity.contains("comments"));
    }

    #[test]
    fn a_chart_block_with_inline_data_becomes_a_table() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("Notebooks")).unwrap();
        let note = "# T\n\n```chart\n{\"type\":\"line\",\"labels\":[\"Mon\",\"Tue\"],\"series\":[{\"name\":\"Sales\",\"values\":[12,18.5]}]}\n```\n";
        std::fs::write(d.path().join("Notebooks/c.md"), note).unwrap();
        let p = prepare(d.path(), "Notebooks/c.md").unwrap();
        assert!(p.text.contains("| Label | Sales |"), "{}", p.text);
        assert!(p.text.contains("| Tue | 18.50 |"), "{}", p.text);
    }
}
