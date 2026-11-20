//! What the suite would make of a file: its format class, the fidelity tier it would get, and
//! the size of the markdown record. Nothing is written.
//!
//! The tier here is a projection from the importer's own report. The regenerate check that makes
//! a tier `safe` for certain (import, render, import again, compare) needs the renderer and
//! arrives with milestone M2; until then `Safe` means "the importer reported nothing dropped".

use hq_convert::ingest::sniff::{Kind, sniff};
use hq_convert::ingest::{ConvertOptions, LossKind, convert_bytes};
use hq_convert::types::ConvertError;
use hq_suite_md::Fidelity;
use serde::Serialize;

/// A file the importer could read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Projection {
    pub kind: Kind,
    /// A short label for grouping files in a report: `docx`, `pdf-text`, `image`, ...
    pub class: &'static str,
    pub fidelity: Fidelity,
    /// Size of the markdown the importer produced.
    pub md_bytes: u64,
    /// The engine that produced it, such as `native:docx`.
    pub engine: String,
    /// Which kinds of loss the importer reported, each once.
    pub loss_kinds: Vec<LossKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// Password protected. Never shrunk.
    Encrypted,
    /// Over a safety limit (size, entries, expansion).
    TooLarge,
    /// A scan or photo: no text layer, and OCR was not used.
    NeedsOcr,
    NeedsLibreOffice,
    /// A format the importer has no reader for.
    Unsupported,
    Error,
}

impl FailureKind {
    pub fn label(self) -> &'static str {
        match self {
            FailureKind::Encrypted => "encrypted",
            FailureKind::TooLarge => "too_large",
            FailureKind::NeedsOcr => "needs_ocr",
            FailureKind::NeedsLibreOffice => "needs_libreoffice",
            FailureKind::Unsupported => "unsupported",
            FailureKind::Error => "error",
        }
    }
}

/// A file the importer could not read, and why, in terms safe to count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Failure {
    pub kind: FailureKind,
    pub file_kind: Kind,
    pub class: &'static str,
}

/// The report label for a file. A PDF with no text layer is its own class because it is kept as
/// page images, not as text.
pub fn class_of(kind: Kind, failure: Option<FailureKind>) -> &'static str {
    match (kind, failure) {
        (Kind::Pdf, Some(FailureKind::NeedsOcr)) => "pdf-scan",
        (Kind::Pdf, _) => "pdf-text",
        (k, _) => k.label(),
    }
}

/// Sorts an importer error into something a report can count. The importer reports its reasons
/// as text for some cases, so this is the one place that reads it.
pub fn classify(err: &ConvertError) -> FailureKind {
    match err {
        ConvertError::Encrypted(_) => FailureKind::Encrypted,
        ConvertError::TooLarge(_) => FailureKind::TooLarge,
        ConvertError::UnsupportedFormat(m) | ConvertError::InboundFailed(m)
            if m.contains("OCR is off") =>
        {
            FailureKind::NeedsOcr
        }
        ConvertError::UnsupportedFormat(m)
        | ConvertError::InboundFailed(m)
        | ConvertError::Other(m)
            if m.contains("LibreOffice") =>
        {
            FailureKind::NeedsLibreOffice
        }
        ConvertError::UnsupportedFormat(_) => FailureKind::Unsupported,
        _ => FailureKind::Error,
    }
}

/// A document read by the importer: its markdown and what the suite makes of it.
#[derive(Debug, Clone)]
pub struct DocumentText {
    pub markdown: String,
    /// A title the file itself names.
    pub title: Option<String>,
    pub projection: Projection,
}

/// Converts `bytes` as the importer would and reports what the suite would keep. `ext` is the
/// file's extension, used only to break ties, as everywhere else in the importer.
pub async fn project(
    bytes: Vec<u8>,
    ext: &str,
    opts: &ConvertOptions,
) -> Result<Projection, Failure> {
    read_document(bytes, ext, opts).await.map(|d| d.projection)
}

/// Like [`project`], and also returns the markdown.
pub async fn read_document(
    bytes: Vec<u8>,
    ext: &str,
    opts: &ConvertOptions,
) -> Result<DocumentText, Failure> {
    let sniffed = sniff(&bytes, ext);
    let fail = |kind: FailureKind| Failure {
        kind,
        file_kind: sniffed.kind,
        class: class_of(sniffed.kind, Some(kind)),
    };
    if sniffed.encrypted {
        return Err(fail(FailureKind::Encrypted));
    }
    match convert_bytes(bytes, ext, None, opts).await {
        Err(e) => Err(fail(classify(&e))),
        Ok(done) => {
            let mut loss_kinds: Vec<LossKind> =
                done.report.warnings.iter().map(|w| w.kind).collect();
            loss_kinds.sort();
            loss_kinds.dedup();
            let projection = Projection {
                kind: sniffed.kind,
                class: class_of(sniffed.kind, None),
                fidelity: if done.report.has_loss() {
                    Fidelity::Lossy
                } else {
                    Fidelity::Safe
                },
                md_bytes: done.markdown.len() as u64,
                engine: done.report.engine,
                loss_kinds,
            };
            Ok(DocumentText {
                markdown: done.markdown,
                title: done.title,
                projection,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::tests::zip_of;
    use hq_convert::ingest::OcrMode;

    fn opts() -> ConvertOptions {
        ConvertOptions {
            ocr: OcrMode::Off,
            external_office: false,
            ..ConvertOptions::default()
        }
    }

    const DOCX_TYPES: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;

    fn docx(body: &str) -> Vec<u8> {
        let doc = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
        );
        zip_of(&[
            ("[Content_Types].xml", DOCX_TYPES),
            ("word/document.xml", doc.as_bytes()),
        ])
    }

    #[tokio::test]
    async fn a_plain_document_projects_safe() {
        let bytes = docx("<w:p><w:r><w:t>Order for March</w:t></w:r></w:p>");
        let p = project(bytes, "docx", &opts()).await.unwrap();
        assert_eq!(p.class, "docx");
        assert_eq!(p.fidelity, Fidelity::Safe);
        assert!(p.md_bytes > 0);
        assert!(p.engine.starts_with("native"));
    }

    #[tokio::test]
    async fn a_file_with_no_text_is_a_counted_failure() {
        let bytes = docx("<w:p/>");
        let f = project(bytes, "docx", &opts()).await.unwrap_err();
        assert_eq!(f.class, "docx");
        assert_eq!(f.kind, FailureKind::Error);
    }

    #[tokio::test]
    async fn garbage_is_unsupported_not_a_panic() {
        let f = project(b"\x00\x01\x02 who knows".to_vec(), "bin", &opts())
            .await
            .unwrap_err();
        assert!(matches!(
            f.kind,
            FailureKind::Unsupported | FailureKind::Error
        ));
    }

    #[tokio::test]
    async fn a_photo_without_ocr_is_needs_ocr() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];
        let f = project(png.to_vec(), "png", &opts()).await.unwrap_err();
        assert_eq!(f.class, "image");
        assert_eq!(f.kind, FailureKind::NeedsOcr);
    }

    #[test]
    fn errors_are_sorted_into_countable_kinds() {
        use ConvertError::*;
        assert_eq!(classify(&Encrypted("x".into())), FailureKind::Encrypted);
        assert_eq!(classify(&TooLarge("x".into())), FailureKind::TooLarge);
        assert_eq!(
            classify(&UnsupportedFormat("image (OCR is off)".into())),
            FailureKind::NeedsOcr
        );
        assert_eq!(
            classify(&InboundFailed(
                "this PDF has almost no text and OCR is off".into()
            )),
            FailureKind::NeedsOcr
        );
        assert_eq!(
            classify(&UnsupportedFormat("zip".into())),
            FailureKind::Unsupported
        );
        assert_eq!(
            classify(&Other("needs LibreOffice".into())),
            FailureKind::NeedsLibreOffice
        );
        assert_eq!(classify(&InboundFailed("boom".into())), FailureKind::Error);
    }

    #[test]
    fn pdf_classes_split_on_the_text_layer() {
        assert_eq!(class_of(Kind::Pdf, None), "pdf-text");
        assert_eq!(class_of(Kind::Pdf, Some(FailureKind::NeedsOcr)), "pdf-scan");
        assert_eq!(class_of(Kind::Xlsx, None), "xlsx");
    }

    #[tokio::test]
    async fn word_comments_come_through_only_when_the_options_ask() {
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let comments = format!(
            r#"<?xml version="1.0"?><w:comments xmlns:w="{w}"><w:comment w:id="0" w:author="Sam"><w:p><w:r><w:t>Check this.</w:t></w:r></w:p></w:comment></w:comments>"#
        );
        let doc = format!(
            r#"<?xml version="1.0"?><w:document xmlns:w="{w}"><w:body><w:p><w:commentRangeStart w:id="0"/><w:r><w:t>Order for March</w:t></w:r><w:commentRangeEnd w:id="0"/></w:p></w:body></w:document>"#
        );
        let bytes = zip_of(&[
            ("[Content_Types].xml", DOCX_TYPES),
            ("word/document.xml", doc.as_bytes()),
            ("word/comments.xml", comments.as_bytes()),
        ]);
        let off = read_document(bytes.clone(), "docx", &opts()).await.unwrap();
        assert_eq!(off.markdown, "Order for March\n");
        let on = ConvertOptions {
            word_comments: Some(crate::word_comment_codec()),
            ..opts()
        };
        let got = read_document(bytes, "docx", &on).await.unwrap();
        assert_eq!(hq_suite_md::strip_comments(&got.markdown), off.markdown);
        assert_eq!(
            hq_suite_md::comments(&hq_suite_md::Document::parse(&got.markdown)).len(),
            1
        );
    }
}
