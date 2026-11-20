//! Making an office note from structured content, and rendering a note as an office file.
//!
//! `create_note` writes a native hqmd note (the truth). `export_note` renders it as `.xlsx`,
//! `.csv`, `.docx` or `.pptx` at a path the caller picked inside the vault; the file is a render
//! that can always be made again from the note.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use hq_suite_calc::{Value, Workbook};
use hq_suite_md::{
    BlockType, ColType, Document, Grid, Severity, TypedBlock, frontmatter, has_errors, validate,
};
use hq_suite_store::{SuitePaths, fsx};
use hq_suite_xlsx::{SheetSpec, write_xlsx_with_charts};
use serde::Deserialize;

use crate::docs;
use crate::shrink::vault_relative;

/// Cells one `office_create` call may write, so a runaway model cannot fill the vault.
const MAX_CREATE_CELLS: usize = 20_000;
/// The folder a note goes in when the caller names none.
const DEFAULT_FOLDER: &str = "Notebooks/Office";

#[derive(Debug, Clone, Deserialize)]
pub struct ColumnInput {
    pub name: String,
    /// `text`, `int`, `num`, `bool`, `date`, `pct` or a three-letter currency code. Default `text`.
    #[serde(rename = "type", default)]
    pub ty: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SheetInput {
    #[serde(default)]
    pub name: Option<String>,
    pub columns: Vec<ColumnInput>,
    /// Rows of cells. A string starting with `=` is a formula.
    pub rows: Vec<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Sheet,
    Doc,
    Slides,
}

impl Kind {
    pub fn parse(s: &str) -> Result<Kind> {
        match s {
            "sheet" => Ok(Kind::Sheet),
            "doc" => Ok(Kind::Doc),
            "slides" => Ok(Kind::Slides),
            other => bail!("kind must be sheet, doc or slides, not `{other}`"),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Kind::Sheet => "sheet",
            Kind::Doc => "doc",
            Kind::Slides => "slides",
        }
    }
}

fn cell_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A file-system safe name for a title.
fn stem_of(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_()".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    let s = s.trim_matches(|c: char| c == '-' || c == ' ');
    if s.is_empty() {
        "Untitled".into()
    } else {
        s.chars().take(80).collect()
    }
}

fn yaml_quoted(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace(['\n', '\r'], " ")
    )
}

fn sheet_block(input: &SheetInput, index: usize) -> Result<String> {
    let name = input
        .name
        .clone()
        .unwrap_or_else(|| format!("Sheet{}", index + 1));
    let types: Vec<String> = input
        .columns
        .iter()
        .map(|c| c.ty.clone().unwrap_or_else(|| "text".into()).to_lowercase())
        .collect();
    for t in &types {
        if ColType::parse(t).is_none() {
            bail!(
                "column type `{t}` is not one of text, int, num, bool, date, pct or a three-letter currency code"
            );
        }
    }
    let width = input.columns.len();
    let mut rows = Vec::new();
    for (i, row) in input.rows.iter().enumerate() {
        if row.len() != width {
            bail!(
                "row {} has {} cells but there are {width} columns",
                i + 1,
                row.len()
            );
        }
        rows.push(row.iter().map(cell_text).collect::<Vec<_>>());
    }
    let grid = Grid {
        header: input.columns.iter().map(|c| c.name.clone()).collect(),
        rows,
    };
    let types_joined = types.join(",");
    let block = TypedBlock::new(
        BlockType::Sheet,
        &[("name", name.as_str()), ("types", types_joined.as_str())],
        &grid.render("\n"),
    );
    Ok(block.render("\n"))
}

/// Writes a native note and returns its vault-relative path. Never replaces a note.
pub fn create_note(
    vault: &Path,
    kind: Kind,
    title: &str,
    folder: Option<&str>,
    markdown: Option<&str>,
    sheets: &[SheetInput],
) -> Result<String> {
    let title = title.trim();
    if title.is_empty() {
        bail!("a title is needed");
    }
    let body = match kind {
        Kind::Sheet => {
            if sheets.is_empty() {
                bail!("a sheet note needs `sheets`: columns and rows");
            }
            let cells: usize = sheets
                .iter()
                .map(|s| s.columns.len() * (s.rows.len() + 1))
                .sum();
            if cells > MAX_CREATE_CELLS {
                bail!("{cells} cells is more than one call may write ({MAX_CREATE_CELLS})");
            }
            let blocks: Result<Vec<String>> = sheets
                .iter()
                .enumerate()
                .map(|(i, s)| sheet_block(s, i))
                .collect();
            format!("# {title}\n\n{}\n", blocks?.join("\n\n"))
        }
        Kind::Doc | Kind::Slides => {
            let text = markdown.map(str::trim).filter(|m| !m.is_empty());
            let text = text.context("`content` (Markdown) is needed for a document or deck")?;
            if text.starts_with("# ") {
                format!("{text}\n")
            } else {
                format!("# {title}\n\n{text}\n")
            }
        }
    };
    let note = format!(
        "---\ntitle: {}\nsuite_kind: {}\nsuite_fidelity: native\n---\n\n{body}",
        yaml_quoted(title),
        kind.name()
    );
    let doc = Document::parse(&note);
    let problems = validate(&doc);
    if has_errors(&problems) {
        let first = problems.iter().find(|d| d.severity == Severity::Error);
        bail!(
            "the note would not be valid hqmd: {}",
            first.map_or("unknown", |d| d.message.as_str())
        );
    }
    let dir = folder.unwrap_or(DEFAULT_FOLDER).trim_matches('/');
    let paths = SuitePaths::new(vault);
    let stem = stem_of(title);
    let rel = (1..)
        .map(|n| match n {
            1 => format!("{dir}/{stem}.md"),
            n => format!("{dir}/{stem} {n}.md"),
        })
        .find(|r| paths.resolve(r).is_ok_and(|p| !p.exists()))
        .context("no free note name")?;
    let target = paths.resolve(&rel)?;
    fsx::atomic_write(&target, note.as_bytes())?;
    Ok(rel)
}

/// What an export made, in the words a person reads.
#[derive(Debug)]
pub struct Exported {
    pub fidelity: String,
}

fn sheets_of(text: &str) -> Result<Vec<SheetSpec>> {
    let doc = Document::parse(text);
    let mut out = Vec::new();
    for (_, parsed) in doc.typed_blocks() {
        let Ok(block) = parsed else { continue };
        if block.ty != BlockType::Sheet {
            continue;
        }
        if block.info.get("data").is_some() {
            bail!("a sheet that keeps its rows in a sidecar cannot be exported yet");
        }
        let grid = Grid::parse(&block.body).map_err(|e| anyhow::anyhow!("{e}"))?;
        let types = block
            .info
            .get("types")
            .map(|t| hq_suite_md::sheet::parse_types(t).unwrap_or_default())
            .unwrap_or_default();
        out.push(SheetSpec {
            name: block.info.get("name").unwrap_or("Sheet").to_string(),
            header: grid.header,
            types,
            rows: grid.rows,
        });
    }
    if out.is_empty() {
        bail!("the note has no sheet block, so there is nothing to put in a spreadsheet");
    }
    Ok(out)
}

fn csv_cell(s: &str) -> String {
    // A text that a spreadsheet program would run as a formula is kept as text.
    let guarded = if s.starts_with(['=', '+', '@']) {
        format!("'{s}")
    } else {
        s.to_string()
    };
    if guarded.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

/// The first sheet's cells, formulas replaced by what they calculate.
fn csv_of(spec: &SheetSpec) -> String {
    let mut book = Workbook::new();
    let si = book.add_sheet("s").unwrap_or(0);
    for (r, row) in spec.rows.iter().enumerate() {
        for (c, text) in row.iter().enumerate() {
            if !text.trim().is_empty() {
                book.set(si, c as u32, r as u32 + 1, text);
            }
        }
    }
    let mut lines = vec![
        spec.header
            .iter()
            .map(|h| csv_cell(h))
            .collect::<Vec<_>>()
            .join(","),
    ];
    for (r, row) in spec.rows.iter().enumerate() {
        let cells: Vec<String> = (0..row.len())
            .map(|c| match book.get(si, c as u32, r as u32 + 1) {
                Value::Empty => String::new(),
                v => csv_cell(&v.to_string()),
            })
            .collect();
        lines.push(cells.join(","));
    }
    lines.join("\n") + "\n"
}

/// The note's sheet blocks as an `.xlsx` or a `.csv`, in memory, with the one-line statement of what
/// was kept. `None` when the note has no sheet block (the caller falls back to its tables).
pub fn sheet_bytes(text: &str, format: &str) -> Option<Result<(Vec<u8>, String)>> {
    let specs = sheets_of(text).ok()?;
    Some(match format {
        "xlsx" => xlsx_with_charts(text, "", &specs),
        "csv" => Ok((csv_of(&specs[0]).into_bytes(), csv_fidelity(specs.len()))),
        other => Err(anyhow::anyhow!("`{other}` is not a sheet format")),
    })
}

/// The workbook with the note's plain `plot` blocks as native charts. A plot that filters or groups
/// has no range behind it and is left out, and the statement says how many.
fn xlsx_with_charts(text: &str, note: &str, specs: &[SheetSpec]) -> Result<(Vec<u8>, String)> {
    let native = crate::charts::native_charts(text, note);
    let (bytes, report) = write_xlsx_with_charts(specs, &native.charts)?;
    let mut line = report.fidelity_line();
    if native.left_out > 0 {
        line.push_str(&format!(
            " {} chart{} that filter or group rows {} not included.",
            native.left_out,
            if native.left_out == 1 { "" } else { "s" },
            if native.left_out == 1 { "is" } else { "are" },
        ));
    }
    Ok((bytes, line))
}

fn csv_fidelity(sheets: usize) -> String {
    let more = if sheets > 1 {
        " Only the first sheet is included."
    } else {
        ""
    };
    format!("Values only: formulas are saved as their results and formats as plain text.{more}")
}

/// Renders the note at `note_rel` as `format` (`xlsx`, `csv`, `docx` or `pptx`) at `out`, a path
/// inside the vault that does not exist yet.
pub fn export_note(vault: &Path, note_rel: &str, format: &str, out: &Path) -> Result<Exported> {
    let paths = SuitePaths::new(vault);
    let note: PathBuf = paths.resolve(note_rel)?;
    if !note_rel.ends_with(".md") {
        bail!("{note_rel} is not a note (a .md file)");
    }
    let text =
        std::fs::read_to_string(&note).with_context(|| format!("{note_rel} cannot be read"))?;
    match format {
        "xlsx" => {
            let (bytes, fidelity) = xlsx_with_charts(&text, note_rel, &sheets_of(&text)?)?;
            fsx::atomic_write(out, &bytes)?;
            Ok(Exported { fidelity })
        }
        "csv" => {
            let specs = sheets_of(&text)?;
            fsx::atomic_write(out, csv_of(&specs[0]).as_bytes())?;
            let more = if specs.len() > 1 {
                " Only the first sheet is included."
            } else {
                ""
            };
            Ok(Exported {
                fidelity: format!(
                    "Values only: formulas are saved as their results and formats as plain text.{more}"
                ),
            })
        }
        "docx" => {
            let rel = vault_relative(vault, &note)?;
            docs::render(vault, &vault.join(rel), Some(out))?;
            Ok(Exported {
                fidelity: "Headings, text, tables and pictures were kept. Sheets and charts appear as plain text.".into(),
            })
        }
        "pptx" => {
            let made = docs::render_slides(vault, &note, Some(out))?;
            let mut line = format!("{} slide(s) after the title.", made.cards);
            if made.pictures_left_out > 0 {
                line.push_str(&format!(
                    " {} picture(s) could not be placed.",
                    made.pictures_left_out
                ));
            }
            Ok(Exported { fidelity: line })
        }
        other => bail!("format must be xlsx, csv, docx or pptx here, not `{other}`"),
    }
}

/// The title of a note, from its frontmatter.
pub fn note_title(text: &str) -> Option<String> {
    frontmatter::get(Document::parse(text).frontmatter(), "title")
}
