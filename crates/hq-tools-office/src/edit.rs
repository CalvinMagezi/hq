//! `office_edit`: change part of a note that already exists, instead of making a copy.
//!
//! Every action goes through `hq-suite-adapter::edit`, which applies the edit as an hqmd patch: a
//! block or cell that changed since the agent last read it is refused and nothing is written, and
//! the rest of the note comes back byte for byte. A call that deletes anything, or touches more
//! than 50 cells, is answered with a preview and a `token` and writes nothing; the same call with
//! the token applies it if the note has not moved. This file is the tool's face (name, words,
//! schema) and its write path through the runtime's note seam.

use std::path::PathBuf;

use anyhow::{Context, Result};
use async_trait::async_trait;
use hq_suite_adapter::edit::{self, Gate, Step};
use serde_json::{Value, json};

use hq_tool_api::registry::HqTool;

pub struct OfficeEditTool {
    pub(crate) vault: PathBuf,
    pub(crate) seam: Option<hq_tool_api::office::SharedSeam>,
}

/// One call, start to finish: a read or a preview answers at once; a write runs under the note's
/// lock (with the seam) and tells open pages the note moved.
fn run(
    vault: &std::path::Path,
    seam: Option<&hq_tool_api::office::SharedSeam>,
    args: &Value,
) -> Result<Value> {
    let note = args.get("note").and_then(Value::as_str).unwrap_or_default();
    let out = match edit::first_step(vault, args, Gate::Agent)? {
        Step::Done(v) => return Ok(v),
        Step::Write => match seam {
            Some(seam) if !note.is_empty() => {
                // A write: under the note's lock, keeping the text it replaces and recording the caller.
                let mut out = None;
                seam.write(note, args, &mut || {
                    out = Some(edit::apply(vault, args, Gate::Agent)?);
                    Ok(())
                })?;
                out.context("the edit produced no result")?
            }
            _ => edit::apply(vault, args, Gate::Agent)?,
        },
    };
    if let Some(revision) = out.get("revision").and_then(Value::as_str) {
        hq_tool_api::office::notify_note_changed(note, revision);
    }
    Ok(out)
}

#[async_trait]
impl HqTool for OfficeEditTool {
    fn name(&self) -> &str {
        "office_edit"
    }

    fn description(&self) -> &str {
        "Change part of a sheet, document, deck or dashboard note that already exists, instead of \
         making a new one. Read first: `blocks` lists the note's blocks (each has an `index` and \
         a `hash`), `cards` lists a deck's cards (each has an `index` and a `hash`). Documents: \
         `replace_block`, `insert_block` (after `after_index`, or at the top), `delete_block`. \
         Sheets (`sheet` is the sheet's number from 0; `row` 0 is the header row; `col` from 0; a \
         formula starts with `=`): `set_cell`, `set_cells` (a rectangle of `values` from `row`, \
         `col`, with the `expected` text of each cell), `add_rows` (at the end, or before `at`), \
         `delete_rows`, `add_column` (`name`, `type`, `default`; on the right, or before column `at`), `delete_column` (`col`), `set_column_type`, \
         `add_sheet` (`name`, `columns`, optional `types` and `rows`). The sheet actions that \
         change structure quote the sheet block's `hash`. Rows or columns inserted or deleted move the \
         formulas, tiles and plots that read what is below or to the right, and the answer says how many. Decks: `add_card` \
         (`title`, `markdown`, after `after_card`), `replace_card`, `move_card` (`to`), \
         `delete_card`, `set_notes` (`text`), `set_card_style` (`transition`, `background`, \
         `color`, `layout`; null clears one); a card is quoted by its `card` number and `hash`. \
         Dashboards: `add_tile` (`from`, `fn`, `where`, `label`, `format`) and `add_plot` (`type`, \
         `from`, `x`, `y`, `where`, `group`, `title`), checked by drawing them before anything is \
         written. If the note changed since you read it the edit is refused and nothing is \
         written: read again and retry. Review notes: `comments` lists a note's comments, `add_comment` \
         (`on` the block's `hash`, `text`, optional `by`) leaves one on a block and `resolve_comment` \
         (`index`, `hash`) closes it; comments are never put in an exported file. A call that deletes anything, or touches more than 50 \
         cells, writes nothing and returns a preview with a `token`; call again with the same \
         arguments and that `token` to apply it. `focus` (`note`, optional `block`, `row`, `col`) \
         is what the owner has selected, if they passed it; it is only a pointer and is echoed \
         back. Call `office_export` afterwards to refresh the file the owner opens."
    }

    fn parameters(&self) -> Value {
        // (name, type, description): one line each keeps the schema readable and the macro shallow.
        const PROPS: &[(&str, &str, &str)] = &[
            ("note", "string", "Vault-relative path of the note."),
            ("index", "integer", "Block index, from `blocks`."),
            (
                "after_index",
                "integer",
                "insert_block, add_sheet, add_tile, add_plot: place after this block (needs `hash`; omit for the top of a document, the end for the others).",
            ),
            (
                "hash",
                "string",
                "The block's (or, for deck actions, the card's) hash, from `blocks` or `cards`.",
            ),
            ("markdown", "string", "The new block's or card's Markdown."),
            (
                "on",
                "string",
                "add_comment: the hash of the block the comment is about.",
            ),
            (
                "by",
                "string",
                "add_comment: who the comment is from, as a label (default HQ).",
            ),
            ("sheet", "integer", "Which sheet block, from 0."),
            (
                "row",
                "integer",
                "Table row, 0 is the header. delete_rows: the first row to delete.",
            ),
            (
                "col",
                "integer",
                "Column, from 0. delete_column: the column to delete.",
            ),
            (
                "at",
                "integer",
                "add_rows: insert before this table row (1 is the first row under the header); omit to add at the end. add_column: insert before this column (from 0); omit to add on the right.",
            ),
            (
                "count",
                "integer",
                "delete_rows: how many rows (default 1).",
            ),
            (
                "text",
                "string",
                "set_cell: the new cell text. set_notes: the speaker notes (empty clears them). add_comment: the comment.",
            ),
            (
                "name",
                "string",
                "add_column: the header. add_sheet: the sheet's name.",
            ),
            (
                "type",
                "string",
                "add_column, set_column_type: text, int, num, bool, date, pct or a currency code. add_plot: bar, line, pie, donut, area, stacked or scatter.",
            ),
            (
                "default",
                "string",
                "add_column: the value every row starts with (not a formula).",
            ),
            ("card", "integer", "Card number from 0, from `cards`."),
            (
                "after_card",
                "integer",
                "add_card: insert after this card (needs `hash`); omit to add at the end.",
            ),
            ("to", "integer", "move_card: the card's new number."),
            ("title", "string", "A card's or a plot's title."),
            (
                "from",
                "string",
                "add_tile, add_plot: the range, such as D2:D40 or [[note]]!D2:D40.",
            ),
            (
                "fn",
                "string",
                "add_tile: sum, count, avg, min, max or median.",
            ),
            (
                "where",
                "string",
                "add_tile, add_plot: a filter such as region=North and qty>3.",
            ),
            (
                "group",
                "string",
                "add_plot: a column to draw one series per value of.",
            ),
            ("x", "string", "add_plot: the labels column."),
            ("y", "string", "add_plot: the values column."),
            (
                "label",
                "string",
                "add_tile: the tile's label (add_plot: its title).",
            ),
            (
                "format",
                "string",
                "add_tile: how to show the number, such as ugx or int.",
            ),
            (
                "token",
                "string",
                "The token a preview returned, to apply that exact change.",
            ),
        ];
        const STYLE: &[(&str, &str)] = &[
            (
                "transition",
                "set_card_style: fade, push, wipe, split, cover, dissolve or none, then slow, medium or fast.",
            ),
            ("background", "set_card_style: #RRGGBB."),
            ("color", "set_card_style: the text colour, #RRGGBB."),
            ("layout", "set_card_style: content or two-column."),
        ];
        let mut props = serde_json::Map::new();
        props.insert(
            "action".into(),
            json!({"type": "string", "enum": edit::ACTIONS}),
        );
        for (name, ty, description) in PROPS {
            props.insert(
                (*name).into(),
                json!({"type": ty, "description": description}),
            );
        }
        for (name, description) in STYLE {
            props.insert(
                (*name).into(),
                json!({"type": ["string", "null"], "description": description}),
            );
        }
        for (name, description) in [
            (
                "values",
                "set_cells: a rectangle of new cell texts or numbers.",
            ),
            (
                "rows",
                "add_rows, add_sheet: the rows, each a list of cells.",
            ),
        ] {
            props.insert(
                name.into(),
                json!({"type": "array", "items": {"type": "array"}, "description": description}),
            );
        }
        props.insert(
            "expected".into(),
            json!({"description": "set_cell: the text the cell holds now. set_cells: a grid of what each cell holds now. delete_rows: optionally, the rows being deleted. delete_column: optionally, the column's header."}),
        );
        for (name, description) in [
            ("columns", "add_sheet: column names."),
            ("types", "add_sheet: a type per column."),
        ] {
            props.insert(
                name.into(),
                json!({"type": "array", "items": {"type": "string"}, "description": description}),
            );
        }
        props.insert(
            "focus".into(),
            json!({
                "type": "object",
                "description": "What the owner has selected, if given. Checked against the note and echoed back; it is a pointer, nothing more.",
                "properties": {
                    "note": {"type": "string"},
                    "block": {"type": "integer"},
                    "row": {"type": "integer"},
                    "col": {"type": "integer"}
                }
            }),
        );
        json!({"type": "object", "required": ["action", "note"], "properties": props})
    }

    fn category(&self) -> &str {
        "office"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let vault = self.vault.clone();
        let seam = self.seam.clone();
        tokio::task::spawn_blocking(move || run(&vault, seam.as_ref(), &args)).await?
    }
}

#[cfg(test)]
#[path = "edit_tests.rs"]
mod tests;
