//! Diagnostic, not a gate: open the suite's Word file in a real LibreOffice and look at the pages.
//!
//! Needs `soffice` on the path. Run with
//! `cargo test -p hq-suite-docs --test soffice -- --ignored --nocapture`.
//! Set `HQ_SUITE_RENDER_OUT=/some/dir` to keep the `.docx` and `.pdf` for looking at
//! (`pdftoppm -png -r 60 out.pdf page`).

use hq_suite_docs::{DocxOptions, render_docx_with};

const NOTE: &str = "# Field report\n\nOpening paragraph.\n\n## Findings\n\nSome text about findings.\n\n### Detail one\n\nMore text.\n\n#### Too deep for the contents\n\nDeep.\n\n## Costs\n\n| Item | Qty |\n|---|---|\n| Sugar | 40 |\n\n## Next steps\n\n- one\n- two\n";

#[test]
#[ignore = "needs LibreOffice (soffice) installed"]
fn libreoffice_opens_the_file_with_contents_and_header_and_makes_a_pdf() {
    let opts = DocxOptions {
        toc: true,
        header: "Draft for review".into(),
    };
    let bytes = render_docx_with("Field report", NOTE, None, &opts).unwrap();
    let dir = match std::env::var_os("HQ_SUITE_RENDER_OUT") {
        Some(d) => std::path::PathBuf::from(d),
        None => std::env::temp_dir().join(format!("hq-suite-soffice-{}", std::process::id())),
    };
    std::fs::create_dir_all(&dir).unwrap();
    let docx = dir.join("report.docx");
    std::fs::write(&docx, &bytes).unwrap();
    let status = std::process::Command::new("soffice")
        .args(["--headless", "--convert-to", "pdf", "--outdir"])
        .arg(&dir)
        .arg(&docx)
        .status()
        .expect("soffice is not installed");
    assert!(status.success());
    let pdf = std::fs::read(dir.join("report.pdf")).unwrap();
    assert!(pdf.starts_with(b"%PDF"), "LibreOffice wrote a PDF");
}
