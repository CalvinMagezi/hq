//! A deck as a native `.pptx` file: the package parts written by hand, no template and no tool.
//!
//! One slide master, two layouts (title and content, title and two columns), a theme in neutral
//! grays and one part per card. Placeholders carry the titles and bodies so PowerPoint's outline
//! view, accessibility checker and other readers find them. Tables are real tables. Pictures are
//! not placed yet; the deck says how many were left out.

use std::io::{Cursor, Write};

use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use crate::deck::{Card, Deck, Item, Layout, Run};

#[derive(Debug, thiserror::Error)]
pub enum PptxError {
    #[error("the file could not be written: {0}")]
    Write(String),
}

const SLIDE_W: i64 = 12_192_000;
const SLIDE_H: i64 = 6_858_000;
const MARGIN: i64 = 609_600;
const TITLE_H: i64 = 1_143_000;
const BODY_TOP: i64 = 1_752_600;
const BODY_H: i64 = SLIDE_H - BODY_TOP - 457_200;
const GAP: i64 = 304_800;
/// Hundredths of a point.
const TITLE_SIZE: u32 = 3600;
const DECK_TITLE_SIZE: u32 = 4800;
const BODY_SIZE: u32 = 2400;
const NESTED_SIZE: u32 = 2000;
const CODE_SIZE: u32 = 1800;
const TABLE_SIZE: u32 = 1800;
const INDENT: i64 = 342_900;
const ROW_H: i64 = 400_050;
/// About one line of body text, and the space left between text and a table.
const LINE_H: i64 = 457_200;
const TABLE_GAP: i64 = 152_400;

const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // Characters XML 1.0 cannot hold would make the file unreadable.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

fn run_xml(run: &Run, size: u32) -> String {
    let mut attrs = format!("lang=\"en-US\" sz=\"{size}\" dirty=\"0\"");
    if run.bold {
        attrs.push_str(" b=\"1\"");
    }
    if run.italic {
        attrs.push_str(" i=\"1\"");
    }
    let font = if run.code {
        "<a:latin typeface=\"Courier New\"/>"
    } else {
        ""
    };
    format!(
        "<a:r><a:rPr {attrs}>{font}</a:rPr><a:t>{}</a:t></a:r>",
        esc(&run.text)
    )
}

fn runs_xml(runs: &[Run], size: u32) -> String {
    runs.iter().map(|r| run_xml(r, size)).collect()
}

fn para(ppr: &str, runs: &[Run], size: u32) -> String {
    format!(
        "<a:p>{ppr}{}<a:endParaRPr lang=\"en-US\" sz=\"{size}\" dirty=\"0\"/></a:p>",
        runs_xml(runs, size)
    )
}

fn bullet_ppr(level: u8, number: bool) -> String {
    let mar = INDENT * (i64::from(level) + 1);
    let mark = if number {
        "<a:buFont typeface=\"+mj-lt\"/><a:buAutoNum type=\"arabicPeriod\"/>".to_string()
    } else {
        "<a:buFont typeface=\"Arial\"/><a:buChar char=\"&#8226;\"/>".to_string()
    };
    format!("<a:pPr marL=\"{mar}\" indent=\"-{INDENT}\" lvl=\"{level}\">{mark}</a:pPr>")
}

fn size_for(level: u8) -> u32 {
    if level == 0 { BODY_SIZE } else { NESTED_SIZE }
}

fn plain_ppr() -> &'static str {
    "<a:pPr marL=\"0\" indent=\"0\"><a:buNone/></a:pPr>"
}

fn items_xml(items: &[Item]) -> String {
    let mut out = String::new();
    for item in items {
        match item {
            Item::Bullet { level, runs } => {
                out.push_str(&para(&bullet_ppr(*level, false), runs, size_for(*level)));
            }
            Item::Number { level, runs } => {
                out.push_str(&para(&bullet_ppr(*level, true), runs, size_for(*level)));
            }
            Item::Paragraph(runs) => out.push_str(&para(plain_ppr(), runs, BODY_SIZE)),
            Item::Code(text) => {
                for line in text.lines() {
                    let run = Run {
                        text: line.to_string(),
                        code: true,
                        ..Run::default()
                    };
                    out.push_str(&para(plain_ppr(), &[run], CODE_SIZE));
                }
            }
            Item::Table(_) => {}
        }
    }
    if out.is_empty() {
        out.push_str("<a:p><a:endParaRPr lang=\"en-US\"/></a:p>");
    }
    out
}

fn shape(id: u32, name: &str, ph: &str, xfrm: Option<(i64, i64, i64, i64)>, body: &str) -> String {
    let sp_pr = match xfrm {
        Some((x, y, w, h)) => format!(
            "<p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{w}\" cy=\"{h}\"/></a:xfrm></p:spPr>"
        ),
        None => "<p:spPr/>".to_string(),
    };
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr><p:ph {ph}/></p:nvPr></p:nvSpPr>{sp_pr}<p:txBody><a:bodyPr><a:normAutofit/></a:bodyPr><a:lstStyle/>{body}</p:txBody></p:sp>"
    )
}

fn border(side: &str) -> String {
    format!(
        "<a:{side} w=\"12700\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:srgbClr val=\"808080\"/></a:solidFill><a:prstDash val=\"solid\"/></a:{side}>"
    )
}

fn table_xml(id: u32, rows: &[Vec<String>], x: i64, y: i64, w: i64) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let col_w = w / cols as i64;
    let grid: String = (0..cols)
        .map(|_| format!("<a:gridCol w=\"{col_w}\"/>"))
        .collect();
    let borders = ["lnL", "lnR", "lnT", "lnB"].map(border).concat();
    let body: String = rows
        .iter()
        .enumerate()
        .map(|(r, row)| {
            let cells: String = (0..cols)
                .map(|c| {
                    let run = Run {
                        text: row.get(c).cloned().unwrap_or_default(),
                        bold: r == 0,
                        ..Run::default()
                    };
                    format!(
                        "<a:tc><a:txBody><a:bodyPr/><a:lstStyle/>{}</a:txBody><a:tcPr>{borders}</a:tcPr></a:tc>",
                        para("", &[run], TABLE_SIZE)
                    )
                })
                .collect();
            format!("<a:tr h=\"{ROW_H}\">{cells}</a:tr>")
        })
        .collect();
    let h = ROW_H * rows.len() as i64;
    format!(
        "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"{id}\" name=\"Table {id}\"/><p:cNvGraphicFramePr><a:graphicFrameLocks noGrp=\"1\"/></p:cNvGraphicFramePr><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{w}\" cy=\"{h}\"/></p:xfrm><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/table\"><a:tbl><a:tblPr firstRow=\"1\"/><a:tblGrid>{grid}</a:tblGrid>{body}</a:tbl></a:graphicData></a:graphic></p:graphicFrame>"
    )
}

fn slide_part(shapes: &str) -> String {
    format!(
        "{XML}<p:sld xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>{shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"
    )
}

fn title_slide(deck: &Deck) -> String {
    let title = Run {
        text: deck.title.clone(),
        ..Run::default()
    };
    let subtitle: Vec<Vec<Run>> = deck.subtitle.clone();
    let mut shapes = shape(
        2,
        "Title 1",
        "type=\"title\"",
        Some((MARGIN, 2_057_400, SLIDE_W - 2 * MARGIN, 1_371_600)),
        &para("", &[title], DECK_TITLE_SIZE),
    );
    if !subtitle.is_empty() {
        let body: String = subtitle
            .iter()
            .map(|r| para(plain_ppr(), r, BODY_SIZE))
            .collect();
        shapes.push_str(&shape(
            3,
            "Content Placeholder 2",
            "idx=\"1\"",
            Some((MARGIN, 3_657_600, SLIDE_W - 2 * MARGIN, 1_371_600)),
            &body,
        ));
    }
    slide_part(&shapes)
}

fn card_slide(card: &Card) -> String {
    let title = Run {
        text: card.title.clone(),
        ..Run::default()
    };
    let mut shapes = shape(
        2,
        "Title 1",
        "type=\"title\"",
        None,
        &para("", &[title], TITLE_SIZE),
    );
    let columns = card.columns.len().max(1);
    let col_w = (SLIDE_W - 2 * MARGIN - GAP * (columns as i64 - 1)) / columns as i64;
    let mut id = 3;
    for (i, items) in card.columns.iter().enumerate() {
        let x = MARGIN + (col_w + GAP) * i as i64;
        let (tables, text): (Vec<&Item>, Vec<&Item>) =
            items.iter().partition(|it| matches!(it, Item::Table(_)));
        let owned: Vec<Item> = text.into_iter().cloned().collect();
        let idx = i + 1;
        // Room for the text above a table: a line each, but never more than half the body.
        let text_h = if tables.is_empty() {
            BODY_H
        } else {
            (owned.len() as i64 * LINE_H).clamp(LINE_H, BODY_H / 2)
        };
        if !(owned.is_empty() && !tables.is_empty()) {
            shapes.push_str(&shape(
                id,
                &format!("Content Placeholder {id}"),
                &format!("idx=\"{idx}\""),
                if columns == 1 && tables.is_empty() {
                    None
                } else {
                    Some((x, BODY_TOP, col_w, text_h))
                },
                &items_xml(&owned),
            ));
            id += 1;
        }
        let table_top = if owned.is_empty() {
            BODY_TOP
        } else {
            BODY_TOP + text_h + TABLE_GAP
        };
        for t in tables {
            if let Item::Table(rows) = t {
                shapes.push_str(&table_xml(id, rows, x, table_top, col_w));
                id += 1;
            }
        }
    }
    slide_part(&shapes)
}

fn rels(entries: &[(&str, &str, &str)]) -> String {
    let body: String = entries
        .iter()
        .map(|(id, kind, target)| {
            format!("<Relationship Id=\"{id}\" Type=\"{REL}/{kind}\" Target=\"{target}\"/>")
        })
        .collect();
    format!(
        "{XML}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{body}</Relationships>"
    )
}

fn layout_part(name: &str, kind: &str, columns: usize) -> String {
    let title = "<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Title 1\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr><p:ph type=\"title\"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang=\"en-US\"/><a:t>Title</a:t></a:r></a:p></p:txBody></p:sp>";
    let col_w = (SLIDE_W - 2 * MARGIN - GAP * (columns as i64 - 1)) / columns as i64;
    let bodies: String = (0..columns)
        .map(|i| {
            let x = MARGIN + (col_w + GAP) * i as i64;
            let id = 3 + i;
            let idx = i + 1;
            let xfrm = if columns == 1 {
                "<p:spPr/>".to_string()
            } else {
                format!("<p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{BODY_TOP}\"/><a:ext cx=\"{col_w}\" cy=\"{BODY_H}\"/></a:xfrm></p:spPr>")
            };
            format!(
                "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Content Placeholder {id}\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr><p:ph idx=\"{idx}\"/></p:nvPr></p:nvSpPr>{xfrm}<p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:pPr lvl=\"0\"/><a:r><a:rPr lang=\"en-US\"/><a:t>Text</a:t></a:r></a:p></p:txBody></p:sp>"
            )
        })
        .collect();
    format!(
        "{XML}<p:sldLayout xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\" type=\"{kind}\" preserve=\"1\"><p:cSld name=\"{name}\"><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>{title}{bodies}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"
    )
}

fn master_part() -> String {
    let level = |n: u32, size: u32| {
        let mar = INDENT * i64::from(n);
        format!(
            "<a:lvl{n}pPr marL=\"{mar}\" indent=\"-{INDENT}\" algn=\"l\" defTabSz=\"914400\" rtl=\"0\" eaLnBrk=\"1\" latinLnBrk=\"0\" hangingPunct=\"1\"><a:lnSpc><a:spcPct val=\"100000\"/></a:lnSpc><a:spcBef><a:spcPts val=\"600\"/></a:spcBef><a:buFont typeface=\"Arial\"/><a:buChar char=\"&#8226;\"/><a:defRPr sz=\"{size}\" kern=\"1200\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mn-lt\"/><a:ea typeface=\"+mn-ea\"/><a:cs typeface=\"+mn-cs\"/></a:defRPr></a:lvl{n}pPr>"
        )
    };
    let body_levels: String = (1..=5)
        .map(|n| level(n, if n == 1 { BODY_SIZE } else { NESTED_SIZE }))
        .collect();
    format!(
        "{XML}<p:sldMaster xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\"><p:cSld><p:bg><p:bgRef idx=\"1001\"><a:schemeClr val=\"bg1\"/></p:bgRef></p:bg><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr><p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Title Placeholder 1\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr><p:ph type=\"title\"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"{MARGIN}\" y=\"457200\"/><a:ext cx=\"{tw}\" cy=\"{TITLE_H}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr><p:txBody><a:bodyPr vert=\"horz\" lIns=\"91440\" tIns=\"45720\" rIns=\"91440\" bIns=\"45720\" rtlCol=\"0\" anchor=\"ctr\"><a:normAutofit/></a:bodyPr><a:lstStyle/><a:p><a:r><a:rPr lang=\"en-US\"/><a:t>Title</a:t></a:r></a:p></p:txBody></p:sp><p:sp><p:nvSpPr><p:cNvPr id=\"3\" name=\"Text Placeholder 2\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr><p:ph type=\"body\" idx=\"1\"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"{MARGIN}\" y=\"{BODY_TOP}\"/><a:ext cx=\"{tw}\" cy=\"{BODY_H}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr><p:txBody><a:bodyPr vert=\"horz\" lIns=\"91440\" tIns=\"45720\" rIns=\"91440\" bIns=\"45720\" rtlCol=\"0\"><a:normAutofit/></a:bodyPr><a:lstStyle/><a:p><a:pPr lvl=\"0\"/><a:r><a:rPr lang=\"en-US\"/><a:t>Text</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld><p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/><p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/><p:sldLayoutId id=\"2147483650\" r:id=\"rId2\"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle><a:lvl1pPr algn=\"l\" defTabSz=\"914400\" rtl=\"0\" eaLnBrk=\"1\" latinLnBrk=\"0\" hangingPunct=\"1\"><a:lnSpc><a:spcPct val=\"90000\"/></a:lnSpc><a:spcBef><a:spcPct val=\"0\"/></a:spcBef><a:buNone/><a:defRPr sz=\"{TITLE_SIZE}\" b=\"1\" kern=\"1200\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mj-lt\"/><a:ea typeface=\"+mj-ea\"/><a:cs typeface=\"+mj-cs\"/></a:defRPr></a:lvl1pPr></p:titleStyle><p:bodyStyle>{body_levels}</p:bodyStyle><p:otherStyle><a:defPPr><a:defRPr lang=\"en-US\"/></a:defPPr></p:otherStyle></p:txStyles></p:sldMaster>",
        tw = SLIDE_W - 2 * MARGIN,
    )
}

/// Neutral grays only: the deck takes a color from the person, not from the writer.
fn theme_part() -> String {
    let fill = "<a:fillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:fillStyleLst>";
    let line = "<a:lnStyleLst><a:ln w=\"6350\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln><a:ln w=\"12700\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln><a:ln w=\"19050\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln></a:lnStyleLst>";
    let effect = "<a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst>";
    let bg = "<a:bgFillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:bgFillStyleLst>";
    format!(
        "{XML}<a:theme xmlns:a=\"{NS_A}\" name=\"HQ Slides\"><a:themeElements><a:clrScheme name=\"Neutral\"><a:dk1><a:srgbClr val=\"000000\"/></a:dk1><a:lt1><a:srgbClr val=\"FFFFFF\"/></a:lt1><a:dk2><a:srgbClr val=\"404040\"/></a:dk2><a:lt2><a:srgbClr val=\"F2F2F2\"/></a:lt2><a:accent1><a:srgbClr val=\"595959\"/></a:accent1><a:accent2><a:srgbClr val=\"7F7F7F\"/></a:accent2><a:accent3><a:srgbClr val=\"A6A6A6\"/></a:accent3><a:accent4><a:srgbClr val=\"BFBFBF\"/></a:accent4><a:accent5><a:srgbClr val=\"D9D9D9\"/></a:accent5><a:accent6><a:srgbClr val=\"262626\"/></a:accent6><a:hlink><a:srgbClr val=\"404040\"/></a:hlink><a:folHlink><a:srgbClr val=\"7F7F7F\"/></a:folHlink></a:clrScheme><a:fontScheme name=\"Plain\"><a:majorFont><a:latin typeface=\"Calibri\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont><a:minorFont><a:latin typeface=\"Calibri\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont></a:fontScheme><a:fmtScheme name=\"Plain\">{fill}{line}{effect}{bg}</a:fmtScheme></a:themeElements></a:theme>"
    )
}

/// The deck as the bytes of a `.pptx` file.
pub fn render_pptx(deck: &Deck) -> Result<Vec<u8>, PptxError> {
    let mut parts: Vec<(String, String)> = Vec::new();
    let n = deck.cards.len() + 1;
    let slide_overrides: String = (1..=n)
        .map(|i| format!("<Override PartName=\"/ppt/slides/slide{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"))
        .collect();
    parts.push((
        "[Content_Types].xml".into(),
        format!("{XML}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/><Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/><Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/><Override PartName=\"/ppt/slideLayouts/slideLayout2.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/><Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/><Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>{slide_overrides}</Types>"),
    ));
    parts.push((
        "_rels/.rels".into(),
        format!("{XML}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{REL}/officeDocument\" Target=\"ppt/presentation.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/><Relationship Id=\"rId3\" Type=\"{REL}/extended-properties\" Target=\"docProps/app.xml\"/></Relationships>"),
    ));
    parts.push((
        "docProps/core.xml".into(),
        format!("{XML}<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>{}</dc:title></cp:coreProperties>", esc(&deck.title)),
    ));
    parts.push((
        "docProps/app.xml".into(),
        format!("{XML}<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>HQ Suite</Application><Slides>{n}</Slides></Properties>"),
    ));
    let slide_ids: String = (1..=n)
        .map(|i| format!("<p:sldId id=\"{}\" r:id=\"rId{}\"/>", 255 + i, i + 1))
        .collect();
    parts.push((
        "ppt/presentation.xml".into(),
        format!("{XML}<p:presentation xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\" saveSubsetFonts=\"1\"><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst><p:sldIdLst>{slide_ids}</p:sldIdLst><p:sldSz cx=\"{SLIDE_W}\" cy=\"{SLIDE_H}\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>"),
    ));
    let mut pres_rels: Vec<(String, &str, String)> = vec![(
        "rId1".into(),
        "slideMaster",
        "slideMasters/slideMaster1.xml".into(),
    )];
    for i in 1..=n {
        pres_rels.push((
            format!("rId{}", i + 1),
            "slide",
            format!("slides/slide{i}.xml"),
        ));
    }
    pres_rels.push((format!("rId{}", n + 2), "theme", "theme/theme1.xml".into()));
    let borrowed: Vec<(&str, &str, &str)> = pres_rels
        .iter()
        .map(|(a, b, c)| (a.as_str(), *b, c.as_str()))
        .collect();
    parts.push(("ppt/_rels/presentation.xml.rels".into(), rels(&borrowed)));
    parts.push(("ppt/slideMasters/slideMaster1.xml".into(), master_part()));
    parts.push((
        "ppt/slideMasters/_rels/slideMaster1.xml.rels".into(),
        rels(&[
            ("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml"),
            ("rId2", "slideLayout", "../slideLayouts/slideLayout2.xml"),
            ("rId3", "theme", "../theme/theme1.xml"),
        ]),
    ));
    parts.push((
        "ppt/slideLayouts/slideLayout1.xml".into(),
        layout_part("Title and Content", "obj", 1),
    ));
    parts.push((
        "ppt/slideLayouts/slideLayout2.xml".into(),
        layout_part("Two Content", "twoObj", 2),
    ));
    for i in 1..=2 {
        parts.push((
            format!("ppt/slideLayouts/_rels/slideLayout{i}.xml.rels"),
            rels(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")]),
        ));
    }
    parts.push(("ppt/theme/theme1.xml".into(), theme_part()));
    parts.push(("ppt/slides/slide1.xml".into(), title_slide(deck)));
    parts.push((
        "ppt/slides/_rels/slide1.xml.rels".into(),
        rels(&[("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml")]),
    ));
    for (i, card) in deck.cards.iter().enumerate() {
        let slide = i + 2;
        let layout = if card.layout == Layout::TwoColumn {
            2
        } else {
            1
        };
        parts.push((format!("ppt/slides/slide{slide}.xml"), card_slide(card)));
        parts.push((
            format!("ppt/slides/_rels/slide{slide}.xml.rels"),
            rels(&[(
                "rId1",
                "slideLayout",
                &format!("../slideLayouts/slideLayout{layout}.xml"),
            )]),
        ));
    }
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, xml) in parts {
        zip.start_file(name, options)
            .map_err(|e| PptxError::Write(e.to_string()))?;
        zip.write_all(xml.as_bytes())
            .map_err(|e| PptxError::Write(e.to_string()))?;
    }
    zip.finish()
        .map(Cursor::into_inner)
        .map_err(|e| PptxError::Write(e.to_string()))
}
