//! Docs: a record (a note) rendered as a Word file, and the check that proves the render says what
//! the record says.
//!
//! Imports are one way and exports are renders (`docs/plans/hq-suite.md`, the conversion contract).
//! Nothing promises that a complicated Word file survives a round trip; what is promised is that
//! what the suite writes can be read back as the same thing. The regenerate check is that promise
//! as a test: read a file into a record, render the record, read the render, and require the two
//! records to be equal on headings, paragraphs, lists, tables, links and the pictures they hold.

use hq_convert::ingest::ConvertOptions;
use hq_export::doc::{Block, Document, Inline, ListItem};
use hq_export::{ExportOptions, Format, RenderOptions};

#[derive(Debug, thiserror::Error)]
pub enum DocsError {
    #[error("the file could not be read as a document ({0})")]
    Unreadable(String),
    #[error("the record could not be rendered: {0}")]
    Render(String),
}

/// What the note's frontmatter asks of the Word file (`suite_toc`, `suite_header`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocxOptions {
    /// A table of contents after the title.
    pub toc: bool,
    /// Plain text for the page header.
    pub header: String,
}

impl DocxOptions {
    /// Reads the two keys from a raw frontmatter block. A bad value counts as unset.
    pub fn from_frontmatter(raw: &str) -> Self {
        let (meta, _) = hq_suite_md::SuiteMeta::read(raw);
        DocxOptions {
            toc: meta.toc.unwrap_or(false),
            header: meta.header.unwrap_or_default(),
        }
    }
}

/// A record as a Word file. `asset_root` is the folder pictures are read from: the note's own
/// folder for a note whose pictures are linked by relative paths, which also stops a link from
/// reaching any picture outside it.
pub fn render_docx(
    title: &str,
    markdown: &str,
    asset_root: Option<&std::path::Path>,
) -> Result<Vec<u8>, DocsError> {
    render_docx_with(title, markdown, asset_root, &DocxOptions::default())
}

/// [`render_docx`] with the note's own Word options. Review comments never reach the file: the
/// `comment` blocks are cut from the record before it is rendered.
pub fn render_docx_with(
    title: &str,
    markdown: &str,
    asset_root: Option<&std::path::Path>,
    options: &DocxOptions,
) -> Result<Vec<u8>, DocsError> {
    let markdown = hq_suite_md::strip_comments(markdown);
    let doc = Document::from_markdown(title, &markdown);
    let opts = ExportOptions {
        render: RenderOptions {
            asset_root: asset_root.map(std::path::Path::to_path_buf),
            theme: hq_export::Theme {
                toc: options.toc,
                header: options.header.clone(),
                ..hq_export::Theme::default()
            },
            ..RenderOptions::default()
        },
        ..ExportOptions::default()
    };
    hq_export::export(&doc, Format::Docx, &opts)
        .map(|o| o.bytes)
        .map_err(|e| DocsError::Render(e.to_string()))
}

/// `markdown` with every relative picture link written from the vault's root instead of from the
/// note's folder (`note_dir` is the folder, vault-relative, `""` at the top). The exporter reads
/// pictures from below one root, and a link that climbs out of the note's folder (`../../_media/..`)
/// would otherwise be refused as leaving it. A link that leaves the vault, or is not relative, is
/// left as it was and so is never embedded.
pub fn rebase_pictures(markdown: &str, note_dir: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(at) = rest.find("![") {
        let after = &rest[at + 2..];
        let Some(alt_end) = after.find("](") else {
            break;
        };
        let (alt, tail) = (&after[..alt_end], &after[alt_end + 2..]);
        let Some(url_end) = tail.find(')') else {
            break;
        };
        let url = &tail[..url_end];
        out.push_str(&rest[..at]);
        match from_root(note_dir, url) {
            Some(rooted) if !alt.contains('\n') => {
                out.push_str(&format!("![{alt}]({rooted})"));
            }
            _ => out.push_str(&format!("![{alt}]({url})")),
        }
        rest = &tail[url_end + 1..];
    }
    out.push_str(rest);
    out
}

fn from_root(note_dir: &str, url: &str) -> Option<String> {
    if url.is_empty() || url.contains("://") || url.starts_with(['/', '#']) || url.contains('\\') {
        return None;
    }
    let mut parts: Vec<&str> = note_dir.split('/').filter(|p| !p.is_empty()).collect();
    for seg in url.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// What the regenerate check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Regenerated {
    /// The record read from the original and the record read back from the render say the same.
    pub equal: bool,
    /// Where they differ, one short line each, at most [`MAX_DIFFERENCES`].
    pub differences: Vec<String>,
    /// Blocks in the first record, so a result of "equal" can be told from "equal and empty".
    pub blocks: usize,
}

const MAX_DIFFERENCES: usize = 8;

/// Read `docx` into a record A, render A as a Word file, read that into B, compare A and B.
pub async fn regenerate_check(docx: &[u8]) -> Result<Regenerated, DocsError> {
    let opts = ConvertOptions {
        ocr: hq_convert::ingest::OcrMode::Off,
        external_office: false,
        ..ConvertOptions::default()
    };
    let read = |bytes: Vec<u8>| {
        let opts = opts.clone();
        async move {
            hq_suite_ingest::read_document(bytes, "docx", &opts)
                .await
                .map_err(|f| DocsError::Unreadable(format!("{:?}", f.kind)))
        }
    };
    let a = read(docx.to_vec()).await?;
    if a.projection.kind.label() != "docx" {
        return Err(DocsError::Unreadable("not a Word document".into()));
    }
    let rendered = render_docx(a.title.as_deref().unwrap_or(""), &a.markdown, None)?;
    let b = read(rendered).await?;
    let (fa, fb) = (fingerprint(&a.markdown), fingerprint(&b.markdown));
    Ok(compare(fa, fb))
}

/// The parts of a record the check compares, with spelling that does not change meaning removed:
/// a picture counts for the picture, not for its address.
fn fingerprint(markdown: &str) -> Vec<Block> {
    Document::from_markdown("", markdown)
        .blocks
        .into_iter()
        .map(normalise_block)
        .filter(|b| !is_empty_paragraph(b))
        .collect()
}

fn is_empty_paragraph(b: &Block) -> bool {
    matches!(b, Block::Paragraph(i) if i.is_empty())
}

fn normalise_block(b: Block) -> Block {
    match b {
        Block::Heading { level, content } => Block::Heading {
            level,
            content: normalise_inlines(content),
        },
        Block::Paragraph(i) => Block::Paragraph(normalise_inlines(i)),
        Block::Quote(bs) => Block::Quote(bs.into_iter().map(normalise_block).collect()),
        Block::List { ordered, items, .. } => Block::List {
            ordered,
            start: 1,
            items: items
                .into_iter()
                .map(|it| ListItem {
                    checked: it.checked,
                    blocks: it.blocks.into_iter().map(normalise_block).collect(),
                })
                .collect(),
        },
        Block::Table(mut t) => {
            t.header = t.header.into_iter().map(normalise_inlines).collect();
            t.rows = t
                .rows
                .into_iter()
                .map(|r| r.into_iter().map(normalise_inlines).collect())
                .collect();
            Block::Table(t)
        }
        other => other,
    }
}

fn normalise_inlines(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(inlines.len());
    for i in inlines {
        let i = match i {
            Inline::Image { alt, .. } => Inline::Image {
                alt,
                src: String::new(),
            },
            Inline::SoftBreak => Inline::Text(" ".into()),
            Inline::Emph(c) => Inline::Emph(normalise_inlines(c)),
            Inline::Strong(c) => Inline::Strong(normalise_inlines(c)),
            Inline::Strike(c) => Inline::Strike(normalise_inlines(c)),
            Inline::Link { url, content } => Inline::Link {
                url,
                content: normalise_inlines(content),
            },
            other => other,
        };
        // Runs of text read the same however the file split them.
        match (out.last_mut(), i) {
            (Some(Inline::Text(prev)), Inline::Text(t)) => prev.push_str(&t),
            (_, i) => out.push(i),
        }
    }
    for i in &mut out {
        if let Inline::Text(t) = i {
            *t = t.split_whitespace().collect::<Vec<_>>().join(" ");
        }
    }
    out.retain(|i| !matches!(i, Inline::Text(t) if t.is_empty()));
    out
}

fn compare(a: Vec<Block>, b: Vec<Block>) -> Regenerated {
    let mut differences = Vec::new();
    if a.len() != b.len() {
        differences.push(format!("{} blocks became {}", a.len(), b.len()));
    }
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        if x != y && differences.len() < MAX_DIFFERENCES {
            differences.push(format!(
                "block {} differs: {} became {}",
                i + 1,
                describe(x),
                describe(y)
            ));
        }
    }
    Regenerated {
        equal: differences.is_empty(),
        blocks: a.len(),
        differences,
    }
}

fn describe(b: &Block) -> String {
    let text = format!("{b:?}");
    let short: String = text.chars().take(90).collect();
    if text.chars().count() > 90 {
        format!("{short}...")
    } else {
        short
    }
}

#[cfg(test)]
mod tests;
