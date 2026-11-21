//! Opens workbooks with a native chart of each kind in LibreOffice and converts them to PDF.
//! Diagnostic, not a gate: `cargo test -p hq-suite-xlsx --test libreoffice -- --ignored`.
//! `XLSX_KEEP=<dir>` keeps the PDFs there (`bar.pdf`, `area.pdf`, ...) so the charts can be looked at.

use hq_suite_md::ColType;
use hq_suite_xlsx::{ChartKind, ChartSeries, ChartSpec, SheetSpec, write_xlsx_with_charts};

fn stock() -> SheetSpec {
    SheetSpec {
        name: "Stock".into(),
        header: vec!["item".into(), "qty".into(), "price".into()],
        types: vec![ColType::Text, ColType::Int, ColType::Int],
        rows: vec![
            vec!["Sugar".into(), "40".into(), "15".into()],
            vec!["Salt".into(), "10".into(), "25".into()],
            vec!["Rice".into(), "30".into(), "5".into()],
            vec!["Beans".into(), "22".into(), "12".into()],
        ],
    }
}

fn points() -> SheetSpec {
    SheetSpec {
        name: "Points".into(),
        header: vec!["x".into(), "y".into()],
        types: vec![ColType::Num, ColType::Num],
        rows: vec![
            vec!["1.5".into(), "3".into()],
            vec!["2".into(), "5.5".into()],
            vec!["3.5".into(), "4".into()],
            vec!["6".into(), "9".into()],
        ],
    }
}

fn chart(kind: ChartKind) -> ChartSpec {
    if kind == ChartKind::Scatter {
        return ChartSpec {
            sheet: 1,
            kind,
            title: Some("y against x".into()),
            x_col: 0,
            labels: vec!["1.5".into(), "2".into(), "3.5".into(), "6".into()],
            series: vec![ChartSeries {
                name: "y".into(),
                col: 1,
                values: vec![3.0, 5.5, 4.0, 9.0],
            }],
            first_row: 2,
            x_title: Some("x".into()),
            y_title: Some("y".into()),
        };
    }
    let two = matches!(
        kind,
        ChartKind::Area | ChartKind::StackedBar | ChartKind::Line
    );
    let mut series = vec![ChartSeries {
        name: "qty".into(),
        col: 1,
        values: vec![40.0, 10.0, 30.0, 22.0],
    }];
    if two {
        series.push(ChartSeries {
            name: "price".into(),
            col: 2,
            values: vec![15.0, 25.0, 5.0, 12.0],
        });
    }
    ChartSpec {
        sheet: 0,
        kind,
        title: Some(format!("{kind:?} chart")),
        x_col: 0,
        labels: vec!["Sugar".into(), "Salt".into(), "Rice".into(), "Beans".into()],
        series,
        first_row: 2,
        x_title: Some("item".into()),
        y_title: Some("qty".into()),
    }
}

#[test]
#[ignore = "needs LibreOffice (`soffice`) on the PATH"]
fn libreoffice_opens_a_workbook_with_a_chart_of_every_kind() {
    let kinds = [
        (ChartKind::Bar, "bar"),
        (ChartKind::Line, "line"),
        (ChartKind::Pie, "pie"),
        (ChartKind::Donut, "donut"),
        (ChartKind::Area, "area"),
        (ChartKind::StackedBar, "stacked"),
        (ChartKind::Scatter, "scatter"),
    ];
    let dir = std::env::temp_dir().join(format!("xlsx-chart-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (kind, name) in kinds {
        let (bytes, report) = write_xlsx_with_charts(&[stock(), points()], &[chart(kind)]).unwrap();
        assert_eq!(report.charts, 1, "{name} was written as a chart");
        let file = dir.join(format!("{name}.xlsx"));
        std::fs::write(&file, bytes).unwrap();
        let out = std::process::Command::new("soffice")
            .args(["--headless", "--convert-to", "pdf", "--outdir"])
            .arg(&dir)
            .arg(&file)
            .output()
            .expect("soffice runs");
        let pdf = dir.join(format!("{name}.pdf"));
        assert!(
            out.status.success() && pdf.exists(),
            "{name}: soffice failed"
        );
        if let Some(keep) = std::env::var_os("XLSX_KEEP") {
            let _ = std::fs::copy(
                &pdf,
                std::path::Path::new(&keep).join(format!("{name}.pdf")),
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
