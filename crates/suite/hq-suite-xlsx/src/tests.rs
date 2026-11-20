use std::io::Read;

use hq_convert::ingest::{ConvertOptions, LossKind, OcrMode, convert_bytes};
use hq_suite_md::ColType;

use super::*;

fn stock_sheet() -> SheetSpec {
    SheetSpec {
        name: "Stock".into(),
        header: vec![
            "item".into(),
            "qty".into(),
            "price".into(),
            "total".into(),
            "share".into(),
            "on".into(),
        ],
        types: vec![
            ColType::Text,
            ColType::Int,
            ColType::Money("ugx".into()),
            ColType::Money("ugx".into()),
            ColType::Pct,
            ColType::Date,
        ],
        rows: vec![
            vec![
                "Sugar".into(),
                "40".into(),
                "4,200".into(),
                "=B2*C2".into(),
                "9.5%".into(),
                "2026-11-18".into(),
            ],
            vec![
                "Salt".into(),
                "10".into(),
                "1500".into(),
                "=B3*C3".into(),
                "0.25".into(),
                "2026-11-19".into(),
            ],
            vec![
                "Total".into(),
                "=SUM(B2:B3)".into(),
                "".into(),
                "=SUM(D2:D3)".into(),
                "".into(),
                "".into(),
            ],
        ],
    }
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
}

fn opts() -> ConvertOptions {
    ConvertOptions {
        ocr: OcrMode::Off,
        external_office: false,
        ..ConvertOptions::default()
    }
}

#[test]
fn every_part_is_well_formed_xml() {
    let (bytes, _) = write_xlsx(&[stock_sheet()]).unwrap();
    for name in [
        "[Content_Types].xml",
        "_rels/.rels",
        "xl/workbook.xml",
        "xl/_rels/workbook.xml.rels",
        "xl/styles.xml",
        "xl/worksheets/sheet1.xml",
    ] {
        roxmltree::Document::parse(&part(&bytes, name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

#[test]
fn formulas_are_real_and_carry_their_calculated_values() {
    let (bytes, report) = write_xlsx(&[stock_sheet()]).unwrap();
    assert_eq!((report.formulas, report.formulas_as_text), (4, 0));
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(
        sheet.contains(r#"<c r="D2" s="#) && sheet.contains("<f>B2*C2</f><v>168000</v>"),
        "{sheet}"
    );
    assert!(sheet.contains("<f>SUM(D2:D3)</f><v>183000</v>"), "{sheet}");
    assert!(part(&bytes, "xl/workbook.xml").contains(r#"fullCalcOnLoad="1""#));
}

#[test]
fn types_become_number_formats_and_date_serials() {
    let (bytes, _) = write_xlsx(&[stock_sheet()]).unwrap();
    let styles = part(&bytes, "xl/styles.xml");
    for code in ["&quot;UGX&quot; #,##0", "0.0%", "yyyy-mm-dd", "#,##0"] {
        assert!(styles.contains(code), "{code}: {styles}");
    }
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    // 2026-11-18 is serial 46344; 9.5% is stored as 0.095; 4,200 as 4200.
    assert!(sheet.contains("<v>46344</v>"), "{sheet}");
    assert!(
        sheet.contains("<v>0.095</v>") && sheet.contains("<v>4200</v>"),
        "{sheet}"
    );
    assert!(sheet.contains(r#"state="frozen""#));
}

#[test]
fn hostile_and_unknown_formulas_are_written_as_text() {
    let mut s = stock_sheet();
    s.rows.push(vec![
        r#"=HYPERLINK("http://example.com","x")"#.into(),
        "=cmd|' /C calc'!A0".into(),
        "=WEBSERVICE(\"http://example.com\")".into(),
        "=[1]Sheet1!A1".into(),
        "=SUM(".into(),
        "=1+1".into(),
    ]);
    let (bytes, report) = write_xlsx(&[s]).unwrap();
    assert_eq!(report.formulas_as_text, 5, "{report:?}");
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    for bad in ["HYPERLINK", "cmd|", "WEBSERVICE", "[1]"] {
        let at = sheet.find(bad).unwrap_or_else(|| panic!("{bad} missing"));
        let before = &sheet[..at];
        assert!(
            before
                .rfind("<f>")
                .is_none_or(|f| f < before.rfind("<c ").unwrap()),
            "{bad} is inside a formula"
        );
    }
    assert!(sheet.contains("<f>1+1</f>"));
    assert!(report.fidelity_line().contains("saved as text"));
}

#[test]
fn the_safety_check_reads_function_names_not_text_in_quotes() {
    assert!(formula_is_safe("SUM(A1:A3)"));
    assert!(formula_is_safe(r#"IF(A1>0,"HYPERLINK(","x")"#));
    assert!(formula_is_safe("A1*2"));
    assert!(!formula_is_safe("HYPERLINK(A1)"));
    assert!(!formula_is_safe("MyMacro(1)"));
    assert!(!formula_is_safe("'[x.xlsx]Sheet'!A1"));
}

#[test]
fn sheet_names_are_made_legal_and_unique() {
    let a = SheetSpec {
        name: "A/B:C?".into(),
        header: vec!["x".into()],
        types: vec![],
        rows: vec![],
    };
    let b = SheetSpec {
        name: "a-b-c-".into(),
        ..a.clone()
    };
    let long = SheetSpec {
        name: "x".repeat(40),
        ..a.clone()
    };
    let (bytes, _) = write_xlsx(&[a, b, long]).unwrap();
    let wb = part(&bytes, "xl/workbook.xml");
    assert!(
        wb.contains(r#"name="A-B-C-""#) && wb.contains(r#"name="a-b-c- 2""#),
        "{wb}"
    );
    assert!(!wb.contains(&"x".repeat(32)));
    assert!(write_xlsx(&[]).is_err());
}

#[tokio::test]
async fn the_importer_reads_the_file_back_with_its_values_and_formulas() {
    let (bytes, _) = write_xlsx(&[stock_sheet()]).unwrap();
    let done = convert_bytes(bytes, "xlsx", None, &opts()).await.unwrap();
    for want in [
        "| Sugar | 40 | UGX 4,200 | UGX 168,000 | 9.5% | 2026-11-18 |",
        "| Total | 50 |",
    ] {
        assert!(done.markdown.contains(want), "{want}:\n{}", done.markdown);
    }
    let formulas = done
        .report
        .warnings
        .iter()
        .find(|w| w.kind == LossKind::FormulasShownAsValues);
    assert!(
        formulas.is_some_and(|w| w.detail.starts_with("4 formulas")),
        "{:?}",
        done.report.warnings
    );
}

fn stock_chart(kind: ChartKind, series: usize) -> ChartSpec {
    ChartSpec {
        sheet: 0,
        kind,
        title: Some("Stock <by> item".into()),
        x_col: 0,
        labels: vec!["Sugar".into(), "Salt".into()],
        series: (0..series)
            .map(|i| ChartSeries {
                name: if i == 0 { "qty".into() } else { "price".into() },
                col: 1 + i as u32,
                values: vec![40.0, 10.0],
            })
            .collect(),
        first_row: 2,
        x_title: Some("item".into()),
        y_title: Some("qty".into()),
    }
}

#[test]
fn a_chart_becomes_native_parts_that_are_wired_together() {
    let charts = [
        stock_chart(ChartKind::Bar, 2),
        stock_chart(ChartKind::Pie, 1),
    ];
    let (bytes, report) = write_xlsx_with_charts(&[stock_sheet()], &charts).unwrap();
    assert_eq!(report.charts, 2);
    assert!(
        report
            .fidelity_line()
            .contains("2 charts kept as native charts")
    );
    for name in [
        "[Content_Types].xml",
        "xl/worksheets/sheet1.xml",
        "xl/worksheets/_rels/sheet1.xml.rels",
        "xl/drawings/drawing1.xml",
        "xl/drawings/_rels/drawing1.xml.rels",
        "xl/charts/chart1.xml",
        "xl/charts/chart2.xml",
    ] {
        roxmltree::Document::parse(&part(&bytes, name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    let types = part(&bytes, "[Content_Types].xml");
    assert!(types.contains("/xl/drawings/drawing1.xml") && types.contains("/xl/charts/chart2.xml"));
    assert!(part(&bytes, "xl/worksheets/sheet1.xml").contains(r#"<drawing r:id="rId1"/>"#));
    let bar = part(&bytes, "xl/charts/chart1.xml");
    // The series read the sheet's own cells, with the title escaped.
    assert!(
        bar.contains("'Stock'!$B$2:$B$3") && bar.contains("'Stock'!$A$2:$A$3"),
        "{bar}"
    );
    assert!(
        bar.contains("'Stock'!$B$1:$B$1") && bar.contains("Stock &lt;by&gt; item"),
        "{bar}"
    );
    assert!(part(&bytes, "xl/charts/chart2.xml").contains("c:pieChart"));
}

#[test]
fn a_chart_that_cannot_be_drawn_is_left_out_not_half_written() {
    let mut short = stock_chart(ChartKind::Line, 1);
    short.series[0].values.pop();
    let two_pie = stock_chart(ChartKind::Pie, 2);
    let mut elsewhere = stock_chart(ChartKind::Bar, 1);
    elsewhere.sheet = 5;
    let (bytes, report) =
        write_xlsx_with_charts(&[stock_sheet()], &[short, two_pie, elsewhere]).unwrap();
    assert_eq!(report.charts, 0);
    assert!(
        report
            .fidelity_line()
            .contains("Charts and pictures are not included")
    );
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    assert!(z.by_name("xl/charts/chart1.xml").is_err());
}

#[test]
fn a_sheet_name_with_a_quote_is_quoted_in_the_chart_reference() {
    let mut s = stock_sheet();
    s.name = "Bob's stock".into();
    let (bytes, _) = write_xlsx_with_charts(&[s], &[stock_chart(ChartKind::Line, 1)]).unwrap();
    assert!(part(&bytes, "xl/charts/chart1.xml").contains("'Bob''s stock'!$B$2:$B$3"));
}

#[tokio::test]
async fn the_importer_still_reads_a_workbook_that_has_charts() {
    let (bytes, _) =
        write_xlsx_with_charts(&[stock_sheet()], &[stock_chart(ChartKind::Bar, 1)]).unwrap();
    let done = convert_bytes(bytes, "xlsx", None, &opts()).await.unwrap();
    assert!(
        done.markdown.contains("| Sugar | 40 |"),
        "{}",
        done.markdown
    );
}
