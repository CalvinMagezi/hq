//! PDFs and images: the text layer through `transmutation`, and OCR for pages
//! that are pictures of text.

use std::path::Path;

use transmutation::{ConversionOptions, Converter, OutputFormat};

use super::ir::{Block, Inline, Parsed};
use super::pdf_layout::{self, Layout};
use super::report::{LossKind, Losses};
use crate::ocr::OcrEngine;
use crate::types::ConvertError;

/// Below this many characters a PDF is treated as having no text layer.
pub const MIN_TEXT_CHARS: usize = 50;

/// The text layer of a PDF as Markdown. `Err` when there is none.
pub async fn text_layer(pdf: &Path) -> Result<String, ConvertError> {
    let converter = Converter::new().map_err(|e| ConvertError::InboundFailed(e.to_string()))?;
    let opts = ConversionOptions {
        optimize_for_llm: true,
        normalize_whitespace: true,
        include_metadata: true,
        ..Default::default()
    };
    let result = converter
        .convert(
            pdf.to_str()
                .ok_or_else(|| ConvertError::Other("path contains invalid UTF-8".into()))?,
        )
        .to(OutputFormat::Markdown {
            split_pages: false,
            optimize_for_llm: true,
        })
        .with_options(opts)
        .execute()
        .await
        .map_err(|e| ConvertError::InboundFailed(e.to_string()))?;
    let md = result
        .content
        .iter()
        .map(|o| String::from_utf8_lossy(&o.data).into_owned())
        .collect::<Vec<_>>()
        .join("\n\n");
    if md.trim().is_empty() {
        return Err(ConvertError::InboundFailed(
            "conversion produced no text".into(),
        ));
    }
    Ok(md)
}

/// The PDF as laid out on the page, when poppler is installed and the PDF holds tables.
/// `None` means "use the plain text layer instead".
pub async fn layout(pdf: &Path) -> Option<Layout> {
    let bin = which::which("pdftotext").ok()?;
    let mut cmd = tokio::process::Command::new(bin);
    cmd.kill_on_drop(true)
        .args(["-layout", "-enc", "UTF-8"])
        .arg(pdf)
        .arg("-");
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        crate::run_tool(cmd, "pdftotext"),
    )
    .await
    .ok()?
    .ok()?;
    let layout = pdf_layout::from_layout_text(&String::from_utf8_lossy(&out));
    (layout.tables > 0).then_some(layout)
}

/// OCR of every page of a scanned PDF.
pub async fn ocr_pdf(pdf: &Path) -> Result<Parsed, ConvertError> {
    let text = OcrEngine::extract_pdf(pdf)
        .await
        .map_err(with_install_hint)?;
    ocr_parsed(&text, true)
}

/// OCR of a picture of text, such as a photographed receipt or a screenshot.
pub async fn ocr_image(image: &Path) -> Result<Parsed, ConvertError> {
    let text = OcrEngine::extract_text(image)
        .await
        .map_err(with_install_hint)?;
    ocr_parsed(&text, false)
}

fn with_install_hint(e: ConvertError) -> ConvertError {
    match e {
        ConvertError::Other(m) if m.contains("not found") => ConvertError::Other(format!(
            "{m}. This file needs OCR: install tesseract-ocr (and poppler-utils for PDFs)"
        )),
        other => other,
    }
}

/// OCR text as paragraphs. Line breaks inside a paragraph are kept, because a
/// receipt or a statement is a list of lines. `[page N]` markers from the PDF
/// path become page headings when there is more than one page.
fn ocr_parsed(text: &str, from_pdf: bool) -> Result<Parsed, ConvertError> {
    let mut losses = Losses::default();
    losses.note(
        LossKind::OcrDerived,
        "text was read from an image by OCR; check names, numbers and amounts against the original",
    );
    let mut blocks: Vec<Block> = Vec::new();
    let pages = text.matches("[page ").count();
    for chunk in text.split("\n\n") {
        let mut lines: Vec<&str> = chunk
            .lines()
            .map(str::trim_end)
            .filter(|l| !l.trim().is_empty())
            .collect();
        if from_pdf
            && let Some(first) = lines.first()
            && let Some(n) = first
                .strip_prefix("[page ")
                .and_then(|r| r.strip_suffix(']'))
        {
            if pages > 1 {
                blocks.push(Block::Heading(2, vec![Inline::text(format!("Page {n}"))]));
            }
            lines.remove(0);
        }
        if lines.is_empty() {
            continue;
        }
        let mut inl: Vec<Inline> = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            if i > 0 {
                inl.push(Inline::Break);
            }
            inl.push(Inline::text(l.trim().to_string()));
        }
        blocks.push(Block::Para(inl));
    }
    if blocks.is_empty() {
        return Err(ConvertError::InboundFailed("OCR found no text".into()));
    }
    Ok(Parsed {
        blocks,
        title: None,
        losses,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocr_pages_become_headings_and_lines_stay_lines() {
        let p = ocr_parsed(
            "[page 1]\nTOTAL 370,000\nPaid by MoMo\n\n[page 2]\nThank you",
            true,
        )
        .unwrap();
        let md = super::super::ir::render(p.blocks);
        assert_eq!(
            md,
            "## Page 1\n\nTOTAL 370,000\\\nPaid by MoMo\n\n## Page 2\n\nThank you\n"
        );
    }

    #[test]
    fn a_single_page_has_no_page_heading() {
        let p = ocr_parsed("[page 1]\nOnly page", true).unwrap();
        assert_eq!(super::super::ir::render(p.blocks), "Only page\n");
    }

    #[test]
    fn empty_ocr_is_an_error() {
        assert!(ocr_parsed("  \n\n", false).is_err());
    }
}
