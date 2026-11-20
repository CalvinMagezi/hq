//! A note's `plot` blocks as native spreadsheet charts for the `.xlsx` export.
//!
//! A chart is exported when it reads a plain range: its labels and values then sit in cells of one
//! sheet, and the workbook's chart can point at them. A plot with `where` or `group` has no single
//! range behind it, so it is left out and counted, and the export's statement says so.

use hq_suite_md::{BlockType, Document, RangeRef};
use hq_suite_xlsx::{ChartKind, ChartSeries, ChartSpec};

use crate::sheets::{NoteBook, plot_of, sheet_of_range};

/// The charts a workbook can carry, and how many plots were left out (they filter, group, or would
/// not draw).
pub struct NativeCharts {
    pub charts: Vec<ChartSpec>,
    pub left_out: usize,
}

/// Charts for the `plot` blocks of the note whose text is `text`, at `note` in the vault.
pub fn native_charts(text: &str, note: &str) -> NativeCharts {
    let doc = Document::parse(text);
    let blocks: Vec<_> = doc.typed_blocks().filter_map(|(_, b)| b.ok()).collect();
    let book = NoteBook::new(&blocks);
    let mut out = NativeCharts {
        charts: Vec::new(),
        left_out: 0,
    };
    for (index, plot) in blocks
        .iter()
        .filter(|b| b.ty == BlockType::Plot)
        .enumerate()
    {
        match one(&blocks, &book, plot, &doc, note, index) {
            Some(spec) => out.charts.push(spec),
            None => out.left_out += 1,
        }
    }
    out
}

fn one(
    blocks: &[hq_suite_md::TypedBlock],
    book: &NoteBook,
    plot: &hq_suite_md::TypedBlock,
    doc: &Document,
    note: &str,
    index: usize,
) -> Option<ChartSpec> {
    if plot.info.get("where").is_some() || plot.info.get("group").is_some() {
        return None;
    }
    let kind = match plot.info.get("type")? {
        "bar" => ChartKind::Bar,
        "line" => ChartKind::Line,
        "pie" => ChartKind::Pie,
        "donut" => ChartKind::Donut,
        _ => return None,
    };
    let from = RangeRef::parse(plot.info.get("from")?).ok()?;
    let sheet = sheet_of_range(blocks, note, &from, "a plot").ok()?;
    let header = from.start.row - 1;
    let column_of = |name: &str| {
        let name = name.trim().to_lowercase();
        (from.start.col..=from.end.col).find(|c| {
            book.book
                .get(sheet, c - 1, header)
                .to_string()
                .trim()
                .to_lowercase()
                == name
        })
    };
    let x = column_of(plot.info.get("x")?)?;
    let y = column_of(plot.info.get("y")?)?;
    // The values come from the same evaluation the phone's chart uses, so the cached numbers match.
    let drawn: serde_json::Value = serde_json::from_str(&plot_of(doc, note, index).ok()?).ok()?;
    let labels: Vec<String> = drawn["labels"]
        .as_array()?
        .iter()
        .map(|l| l.as_str().unwrap_or_default().to_string())
        .collect();
    let series = drawn["series"].as_array()?;
    let values: Vec<f64> = series.first()?["values"]
        .as_array()?
        .iter()
        .filter_map(serde_json::Value::as_f64)
        .collect();
    Some(ChartSpec {
        sheet,
        kind,
        title: plot.info.get("title").map(str::to_string),
        x_col: x - 1,
        labels,
        series: vec![ChartSeries {
            name: book.book.get(sheet, y - 1, header).to_string(),
            col: y - 1,
            values,
        }],
        first_row: from.start.row as usize + 1,
        x_title: matches!(kind, ChartKind::Bar | ChartKind::Line)
            .then(|| plot.info.get("x").map(str::to_string))
            .flatten(),
        y_title: matches!(kind, ChartKind::Bar | ChartKind::Line)
            .then(|| plot.info.get("y").map(str::to_string))
            .flatten(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "---\nsuite_kind: dashboard\n---\n```sheet name=Stock types=\"text,int\"\n| item | qty |\n|---|---|\n| Sugar | 40 |\n| Salt | 10 |\n```\n\n```plot type=bar from=\"A1:B3\" x=item y=qty title=\"By item\"\n```\n\n```plot type=bar from=\"A1:B3\" x=item y=qty where=\"qty>20\"\n```\n";

    #[test]
    fn a_plain_plot_becomes_a_chart_and_a_filtered_one_is_left_out() {
        let got = native_charts(NOTE, "Notebooks/s.md");
        assert_eq!((got.charts.len(), got.left_out), (1, 1));
        let c = &got.charts[0];
        assert_eq!((c.sheet, c.x_col, c.first_row), (0, 0, 2));
        assert_eq!(c.labels, ["Sugar", "Salt"]);
        assert_eq!(c.series[0].col, 1);
        assert_eq!(c.series[0].values, [40.0, 10.0]);
        assert_eq!(c.series[0].name, "qty");
    }

    #[test]
    fn the_export_carries_the_chart_and_says_what_it_left_out() {
        let (bytes, line) = crate::author::sheet_bytes(NOTE, "xlsx").unwrap().unwrap();
        let zip = std::io::Cursor::new(bytes);
        let mut z = zip::ZipArchive::new(zip).unwrap();
        assert!(z.by_name("xl/charts/chart1.xml").is_ok());
        assert!(z.by_name("xl/charts/chart2.xml").is_err());
        assert!(line.contains("1 chart kept as native chart"), "{line}");
        assert!(
            line.contains("1 chart that filter or group rows is not included"),
            "{line}"
        );
    }
}
