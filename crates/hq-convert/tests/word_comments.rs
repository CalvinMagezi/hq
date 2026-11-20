//! Word comments through the public importer: carried only when asked, never changing the rest.

use std::io::{Cursor, Write};

use hq_convert::ingest::{ConvertOptions, LossKind, OcrMode, convert_bytes};

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;

const TYPES: &str = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;

fn docx() -> Vec<u8> {
    let document = format!(
        r#"<?xml version="1.0"?><w:document {NS}><w:body>
        <w:p><w:r><w:t>Order for March</w:t></w:r></w:p>
        <w:p><w:commentRangeStart w:id="0"/><w:r><w:t>Six bags at 6,000 each.</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r></w:p>
        <w:p><w:r><w:t>Thanks.</w:t></w:r><w:ins w:id="9" w:author="Sam"><w:r><w:t> Added later.</w:t></w:r></w:ins></w:p>
        </w:body></w:document>"#
    );
    let comments = format!(
        r#"<?xml version="1.0"?><w:comments {NS}><w:comment w:id="0" w:author="Sam" w:date="2026-11-20T08:00:00Z"><w:p><w:r><w:t>Is 6,000 right?</w:t></w:r></w:p></w:comment><w:comment w:id="1" w:author="Ana"><w:p><w:r><w:t>Never referenced.</w:t></w:r></w:p></w:comment></w:comments>"#
    );
    let mut buf = Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, data) in [
            ("[Content_Types].xml", TYPES.to_string()),
            ("word/document.xml", document),
            ("word/comments.xml", comments),
        ] {
            w.start_file(name, opts).unwrap();
            w.write_all(data.as_bytes()).unwrap();
        }
        w.finish().unwrap();
    }
    buf.into_inner()
}

fn opts(word_comments: bool) -> ConvertOptions {
    ConvertOptions {
        ocr: OcrMode::Off,
        external_office: false,
        word_comments,
        ..ConvertOptions::default()
    }
}

#[tokio::test]
async fn off_by_default_the_import_is_what_it_always_was() {
    assert!(!ConvertOptions::default().word_comments);
    let done = convert_bytes(docx(), "docx", None, &opts(false))
        .await
        .unwrap();
    assert_eq!(
        done.markdown,
        "Order for March\n\nSix bags at 6,000 each.\n\nThanks. Added later.\n"
    );
    assert!(done.report.warnings.iter().all(|w| !matches!(
        w.kind,
        LossKind::CommentsCarried | LossKind::CommentsUnanchored
    )));
}

#[tokio::test]
async fn asked_for_they_follow_their_block_and_the_report_counts_them() {
    let plain = convert_bytes(docx(), "docx", None, &opts(false))
        .await
        .unwrap();
    let done = convert_bytes(docx(), "docx", None, &opts(true))
        .await
        .unwrap();
    assert_eq!(hq_suite_md::strip_comments(&done.markdown), plain.markdown);
    let doc = hq_suite_md::Document::parse(&done.markdown);
    let found = hq_suite_md::comments(&doc);
    assert_eq!(found.len(), 1, "{}", done.markdown);
    assert!(!found[0].orphaned);
    assert_eq!(found[0].by.as_deref(), Some("Sam"));
    assert_eq!(found[0].at.as_deref(), Some("2026-11-20"));
    assert_eq!(found[0].text, "Is 6,000 right?");
    let kinds: Vec<_> = done.report.warnings.iter().map(|w| w.kind).collect();
    assert!(kinds.contains(&LossKind::CommentsCarried), "{kinds:?}");
    assert!(kinds.contains(&LossKind::CommentsUnanchored), "{kinds:?}");
    assert!(kinds.contains(&LossKind::TrackedChanges), "{kinds:?}");
    // The one that could not be placed is a loss, so the outcome says so.
    assert!(done.report.has_loss());
}
