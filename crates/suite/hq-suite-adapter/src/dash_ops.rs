//! Dashboard edits: add a `tile` (one number) or a `plot` (a chart) to a note from structured
//! arguments instead of hand-written block text.
//!
//! The block is built, put in the note in memory and evaluated with the same code the apps use to
//! draw it (`sheets::tile_of`, `sheets::plot_of`) before anything is written. A block that would not
//! draw is refused with the evaluator's own plain sentence, so the note never gains a dead tile.

use anyhow::{Result, bail};
use hq_suite_md::block::Severity;
use hq_suite_md::{BlockType, Document, Op, TypedBlock};
use serde_json::{Value, json};

use crate::edit_plan::{Planned, opt_str, patch, place_after, req_str};
use crate::sheets::{plot_in, tile_in};
use std::path::Path;

/// Longest label, title or `where` a block header may carry.
const MAX_ATTR_CHARS: usize = 200;

fn attr<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>> {
    match opt_str(args, key).map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) if v.chars().count() > MAX_ATTR_CHARS || v.contains(['\n', '\r', '`']) => {
            bail!("`{key}` is one line of at most {MAX_ATTR_CHARS} characters, with no backtick")
        }
        other => Ok(other),
    }
}

/// Inserts the block, checks it as a block and returns the note with it and its place.
fn put(
    doc: &Document,
    args: &Value,
    ty: BlockType,
    attrs: &[(&str, &str)],
) -> Result<(Document, usize)> {
    let block = TypedBlock::new(ty, attrs, "").render("\n");
    let parsed = TypedBlock::parse(&block).map_err(|e| anyhow::anyhow!("{e}"))?;
    if let Some(p) = parsed
        .validate()
        .into_iter()
        .find(|p| p.severity == Severity::Error)
    {
        bail!("{}", p.message);
    }
    let (at, index) = place_after(doc, args)?;
    let new = patch(
        doc,
        vec![Op::InsertAfter {
            at,
            markdown: block,
        }],
    )?;
    Ok((new, index))
}

/// How many blocks of `ty` come before node `index`: the new block's number among its kind.
fn ordinal(doc: &Document, ty: BlockType, index: usize) -> usize {
    doc.typed_blocks()
        .filter(|(i, b)| *i < index && b.as_ref().is_ok_and(|b| b.ty == ty))
        .count()
}

/// Adds a tile: one number computed from a range (`from`, `fn`, `where`, `label`, `format`).
pub fn add_tile(vault: &Path, doc: &Document, note: &str, args: &Value) -> Result<Planned> {
    let from = req_str(args, "from")?.trim();
    let function = opt_str(args, "fn").unwrap_or("sum").trim();
    let mut attrs: Vec<(&str, &str)> = vec![("from", from), ("fn", function)];
    for key in ["label", "format", "where", "group"] {
        if let Some(v) = attr(args, key)? {
            attrs.push((key, v));
        }
    }
    let (new, index) = put(doc, args, BlockType::Tile, &attrs)?;
    let view = tile_in(
        Some(vault),
        &new,
        note,
        ordinal(&new, BlockType::Tile, index),
    )?;
    let hash = new.node(index).map(|n| n.hash()).unwrap_or_default();
    let summary = format!("Added the tile `{}`, which shows {}", view.label, view.text);
    Ok(Planned::new(new, summary).with(json!({
        "block_index": index,
        "block_hash": hash,
        "tile": {"label": view.label, "text": view.text, "cells": view.cells},
    })))
}

/// Adds a plot: a chart from a range (`type`, `from`, `x`, `y`, `where`, `group`, `title`).
pub fn add_plot(vault: &Path, doc: &Document, note: &str, args: &Value) -> Result<Planned> {
    let kind = req_str(args, "type")?.trim();
    let from = req_str(args, "from")?.trim();
    let (x, y) = (req_str(args, "x")?.trim(), req_str(args, "y")?.trim());
    let mut attrs: Vec<(&str, &str)> = vec![("type", kind), ("from", from), ("x", x), ("y", y)];
    let title = match attr(args, "title")? {
        Some(t) => Some(t),
        None => attr(args, "label")?,
    };
    if let Some(t) = title {
        attrs.push(("title", t));
    }
    for key in ["where", "group"] {
        if let Some(v) = attr(args, key)? {
            attrs.push((key, v));
        }
    }
    let (new, index) = put(doc, args, BlockType::Plot, &attrs)?;
    let drawn = plot_in(
        Some(vault),
        &new,
        note,
        ordinal(&new, BlockType::Plot, index),
    )?;
    let chart: Value = serde_json::from_str(&drawn)?;
    let count = |k: &str| chart[k].as_array().map_or(0, Vec::len);
    let hash = new.node(index).map(|n| n.hash()).unwrap_or_default();
    let summary = format!(
        "Added a {kind} chart of `{y}` by `{x}` with {} labels and {} series",
        count("labels"),
        count("series")
    );
    Ok(Planned::new(new, summary).with(json!({
        "block_index": index,
        "block_hash": hash,
        "chart": {"type": kind, "labels": count("labels"), "series": count("series")},
    })))
}

#[cfg(test)]
#[path = "dash_ops_tests.rs"]
mod tests;
