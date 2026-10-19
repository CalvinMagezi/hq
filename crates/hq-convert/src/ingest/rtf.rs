//! Rich Text Format (`.rtf`) to Markdown.
//!
//! RTF is a stream of groups and control words. This reader keeps a stack of
//! character formatting, reads the font and style tables to find monospace
//! fonts and headings, decodes code pages and Unicode escapes, and turns
//! `\trowd`/`\cell`/`\row` into tables and `\field` hyperlinks into links.

use std::collections::HashMap;

use encoding_rs::Encoding;

use super::ir::{Block, Inline, ListItem, Parsed, Run, Table, is_mono_font};
use super::report::{LossKind, Losses};
use crate::types::ConvertError;

const MAX_DEPTH: usize = 400;

pub fn read(bytes: &[u8]) -> Result<Parsed, ConvertError> {
    let head = bytes.iter().take(64).copied().collect::<Vec<u8>>();
    let head = String::from_utf8_lossy(&head);
    if !head
        .trim_start_matches('\u{feff}')
        .trim_start()
        .starts_with("{\\rtf")
    {
        return Err(ConvertError::InboundFailed("not an RTF document".into()));
    }
    let mut p = Parser::new(bytes);
    p.run()?;
    Ok(p.finish())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dest {
    Body,
    FontTable,
    StyleSheet,
    /// Text of a list marker such as "1." or a bullet.
    Marker,
    FieldInstr,
    /// Anything whose text is not content: colour table, headers, pictures.
    Skip,
}

#[derive(Clone)]
struct Group {
    bold: bool,
    italic: bool,
    strike: bool,
    font: Option<i32>,
    dest: Dest,
    /// Characters still to drop after a `\uN` escape.
    uc: usize,
    skip_next: usize,
    link: Option<String>,
}

#[derive(Default, Clone)]
struct FontInfo {
    name: String,
    charset: Option<i32>,
}

#[derive(Default)]
struct ParaProps {
    style: Option<i32>,
    outline: Option<u8>,
    in_list: bool,
    ilvl: usize,
    in_table: bool,
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
    stack: Vec<Group>,
    fonts: HashMap<i32, FontInfo>,
    styles: HashMap<i32, String>,
    default_cp: u16,
    /// Table entry being defined: font or style number, and its name so far.
    defining: Option<i32>,
    dest_buf: String,
    pending_bytes: Vec<u8>,
    pending_link: Option<String>,
    /// First half of a UTF-16 surrogate pair written as two `\u` escapes.
    high: Option<u32>,
    runs: Vec<Inline>,
    marker: Option<String>,
    para: ParaProps,
    blocks: Vec<Block>,
    list_items: Vec<ListItem>,
    row: Vec<Vec<Inline>>,
    rows: Vec<Vec<Vec<Inline>>>,
    /// The row being defined: how many `\cellx` it has and whether any cell is marked merged.
    def_cells: usize,
    def_merged: bool,
    /// The same for the rows collected so far.
    table_cells: usize,
    losses: Losses,
}

impl<'a> Parser<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self {
            b,
            i: 0,
            stack: vec![Group {
                bold: false,
                italic: false,
                strike: false,
                font: None,
                dest: Dest::Body,
                uc: 1,
                skip_next: 0,
                link: None,
            }],
            fonts: HashMap::new(),
            styles: HashMap::new(),
            default_cp: 1252,
            defining: None,
            dest_buf: String::new(),
            pending_bytes: Vec::new(),
            pending_link: None,
            high: None,
            runs: Vec::new(),
            marker: None,
            para: ParaProps::default(),
            blocks: Vec::new(),
            list_items: Vec::new(),
            row: Vec::new(),
            rows: Vec::new(),
            def_cells: 0,
            def_merged: false,
            table_cells: 0,
            losses: Losses::default(),
        }
    }

    fn top(&mut self) -> &mut Group {
        self.stack.last_mut().expect("group stack is never empty")
    }

    fn run(&mut self) -> Result<(), ConvertError> {
        while self.i < self.b.len() {
            let c = self.b[self.i];
            match c {
                b'{' => {
                    self.flush_bytes();
                    self.i += 1;
                    if self.stack.len() >= MAX_DEPTH {
                        return Err(ConvertError::TooLarge(
                            "RTF groups nested too deeply".into(),
                        ));
                    }
                    let g = self
                        .stack
                        .last()
                        .cloned()
                        .expect("group stack is never empty");
                    self.stack.push(g);
                }
                b'}' => {
                    self.flush_bytes();
                    self.i += 1;
                    self.close_group();
                }
                b'\\' => self.control(),
                b'\r' | b'\n' => self.i += 1,
                _ => {
                    // Plain text: take the whole run of ordinary bytes.
                    let start = self.i;
                    while self.i < self.b.len()
                        && !matches!(self.b[self.i], b'{' | b'}' | b'\\' | b'\r' | b'\n')
                    {
                        self.i += 1;
                    }
                    let chunk = self.b[start..self.i].to_vec();
                    self.flush_bytes();
                    self.text_bytes(&chunk);
                }
            }
        }
        self.flush_bytes();
        self.end_paragraph();
        self.flush_table();
        self.flush_list();
        Ok(())
    }

    fn close_group(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        let closed = self.stack.pop().expect("checked above");
        match closed.dest {
            Dest::FieldInstr if self.stack.last().map(|g| g.dest) != Some(Dest::FieldInstr) => {
                self.pending_link = hyperlink(&self.dest_buf);
                self.dest_buf.clear();
            }
            Dest::Marker if self.stack.last().map(|g| g.dest) != Some(Dest::Marker) => {
                let m = self.dest_buf.trim().to_string();
                self.dest_buf.clear();
                if !m.is_empty() {
                    self.marker = Some(m);
                }
                self.para.in_list = true;
            }
            Dest::FontTable | Dest::StyleSheet => {
                self.finish_definition();
                self.defining = None;
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------- control words

    fn control(&mut self) {
        self.i += 1; // the backslash
        let Some(&c) = self.b.get(self.i) else { return };
        if !c.is_ascii_alphabetic() {
            self.i += 1;
            match c {
                b'\'' => {
                    let hex = self
                        .b
                        .get(self.i..self.i + 2)
                        .and_then(|h| std::str::from_utf8(h).ok());
                    if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                        self.i += 2;
                        if self.top().skip_next > 0 {
                            self.top().skip_next -= 1;
                        } else {
                            self.pending_bytes.push(v);
                        }
                    }
                }
                b'*' => {
                    self.flush_bytes();
                    // An ignorable destination: skip its group unless it is one we read.
                    let rest = &self.b[self.i..];
                    let word = rest.strip_prefix(b"\\").unwrap_or(rest);
                    let name: String = word
                        .iter()
                        .take_while(|c| c.is_ascii_alphabetic())
                        .map(|&c| c as char)
                        .collect();
                    match name.as_str() {
                        "fldinst" => self.top().dest = Dest::FieldInstr,
                        "pntext" | "listtext" => self.top().dest = Dest::Marker,
                        "shpinst" => {}
                        _ => self.top().dest = Dest::Skip,
                    }
                }
                b'~' => self.emit("\u{a0}"),
                b'_' => self.emit("-"),
                b'-' | b'|' | b':' => {}
                b'\\' | b'{' | b'}' => {
                    self.flush_bytes();
                    let s = (c as char).to_string();
                    self.emit(&s);
                }
                b'\n' | b'\r' => {
                    self.flush_bytes();
                    self.end_paragraph();
                }
                _ => {}
            }
            return;
        }
        self.flush_bytes();
        let start = self.i;
        while self.i < self.b.len() && self.b[self.i].is_ascii_alphabetic() {
            self.i += 1;
        }
        let word = String::from_utf8_lossy(&self.b[start..self.i]).into_owned();
        let mut param: Option<i32> = None;
        let neg = self.b.get(self.i) == Some(&b'-');
        let ds = if neg { self.i + 1 } else { self.i };
        let mut j = ds;
        while j < self.b.len() && self.b[j].is_ascii_digit() {
            j += 1;
        }
        if j > ds {
            let n: i64 = String::from_utf8_lossy(&self.b[ds..j]).parse().unwrap_or(0);
            param = Some((if neg { -n } else { n }).clamp(i32::MIN as i64, i32::MAX as i64) as i32);
            self.i = j;
        }
        if self.b.get(self.i) == Some(&b' ') {
            self.i += 1;
        }
        self.word(&word, param);
    }

    fn word(&mut self, w: &str, p: Option<i32>) {
        // Inside a destination that is not body text, paragraph and formatting
        // words must not touch the document's own paragraph state.
        let allowed = match self.top().dest {
            Dest::Skip => false,
            Dest::Marker => matches!(
                w,
                "u" | "tab"
                    | "emdash"
                    | "endash"
                    | "bullet"
                    | "lquote"
                    | "rquote"
                    | "ldblquote"
                    | "rdblquote"
                    | "emspace"
                    | "enspace"
                    | "f"
            ),
            Dest::FontTable | Dest::StyleSheet => matches!(w, "f" | "s" | "fcharset" | "u"),
            Dest::FieldInstr => matches!(w, "u"),
            Dest::Body => true,
        };
        if !allowed {
            return;
        }
        let on = p != Some(0);
        match w {
            "ansicpg" => self.default_cp = p.unwrap_or(1252).clamp(0, u16::MAX as i32) as u16,
            "uc" => self.top().uc = p.unwrap_or(1).max(0) as usize,
            "fonttbl" => self.top().dest = Dest::FontTable,
            "stylesheet" => self.top().dest = Dest::StyleSheet,
            "colortbl" | "info" | "header" | "headerl" | "headerr" | "headerf" | "footer"
            | "footerl" | "footerr" | "footerf" | "listtable" | "listoverridetable" | "rsidtbl"
            | "generator" | "private" | "xmlnstbl" | "themedata" | "colorschememapping"
            | "datastore" | "latentstyles" | "revtbl" | "bkmkstart" | "bkmkend" | "footnote"
            | "annotation" | "atnauthor" | "atnid" => self.top().dest = Dest::Skip,
            "pict" => {
                self.top().dest = Dest::Skip;
                self.losses.hit(LossKind::ImagesOmitted);
            }
            "object" => {
                self.top().dest = Dest::Skip;
                self.losses.hit(LossKind::EmbeddedObjectsOmitted);
            }
            "fldrslt" => {
                let link = self.pending_link.clone();
                let top = self.top();
                top.dest = Dest::Body;
                top.link = link;
            }
            "fldinst" => self.top().dest = Dest::FieldInstr,
            "listtext" | "pntext" => self.top().dest = Dest::Marker,
            "f" => {
                let d = self.top().dest;
                if matches!(d, Dest::FontTable) {
                    self.finish_definition();
                    self.defining = p;
                } else {
                    self.top().font = p;
                }
            }
            "fcharset" if self.top().dest == Dest::FontTable => {
                if let Some(id) = self.defining {
                    self.fonts.entry(id).or_default().charset = p;
                }
            }
            "s" if self.top().dest == Dest::StyleSheet => {
                self.finish_definition();
                self.defining = p;
            }
            "s" => self.para.style = p,
            "plain" => {
                let g = self.top();
                g.bold = false;
                g.italic = false;
                g.strike = false;
            }
            "b" => self.top().bold = on,
            "i" => self.top().italic = on,
            "strike" | "striked" => self.top().strike = on,
            "pard" => {
                // Paragraph properties reset; a paragraph that follows a finished table row ends the table.
                if !self.runs.is_empty() {
                    self.end_paragraph();
                }
                self.para = ParaProps::default();
                self.marker = None;
            }
            "intbl" => self.para.in_table = true,
            "ls" => self.para.in_list = true,
            "ilvl" => self.para.ilvl = p.unwrap_or(0).clamp(0, 8) as usize,
            "outlinelevel" => self.para.outline = p.map(|n| (n.clamp(0, 8) + 1) as u8),
            "par" => self.end_paragraph(),
            "sect" | "page" => {}
            "line" => self.push_inline(Inline::Break),
            "tab" => self.emit(" "),
            "trowd" => {
                self.flush_paragraph_for_table();
                self.row.clear();
                self.def_cells = 0;
                self.def_merged = false;
            }
            "cellx" => self.def_cells += 1,
            // A cell marked as merged: the row has fewer cells on purpose.
            "clmgf" | "clmrg" | "clvmgf" | "clvmrg" => self.def_merged = true,
            "cell" | "nestcell" => self.end_cell(),
            "row" | "nestrow" => self.end_row(),
            "emdash" => self.emit("\u{2014}"),
            "endash" => self.emit("\u{2013}"),
            "emspace" | "enspace" | "qmspace" => self.emit(" "),
            "bullet" => self.emit("\u{2022}"),
            "lquote" => self.emit("\u{2018}"),
            "rquote" => self.emit("\u{2019}"),
            "ldblquote" => self.emit("\u{201c}"),
            "rdblquote" => self.emit("\u{201d}"),
            "u" => {
                if let Some(n) = p {
                    let code = if n < 0 { (n + 65536) as u32 } else { n as u32 };
                    // A surrogate half is stored as its own escape; pair them up.
                    self.unicode(code);
                    let uc = self.top().uc;
                    self.top().skip_next = uc;
                }
            }
            _ => {}
        }
    }

    // ----------------------------------------------------------------- text

    fn unicode(&mut self, code: u32) {
        if (0xD800..0xDC00).contains(&code) {
            self.high = Some(code);
            return;
        }
        if (0xDC00..0xE000).contains(&code) {
            if let Some(hi) = self.high.take() {
                let c = 0x10000 + ((hi - 0xD800) << 10) + (code - 0xDC00);
                if let Some(ch) = char::from_u32(c) {
                    self.emit(&ch.to_string());
                }
            }
            return;
        }
        self.high = None;
        if let Some(ch) = char::from_u32(code) {
            self.emit(&ch.to_string());
        }
    }

    fn encoding(&self) -> &'static Encoding {
        let cp = match self
            .stack
            .last()
            .and_then(|g| g.font)
            .and_then(|f| self.fonts.get(&f))
            .and_then(|f| f.charset)
        {
            Some(cs) => charset_codepage(cs).unwrap_or(self.default_cp),
            None => self.default_cp,
        };
        codepage_encoding(cp)
    }

    /// Decodes bytes from `\'hh` escapes in the code page of the current font.
    fn flush_bytes(&mut self) {
        if self.pending_bytes.is_empty() {
            return;
        }
        let bytes = std::mem::take(&mut self.pending_bytes);
        let (s, _, _) = self.encoding().decode(&bytes);
        let s = s.into_owned();
        self.emit(&s);
    }

    fn text_bytes(&mut self, chunk: &[u8]) {
        // Characters after \uN that stand in for it are dropped.
        let mut rest = chunk;
        while self.top().skip_next > 0 && !rest.is_empty() {
            rest = &rest[1..];
            self.top().skip_next -= 1;
        }
        if rest.is_empty() {
            return;
        }
        // Plain bytes in the document are in the default code page too.
        let (s, _, _) = self.encoding().decode(rest);
        let s = s.into_owned();
        self.emit(&s);
    }

    fn emit(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        match self.top().dest {
            Dest::Skip => {}
            Dest::FontTable | Dest::StyleSheet => {
                for ch in s.chars() {
                    if ch == ';' {
                        self.finish_definition();
                    } else {
                        self.dest_buf.push(ch);
                    }
                }
            }
            Dest::Marker | Dest::FieldInstr => self.dest_buf.push_str(s),
            Dest::Body => {
                let g = self.top().clone();
                let mono = g
                    .font
                    .and_then(|f| self.fonts.get(&f))
                    .is_some_and(|f| is_mono_font(&f.name));
                self.push_inline(Inline::Run(Run {
                    text: s.to_string(),
                    bold: g.bold,
                    italic: g.italic,
                    strike: g.strike,
                    code: mono,
                    link: g.link,
                }));
            }
        }
    }

    fn finish_definition(&mut self) {
        let name = self.dest_buf.trim().to_string();
        self.dest_buf.clear();
        let Some(id) = self.defining else { return };
        if name.is_empty() {
            return;
        }
        match self.stack.last().map(|g| g.dest) {
            Some(Dest::FontTable) => self.fonts.entry(id).or_default().name = name,
            Some(Dest::StyleSheet) => {
                self.styles.insert(id, name.to_lowercase());
            }
            _ => {}
        }
    }

    fn push_inline(&mut self, i: Inline) {
        if matches!(self.top().dest, Dest::Skip) {
            return;
        }
        self.runs.push(i);
    }

    // ------------------------------------------------------------ paragraphs

    fn end_paragraph(&mut self) {
        let runs = std::mem::take(&mut self.runs);
        let marker = self.marker.take();
        let has_text = runs
            .iter()
            .any(|r| matches!(r, Inline::Run(x) if !x.text.trim().is_empty()));
        if self.para.in_table {
            // Another paragraph inside the same cell.
            if has_text {
                if self.cell_has_text() {
                    self.runs.push(Inline::Break);
                }
                self.runs.extend(runs);
            }
            return;
        }
        if !has_text {
            return;
        }
        self.flush_table();
        let style = self
            .para
            .style
            .and_then(|s| self.styles.get(&s))
            .cloned()
            .unwrap_or_default();
        let level = style
            .strip_prefix("heading ")
            .and_then(|n| n.trim().parse::<u8>().ok())
            .or(self.para.outline);
        if style == "title" {
            self.flush_list();
            self.blocks.push(Block::Heading(0, runs));
        } else if let Some(l) = level {
            self.flush_list();
            self.blocks.push(Block::Heading(l.clamp(1, 6), runs));
        } else if self.para.in_list || marker.is_some() {
            let ordered = marker.as_deref().is_some_and(looks_numbered);
            self.list_items.push(ListItem {
                level: self.para.ilvl,
                ordered,
                content: runs,
            });
        } else {
            self.flush_list();
            let all_code = runs.iter().all(|i| {
                matches!(i, Inline::Run(r) if r.text.trim().is_empty() || r.code)
                    || matches!(i, Inline::Break)
            });
            if all_code {
                let text: String = runs
                    .iter()
                    .map(|i| match i {
                        Inline::Run(r) => r.text.clone(),
                        _ => "\n".to_string(),
                    })
                    .collect();
                self.blocks.push(Block::Code { lang: None, text });
            } else {
                self.blocks.push(Block::Para(runs));
            }
        }
    }

    fn cell_has_text(&self) -> bool {
        self.runs
            .iter()
            .any(|r| matches!(r, Inline::Run(x) if !x.text.trim().is_empty()))
    }

    fn flush_list(&mut self) {
        if !self.list_items.is_empty() {
            self.blocks
                .push(Block::List(std::mem::take(&mut self.list_items)));
        }
    }

    // ----------------------------------------------------------------- tables

    fn flush_paragraph_for_table(&mut self) {
        if !self.runs.is_empty() && !self.para.in_table {
            self.end_paragraph();
        }
        self.flush_list();
    }

    fn end_cell(&mut self) {
        let mut cell = std::mem::take(&mut self.runs);
        while matches!(cell.last(), Some(Inline::Break)) {
            cell.pop();
        }
        self.row.push(cell);
    }

    fn end_row(&mut self) {
        if !self.runs.is_empty() {
            self.end_cell();
        }
        if !self.row.is_empty() {
            // Two tables with nothing between them arrive as consecutive rows. A row defined with
            // a different number of cells, and no merge marks to explain it, starts a new table
            // instead of being padded into the one before.
            let starts_new = !self.rows.is_empty()
                && self.def_cells > 0
                && self.table_cells > 0
                && self.def_cells != self.table_cells
                && !self.def_merged;
            if starts_new {
                self.flush_table();
            }
            if self.rows.is_empty() {
                self.table_cells = self.def_cells;
            }
            self.rows.push(std::mem::take(&mut self.row));
        }
    }

    fn flush_table(&mut self) {
        if !self.rows.is_empty() {
            let table = Table {
                rows: std::mem::take(&mut self.rows),
            };
            // Table text that has no content at all (layout rows) is dropped.
            if table.rows.iter().flatten().any(|c| !c.is_empty()) {
                self.flush_list();
                self.blocks.push(Block::Table(table));
            }
        }
    }

    fn finish(mut self) -> Parsed {
        self.flush_table();
        self.flush_list();
        Parsed {
            blocks: std::mem::take(&mut self.blocks),
            title: None,
            losses: self.losses,
        }
    }
}

fn hyperlink(instr: &str) -> Option<String> {
    let t = instr.trim();
    let rest = t
        .strip_prefix("HYPERLINK")
        .or_else(|| t.strip_prefix("hyperlink"))?
        .trim();
    if rest.starts_with("\\l") {
        return None;
    }
    match rest.find('"') {
        Some(a) => {
            let r = &rest[a + 1..];
            r.find('"')
                .map(|b| r[..b].to_string())
                .filter(|u| !u.is_empty())
        }
        None => rest.split_whitespace().next().map(str::to_string),
    }
}

/// Whether a list marker ("1.", "a)", "iv.") numbers items rather than bullets them.
fn looks_numbered(marker: &str) -> bool {
    let m = marker.trim().trim_end_matches(['.', ')', ':']);
    !m.is_empty()
        && m.chars().count() <= 6
        && (m.chars().all(|c| c.is_ascii_digit())
            || (m.chars().count() == 1 && m.chars().all(|c| c.is_ascii_alphabetic()))
            || m.chars().all(|c| "ivxlcdmIVXLCDM".contains(c)))
}

fn charset_codepage(cs: i32) -> Option<u16> {
    Some(match cs {
        0 => 1252,
        77 => 10000,
        128 => 932,
        129 => 949,
        134 => 936,
        136 => 950,
        161 => 1253,
        162 => 1254,
        163 => 1258,
        177 => 1255,
        178 => 1256,
        186 => 1257,
        204 => 1251,
        222 => 874,
        238 => 1250,
        _ => return None,
    })
}

fn codepage_encoding(cp: u16) -> &'static Encoding {
    match cp {
        874 => encoding_rs::WINDOWS_874,
        932 => encoding_rs::SHIFT_JIS,
        936 => encoding_rs::GBK,
        949 => encoding_rs::EUC_KR,
        950 => encoding_rs::BIG5,
        1250 => encoding_rs::WINDOWS_1250,
        1251 => encoding_rs::WINDOWS_1251,
        1253 => encoding_rs::WINDOWS_1253,
        1254 => encoding_rs::WINDOWS_1254,
        1255 => encoding_rs::WINDOWS_1255,
        1256 => encoding_rs::WINDOWS_1256,
        1257 => encoding_rs::WINDOWS_1257,
        1258 => encoding_rs::WINDOWS_1258,
        10000 => encoding_rs::MACINTOSH,
        _ => encoding_rs::WINDOWS_1252,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(rtf: &str) -> String {
        super::super::ir::render(read(rtf.as_bytes()).unwrap().blocks)
    }

    #[test]
    fn paragraphs_emphasis_and_special_characters() {
        let rtf = r"{\rtf1\ansi\ansicpg1252{\fonttbl{\f0 Times New Roman;}}\f0 Plain {\b bold} and {\i italic}\par Caf\'e9 \emdash  done\par}";
        assert_eq!(
            md(rtf),
            "Plain **bold** and *italic*\n\nCaf\u{e9} \u{2014} done\n"
        );
    }

    #[test]
    fn unicode_escapes_drop_their_fallback_character() {
        // Built at run time: the escapes are a backslash, "u" and a decimal number.
        let esc = |n: u32| format!("{}u{n}?", '\\');
        let rtf = format!(
            "{{{b}rtf1{b}ansi{b}uc1 Price: {}100 and {}{}{b}par}}",
            esc(8364),
            esc(1050),
            esc(1080),
            b = '\\'
        );
        assert_eq!(md(&rtf), "Price: \u{20ac}100 and \u{41a}\u{438}\n");
    }

    #[test]
    fn surrogate_pairs_combine() {
        let esc = |n: i32| format!("{}u{n}?", '\\');
        let rtf = format!(
            "{{{b}rtf1{b}ansi {}{} done{b}par}}",
            esc(-10179),
            esc(-8704),
            b = '\\'
        );
        assert_eq!(md(&rtf), "\u{1f600} done\n");
    }

    #[test]
    fn headings_come_from_the_stylesheet() {
        let rtf = r"{\rtf1\ansi{\stylesheet{\s1 heading 1;}{\s2 heading 2;}}\pard\s1 Intro\par\pard\s2 Detail\par\pard Body text\par}";
        assert_eq!(md(rtf), "# Intro\n\n## Detail\n\nBody text\n");
    }

    #[test]
    fn tables_collect_cells_by_row() {
        let rtf = r"{\rtf1\ansi\trowd\cellx2000\cellx4000\intbl Site\cell Farmers\cell\row\trowd\cellx2000\cellx4000\intbl Mukono\cell 42\cell\row\pard After\par}";
        assert_eq!(
            md(rtf),
            "| Site | Farmers |\n| --- | --- |\n| Mukono | 42 |\n\nAfter\n"
        );
    }

    #[test]
    fn two_tables_with_nothing_between_stay_two_tables() {
        let rtf = r"{\rtf1\ansi\trowd\cellx2000\cellx4000\intbl Bill to\cell Details\cell\row\trowd\cellx2000\cellx4000\intbl Farmers\cell INV-1\cell\row\trowd\cellx1000\cellx2000\cellx3000\cellx4000\intbl Item\cell Qty\cell Price\cell Amount\cell\row\trowd\cellx1000\cellx2000\cellx3000\cellx4000\intbl Seed\cell 2\cell 5\cell 10\cell\row\pard After\par}";
        assert_eq!(
            md(rtf),
            "| Bill to | Details |\n| --- | --- |\n| Farmers | INV-1 |\n\n| Item | Qty | Price | Amount |\n| --- | --- | --- | --- |\n| Seed | 2 | 5 | 10 |\n\nAfter\n"
        );
    }

    #[test]
    fn a_row_with_merged_cells_stays_in_its_table() {
        // Word marks merged cells; the row keeps its place and is padded as before.
        let rtf = r"{\rtf1\ansi\trowd\cellx1000\cellx2000\cellx3000\intbl A\cell B\cell C\cell\row\trowd\clmgf\cellx1000\clmrg\cellx2000\cellx3000\intbl Wide\cell\cell D\cell\row\pard\par}";
        let out = md(rtf);
        assert_eq!(
            out.lines().filter(|l| l.contains("---")).count(),
            1,
            "{out}"
        );
    }

    #[test]
    fn hyperlink_fields_become_links() {
        let rtf = r#"{\rtf1\ansi See {\field{\*\fldinst{HYPERLINK "https://example.com/p"}}{\fldrslt{\ul the plan}}} now\par}"#;
        assert_eq!(md(rtf), "See [the plan](https://example.com/p) now\n");
    }

    #[test]
    fn list_markers_decide_bullets_and_numbers() {
        let rtf = r"{\rtf1\ansi\pard{\listtext\'b7\tab}\ls1 first\par\pard{\listtext 1.\tab}\ls2 numbered\par}";
        assert_eq!(md(rtf), "- first\n1. numbered\n");
    }

    #[test]
    fn monospace_fonts_become_code() {
        let rtf =
            r"{\rtf1\ansi{\fonttbl{\f0 Arial;}{\f1 Courier New;}}\f0 Run \f1 seedctl\f0  now\par}";
        assert_eq!(md(rtf), "Run `seedctl` now\n");
    }

    #[test]
    fn multibyte_code_pages_decode_pairs() {
        // "日本" in Shift-JIS is 93 FA 96 7B.
        let rtf = r"{\rtf1\ansi\ansicpg932{\fonttbl{\f0\fcharset128 MS Mincho;}}\f0 \'93\'fa\'96\'7b\par}";
        assert_eq!(md(rtf), "\u{65e5}\u{672c}\n");
    }

    #[test]
    fn colour_tables_pictures_and_info_are_not_text() {
        let rtf = r"{\rtf1\ansi{\colortbl;\red0\green0\blue0;}{\info{\title Secret}}Visible{\pict\wmetafile8 0102ab}\par}";
        assert_eq!(md(rtf), "Visible\n");
    }

    #[test]
    fn text_that_is_not_rtf_is_refused() {
        assert!(read(b"plain text").is_err());
    }
}
