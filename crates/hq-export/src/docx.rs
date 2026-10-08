//! A native Word document from the shared model.
//!
//! No template and no external tool. Headings use real Word heading styles (so
//! the navigation pane and a table of contents work), lists use real numbering,
//! and code, quotes and callouts are single-cell shaded tables, which every
//! Word-compatible reader draws the same way.

use std::io::Cursor;
use std::path::Path;

use docx_rs as d;

use crate::assets::AssetLoader;
use crate::doc::{Align, Block, Document, Inline, ListItem, Table};
use crate::error::ExportError;
use crate::theme::Theme;
use crate::util::{callout_color, capitalise};

/// Largest decoded image we will embed, in pixels.
const MAX_IMAGE_PIXELS: u64 = 40_000_000;
/// Widest an image is drawn, in pixels at 96 dpi (about 6.25 inches).
const MAX_IMAGE_WIDTH_PX: f64 = 600.0;
const EMU_PER_PX: u32 = 9525;

const MONO: &str = "Consolas";

enum Child {
    // Both boxed: docx-rs paragraphs and tables are several hundred bytes
    // each, and clippy rightly objects to a Vec of such large variants.
    P(Box<d::Paragraph>),
    T(Box<d::Table>),
}

impl Child {
    fn p(paragraph: d::Paragraph) -> Self {
        Child::P(Box::new(paragraph))
    }

    fn t(table: d::Table) -> Self {
        Child::T(Box::new(table))
    }
}

#[derive(Clone, Copy, Default)]
struct Ctx {
    /// Left indent in twentieths of a point.
    indent: i32,
}

#[derive(Clone, Default)]
struct Fmt {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    underline: bool,
    color: Option<String>,
    size: Option<usize>,
}

enum Piece {
    Run(Box<d::Run>),
    Link(d::Hyperlink),
}

impl Piece {
    fn run(run: d::Run) -> Self {
        Piece::Run(Box::new(run))
    }
}

struct Writer {
    accent: String,
    next_numbering: usize,
    abstracts: Vec<d::AbstractNumbering>,
    numberings: Vec<d::Numbering>,
    assets: AssetLoader,
}

pub fn to_docx(
    doc: &Document,
    theme: &Theme,
    asset_root: Option<&Path>,
) -> Result<Vec<u8>, ExportError> {
    let accent = theme.accent.trim_start_matches('#').to_owned();
    let accent = if matches!(accent.len(), 6 | 8) {
        accent[..6].to_owned()
    } else if accent.len() == 3 {
        accent.chars().flat_map(|c| [c, c]).collect()
    } else {
        "1F4E79".to_owned()
    };
    let mut w = Writer {
        accent: accent.clone(),
        // docx-rs always writes a default numbering definition with id 1; reusing
        // that id would give the file two definitions with the same id.
        next_numbering: 2,
        abstracts: Vec::new(),
        numberings: Vec::new(),
        assets: AssetLoader::new(asset_root),
    };

    let mut children = vec![Child::p(
        d::Paragraph::new()
            .style("Title")
            .add_run(d::Run::new().add_text(&doc.title)),
    )];
    children.extend(w.blocks(&doc.blocks, Ctx::default()));

    let font = theme.font.clone().unwrap_or_else(|| "Calibri".to_owned());
    let mut docx = d::Docx::new()
        .default_fonts(
            d::RunFonts::new()
                .ascii(&font)
                .hi_ansi(&font)
                .east_asia(&font)
                .cs(&font),
        )
        .default_size(22)
        .page_size(11906, 16838)
        .page_margin(
            d::PageMargin::new()
                .top(1247)
                .bottom(1361)
                .left(1134)
                .right(1134),
        )
        .add_style(
            d::Style::new("Title", d::StyleType::Paragraph)
                .name("Title")
                .size(48)
                .bold()
                .color(&accent),
        );
    for (n, size) in [40usize, 32, 28, 26, 24, 22].into_iter().enumerate() {
        let n = n + 1;
        docx = docx.add_style(
            d::Style::new(format!("Heading{n}"), d::StyleType::Paragraph)
                .name(format!("heading {n}"))
                .size(size)
                .bold()
                .color(&accent),
        );
    }

    let mut footer = d::Paragraph::new().align(d::AlignmentType::Right);
    if !theme.footer.is_empty() {
        footer = footer.add_run(
            d::Run::new()
                .add_text(format!("{}   ", theme.footer))
                .size(16)
                .color("777777"),
        );
    }
    footer = footer.add_page_num(d::PageNum::new());
    docx = docx.footer(d::Footer::new().add_paragraph(footer));

    for child in children {
        docx = match child {
            Child::P(p) => docx.add_paragraph(*p),
            Child::T(t) => docx.add_table(*t),
        };
    }
    for a in w.abstracts {
        docx = docx.add_abstract_numbering(a);
    }
    for n in w.numberings {
        docx = docx.add_numbering(n);
    }

    let mut out = Cursor::new(Vec::new());
    docx.build()
        .pack(&mut out)
        .map_err(|e| ExportError::Render(e.to_string()))?;
    Ok(out.into_inner())
}

impl Writer {
    fn blocks(&mut self, blocks: &[Block], ctx: Ctx) -> Vec<Child> {
        let mut out = Vec::new();
        for block in blocks {
            self.block(block, ctx, &mut out);
        }
        out
    }

    fn block(&mut self, block: &Block, ctx: Ctx, out: &mut Vec<Child>) {
        match block {
            Block::Heading { level, content } => {
                let mut p = d::Paragraph::new()
                    .style(&format!("Heading{}", (*level).clamp(1, 6)))
                    .keep_next(true);
                p = self.fill(p, content, &Fmt::default());
                out.push(Child::p(p));
            }
            Block::Paragraph(content) => {
                let mut p = indented(d::Paragraph::new(), ctx.indent);
                p = self.fill(p, content, &Fmt::default());
                out.push(Child::p(p));
            }
            Block::Quote(body) => {
                let inner = self.blocks(body, Ctx::default());
                out.push(Child::t(boxed(&self.accent.clone(), "F7F7F7", None, inner)));
                out.push(Child::p(d::Paragraph::new()));
            }
            Block::Callout { kind, title, body } => {
                let color = callout_color(kind).trim_start_matches('#').to_owned();
                let label = title.clone().unwrap_or_else(|| capitalise(kind));
                let inner = self.blocks(body, Ctx::default());
                out.push(Child::t(boxed(
                    &color,
                    &tint(&color),
                    Some((label, color.clone())),
                    inner,
                )));
                out.push(Child::p(d::Paragraph::new()));
            }
            Block::List {
                ordered,
                start,
                items,
            } => self.list(*ordered, *start, items, ctx, out),
            Block::Code { text, .. } => {
                let lines: Vec<d::Paragraph> = text
                    .split('\n')
                    .map(|line| {
                        d::Paragraph::new().add_run(
                            d::Run::new()
                                .add_text(line)
                                .fonts(mono_fonts())
                                .size(18),
                        )
                    })
                    .collect();
                let mut cell = d::TableCell::new().shading(d::Shading::new().fill("F5F5F5"));
                for l in lines {
                    cell = cell.add_paragraph(l);
                }
                out.push(Child::t(
                    d::Table::new(vec![d::TableRow::new(vec![cell])])
                        .width(5000, d::WidthType::Pct),
                ));
                out.push(Child::p(d::Paragraph::new()));
            }
            Block::Table(t) => {
                if let Some(table) = self.table(t) {
                    out.push(Child::t(table));
                    out.push(Child::p(d::Paragraph::new()));
                }
            }
            Block::Rule => out.push(Child::p(
                d::Paragraph::new()
                    .align(d::AlignmentType::Center)
                    .add_run(d::Run::new().add_text("\u{2500}".repeat(40)).color("BBBBBB")),
            )),
        }
    }

    fn list(
        &mut self,
        ordered: bool,
        start: u64,
        items: &[ListItem],
        ctx: Ctx,
        out: &mut Vec<Child>,
    ) {
        let id = self.next_numbering;
        self.next_numbering += 1;
        let left = ctx.indent + 720;
        let (format, text) = if ordered {
            ("decimal", "%1.")
        } else {
            ("bullet", "\u{2022}")
        };
        self.abstracts.push(
            d::AbstractNumbering::new(id).add_level(
                d::Level::new(
                    0,
                    d::Start::new(start as usize),
                    d::NumberFormat::new(format),
                    d::LevelText::new(text),
                    d::LevelJc::new("left"),
                )
                .indent(Some(left), Some(d::SpecialIndentType::Hanging(360)), None, None),
            ),
        );
        self.numberings.push(d::Numbering::new(id, id));

        for item in items {
            let mut rest: &[Block] = &item.blocks;
            let mut lead = match rest.first() {
                Some(Block::Paragraph(_)) => {
                    let Some(Block::Paragraph(content)) = rest.first() else {
                        unreachable!("matched above")
                    };
                    rest = &rest[1..];
                    let mut p = d::Paragraph::new();
                    if item.checked.is_none() {
                        p = p.numbering(d::NumberingId::new(id), d::IndentLevel::new(0));
                    } else {
                        p = indented(p, left);
                    }
                    if let Some(done) = item.checked {
                        let glyph = if done { "\u{2611} " } else { "\u{2610} " };
                        p = p.add_run(d::Run::new().add_text(glyph));
                    }
                    self.fill(p, content, &Fmt::default())
                }
                // An item that opens with something other than text still
                // needs its marker, so give it an empty first line.
                _ => d::Paragraph::new().numbering(d::NumberingId::new(id), d::IndentLevel::new(0)),
            };
            lead = lead.keep_lines(false);
            out.push(Child::p(lead));
            for block in rest {
                match block {
                    Block::List {
                        ordered,
                        start,
                        items,
                    } => self.list(
                        *ordered,
                        *start,
                        items,
                        Ctx {
                            indent: ctx.indent + 360,
                        },
                        out,
                    ),
                    other => self.block(other, Ctx { indent: left }, out),
                }
            }
        }
    }

    fn table(&mut self, t: &Table) -> Option<d::Table> {
        let n = t.columns();
        if n == 0 {
            return None;
        }
        let align = |i: usize| match t.aligns.get(i) {
            Some(Align::Right) => d::AlignmentType::Right,
            Some(Align::Center) => d::AlignmentType::Center,
            _ => d::AlignmentType::Left,
        };
        let mut rows = Vec::new();
        let header: Vec<d::TableCell> = (0..n)
            .map(|i| {
                let fmt = Fmt {
                    bold: true,
                    ..Fmt::default()
                };
                let content = t.header.get(i).map(Vec::as_slice).unwrap_or(&[]);
                let p = self.fill(d::Paragraph::new().align(align(i)), content, &fmt);
                d::TableCell::new()
                    .shading(d::Shading::new().fill("EEF0F2"))
                    .add_paragraph(p)
            })
            .collect();
        rows.push(d::TableRow::new(header).cant_split());
        for row in &t.rows {
            let cells: Vec<d::TableCell> = (0..n)
                .map(|i| {
                    let content = row.get(i).map(Vec::as_slice).unwrap_or(&[]);
                    let p = self.fill(d::Paragraph::new().align(align(i)), content, &Fmt::default());
                    d::TableCell::new().add_paragraph(p)
                })
                .collect();
            rows.push(d::TableRow::new(cells).cant_split());
        }
        Some(d::Table::new(rows).width(5000, d::WidthType::Pct))
    }

    /// Append the inline content to `paragraph`.
    fn fill(&mut self, mut paragraph: d::Paragraph, inlines: &[Inline], fmt: &Fmt) -> d::Paragraph {
        let mut pieces = Vec::new();
        self.pieces(inlines, fmt, &mut pieces);
        for piece in pieces {
            paragraph = match piece {
                Piece::Run(r) => paragraph.add_run(*r),
                Piece::Link(h) => paragraph.add_hyperlink(h),
            };
        }
        paragraph
    }

    fn pieces(&mut self, inlines: &[Inline], fmt: &Fmt, out: &mut Vec<Piece>) {
        for inline in inlines {
            match inline {
                Inline::Text(t) => out.push(Piece::run(run(t, fmt))),
                Inline::Emph(c) => self.pieces(
                    c,
                    &Fmt {
                        italic: true,
                        ..fmt.clone()
                    },
                    out,
                ),
                Inline::Strong(c) => self.pieces(
                    c,
                    &Fmt {
                        bold: true,
                        ..fmt.clone()
                    },
                    out,
                ),
                Inline::Strike(c) => self.pieces(
                    c,
                    &Fmt {
                        strike: true,
                        ..fmt.clone()
                    },
                    out,
                ),
                Inline::Code(c) => out.push(Piece::run(run(
                    c,
                    &Fmt {
                        code: true,
                        ..fmt.clone()
                    },
                ))),
                Inline::Link { url, content } => {
                    let linked = Fmt {
                        underline: true,
                        color: Some(self.accent.clone()),
                        ..fmt.clone()
                    };
                    if is_external(url) {
                        let mut inner = Vec::new();
                        self.pieces(content, &linked, &mut inner);
                        let mut link = d::Hyperlink::new(url.trim(), d::HyperlinkType::External);
                        for piece in inner {
                            if let Piece::Run(r) = piece {
                                link = link.add_run(*r);
                            }
                        }
                        out.push(Piece::Link(link));
                    } else {
                        // Relative paths and fragments mean nothing outside the vault.
                        self.pieces(content, fmt, out);
                    }
                }
                Inline::Image { alt, src } => match self.picture(src) {
                    Some(pic) => out.push(Piece::run(d::Run::new().add_image(pic))),
                    None => {
                        let label = if alt.is_empty() { "image" } else { alt };
                        out.push(Piece::run(run(
                            &format!("[{label} unavailable]"),
                            &Fmt {
                                italic: true,
                                ..fmt.clone()
                            },
                        )));
                    }
                },
                Inline::SoftBreak => out.push(Piece::run(run(" ", fmt))),
                Inline::HardBreak => out.push(Piece::run(
                    d::Run::new().add_break(d::BreakType::TextWrapping),
                )),
            }
        }
    }

    /// Decode first, so the Word library (which panics on bad data) only ever
    /// sees an image known to be good.
    fn picture(&mut self, src: &str) -> Option<d::Pic> {
        let asset = self.assets.load(src)?;
        if asset.ext == "svg" {
            return None;
        }
        let (w, h) = image::ImageReader::new(Cursor::new(&asset.bytes))
            .with_guessed_format()
            .ok()?
            .into_dimensions()
            .ok()?;
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS {
            return None;
        }
        let decoded = image::load_from_memory(&asset.bytes).ok()?;
        let mut png = Cursor::new(Vec::new());
        decoded.write_to(&mut png, image::ImageFormat::Png).ok()?;
        let scale = (MAX_IMAGE_WIDTH_PX / f64::from(w)).min(1.0);
        let draw_w = ((f64::from(w) * scale) as u32).max(1);
        let draw_h = ((f64::from(h) * scale) as u32).max(1);
        Some(
            d::Pic::new_with_dimensions(png.into_inner(), w, h)
                .size(draw_w * EMU_PER_PX, draw_h * EMU_PER_PX),
        )
    }
}

fn run(text: &str, fmt: &Fmt) -> d::Run {
    let mut r = d::Run::new().add_text(text);
    if fmt.bold {
        r = r.bold();
    }
    if fmt.italic {
        r = r.italic();
    }
    if fmt.strike {
        r = r.strike();
    }
    if fmt.underline {
        r = r.underline("single");
    }
    if fmt.code {
        r = r
            .fonts(mono_fonts())
            .size(19)
            .shading(d::Shading::new().fill("F2F3F4"));
    }
    if let Some(color) = &fmt.color {
        r = r.color(color);
    }
    if let Some(size) = fmt.size {
        r = r.size(size);
    }
    r
}

fn mono_fonts() -> d::RunFonts {
    d::RunFonts::new().ascii(MONO).hi_ansi(MONO).cs(MONO)
}

fn indented(p: d::Paragraph, left: i32) -> d::Paragraph {
    if left > 0 {
        p.indent(Some(left), None, None, None)
    } else {
        p
    }
}

/// A single-cell table with a coloured left edge: how quotes and callouts are
/// drawn. `label` is `(text, colour)`.
fn boxed(edge: &str, fill: &str, label: Option<(String, String)>, children: Vec<Child>) -> d::Table {
    let mut cell = d::TableCell::new()
        .shading(d::Shading::new().fill(fill))
        .set_borders(
            d::TableCellBorders::with_empty().set(
                d::TableCellBorder::new(d::TableCellBorderPosition::Left)
                    .size(24)
                    .color(edge),
            ),
        );
    let mut has_paragraph = false;
    if let Some((text, color)) = label {
        cell = cell.add_paragraph(
            d::Paragraph::new().add_run(d::Run::new().add_text(text).bold().color(color)),
        );
        has_paragraph = true;
    }
    let mut last_was_table = false;
    for child in children {
        match child {
            Child::P(p) => {
                cell = cell.add_paragraph(*p);
                has_paragraph = true;
                last_was_table = false;
            }
            Child::T(t) => {
                cell = cell.add_table(*t);
                last_was_table = true;
            }
        }
    }
    // A cell must end in a paragraph.
    if !has_paragraph || last_was_table {
        cell = cell.add_paragraph(d::Paragraph::new());
    }
    d::Table::new(vec![d::TableRow::new(vec![cell])]).width(5000, d::WidthType::Pct)
}

/// A light version of `hex` for a callout background.
fn tint(hex: &str) -> String {
    let channel = |i: usize| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("ff"), 16).unwrap_or(255);
    let mix = |c: u8| ((u32::from(c) * 12 + 255 * 88) / 100) as u8;
    format!("{:02X}{:02X}{:02X}", mix(channel(0)), mix(channel(2)), mix(channel(4)))
}

fn is_external(url: &str) -> bool {
    let url = url.trim();
    let scheme = url.split_once(':').map(|(s, _)| s.to_ascii_lowercase());
    matches!(scheme.as_deref(), Some("http" | "https" | "mailto" | "tel"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn docx(md: &str) -> Vec<u8> {
        to_docx(&Document::from_markdown("Title", md), &Theme::default(), None).unwrap()
    }

    fn part(bytes: &[u8], name: &str) -> String {
        let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut s = String::new();
        z.by_name(name)
            .unwrap_or_else(|_| panic!("missing part {name}"))
            .read_to_string(&mut s)
            .unwrap();
        s
    }

    fn has_part(bytes: &[u8], name: &str) -> bool {
        zip::ZipArchive::new(Cursor::new(bytes)).unwrap().by_name(name).is_ok()
    }

    #[test]
    fn the_package_has_the_parts_word_needs() {
        let b = docx("hello\n");
        for name in ["[Content_Types].xml", "word/document.xml", "word/styles.xml", "_rels/.rels"] {
            assert!(has_part(&b, name), "{name}");
        }
    }

    #[test]
    fn headings_use_word_heading_styles_and_text_is_present() {
        let b = docx("## Sites\n\nSome **bold** words.\n");
        let xml = part(&b, "word/document.xml");
        assert!(xml.contains("w:val=\"Heading2\""), "{xml}");
        assert!(xml.contains("Sites") && xml.contains("bold") && xml.contains("words."));
        assert!(part(&b, "word/styles.xml").contains("w:styleId=\"Heading2\""));
    }

    #[test]
    fn markup_in_notes_is_escaped_not_interpreted() {
        let b = docx("a <w:p>x</w:p> & \"q\" `<w:t>`\n");
        let xml = part(&b, "word/document.xml");
        assert!(!xml.contains("<w:p>x"), "raw html is dropped, text is escaped: {xml}");
        assert!(xml.contains("&amp;") || xml.contains('&'));
    }

    #[test]
    fn tables_have_a_shaded_bold_header_and_all_cells() {
        let xml = part(&docx("| a | b |\n|:--|--:|\n| 1 | 2 |\n"), "word/document.xml");
        assert!(xml.contains("<w:tbl>"));
        assert!(xml.contains("EEF0F2"), "header shading");
        assert!(xml.contains("w:jc w:val=\"right\""), "right alignment");
        for cell in ["a", "b", "1", "2"] {
            assert!(xml.contains(&format!(">{cell}<")), "{cell}: {xml}");
        }
    }

    #[test]
    fn each_list_gets_its_own_numbering_and_ordered_lists_keep_their_start() {
        let b = docx("3. c\n4. d\n\n- x\n  - y\n");
        let numbering = part(&b, "word/numbering.xml");
        assert!(numbering.contains("w:start w:val=\"3\""), "{numbering}");
        assert!(numbering.contains("w:numFmt w:val=\"bullet\""));

        // Word refuses or repairs a file whose numbering ids collide, so every
        // definition and instance must carry a distinct id.
        let ids = |tag: &str| -> Vec<String> {
            numbering
                .split(tag)
                .skip(1)
                .filter_map(|rest| rest.split('"').next().map(str::to_owned))
                .collect()
        };
        let abstracts = ids("<w:abstractNum w:abstractNumId=\"");
        let nums = ids("<w:num w:numId=\"");
        let unique = |v: &[String]| v.iter().collect::<std::collections::HashSet<_>>().len() == v.len();
        assert!(unique(&abstracts), "duplicate abstractNumId in {abstracts:?}");
        assert!(unique(&nums), "duplicate numId in {nums:?}");
        // docx-rs's default definition plus our three lists
        assert_eq!(abstracts.len(), 4, "{abstracts:?}");
    }

    #[test]
    fn external_links_become_hyperlinks_and_others_become_text() {
        let b = docx("[ok](https://example.com) [no](javascript:alert(1)) [rel](notes/x.md)\n");
        let rels = part(&b, "word/_rels/document.xml.rels");
        assert!(rels.contains("https://example.com"), "{rels}");
        assert!(!rels.contains("javascript"), "{rels}");
        assert!(!rels.contains("notes/x.md"), "{rels}");
        let xml = part(&b, "word/document.xml");
        assert!(xml.contains(">no<") && xml.contains(">rel<"));
    }

    #[test]
    fn callouts_quotes_and_code_render_as_shaded_cells() {
        let xml = part(
            &docx("> [!warning] Careful\n> body\n\n> a quote\n\n```rust\nlet x = 1;\n```\n"),
            "word/document.xml",
        );
        assert_eq!(xml.matches("<w:tbl>").count(), 3, "{xml}");
        assert!(xml.contains("Careful") && xml.contains("let x = 1;"));
        assert!(xml.contains("F5F5F5"), "code background");
    }

    #[test]
    fn task_items_show_checkbox_glyphs() {
        let xml = part(&docx("- [x] done\n- [ ] todo\n"), "word/document.xml");
        assert!(xml.contains('\u{2611}') && xml.contains('\u{2610}'));
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(w, h)
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }

    fn docx_with_root(md: &str, root: &Path) -> Vec<u8> {
        to_docx(&Document::from_markdown("T", md), &Theme::default(), Some(root)).unwrap()
    }

    #[test]
    fn images_inside_the_root_are_embedded_and_scaled_down() {
        let root = tempfile::tempdir().unwrap();
        let img = root.path().join("big.png");
        std::fs::write(&img, png(1200, 600)).unwrap();
        let b = docx_with_root(&format!("![big]({})\n", img.display()), root.path());
        assert!(zip::ZipArchive::new(Cursor::new(&b[..])).unwrap().file_names().any(|n| n.starts_with("word/media/")));
        let xml = part(&b, "word/document.xml");
        // 600 px wide at 9525 EMU per pixel
        assert!(xml.contains(&format!("cx=\"{}\"", 600 * EMU_PER_PX)), "{xml}");
        assert!(xml.contains(&format!("cy=\"{}\"", 300 * EMU_PER_PX)), "{xml}");
    }

    #[test]
    fn a_corrupt_or_unsupported_image_never_panics_or_fails_the_export() {
        let root = tempfile::tempdir().unwrap();
        let bad = root.path().join("bad.png");
        std::fs::write(&bad, b"not an image").unwrap();
        let truncated = root.path().join("cut.png");
        std::fs::write(&truncated, &png(50, 50)[..40]).unwrap();
        let svg = root.path().join("v.svg");
        std::fs::write(&svg, b"<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
        let md = format!("![a]({}) ![b]({}) ![c]({})\n", bad.display(), truncated.display(), svg.display());
        let b = docx_with_root(&md, root.path());
        let xml = part(&b, "word/document.xml");
        assert_eq!(xml.matches("unavailable]").count(), 3, "{xml}");
    }

    #[test]
    fn images_outside_the_root_and_remote_ones_are_not_embedded() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("s.png");
        std::fs::write(&secret, png(10, 10)).unwrap();
        let md = format!("![x]({}) ![y](https://example.com/a.png)\n", secret.display());
        let b = docx_with_root(&md, root.path());
        assert!(!zip::ZipArchive::new(Cursor::new(&b[..])).unwrap().file_names().any(|n| n.starts_with("word/media/")));
    }

    #[test]
    fn tint_lightens_towards_white() {
        assert_eq!(tint("000000"), "E0E0E0");
        assert_eq!(tint("FFFFFF"), "FFFFFF");
        assert_eq!(tint("zz"), "FFFFFF", "malformed input degrades to white");
    }

    #[test]
    fn theme_accent_reaches_the_styles() {
        let theme = Theme {
            accent: "#c62828".into(),
            font: Some("Poppins".into()),
            footer: "Acme".into(),
        };
        let b = to_docx(&Document::from_markdown("T", "## h\n"), &theme, None).unwrap();
        let styles = part(&b, "word/styles.xml");
        assert!(styles.contains("C62828") || styles.contains("c62828"), "{styles}");
        assert!(styles.contains("Poppins"), "default font");
        assert!(has_part(&b, "word/footer1.xml"));
    }
}
