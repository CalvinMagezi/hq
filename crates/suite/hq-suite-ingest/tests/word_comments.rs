//! Word comments as hqmd `comment` blocks, through the public importer and the suite's codec.

use std::io::{Cursor, Write};

use hq_convert::ingest::{ConvertOptions, Converted, LossKind, OcrMode, convert_bytes};
use hq_suite_ingest::word_comment_codec;
use hq_suite_md::{CommentState, Document, comments, strip_comments};

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";

const TYPES: &str = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;

fn docx(body: &str, extra: &[(&str, &str)]) -> Vec<u8> {
    let document =
        format!(r#"<?xml version="1.0"?><w:document {NS}><w:body>{body}</w:body></w:document>"#);
    let mut buf = Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default();
        let mut entries: Vec<(&str, &str)> = vec![
            ("[Content_Types].xml", TYPES),
            ("word/document.xml", &document),
        ];
        entries.extend_from_slice(extra);
        for (name, data) in entries {
            w.start_file(name, opts).unwrap();
            w.write_all(data.as_bytes()).unwrap();
        }
        w.finish().unwrap();
    }
    buf.into_inner()
}

fn opts(on: bool) -> ConvertOptions {
    ConvertOptions {
        ocr: OcrMode::Off,
        external_office: false,
        word_comments: on.then(word_comment_codec),
        ..ConvertOptions::default()
    }
}

async fn run(bytes: &[u8], on: bool) -> Converted {
    convert_bytes(bytes.to_vec(), "docx", None, &opts(on))
        .await
        .unwrap()
}

fn details(c: &Converted) -> Vec<String> {
    c.report.warnings.iter().map(|w| w.detail.clone()).collect()
}

fn commented_doc() -> Vec<u8> {
    let comments = format!(
        r#"<w:comments {NS} xmlns:w14="{W14}"><w:comment w:id="0" w:author="Sam &quot;the&quot; `Reviewer`" w:date="2026-11-20T10:15:00Z"><w:p w14:paraId="AAA1"><w:r><w:t>Is this the final figure?</w:t></w:r></w:p><w:p w14:paraId="AAA2"><w:r><w:t>Please check.</w:t></w:r></w:p></w:comment>
          <w:comment w:id="1" w:author="Ana" w:date="2026-11-21T09:00:00Z"><w:p w14:paraId="BBB1"><w:r><w:t>Agreed, fixed.</w:t></w:r></w:p></w:comment>
          <w:comment w:id="2" w:author="Lee"><w:p w14:paraId="CCC1"><w:r><w:t>On the table.</w:t></w:r></w:p></w:comment>
          <w:comment w:id="3" w:author="Kim" w:date="2026-11-22T09:00:00Z"><w:p w14:paraId="DDD1"><w:r><w:t>In a header only.</w:t></w:r></w:p></w:comment></w:comments>"#
    );
    let ext = format!(
        r#"<w15:commentsEx xmlns:w15="{W15}"><w15:commentEx w15:paraId="AAA2" w15:done="1"/><w15:commentEx w15:paraId="BBB1" w15:paraIdParent="AAA2" w15:done="1"/><w15:commentEx w15:paraId="CCC1" w15:done="0"/></w15:commentsEx>"#
    );
    let body = r#"<w:p><w:r><w:t>Intro line</w:t></w:r></w:p>
      <w:p><w:commentRangeStart w:id="0"/><w:r><w:t>Budget is 6,000.</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r></w:p>
      <w:tbl><w:tr><w:tc><w:p><w:r><w:t>Item</w:t></w:r></w:p></w:tc><w:tc><w:p><w:commentRangeStart w:id="2"/><w:r><w:t>Cost</w:t></w:r><w:commentRangeEnd w:id="2"/></w:p></w:tc></w:tr>
        <w:tr><w:tc><w:p><w:r><w:t>Bags</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>6</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
      <w:p><w:r><w:t>Closing line</w:t></w:r></w:p>"#;
    docx(
        body,
        &[
            ("word/comments.xml", &comments),
            ("word/commentsExtended.xml", &ext),
        ],
    )
}

#[tokio::test]
async fn comments_follow_their_block_with_its_hash_and_the_rest_of_the_note_is_unchanged() {
    let bytes = commented_doc();
    let got = run(&bytes, true).await;
    let written = &got.markdown;
    let plain = run(&bytes, false).await.markdown;
    assert_eq!(
        strip_comments(written),
        plain,
        "comments are the only difference"
    );
    let doc = Document::parse(written);
    let found = comments(&doc);
    assert_eq!(found.len(), 3, "{written}");
    assert!(found.iter().all(|c| !c.orphaned), "{written}");
    let node_of = |on: &str| {
        doc.nodes()
            .iter()
            .find(|n| n.hash() == on)
            .map(|n| n.src().to_string())
            .unwrap()
    };
    assert_eq!(node_of(&found[0].on), "Budget is 6,000.");
    assert_eq!(found[0].by.as_deref(), Some("Sam the Reviewer"));
    assert_eq!(found[0].at.as_deref(), Some("2026-11-20"));
    assert_eq!(found[0].state, CommentState::Resolved);
    assert_eq!(found[0].text, "Is this the final figure?\nPlease check.");
    // A reply keeps its own author and date, on the block of the comment it answers.
    assert_eq!(found[1].on, found[0].on);
    assert_eq!(found[1].by.as_deref(), Some("Ana"));
    assert_eq!(found[1].at.as_deref(), Some("2026-11-21"));
    assert_eq!(found[1].state, CommentState::Resolved);
    // A comment inside a table cell sits on the table; no date is written when Word has none.
    assert!(node_of(&found[2].on).starts_with("| Item | Cost |"));
    assert_eq!(found[2].at, None);
    assert_eq!(found[2].state, CommentState::Open);
    let notes = details(&got);
    assert!(
        notes.contains(&"3 Word comments carried as comment blocks".to_string()),
        "{notes:?}"
    );
    assert!(
        notes
            .iter()
            .any(|n| n.starts_with("1 Word comment not carried")),
        "{notes:?}"
    );
}

#[tokio::test]
async fn a_comment_on_a_list_item_sits_after_the_list() {
    let numbering = format!(
        r#"<w:numbering {NS}><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:numFmt w:val="decimal"/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#
    );
    let comments_xml = format!(
        r#"<w:comments {NS}><w:comment w:id="7" w:author="Sam"><w:p><w:r><w:t>second?</w:t></w:r></w:p></w:comment></w:comments>"#
    );
    let item = |t: &str, c: &str| {
        format!(
            r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr>{c}<w:r><w:t>{t}</w:t></w:r></w:p>"#
        )
    };
    let body = [
        item("first", ""),
        item("second", r#"<w:commentRangeStart w:id="7"/>"#),
        item("third", ""),
    ]
    .concat();
    let bytes = docx(
        &body,
        &[
            ("word/numbering.xml", &numbering),
            ("word/comments.xml", &comments_xml),
        ],
    );
    let md = run(&bytes, true).await.markdown;
    assert_eq!(strip_comments(&md), "1. first\n2. second\n3. third\n");
    assert_eq!(comments(&Document::parse(&md)).len(), 1);
}

#[tokio::test]
async fn the_codec_writes_what_the_importer_reports() {
    let doc = format!(
        r#"<?xml version="1.0"?><w:comments {NS}><w:comment w:id="0" w:author="Sam" w:date="2026-11-20T08:00:00Z"><w:p><w:r><w:t>Is 6,000 right?</w:t></w:r></w:p></w:comment><w:comment w:id="1" w:author="Ana"><w:p><w:r><w:t>Never referenced.</w:t></w:r></w:p></w:comment></w:comments>"#
    );
    let body = r#"<w:p><w:r><w:t>Order for March</w:t></w:r></w:p>
      <w:p><w:commentRangeStart w:id="0"/><w:r><w:t>Six bags at 6,000 each.</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r></w:p>"#;
    let bytes = docx(body, &[("word/comments.xml", &doc)]);
    let on = run(&bytes, true).await;
    let kinds: Vec<_> = on.report.warnings.iter().map(|w| w.kind).collect();
    assert!(kinds.contains(&LossKind::CommentsCarried), "{kinds:?}");
    assert!(kinds.contains(&LossKind::CommentsUnanchored), "{kinds:?}");
    assert!(on.report.has_loss());
    assert_eq!(comments(&Document::parse(&on.markdown)).len(), 1);
}
