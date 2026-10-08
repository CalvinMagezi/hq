//! Export a note, read the file back with the repo's own document reader, and
//! check the content survived. This is an independent check that the files are
//! valid, not just that our writer produced bytes.

use hq_convert::inbound::InboundConverter;
use hq_export::{Document, ExportOptions, Format, export};

const NOTE: &str = "## Field visits\n\nWe visited **Mukono** and Wakiso this week.\n\n\
| Site | Farmers | Status |\n|:-----|--------:|:------:|\n| Mukono | 42 | done |\n| Wakiso | 17 | pending |\n\n\
- first item\n- second item\n\n1. step one\n2. step two\n";

async fn read_back(format: Format, ext: &str) -> String {
    let doc = Document::from_markdown("Weekly planning", NOTE);
    let out = export(&doc, format, &ExportOptions::default()).expect("export");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("note.{ext}"));
    std::fs::write(&path, &out.bytes).unwrap();
    InboundConverter::new()
        .unwrap()
        .convert(&path)
        .await
        .unwrap_or_else(|e| panic!("reading back {ext}: {e}"))
}

#[tokio::test]
async fn docx_reads_back_with_its_text_and_table() {
    let md = read_back(Format::Docx, "docx").await;
    for expected in ["Weekly planning", "Field visits", "Mukono", "Wakiso", "42", "pending", "first item", "step two"] {
        assert!(md.contains(expected), "docx lost {expected:?}:\n{md}");
    }
}

#[tokio::test]
async fn xlsx_reads_back_with_its_cells() {
    let md = read_back(Format::Xlsx, "xlsx").await;
    for expected in ["Site", "Farmers", "Mukono", "42", "Wakiso", "17"] {
        assert!(md.contains(expected), "xlsx lost {expected:?}:\n{md}");
    }
}

#[tokio::test]
async fn pdf_text_can_be_extracted() {
    let md = read_back(Format::Pdf, "pdf").await;
    for expected in ["Field visits", "Mukono", "Wakiso"] {
        assert!(md.contains(expected), "pdf lost {expected:?}:\n{md}");
    }
}

#[tokio::test]
async fn html_and_csv_read_back_too() {
    let html = read_back(Format::Html, "html").await;
    assert!(html.contains("Mukono") && html.contains("first item"), "{html}");
    let csv = read_back(Format::Csv, "csv").await;
    assert!(csv.contains("Mukono") && csv.contains("42"), "{csv}");
}
