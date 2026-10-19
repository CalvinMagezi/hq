//! Whole files through the import pipeline.
//!
//! The fixtures are invented documents (see `tests/fixtures/corpus/README.md`).
//! Tests that need poppler, LibreOffice or tesseract are ignored; run them with
//! `cargo test -p hq-convert --test corpus -- --ignored` on a machine that has them.

use std::path::PathBuf;

use hq_convert::ingest::{ConversionReport, ConvertOptions, LossKind, Outcome, convert_file};
use hq_convert::types::ConvertError;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/corpus")
        .join(name)
}

async fn convert(name: &str) -> (String, ConversionReport) {
    let c = convert_file(&fixture(name), &ConvertOptions::default())
        .await
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    (c.markdown, c.report)
}

fn assert_has(md: &str, name: &str, needles: &[&str]) {
    for n in needles {
        assert!(md.contains(n), "{name} lost {n:?}:\n{md}");
    }
}

#[tokio::test]
async fn the_same_invoice_reads_the_same_from_docx_and_odt() {
    let (docx, r) = convert("invoice.docx").await;
    let (odt, _) = convert("invoice.odt").await;
    assert_eq!(docx, odt, "the two formats should give identical Markdown");
    assert_eq!(r.engine, "native:docx");
    assert_eq!(r.outcome, Outcome::Ok);
    assert_eq!(
        (r.stats.tables, r.stats.list_items, r.stats.links),
        (2, 3, 1)
    );
    assert_has(
        &docx,
        "invoice",
        &[
            "# INVOICE",
            "**Example Agro Supplies Ltd**",
            "| Maize seed, 25 kg bag | 10 | 85,000 | 850,000 |",
            "| **TOTAL** |  |  | **2,430,000** |",
            "### Payment details",
            "- Mobile Money: 0700 000 000 (Example Agro Supplies)",
            "[example.com/pay](https://example.com/pay/INV-0042)",
            "*Late payments attract 2% interest per month.*",
        ],
    );
}

#[tokio::test]
async fn the_invoice_survives_rtf_too() {
    let (md, r) = convert("invoice.rtf").await;
    assert_eq!(r.engine, "native:rtf");
    assert_has(
        &md,
        "invoice.rtf",
        &[
            "# INVOICE",
            "**Example Agro Supplies Ltd**",
            "| Maize seed, 25 kg bag | 10 | 85,000 | 850,000 |",
            "### Payment details",
            "- Reference: INV-0042",
            "[example.com/pay](https://example.com/pay/INV-0042)",
        ],
    );
}

#[tokio::test]
async fn the_proposal_keeps_its_structure_in_docx_and_odt() {
    for name in ["proposal.docx", "proposal.odt"] {
        let (md, r) = convert(name).await;
        assert_eq!(r.outcome, Outcome::Ok, "{name}");
        assert_has(
            &md,
            name,
            &[
                "# Community Seed Bank Proposal",
                "## 1. Background",
                "### 1.1 Objectives",
                "**up to 30%**",
                "*field notes*",
                "[baseline report](https://example.com/report)",
                "`seedctl --report`",
                "- Reduce seed loss",
                "1. Survey sites",
                "3. Train farmers",
                "| Item | Quantity | Cost |",
                "> Rains may delay site visits by two weeks.",
                "```\ndef weekly(loss):\n    return round(loss * 0.3, 2)\n```",
                "*[image]*",
            ],
        );
    }
}

/// Without LibreOffice in the way, so the result does not depend on the machine.
async fn convert_without_office(name: &str) -> (String, ConversionReport) {
    let opts = ConvertOptions {
        external_office: false,
        ..ConvertOptions::default()
    };
    let c = convert_file(&fixture(name), &opts)
        .await
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    (c.markdown, c.report)
}

#[tokio::test]
async fn spreadsheets_show_dates_as_dates_in_all_three_formats() {
    for name in ["tracker.xlsx", "tracker.xls", "tracker.ods"] {
        let (md, r) = convert_without_office(name).await;
        assert!(
            r.engine.starts_with("calamine:") || r.engine == "native:ods",
            "{name}: {}",
            r.engine
        );
        assert_has(
            &md,
            name,
            &[
                "## Q3 Sales",
                "**Q3 2026 field sales (invented data)**",
                "| Date | Site | Farmers | Revenue (UGX) | Growth | Status |",
                "| 2026-07-01 | Mukono |",
                "## Notes",
                "Revenue is net of transport.",
            ],
        );
        assert!(!md.contains("46"), "{name} shows a serial number:\n{md}");
    }
}

#[tokio::test]
async fn number_formats_are_applied_in_xlsx_and_ods() {
    for name in ["tracker.xlsx", "tracker.ods"] {
        let (md, r) = convert_without_office(name).await;
        // 2700000 under `#,##0` and 0.097 under `0.0%`.
        assert_has(&md, name, &["| 2,700,000 |", "| 9.7% |", "| -8.1% |"]);
        assert!(!md.contains("0.097"), "{name}:\n{md}");
        assert!(
            r.warnings
                .iter()
                .all(|w| w.kind != LossKind::NumberFormatsNotApplied),
            "{name}"
        );
        assert_eq!(r.outcome, Outcome::Ok, "{name}");
    }
}

#[tokio::test]
async fn an_old_xls_without_libreoffice_keeps_plain_numbers_and_says_so() {
    let (md, r) = convert_without_office("tracker.xls").await;
    assert_eq!(r.engine, "calamine:xls");
    assert_has(&md, "tracker.xls", &["| 0.097 |"]);
    assert!(
        r.warnings
            .iter()
            .any(|w| w.kind == LossKind::NumberFormatsNotApplied),
        "{:?}",
        r.warnings
    );
}

#[tokio::test]
#[ignore = "needs LibreOffice (soffice)"]
async fn an_old_xls_is_re_saved_by_libreoffice_so_its_formats_survive() {
    let (md, r) = convert("tracker.xls").await;
    assert_eq!(r.engine, "libreoffice+calamine:xlsx");
    assert_has(&md, "tracker.xls", &["| 2,700,000 |", "| 9.7% |"]);
    assert!(
        r.warnings
            .iter()
            .all(|w| w.kind != LossKind::NumberFormatsNotApplied)
    );
}

#[tokio::test]
async fn decks_give_one_section_per_slide_with_notes_and_tables() {
    for name in ["training.pptx", "training.odp"] {
        let (md, _) = convert(name).await;
        assert_has(
            &md,
            name,
            &[
                "## Slide 1: Seed Storage Training",
                "## Slide 2: Agenda",
                "- Why seed is lost\n    - Moisture\n    - Pests\n- Hermetic storage",
                "### Notes",
                "Allow 10 minutes. Ask who has lost seed.",
                "| Item | Qty | UGX |",
                "| Hermetic bag | 400 | 6,000,000 |",
            ],
        );
    }
}

#[tokio::test]
async fn one_page_in_three_encodings_gives_the_same_content() {
    for name in ["page.html", "page-latin1.html", "page-utf16.html"] {
        let (md, r) = convert(name).await;
        assert_eq!(r.engine, "native:html", "{name}");
        assert_has(
            &md,
            name,
            &[
                "Seed bank update",
                "[full report](https://example.com/report)",
                "| Mukono | 42 | done |",
                "- Install\n    - Bags",
                "1. First",
                "> Seed saved is food saved.",
                "```python\nprint(\"hello\")\n```",
                "caf\u{e9}",
            ],
        );
        assert!(
            !md.contains("alert(1)") && !md.contains("color:red"),
            "{name} kept script or style"
        );
    }
}

#[tokio::test]
async fn xml_records_become_a_table_and_other_xml_an_outline() {
    let (records, _) = convert("records.xml").await;
    assert_has(
        &records,
        "records.xml",
        &[
            "| site | farmers | note |",
            "| Mukono | 42 | Done on time |",
        ],
    );
    let (notes, _) = convert("notes.xml").await;
    assert_has(
        &notes,
        "notes.xml",
        &["- **report**", "**para**: Seed loss fell to 12%."],
    );
}

#[tokio::test]
async fn delimited_and_plain_text() {
    let (csv, _) = convert("data.csv").await;
    let (tsv, _) = convert("data.tsv").await;
    assert_eq!(csv, tsv);
    assert_eq!(
        csv,
        "| Site | Farmers | Status |\n| --- | --- | --- |\n| Mukono | 42 | done |\n| Wakiso | 17 | pending |\n"
    );
    let (txt, r) = convert("plain.txt").await;
    assert!(txt.starts_with("Field note"), "{txt}");
    assert_eq!(r.source_format, "txt");
}

#[tokio::test]
async fn a_file_is_read_by_its_content_not_its_name() {
    let dir = tempfile::tempdir().unwrap();
    for wrong in ["report.pdf", "report", "report.txt"] {
        let path = dir.path().join(wrong);
        std::fs::copy(fixture("invoice.docx"), &path).unwrap();
        let c = convert_file(&path, &ConvertOptions::default())
            .await
            .unwrap();
        assert_eq!(c.report.source_format, "docx", "{wrong}");
        assert!(c.markdown.contains("# INVOICE"));
        let named = wrong.contains('.');
        assert_eq!(
            c.report
                .warnings
                .iter()
                .any(|w| w.kind == LossKind::ExtensionMismatch),
            named,
            "{wrong}"
        );
    }
}

#[tokio::test]
async fn the_report_has_a_hash_and_counts() {
    let (_, r) = convert("invoice.docx").await;
    assert_eq!(r.source_sha256.len(), 64);
    assert_eq!(
        r.source_bytes,
        std::fs::metadata(fixture("invoice.docx")).unwrap().len()
    );
    assert!(r.stats.words > 40);
}

// ------------------------------------------------------------ safety limits

#[tokio::test]
async fn a_container_that_expands_past_the_limit_is_refused() {
    // 8 MB of zeros compresses to a few KB: the classic zip bomb shape.
    let bomb = {
        use std::io::Write;
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        w.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
        w.write_all(&vec![0u8; 8 * 1024 * 1024]).unwrap();
        w.finish().unwrap();
        buf.into_inner()
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bomb.docx");
    std::fs::write(&path, &bomb).unwrap();
    assert!(
        bomb.len() < 100_000,
        "the bomb must be small on disk, was {}",
        bomb.len()
    );
    let mut opts = ConvertOptions::default();
    opts.limits.max_entry_bytes = 1024 * 1024;
    let err = convert_file(&path, &opts).await.unwrap_err();
    assert!(matches!(err, ConvertError::TooLarge(_)), "{err}");
}

#[tokio::test]
async fn an_input_over_the_size_limit_is_refused_before_it_is_read() {
    let mut opts = ConvertOptions::default();
    opts.limits.max_input_bytes = 100;
    let err = convert_file(&fixture("invoice.docx"), &opts)
        .await
        .unwrap_err();
    assert!(matches!(err, ConvertError::TooLarge(_)), "{err}");
}

#[tokio::test]
async fn binary_files_nobody_reads_are_unsupported_not_garbled() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("archive.bin");
    std::fs::write(&path, [0u8, 1, 2, 3, 0xff, 0xfe, 0, 9]).unwrap();
    let err = convert_file(&path, &ConvertOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, ConvertError::UnsupportedFormat(_)), "{err}");
}

#[tokio::test]
async fn a_text_file_that_is_not_a_zip_is_not_a_docx() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.docx");
    std::fs::write(&path, "just some notes\n").unwrap();
    let c = convert_file(&path, &ConvertOptions::default())
        .await
        .unwrap();
    assert_eq!(c.report.source_format, "txt");
    assert!(
        c.report
            .warnings
            .iter()
            .any(|w| w.kind == LossKind::ExtensionMismatch)
    );
}

// ------------------------------------------------ paths that need other tools

#[tokio::test]
#[ignore = "needs pdftotext (poppler-utils)"]
async fn a_statement_pdf_is_one_table_across_its_pages() {
    let (md, r) = convert("statement.pdf").await;
    assert_eq!(r.engine, "poppler:pdftotext-layout");
    assert_eq!((r.stats.tables, r.stats.headings), (1, 0), "{md}");
    assert_has(
        &md,
        "statement.pdf",
        &[
            "| Date | Transaction ID | Type | Details | Amount (UGX) | Balance (UGX) |",
            "| 01/09/2026 | MP26090000 | Deposit |",
        ],
    );
    let rows = md
        .lines()
        .filter(|l| l.starts_with("| ") && l.contains("/2026 |"))
        .count();
    assert_eq!(rows, 52, "every transaction on both pages is a row");
}

#[tokio::test]
#[ignore = "needs pdftotext (poppler-utils)"]
async fn an_invoice_pdf_keeps_its_line_items_in_columns() {
    let (md, _) = convert("invoice.pdf").await;
    assert_has(
        &md,
        "invoice.pdf",
        &[
            "| Description | Qty | Unit price (UGX) | Amount (UGX) |",
            "| Maize seed, 25 kg bag | 10 | 85,000 | 850,000 |",
            "| Bill to | Invoice details |",
            "- Bank: Example Bank, account 0000000000",
        ],
    );
}

#[tokio::test]
#[ignore = "needs LibreOffice (soffice)"]
async fn a_legacy_doc_opens_through_libreoffice_and_reads_like_the_docx() {
    let (legacy, r) = convert("invoice.doc").await;
    let (modern, _) = convert("invoice.docx").await;
    assert_eq!(r.engine, "libreoffice+native:docx");
    assert!(
        r.warnings
            .iter()
            .any(|w| w.kind == LossKind::LegacyConverted)
    );
    assert_eq!(legacy, modern);
}

#[tokio::test]
#[ignore = "needs tesseract-ocr"]
async fn a_photographed_receipt_is_read_by_ocr_and_marked_uncertain() {
    let (md, r) = convert("receipt.jpg").await;
    assert_eq!(r.engine, "ocr:tesseract");
    assert_eq!(r.outcome, Outcome::Degraded);
    assert!(r.warnings.iter().any(|w| w.kind == LossKind::OcrDerived));
    assert_has(&md, "receipt.jpg", &["RECEIPT", "370,000"]);
}
