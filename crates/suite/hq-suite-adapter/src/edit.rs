//! `office_edit` and its REST route: one entry for every way of changing part of a note.
//!
//! An action is planned first (`edit_plan::Planned`: the note as it would be, a summary, how many
//! cells it touches, whether it removes anything) and only then written. That is what lets an
//! agent's call that deletes anything, or touches more than 50 cells, come back as a preview with
//! a `token`; the same call with the token applies it, and only if the note is still at the
//! revision the preview was made from. The owner's own edits from the apps are one step.

use std::path::Path;

use anyhow::{Context, Result, bail};
use hq_suite_md::{Addr, Document, Op};
use hq_suite_store::fsx;
use serde_json::{Value, json};

use crate::blocks::{self, BlocksView, check_markdown, view_of};
use crate::edit_plan::{
    PREVIEW_CELLS, Planned, check_focus, load, opt_str, opt_u64, patch, req_str, req_u64, token_for,
};
use crate::{comments, dash_ops, sheet_ops, sheets, slide_ops};

/// Longest source excerpt a listing returns per block, so a read stays small.
const EXCERPT_CHARS: usize = 400;

/// Every action, for the sentence that names the unknown one.
pub const ACTIONS: &[&str] = &[
    "blocks",
    "replace_block",
    "insert_block",
    "delete_block",
    "set_cell",
    "set_cells",
    "add_rows",
    "delete_rows",
    "add_column",
    "delete_column",
    "set_column_type",
    "add_sheet",
    "cards",
    "add_card",
    "replace_card",
    "move_card",
    "delete_card",
    "set_notes",
    "set_card_style",
    "add_tile",
    "add_plot",
    "comments",
    "add_comment",
    "resolve_comment",
];

/// Who is asking. An agent's change that deletes or is large is previewed first; the owner,
/// editing in an app, sees the result at once and has the note's history to go back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    Agent,
    Owner,
}

/// What the first look at a call finds.
pub enum Step {
    /// Nothing to write: a read, or a preview. This is the reply.
    Done(Value),
    /// A change to write, which [`apply`] does.
    Write,
}

/// A block listing as an agent reads it.
pub fn listing(view: &BlocksView) -> Value {
    let blocks: Vec<Value> = view
        .blocks
        .iter()
        .map(|b| {
            let src: String = b.src.chars().take(EXCERPT_CHARS).collect();
            json!({"index": b.index, "hash": b.hash, "kind": b.kind, "src": src})
        })
        .collect();
    json!({"revision": view.revision, "blocks": blocks})
}

/// Whether a call changes the note, so the runtime takes the note's lock for it.
pub fn is_write(args: &Value) -> bool {
    !matches!(
        args.get("action").and_then(Value::as_str),
        Some("blocks" | "cards" | "comments")
    )
}

/// The revision of the note's file as it is now, for the `note_changed` event and a caller that
/// wants to know whether it moved.
pub fn note_revision(vault: &Path, note: &str) -> Option<String> {
    let (_, doc) = load(vault, note).ok()?;
    Some(doc.revision())
}

fn comment_listing(doc: &Document) -> Value {
    serde_json::to_value(comments::view(doc)).unwrap_or(Value::Null)
}

fn block_listing(doc: &Document) -> Value {
    listing(&view_of(doc))
}

/// Plans one write action against the note as it is now.
fn plan(vault: &Path, doc: &Document, note: &str, args: &Value) -> Result<Planned> {
    let action = req_str(args, "action")?;
    let index = |key: &str| -> Result<usize> { Ok(req_u64(args, key)? as usize) };
    Ok(match action {
        "replace_block" => {
            let markdown = req_str(args, "markdown")?;
            check_markdown(markdown)?;
            let n = index("index")?;
            let new = patch(
                doc,
                vec![Op::Replace {
                    at: Addr {
                        index: n,
                        hash: req_str(args, "hash")?.to_string(),
                    },
                    markdown: markdown.to_string(),
                }],
            )?;
            let extra = block_listing(&new);
            Planned::new(new, format!("Replaced block {n}")).with(extra)
        }
        "insert_block" => {
            let markdown = req_str(args, "markdown")?;
            check_markdown(markdown)?;
            let at = match opt_u64(args, "after_index")? {
                Some(i) => Some(Addr {
                    index: i as usize,
                    hash: req_str(args, "hash")?.to_string(),
                }),
                None => None,
            };
            let said = at.as_ref().map_or("the top".to_string(), |a| {
                format!("after block {}", a.index)
            });
            let new = patch(
                doc,
                vec![Op::InsertAfter {
                    at,
                    markdown: markdown.to_string(),
                }],
            )?;
            let extra = block_listing(&new);
            Planned::new(new, format!("Inserted a block at {said}")).with(extra)
        }
        "delete_block" => {
            let n = index("index")?;
            let new = patch(
                doc,
                vec![Op::Delete {
                    at: Addr {
                        index: n,
                        hash: req_str(args, "hash")?.to_string(),
                    },
                }],
            )?;
            let extra = block_listing(&new);
            Planned::new(new, format!("Deleted block {n}"))
                .removing()
                .with(extra)
        }
        "set_cell" => {
            let sheet = opt_u64(args, "sheet")?.unwrap_or(0) as usize;
            let (row, col) = (req_u64(args, "row")? as u32, req_u64(args, "col")? as u32);
            let new = sheets::cell_patched(
                doc,
                sheet,
                row,
                col,
                req_str(args, "expected")?,
                req_str(args, "text")?,
            )?;
            Planned::new(new, format!("Set one cell (row {row}, column {col})")).touching(1)
        }
        "set_cells" => sheet_ops::set_cells(doc, args)?,
        "add_rows" => sheet_ops::add_rows(doc, note, args)?,
        "delete_rows" => sheet_ops::delete_rows(doc, note, args)?,
        "add_column" => sheet_ops::add_column(doc, note, args)?,
        "delete_column" => sheet_ops::delete_column(doc, note, args)?,
        "set_column_type" => sheet_ops::set_column_type(doc, args)?,
        "add_sheet" => sheet_ops::add_sheet(doc, args)?,
        "add_card" => slide_ops::add_card(doc, args)?,
        "replace_card" => slide_ops::replace_card(doc, args)?,
        "move_card" => slide_ops::move_card(doc, args)?,
        "delete_card" => slide_ops::delete_card(doc, args)?,
        "set_notes" => slide_ops::set_notes(doc, args)?,
        "set_card_style" => slide_ops::set_card_style(doc, args)?,
        "add_tile" => dash_ops::add_tile(vault, doc, note, args)?,
        "add_plot" => dash_ops::add_plot(vault, doc, note, args)?,
        "add_comment" => {
            // The label is typed text, never an identity; an agent's defaults to "HQ".
            let by = opt_str(args, "by").unwrap_or("HQ");
            let at = opt_str(args, "at")
                .map(str::to_string)
                .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
            let new = comments::add_in(doc, req_str(args, "on")?, by, &at, req_str(args, "text")?)?;
            let extra = comment_listing(&new);
            Planned::new(new, "Added a comment".to_string()).with(extra)
        }
        "resolve_comment" => {
            let n = index("index")?;
            let new = comments::resolve_in(doc, n, req_str(args, "hash")?)?;
            let extra = comment_listing(&new);
            Planned::new(new, format!("Resolved comment {n}")).with(extra)
        }
        other => bail!(
            "unknown action `{other}`; use one of: {}",
            ACTIONS.join(", ")
        ),
    })
}

fn with_focus(vault: &Path, args: &Value, mut out: Value) -> Result<Value> {
    if let Some(f) = args.get("focus").filter(|f| !f.is_null()) {
        out["focus"] = check_focus(vault, f)?;
    }
    Ok(out)
}

fn preview_of(doc: &Document, args: &Value, planned: &Planned, gate: Gate) -> Value {
    let mut out = json!({
        "preview": true,
        "applied": false,
        "summary": planned.summary,
        "touched": planned.touched,
        "destructive": planned.destructive,
        "revision": doc.revision(),
    });
    if gate == Gate::Agent {
        out["token"] = json!(token_for(&doc.revision(), args));
        out["next"] = json!(format!(
            "Nothing was written. To apply this, call office_edit again with the same arguments and this `token`; it applies only if the note has not changed since. Changes that delete anything or touch more than {PREVIEW_CELLS} cells are shown first."
        ));
    }
    out
}

/// The first look at a call: a read answers at once, a change that must be shown first comes back
/// as a preview, anything else is left for [`apply`].
pub fn first_step(vault: &Path, args: &Value, gate: Gate) -> Result<Step> {
    let action = req_str(args, "action")?;
    let note = req_str(args, "note")?;
    match action {
        "blocks" => Ok(Step::Done(with_focus(
            vault,
            args,
            listing(&blocks::list(vault, note)?),
        )?)),
        "cards" => {
            let (_, doc) = load(vault, note)?;
            Ok(Step::Done(with_focus(vault, args, slide_ops::list(&doc)?)?))
        }
        "comments" => {
            let (_, doc) = load(vault, note)?;
            Ok(Step::Done(with_focus(vault, args, comment_listing(&doc))?))
        }
        _ => {
            if let Some(f) = args.get("focus").filter(|f| !f.is_null()) {
                check_focus(vault, f)?;
            }
            let (_, doc) = load(vault, note)?;
            let planned = plan(vault, &doc, note, args)?;
            let dry = args.get("dry_run").and_then(Value::as_bool) == Some(true);
            let wants = gate == Gate::Agent && planned.needs_preview();
            if dry || (wants && args.get("token").is_none()) {
                return Ok(Step::Done(with_focus(
                    vault,
                    args,
                    preview_of(&doc, args, &planned, gate),
                )?));
            }
            Ok(Step::Write)
        }
    }
}

/// Makes the change. The note is read again, so an agent's change that had to be previewed goes
/// through only with the token of a preview made from this very revision and request.
pub fn apply(vault: &Path, args: &Value, gate: Gate) -> Result<Value> {
    let note = req_str(args, "note")?;
    // The focus is checked against the note as it is before the change, so a change that removes
    // the focused block does not fail after it was written.
    let focus = match args.get("focus").filter(|f| !f.is_null()) {
        Some(f) => Some(check_focus(vault, f)?),
        None => None,
    };
    let (file, doc) = load(vault, note)?;
    let planned = plan(vault, &doc, note, args)?;
    if gate == Gate::Agent && planned.needs_preview() {
        let given = args
            .get("token")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if given != token_for(&doc.revision(), args) {
            bail!(
                "this change must be previewed first, and the token does not fit: either the note changed since the preview or the request is not the one previewed. Call office_edit again without a token to get a new preview"
            );
        }
    }
    fsx::atomic_write(&file, planned.doc.render().as_bytes())
        .with_context(|| format!("{note} could not be written"))?;
    let mut out = json!({
        "changed": true,
        "applied": true,
        "summary": planned.summary,
        "revision": planned.doc.revision(),
    });
    if let (Some(o), Value::Object(extra)) = (out.as_object_mut(), planned.extra) {
        o.extend(extra);
    }
    if let Some(f) = focus {
        out["focus"] = f;
    }
    Ok(out)
}

/// One call, start to finish, for a caller that is not under a note lock (a test, the REST route).
pub fn run(vault: &Path, args: &Value, gate: Gate) -> Result<Value> {
    match first_step(vault, args, gate)? {
        Step::Done(v) => Ok(v),
        Step::Write => apply(vault, args, gate),
    }
}

#[cfg(test)]
#[path = "edit_tests.rs"]
mod tests;
