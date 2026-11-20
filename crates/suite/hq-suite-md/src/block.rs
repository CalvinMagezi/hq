//! Typed blocks: fenced code blocks whose header names one of the suite's block types.
//!
//! ````text
//! ```sheet name="Stock" types="text,int,ugx"
//! | item | qty | price |
//! |------|-----|-------|
//! | Sugar | 40 | 4200 |
//! ```
//! ````
//!
//! A fenced block with any other header is just code and is carried as written. A typed block
//! must start in the first column and sit at the top level of the note (not inside a list or a
//! quote); anywhere else it is ordinary code, so a viewer that knows nothing of hqmd and one
//! that does agree on where the blocks are.

use std::fmt;

use crate::info::{Attr, Info, InfoError, first_word};
use crate::reference::{RangeRef, safe_rel_path};
use crate::sheet::{Grid, TableError, parse_types};

/// The block types hqmd v1 defines. `fields` (Docs) and slide layout comments are reserved and
/// not yet part of the language; a block using them is kept as code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockType {
    /// A typed grid with formulas.
    Sheet,
    /// One number computed from a range.
    Tile,
    /// A chart drawn from a range.
    Plot,
    /// A chart with its data inline, as HQ chat already draws (`docs/CHART_BLOCKS.md`).
    Chart,
    /// A review note anchored to another block by its hash. Never part of an export.
    Comment,
}

impl BlockType {
    pub const ALL: &'static [BlockType] = &[
        BlockType::Sheet,
        BlockType::Tile,
        BlockType::Plot,
        BlockType::Chart,
        BlockType::Comment,
    ];

    pub fn name(self) -> &'static str {
        match self {
            BlockType::Sheet => "sheet",
            BlockType::Tile => "tile",
            BlockType::Plot => "plot",
            BlockType::Chart => "chart",
            BlockType::Comment => "comment",
        }
    }

    pub fn parse(s: &str) -> Option<BlockType> {
        BlockType::ALL.iter().copied().find(|t| t.name() == s)
    }

    /// Attributes the type understands, `id` always included.
    fn allowed_attrs(self) -> &'static [&'static str] {
        match self {
            BlockType::Sheet => &["id", "name", "types", "data", "rows"],
            BlockType::Tile => &[
                "id", "from", "fn", "label", "format", "where", "group", "size",
            ],
            BlockType::Plot => &[
                "id", "type", "from", "x", "y", "title", "where", "group", "size",
            ],
            BlockType::Chart => &["id"],
            BlockType::Comment => &["id", "on", "by", "at", "state"],
        }
    }
}

/// How serious a problem is. Errors mean the block cannot be used as written; warnings mean it
/// can, but something is probably not what the author meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub severity: Severity,
    /// A short stable code, such as `sheet.ragged_row`, for tests and for tools that filter.
    pub code: &'static str,
    pub message: String,
}

impl Problem {
    fn error(code: &'static str, message: impl Into<String>) -> Self {
        Problem {
            severity: Severity::Error,
            code,
            message: message.into(),
        }
    }
    fn warn(code: &'static str, message: impl Into<String>) -> Self {
        Problem {
            severity: Severity::Warning,
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockError {
    /// The source is not a fenced block starting in the first column.
    NotFenced,
    Info(InfoError),
    /// The header names a type this version does not define.
    UnknownType(String),
}

impl fmt::Display for BlockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BlockError::NotFenced => write!(f, "not a fenced block"),
            BlockError::Info(e) => write!(f, "{e}"),
            BlockError::UnknownType(t) => write!(f, "`{t}` is not a block type"),
        }
    }
}

impl std::error::Error for BlockError {}

impl From<InfoError> for BlockError {
    fn from(e: InfoError) -> Self {
        BlockError::Info(e)
    }
}

/// A typed block taken apart. `body` uses `\n` whatever the file uses; [`render`](Self::render)
/// puts the file's own line ending back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedBlock {
    pub ty: BlockType,
    pub info: Info,
    pub body: String,
    /// False when the source ended without a closing fence.
    pub closed: bool,
}

/// Splits a fence line into its marker character, its length and the text after it.
fn fence_open(line: &str) -> Option<(char, usize, &str)> {
    let c = line.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = line.chars().take_while(|x| *x == c).count();
    (n >= 3).then(|| (c, n, &line[n..]))
}

/// Whether a fenced source starting in the first column names a block type of ours, judged by
/// the first word of its header alone. Cheap; used while splitting a document into nodes.
pub fn detect(src: &str) -> Option<BlockType> {
    let first = src.lines().next()?;
    let (_, _, info) = fence_open(first.trim_end())?;
    BlockType::parse(first_word(info)?)
}

impl TypedBlock {
    /// Reads a block from its source text (opening fence through closing fence).
    pub fn parse(src: &str) -> Result<TypedBlock, BlockError> {
        let mut lines = src.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l));
        let first = lines.next().ok_or(BlockError::NotFenced)?;
        let (fence_char, fence_len, info_text) = fence_open(first).ok_or(BlockError::NotFenced)?;
        let info = Info::parse(info_text)?;
        let ty = BlockType::parse(&info.kind)
            .ok_or_else(|| BlockError::UnknownType(info.kind.clone()))?;
        let mut body: Vec<&str> = lines.collect();
        let closed = body.last().is_some_and(|l| {
            let t = l.trim_end();
            t.len() >= fence_len && t.chars().all(|c| c == fence_char)
        });
        if closed {
            body.pop();
        }
        Ok(TypedBlock {
            ty,
            info,
            body: body.join("\n"),
            closed,
        })
    }

    /// Builds a block to be written. The fence is made long enough that nothing in the body can
    /// close it early.
    pub fn new(ty: BlockType, attrs: &[(&str, &str)], body: &str) -> TypedBlock {
        TypedBlock {
            ty,
            info: Info {
                kind: ty.name().to_string(),
                attrs: attrs.iter().map(|(k, v)| Attr::new(*k, *v)).collect(),
            },
            body: body
                .replace("\r\n", "\n")
                .trim_end_matches('\n')
                .to_string(),
            closed: true,
        }
    }

    /// The source text, with `eol` as the line ending and no trailing newline.
    pub fn render(&self, eol: &str) -> String {
        let fence_char = if self.info.has_backtick() { '~' } else { '`' };
        let fence = fence_char
            .to_string()
            .repeat(fence_len_for(&self.body, fence_char));
        let mut out = format!("{fence}{}", self.info.render());
        if !self.body.is_empty() {
            out.push_str(eol);
            out.push_str(&self.body.replace('\n', eol));
        }
        if self.closed {
            out.push_str(eol);
            out.push_str(&fence);
        }
        out
    }

    /// Everything wrong with the block, worst first is not promised; callers sort if they care.
    pub fn validate(&self) -> Vec<Problem> {
        let mut out = Vec::new();
        if !self.closed {
            out.push(Problem::warn(
                "block.unclosed",
                "the block has no closing fence",
            ));
        }
        let allowed = self.ty.allowed_attrs();
        for a in &self.info.attrs {
            if !allowed.contains(&a.key.as_str()) {
                out.push(Problem::warn(
                    "block.unknown_attr",
                    format!("`{}` is not an attribute of {}", a.key, self.ty.name()),
                ));
            }
        }
        if let Some(id) = self.info.id()
            && !valid_id(id)
        {
            out.push(Problem::error(
                "block.bad_id",
                format!("`{id}` is not a valid id; use lowercase letters, digits, - and _"),
            ));
        }
        match self.ty {
            BlockType::Sheet => self.validate_sheet(&mut out),
            BlockType::Tile => self.validate_tile(&mut out),
            BlockType::Plot => self.validate_plot(&mut out),
            BlockType::Chart => self.validate_chart(&mut out),
            BlockType::Comment => self.validate_comment(&mut out),
        }
        out
    }

    fn require<'a>(
        &'a self,
        key: &str,
        code: &'static str,
        out: &mut Vec<Problem>,
    ) -> Option<&'a str> {
        let v = self.info.get(key).filter(|v| !v.trim().is_empty());
        if v.is_none() {
            out.push(Problem::error(
                code,
                format!("{} needs {key}=", self.ty.name()),
            ));
        }
        v
    }

    fn check_from(&self, out: &mut Vec<Problem>) {
        if let Some(from) = self.require("from", "block.missing_from", out)
            && let Err(e) = RangeRef::parse(from)
        {
            out.push(Problem::error("block.bad_from", format!("from=: {e}")));
        }
    }

    fn validate_sheet(&self, out: &mut Vec<Problem>) {
        if self.info.get("name").is_some_and(|n| n.trim().is_empty()) {
            out.push(Problem::error("sheet.empty_name", "name= is empty"));
        }
        let sidecar = self.info.get("data");
        if let Some(p) = sidecar
            && let Err(e) = safe_rel_path(p)
        {
            out.push(Problem::error("sheet.bad_data_path", format!("data=: {e}")));
        }
        if let Some(r) = self.info.get("rows")
            && r.parse::<u64>().is_err()
        {
            out.push(Problem::error(
                "sheet.bad_rows",
                format!("rows= must be a whole number, got `{r}`"),
            ));
        }
        if sidecar.is_some() && self.info.get("rows").is_none() {
            out.push(Problem::warn(
                "sheet.sidecar_without_rows",
                "a sheet with data= should say how many rows it has with rows=",
            ));
        }
        let grid = match Grid::parse(&self.body) {
            Ok(g) => Some(g),
            Err(e) => {
                out.push(Problem::error(table_code(&e), e.to_string()));
                None
            }
        };
        if let Some(types) = self.info.get("types") {
            match parse_types(types) {
                Err(bad) => out.push(Problem::error(
                    "sheet.bad_type",
                    format!("`{bad}` is not a column type (text, int, num, bool, date, pct or a currency code)"),
                )),
                Ok(list) => {
                    if let Some(g) = &grid
                        && list.len() != g.columns()
                    {
                        out.push(Problem::error(
                            "sheet.types_mismatch",
                            format!("types= lists {} columns but the table has {}", list.len(), g.columns()),
                        ));
                    }
                }
            }
        }
    }

    fn validate_tile(&self, out: &mut Vec<Problem>) {
        self.check_from(out);
        self.check_size(out);
        if let Some(f) = self.require("fn", "tile.missing_fn", out)
            && !["sum", "count", "avg", "min", "max", "median"].contains(&f)
        {
            out.push(Problem::error(
                "tile.bad_fn",
                format!("`{f}` is not one of sum, count, avg, min, max, median"),
            ));
        }
        if self.info.get("label").is_some_and(|l| l.trim().is_empty()) {
            out.push(Problem::warn("tile.empty_label", "label= is empty"));
        }
        if let Some(fmt) = self.info.get("format")
            && crate::sheet::ColType::parse(fmt).is_none()
        {
            out.push(Problem::error(
                "tile.bad_format",
                format!("format= `{fmt}` is not a column type"),
            ));
        }
        if !self.body.trim().is_empty() {
            out.push(Problem::warn(
                "tile.body",
                "a tile takes no body; the text inside is ignored",
            ));
        }
    }

    /// `size=` is a view hint for a dashboard: how wide the block draws.
    fn check_size(&self, out: &mut Vec<Problem>) {
        if let Some(s) = self.info.get("size")
            && !["small", "wide", "full"].contains(&s)
        {
            out.push(Problem::warn(
                "block.bad_size",
                format!("size= `{s}` is not small, wide or full"),
            ));
        }
    }

    fn validate_plot(&self, out: &mut Vec<Problem>) {
        self.check_from(out);
        self.check_size(out);
        if let Some(t) = self.require("type", "plot.missing_type", out)
            && !["bar", "line", "pie", "donut", "area", "stacked", "scatter"].contains(&t)
        {
            out.push(Problem::error(
                "plot.bad_type",
                format!("`{t}` is not one of bar, line, pie, donut, area, stacked, scatter"),
            ));
        }
        self.require("x", "plot.missing_x", out);
        self.require("y", "plot.missing_y", out);
        if !self.body.trim().is_empty() {
            out.push(Problem::warn(
                "plot.body",
                "a plot takes no body; the text inside is ignored",
            ));
        }
    }

    fn validate_comment(&self, out: &mut Vec<Problem>) {
        if let Some(on) = self.require("on", "comment.missing_on", out)
            && !is_block_hash(on)
        {
            out.push(Problem::error(
                "comment.bad_on",
                format!("on= `{on}` is not a block hash (12 lowercase hex characters)"),
            ));
        }
        if let Some(at) = self.info.get("at")
            && chrono::NaiveDate::parse_from_str(at, "%Y-%m-%d").is_err()
            && chrono::DateTime::parse_from_rfc3339(at).is_err()
        {
            out.push(Problem::warn(
                "comment.bad_at",
                format!("at= `{at}` is not an ISO 8601 date"),
            ));
        }
        if let Some(state) = self.info.get("state")
            && !["open", "resolved"].contains(&state)
        {
            out.push(Problem::error(
                "comment.bad_state",
                format!("state= `{state}` is not open or resolved"),
            ));
        }
        if self.body.trim().is_empty() {
            out.push(Problem::warn("comment.empty", "the comment has no text"));
        }
    }

    fn validate_chart(&self, out: &mut Vec<Problem>) {
        match serde_json::from_str::<serde_json::Value>(&self.body) {
            Ok(serde_json::Value::Object(_)) => {}
            Ok(_) => out.push(Problem::error(
                "chart.not_object",
                "a chart body must be one JSON object",
            )),
            Err(e) => out.push(Problem::error(
                "chart.bad_json",
                format!("the chart body is not JSON: {e}"),
            )),
        }
    }
}

fn table_code(e: &TableError) -> &'static str {
    match e {
        TableError::Empty => "sheet.empty",
        TableError::NoDelimiter => "sheet.no_delimiter",
        TableError::RaggedRow { .. } => "sheet.ragged_row",
        TableError::BlankLine { .. } => "sheet.blank_line",
        TableError::TooManyColumns(_) => "sheet.too_many_columns",
        TableError::TooManyCells(_) => "sheet.too_many_cells",
        TableError::DuplicateHeader(_) => "sheet.duplicate_header",
    }
}

/// Twelve lowercase hex characters, the shape of [`crate::hash::block_hash`].
pub fn is_block_hash(s: &str) -> bool {
    s.len() == crate::hash::BLOCK_HASH_LEN
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `[a-z0-9][a-z0-9_-]{0,63}`
pub fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    id.len() <= 64
        && matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// One more than the longest run of `fence_char` in the body, and at least three.
fn fence_len_for(body: &str, fence_char: char) -> usize {
    let mut longest = 0;
    let mut run = 0;
    for c in body.chars() {
        if c == fence_char {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    (longest + 1).max(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(b: &TypedBlock) -> Vec<&'static str> {
        b.validate().iter().map(|p| p.code).collect()
    }

    const SHEET: &str = "```sheet name=\"Stock\" types=\"text,int,ugx,ugx\"\n| item | qty | price | total |\n|------|-----|-------|-------|\n| Sugar | 40 | 4200 | =B2*C2 |\n```";

    #[test]
    fn parses_and_renders_a_sheet() {
        let b = TypedBlock::parse(SHEET).unwrap();
        assert_eq!(b.ty, BlockType::Sheet);
        assert_eq!(b.info.get("name"), Some("Stock"));
        assert!(b.closed);
        assert!(codes(&b).is_empty(), "{:?}", b.validate());
        assert_eq!(
            b.render("\n"),
            SHEET,
            "an unedited block prints as it was written"
        );
    }

    #[test]
    fn render_uses_the_files_line_ending() {
        let b = TypedBlock::parse(SHEET).unwrap();
        let crlf = b.render("\r\n");
        assert!(crlf.contains("\r\n") && !crlf.replace("\r\n", "").contains('\n'));
        assert_eq!(TypedBlock::parse(&crlf).unwrap(), b);
    }

    #[test]
    fn detect_looks_only_at_the_header() {
        assert_eq!(detect("```sheet name=x\n```"), Some(BlockType::Sheet));
        assert_eq!(detect("~~~tile\n~~~"), Some(BlockType::Tile));
        assert_eq!(detect("```rust\nfn main(){}\n```"), None);
        assert_eq!(detect("```fields\n```"), None, "reserved, not yet defined");
        assert_eq!(
            detect("  ```sheet\n```"),
            None,
            "must start in the first column"
        );
        assert_eq!(detect("``sheet\n"), None);
    }

    #[test]
    fn a_body_with_fences_gets_a_longer_fence() {
        let b = TypedBlock::new(BlockType::Chart, &[], "{\"note\": \"```\"}");
        let text = b.render("\n");
        assert!(text.starts_with("````chart"), "{text}");
        assert_eq!(TypedBlock::parse(&text).unwrap().body, b.body);
    }

    #[test]
    fn a_backtick_in_a_value_switches_to_tildes() {
        let b = TypedBlock::new(
            BlockType::Tile,
            &[("from", "A1:A3"), ("fn", "sum"), ("label", "a`b")],
            "",
        );
        let text = b.render("\n");
        assert!(text.starts_with("~~~tile"), "{text}");
        assert_eq!(TypedBlock::parse(&text).unwrap().info, b.info);
    }

    #[test]
    fn unclosed_blocks_are_read_and_flagged() {
        let b = TypedBlock::parse("```tile from=A1:A3 fn=sum").unwrap();
        assert!(!b.closed);
        assert_eq!(codes(&b), ["block.unclosed"]);
        assert_eq!(b.render("\n"), "```tile from=A1:A3 fn=sum");
    }

    #[test]
    fn sheet_problems_are_specific() {
        let ragged = TypedBlock::parse("```sheet\n| a | b |\n|---|---|\n| 1 |\n```").unwrap();
        assert_eq!(codes(&ragged), ["sheet.ragged_row"]);
        let types =
            TypedBlock::parse("```sheet types=\"text,int\"\n| a |\n|---|\n| 1 |\n```").unwrap();
        assert_eq!(codes(&types), ["sheet.types_mismatch"]);
        let bad =
            TypedBlock::parse("```sheet types=\"text,water\"\n| a | b |\n|---|---|\n```").unwrap();
        assert_eq!(codes(&bad), ["sheet.bad_type"]);
        let path =
            TypedBlock::parse("```sheet data=\"../secret.zst\" rows=9\n| a |\n|---|\n```").unwrap();
        assert_eq!(codes(&path), ["sheet.bad_data_path"]);
        let norows =
            TypedBlock::parse("```sheet data=\"_data/suite/s.csv.zst\"\n| a |\n|---|\n```")
                .unwrap();
        assert_eq!(codes(&norows), ["sheet.sidecar_without_rows"]);
        let id = TypedBlock::parse("```sheet id=Bad_Id\n| a |\n|---|\n```").unwrap();
        assert_eq!(codes(&id), ["block.bad_id"]);
        let extra = TypedBlock::parse("```sheet colour=red\n| a |\n|---|\n```").unwrap();
        assert_eq!(codes(&extra), ["block.unknown_attr"]);
    }

    #[test]
    fn tiles_and_plots_are_checked() {
        let ok = TypedBlock::parse(
            "```tile from=\"[[Stock]]!D2:D40\" fn=sum label=\"Stock value\" format=ugx\n```",
        )
        .unwrap();
        assert!(codes(&ok).is_empty(), "{:?}", ok.validate());
        let missing = TypedBlock::parse("```tile\n```").unwrap();
        assert_eq!(codes(&missing), ["block.missing_from", "tile.missing_fn"]);
        let badfn = TypedBlock::parse("```tile from=A1:A2 fn=total\n```").unwrap();
        assert_eq!(codes(&badfn), ["tile.bad_fn"]);
        let badfrom = TypedBlock::parse("```tile from=D fn=sum\n```").unwrap();
        assert_eq!(codes(&badfrom), ["block.bad_from"]);
        let withbody = TypedBlock::parse("```tile from=A1:A2 fn=sum\nhello\n```").unwrap();
        assert_eq!(codes(&withbody), ["tile.body"]);

        let plot =
            TypedBlock::parse("```plot type=bar from=\"[[Stock]]!A2:D40\" x=item y=total\n```")
                .unwrap();
        assert!(codes(&plot).is_empty(), "{:?}", plot.validate());
        let noxy = TypedBlock::parse("```plot type=radar from=A1:B2\n```").unwrap();
        assert_eq!(
            codes(&noxy),
            ["plot.bad_type", "plot.missing_x", "plot.missing_y"]
        );
    }

    #[test]
    fn the_size_hint_of_a_dashboard_block_is_known_and_checked() {
        let tile = TypedBlock::parse("```tile from=A1:A2 fn=sum size=wide\n```").unwrap();
        assert!(codes(&tile).is_empty(), "{:?}", tile.validate());
        let plot = TypedBlock::parse("```plot type=bar from=A1:B2 x=a y=b size=full\n```").unwrap();
        assert!(codes(&plot).is_empty(), "{:?}", plot.validate());
        let bad = TypedBlock::parse("```tile from=A1:A2 fn=sum size=huge\n```").unwrap();
        assert_eq!(codes(&bad), ["block.bad_size"]);
    }

    #[test]
    fn chart_bodies_are_one_json_object() {
        let ok =
            TypedBlock::parse("```chart\n{\"type\":\"bar\",\"labels\":[\"a\"],\"series\":[]}\n```")
                .unwrap();
        assert!(codes(&ok).is_empty());
        let arr = TypedBlock::parse("```chart\n[1,2]\n```").unwrap();
        assert_eq!(codes(&arr), ["chart.not_object"]);
        let junk = TypedBlock::parse("```chart\nnope\n```").unwrap();
        assert_eq!(codes(&junk), ["chart.bad_json"]);
    }

    #[test]
    fn parse_refuses_what_is_not_a_block() {
        assert_eq!(TypedBlock::parse("plain"), Err(BlockError::NotFenced));
        assert!(matches!(
            TypedBlock::parse("```rust\n```"),
            Err(BlockError::Info(_)) | Err(BlockError::UnknownType(_))
        ));
        assert!(matches!(
            TypedBlock::parse("```sheet stock\n```"),
            Err(BlockError::Info(InfoError::BareWord(_)))
        ));
    }

    #[test]
    fn comments_are_checked_and_keep_unknown_attributes() {
        let src = "```comment on=0123456789ab by=\"Sam\" at=2026-11-20 state=open mood=hopeful\nCheck this figure.\n```";
        let b = TypedBlock::parse(src).unwrap();
        assert_eq!(b.ty, BlockType::Comment);
        assert_eq!(codes(&b), ["block.unknown_attr"]);
        assert_eq!(b.render("\n"), src, "unknown attributes survive as written");
        let bad = TypedBlock::parse("```comment on=XYZ state=maybe at=soon\n```").unwrap();
        assert_eq!(
            codes(&bad),
            [
                "comment.bad_on",
                "comment.bad_at",
                "comment.bad_state",
                "comment.empty"
            ]
        );
        let none = TypedBlock::parse("```comment\nhi\n```").unwrap();
        assert_eq!(codes(&none), ["comment.missing_on"]);
    }
}
