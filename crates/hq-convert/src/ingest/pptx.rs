//! PowerPoint presentations (`.pptx`) to Markdown: one section per slide, in
//! slide order, with the title, bullets, tables and speaker notes.

use std::collections::HashMap;

use roxmltree::Node;

use super::ir::{Block, Inline, ListItem, Parsed, Run, Table, clean_alt, is_mono_font};
use super::package::{Package, attr, child, core_title, is, parse_xml, read_rels, resolve_part};
use super::report::{LossKind, Losses};
use crate::types::ConvertError;

const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

pub fn read(pkg: &mut Package<'_>) -> Result<Parsed, ConvertError> {
    let max = pkg.nodes_limit();
    let slides = slide_paths(pkg, max)?;
    if slides.is_empty() {
        return Err(ConvertError::InboundFailed(
            "no slides found: not a PowerPoint presentation".into(),
        ));
    }
    let mut losses = Losses::default();
    let mut blocks: Vec<Block> = Vec::new();
    let mut any_content = false;
    for (i, path) in slides.iter().enumerate() {
        let Some(xml) = pkg.read_text(path)? else {
            continue;
        };
        let dir = path.rsplit_once('/').map_or("", |(d, _)| d).to_string();
        let file = path.rsplit('/').next().unwrap_or(path);
        let rels_path = format!("{dir}/_rels/{file}.rels");
        let rels = pkg
            .read_text(&rels_path)?
            .map(|x| read_rels(&x, max))
            .unwrap_or_default();
        let notes_xml = match rels
            .values()
            .find(|(t, ext)| !ext && t.contains("notesSlide"))
        {
            Some((target, _)) => pkg.read_text(&resolve_part(&dir, target))?,
            None => None,
        };
        let doc = parse_xml(&xml, false, max)?;
        let notes = match &notes_xml {
            Some(x) => notes_blocks(x, max, &mut losses),
            None => Vec::new(),
        };
        let hidden = doc.root_element().attribute("show") == Some("0");
        let mut slide = Slide {
            rels: &rels,
            losses: &mut losses,
            title: None,
            body: Vec::new(),
        };
        if let Some(tree) = doc
            .root_element()
            .descendants()
            .find(|n| is(*n, P, "spTree"))
        {
            slide.shapes(tree);
        }
        let heading = match (&slide.title, hidden) {
            (Some(t), false) => format!("Slide {}: {t}", i + 1),
            (Some(t), true) => format!("Slide {}: {t} (hidden)", i + 1),
            (None, false) => format!("Slide {}", i + 1),
            (None, true) => format!("Slide {} (hidden)", i + 1),
        };
        any_content |= slide.title.is_some() || !slide.body.is_empty() || !notes.is_empty();
        blocks.push(Block::Heading(2, vec![Inline::text(heading)]));
        blocks.append(&mut slide.body);
        if !notes.is_empty() {
            blocks.push(Block::Heading(3, vec![Inline::text("Notes")]));
            blocks.extend(notes);
        }
    }
    if !any_content {
        return Err(ConvertError::InboundFailed(
            "no text found on any slide".into(),
        ));
    }
    let title = pkg
        .read_text("docProps/core.xml")?
        .and_then(|x| core_title(&x, max));
    Ok(Parsed {
        blocks,
        title,
        losses,
    })
}

/// Slide parts in presentation order.
fn slide_paths(pkg: &mut Package<'_>, max: u32) -> Result<Vec<String>, ConvertError> {
    let mut ordered: Vec<String> = Vec::new();
    if let (Some(pres), Some(rels)) = (
        pkg.read_text("ppt/presentation.xml")?,
        pkg.read_text("ppt/_rels/presentation.xml.rels")?,
    ) {
        let rels = read_rels(&rels, max);
        if let Ok(doc) = parse_xml(&pres, false, max) {
            for id in doc.descendants().filter(|n| is(*n, P, "sldId")) {
                if let Some((target, _)) = attr(id, R, "id").and_then(|r| rels.get(r)) {
                    let path = resolve_part("ppt", target);
                    if pkg.has(&path) {
                        ordered.push(path);
                    }
                }
            }
        }
    }
    if ordered.is_empty() {
        // No usable slide list: fall back to the numbering in the file names.
        let mut found: Vec<(u32, String)> = pkg
            .names()
            .iter()
            .filter_map(|n| {
                let num = n
                    .strip_prefix("ppt/slides/slide")?
                    .strip_suffix(".xml")?
                    .parse::<u32>()
                    .ok()?;
                Some((num, n.clone()))
            })
            .collect();
        found.sort();
        ordered = found.into_iter().map(|(_, n)| n).collect();
    }
    Ok(ordered)
}

fn notes_blocks(xml: &str, max: u32, losses: &mut Losses) -> Vec<Block> {
    let Ok(doc) = parse_xml(xml, false, max) else {
        return Vec::new();
    };
    let rels = HashMap::new();
    let mut slide = Slide {
        rels: &rels,
        losses,
        title: None,
        body: Vec::new(),
    };
    if let Some(tree) = doc
        .root_element()
        .descendants()
        .find(|n| is(*n, P, "spTree"))
    {
        for sp in tree.children().filter(|n| is(*n, P, "sp")) {
            let kind = placeholder(sp);
            // The notes page also carries the slide image and a page number.
            if matches!(kind.as_ref().map(|k| k.0.as_str()), Some("body")) {
                slide.text_shape(sp, false);
            }
        }
    }
    slide.body
}

struct Slide<'a> {
    rels: &'a HashMap<String, (String, bool)>,
    losses: &'a mut Losses,
    title: Option<String>,
    body: Vec<Block>,
}

/// The placeholder type and index of a shape, if it is a placeholder.
fn placeholder(sp: Node<'_, '_>) -> Option<(String, Option<String>)> {
    let nv = child(sp, P, "nvSpPr")?;
    let ph = child(child(nv, P, "nvPr")?, P, "ph")?;
    Some((
        ph.attribute("type").unwrap_or("body").to_string(),
        ph.attribute("idx").map(str::to_string),
    ))
}

impl Slide<'_> {
    fn shapes(&mut self, tree: Node<'_, '_>) {
        for n in tree.children().filter(|n| n.is_element()) {
            match (n.tag_name().namespace(), n.tag_name().name()) {
                (Some(P), "sp") => {
                    let ph = placeholder(n);
                    match ph.as_ref().map(|p| p.0.as_str()) {
                        Some("title" | "ctrTitle") => {
                            let text = shape_text(n);
                            if !text.is_empty() && self.title.is_none() {
                                self.title = Some(text);
                            }
                        }
                        // Footers, dates and slide numbers repeat on every slide.
                        Some("sldNum" | "dt" | "ftr" | "hdr" | "sldImg") => {}
                        Some("subTitle") => self.text_shape(n, false),
                        Some(_) => self.text_shape(n, true),
                        None => self.text_shape(n, false),
                    }
                }
                (Some(P), "grpSp") => self.shapes(n),
                (Some(P), "pic") => {
                    let alt = n
                        .descendants()
                        .find(|d| is(*d, P, "cNvPr"))
                        .and_then(|d| d.attribute("descr").or_else(|| d.attribute("title")))
                        .unwrap_or("")
                        .to_string();
                    self.body
                        .push(Block::Para(vec![Inline::Image(clean_alt(&alt))]));
                    self.losses.hit(LossKind::ImagesOmitted);
                }
                (Some(P), "graphicFrame") => self.frame(n),
                (Some(MC), "AlternateContent") => {
                    if let Some(c) = child(n, MC, "Choice") {
                        self.shapes(c);
                    }
                }
                _ => {}
            }
        }
    }

    fn frame(&mut self, f: Node<'_, '_>) {
        if let Some(tbl) = f.descendants().find(|d| is(*d, A, "tbl")) {
            let mut table = Table::default();
            for tr in tbl.children().filter(|c| is(*c, A, "tr")) {
                let mut row: Vec<Vec<Inline>> = Vec::new();
                for tc in tr.children().filter(|c| is(*c, A, "tc")) {
                    // Cells covered by a merge hold no text of their own.
                    let mut cell = Vec::new();
                    if let Some(tx) = child(tc, A, "txBody") {
                        for (i, p) in tx.children().filter(|c| is(*c, A, "p")).enumerate() {
                            let inl = self.inlines(p);
                            if inl.is_empty() {
                                continue;
                            }
                            if i > 0 && !cell.is_empty() {
                                cell.push(Inline::Break);
                            }
                            cell.extend(inl);
                        }
                    }
                    row.push(cell);
                }
                if !row.is_empty() {
                    table.rows.push(row);
                }
            }
            if !table.rows.is_empty() {
                self.body.push(Block::Table(table));
            }
        } else if f
            .descendants()
            .any(|d| d.is_element() && matches!(d.tag_name().name(), "chart" | "relIds"))
        {
            self.losses.hit(LossKind::ChartsOmitted);
        } else {
            self.losses.hit(LossKind::EmbeddedObjectsOmitted);
        }
    }

    /// A text shape. `body_default` is true for body placeholders, whose
    /// paragraphs are bullets unless they say otherwise.
    fn text_shape(&mut self, sp: Node<'_, '_>, body_default: bool) {
        let Some(tx) = child(sp, P, "txBody") else {
            return;
        };
        let mut items: Vec<ListItem> = Vec::new();
        let flush = |items: &mut Vec<ListItem>, body: &mut Vec<Block>| {
            if !items.is_empty() {
                body.push(Block::List(std::mem::take(items)));
            }
        };
        for p in tx.children().filter(|c| is(*c, A, "p")) {
            let inl = self.inlines(p);
            let has_text = inl.iter().any(|i| match i {
                Inline::Run(r) => !r.text.trim().is_empty(),
                _ => false,
            });
            if !has_text {
                continue;
            }
            let ppr = child(p, A, "pPr");
            let level = ppr
                .and_then(|x| x.attribute("lvl"))
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);
            let numbered = ppr.and_then(|x| child(x, A, "buAutoNum")).is_some();
            let bulleted = ppr.and_then(|x| child(x, A, "buChar")).is_some();
            let none = ppr.and_then(|x| child(x, A, "buNone")).is_some();
            if numbered || bulleted || (body_default && !none) {
                items.push(ListItem {
                    level,
                    ordered: numbered,
                    content: inl,
                });
            } else {
                flush(&mut items, &mut self.body);
                self.body.push(Block::Para(inl));
            }
        }
        flush(&mut items, &mut self.body);
    }

    fn inlines(&mut self, p: Node<'_, '_>) -> Vec<Inline> {
        let mut out = Vec::new();
        for n in p.children().filter(|c| c.is_element()) {
            match (n.tag_name().namespace() == Some(A), n.tag_name().name()) {
                (true, "r" | "fld") => {
                    let text: String = n
                        .children()
                        .filter(|c| is(*c, A, "t"))
                        .filter_map(|c| c.text())
                        .collect();
                    if text.is_empty() {
                        continue;
                    }
                    let rpr = child(n, A, "rPr");
                    let link = rpr
                        .and_then(|r| child(r, A, "hlinkClick"))
                        .and_then(|h| attr(h, R, "id"))
                        .and_then(|id| self.rels.get(id))
                        .filter(|(_, external)| *external)
                        .map(|(t, _)| t.clone());
                    let mono = rpr
                        .and_then(|r| child(r, A, "latin"))
                        .and_then(|l| l.attribute("typeface"))
                        .is_some_and(is_mono_font);
                    out.push(Inline::Run(Run {
                        text,
                        bold: rpr.and_then(|r| r.attribute("b")) == Some("1"),
                        italic: rpr.and_then(|r| r.attribute("i")) == Some("1"),
                        strike: rpr
                            .and_then(|r| r.attribute("strike"))
                            .is_some_and(|s| s != "noStrike"),
                        code: mono,
                        link,
                    }));
                }
                (true, "br") => out.push(Inline::Break),
                _ => {}
            }
        }
        out
    }
}

/// All the text of a title shape, as one line.
fn shape_text(sp: Node<'_, '_>) -> String {
    let Some(tx) = child(sp, P, "txBody") else {
        return String::new();
    };
    let mut parts: Vec<String> = Vec::new();
    for p in tx.children().filter(|c| is(*c, A, "p")) {
        let t: String = p
            .descendants()
            .filter(|d| is(*d, A, "t"))
            .filter_map(|d| d.text())
            .collect();
        if !t.trim().is_empty() {
            parts.push(t.trim().to_string());
        }
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::package::Limits;
    use crate::ingest::testutil::zip_of;

    const NS: &str = r#"xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

    fn slide(shapes: &str) -> String {
        format!(r#"<p:sld {NS}><p:cSld><p:spTree>{shapes}</p:spTree></p:cSld></p:sld>"#)
    }

    fn title(t: &str) -> String {
        format!(
            r#"<p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>{t}</a:t></a:r></a:p></p:txBody></p:sp>"#
        )
    }

    fn body(paras: &[(&str, u8)]) -> String {
        let ps: String = paras
            .iter()
            .map(|(t, l)| format!(r#"<a:p><a:pPr lvl="{l}"/><a:r><a:t>{t}</a:t></a:r></a:p>"#))
            .collect();
        format!(
            r#"<p:sp><p:nvSpPr><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr><p:txBody>{ps}</p:txBody></p:sp>"#
        )
    }

    fn md(entries: &[(&str, Vec<u8>)]) -> String {
        let bytes = zip_of(entries);
        let mut pkg = Package::open(&bytes, &Limits::default()).unwrap();
        super::super::ir::render(read(&mut pkg).unwrap().blocks)
    }

    #[test]
    fn slides_in_order_with_titles_bullets_and_notes() {
        let pres = format!(
            r#"<p:presentation {NS}><p:sldIdLst><p:sldId id="256" r:id="rId2"/><p:sldId id="257" r:id="rId1"/></p:sldIdLst></p:presentation>"#
        );
        let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Target="slides/slide1.xml"/><Relationship Id="rId2" Target="slides/slide2.xml"/></Relationships>"#;
        let s1 = slide(&format!(
            "{}{}",
            title("Second in file"),
            body(&[("late", 0)])
        ));
        let s2 = slide(&format!(
            "{}{}",
            title("Agenda"),
            body(&[("Moisture", 0), ("Pests", 1)])
        ));
        let s2_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Target="../notesSlides/notesSlide2.xml"/></Relationships>"#;
        let notes = format!(
            r#"<p:notes {NS}><p:cSld><p:spTree><p:sp><p:nvSpPr><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>Allow ten minutes.</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>"#
        );
        let out = md(&[
            ("ppt/presentation.xml", pres.into_bytes()),
            ("ppt/_rels/presentation.xml.rels", rels.as_bytes().to_vec()),
            ("ppt/slides/slide1.xml", s1.into_bytes()),
            ("ppt/slides/slide2.xml", s2.into_bytes()),
            (
                "ppt/slides/_rels/slide2.xml.rels",
                s2_rels.as_bytes().to_vec(),
            ),
            ("ppt/notesSlides/notesSlide2.xml", notes.into_bytes()),
        ]);
        assert_eq!(
            out,
            "## Slide 1: Agenda\n\n- Moisture\n    - Pests\n\n### Notes\n\nAllow ten minutes.\n\n## Slide 2: Second in file\n\n- late\n"
        );
    }

    #[test]
    fn tables_and_pictures() {
        let tbl = r#"<p:graphicFrame><a:graphic><a:graphicData><a:tbl>
          <a:tr><a:tc><a:txBody><a:p><a:r><a:t>Item</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>UGX</a:t></a:r></a:p></a:txBody></a:tc></a:tr>
          <a:tr><a:tc><a:txBody><a:p><a:r><a:t>Bag</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>6,000</a:t></a:r></a:p></a:txBody></a:tc></a:tr>
          </a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#;
        let pic = r#"<p:pic><p:nvPicPr><p:cNvPr id="4" name="Picture 3" descr="Field photo"/></p:nvPicPr></p:pic>"#;
        let out = md(&[(
            "ppt/slides/slide1.xml",
            slide(&format!("{}{tbl}{pic}", title("Costs"))).into_bytes(),
        )]);
        assert_eq!(
            out,
            "## Slide 1: Costs\n\n| Item | UGX |\n| --- | --- |\n| Bag | 6,000 |\n\n*[image: Field photo]*\n"
        );
    }

    #[test]
    fn footers_and_slide_numbers_are_not_content() {
        let footer = r#"<p:sp><p:nvSpPr><p:nvPr><p:ph type="sldNum"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:fld><a:t>3</a:t></a:fld></a:p></p:txBody></p:sp>"#;
        let out = md(&[(
            "ppt/slides/slide1.xml",
            slide(&format!("{}{footer}", title("Only"))).into_bytes(),
        )]);
        assert_eq!(out, "## Slide 1: Only\n");
    }

    #[test]
    fn a_container_without_slides_is_not_a_presentation() {
        let bytes = zip_of(&[("other.xml", b"<a/>".to_vec())]);
        let mut pkg = Package::open(&bytes, &Limits::default()).unwrap();
        assert!(read(&mut pkg).is_err());
    }
}
