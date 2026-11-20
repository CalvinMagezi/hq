//! Opens a workbook with a native chart in LibreOffice and converts it to PDF.
//! Diagnostic, not a gate: `cargo test -p hq-suite-xlsx --test libreoffice -- --ignored`.
//! `XLSX_KEEP=<dir>` keeps the PDF there so the chart can be looked at.

use hq_suite_md::ColType;
use hq_suite_xlsx::{ChartKind, ChartSeries, ChartSpec, SheetSpec, write_xlsx_with_charts};

#[test]
#[ignore = "needs LibreOffice (`soffice`) on the PATH"]
fn libreoffice_opens_a_workbook_with_charts() {
    let sheet = SheetSpec {
        name: "Stock".into(),
        header: vec!["item".into(), "qty".into()],
        types: vec![ColType::Text, ColType::Int],
        rows: vec![
            vec!["Sugar".into(), "40".into()],
            vec!["Salt".into(), "10".into()],
        ],
    };
    let chart = ChartSpec {
        sheet: 0,
        kind: ChartKind::Bar,
        title: Some("Stock by item".into()),
        x_col: 0,
        labels: vec!["Sugar".into(), "Salt".into()],
        series: vec![ChartSeries {
            name: "qty".into(),
            col: 1,
            values: vec![40.0, 10.0],
        }],
        first_row: 2,
        x_title: Some("item".into()),
        y_title: Some("qty".into()),
    };
    let (bytes, _) = write_xlsx_with_charts(&[sheet], &[chart]).unwrap();
    let dir = std::env::temp_dir().join(format!("xlsx-chart-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("c.xlsx");
    std::fs::write(&file, bytes).unwrap();
    let out = std::process::Command::new("soffice")
        .args(["--headless", "--convert-to", "pdf", "--outdir"])
        .arg(&dir)
        .arg(&file)
        .output()
        .expect("soffice runs");
    assert!(out.status.success() && dir.join("c.pdf").exists());
    if let Some(keep) = std::env::var_os("XLSX_KEEP") {
        let _ = std::fs::copy(dir.join("c.pdf"), std::path::Path::new(&keep).join("c.pdf"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
