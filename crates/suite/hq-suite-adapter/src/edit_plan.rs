//! What every `office_edit` action shares: the plan an action makes before anything is written,
//! the argument readers, the patch helpers, the preview token and the `focus` echo.
//!
//! An action never writes. It reads the note, builds the note as it would be after the change (an
//! hqmd patch, so a stale block or cell is refused and the rest comes back byte for byte) and
//! returns that as a [`Planned`]. `edit::run` decides whether to show it as a preview or to write it.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use hq_suite_md::{Addr, Document, Op, Patch, PatchError, apply};
use hq_suite_store::SuitePaths;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// An edit that touches more cells than this is shown as a preview first.
pub const PREVIEW_CELLS: usize = 50;

/// One change, worked out and not written.
#[derive(Debug, Clone)]
pub struct Planned {
    /// The note as it would be after the change.
    pub doc: Document,
    /// What the change does, in a sentence or two, for a preview and for the reply.
    pub summary: String,
    /// How many cells the change addresses (zero for a change that is not about cells).
    pub touched: usize,
    /// The change removes something (rows, a card, a block).
    pub destructive: bool,
    /// Action-specific fields for the reply, merged into the result object.
    pub extra: Value,
}

impl Planned {
    pub fn new(doc: Document, summary: impl Into<String>) -> Planned {
        Planned {
            doc,
            summary: summary.into(),
            touched: 0,
            destructive: false,
            extra: json!({}),
        }
    }

    pub fn touching(mut self, cells: usize) -> Planned {
        self.touched = cells;
        self
    }

    pub fn removing(mut self) -> Planned {
        self.destructive = true;
        self
    }

    pub fn with(mut self, extra: Value) -> Planned {
        self.extra = extra;
        self
    }

    /// Whether an agent's call must be shown as a preview before it is applied.
    pub fn needs_preview(&self) -> bool {
        self.destructive || self.touched > PREVIEW_CELLS
    }
}

pub fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("`{key}` is needed for this action"))
}

pub fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

pub fn req_u64(args: &Value, key: &str) -> Result<u64> {
    args.get(key)
        .and_then(Value::as_u64)
        .with_context(|| format!("`{key}` (a whole number) is needed for this action"))
}

pub fn opt_u64(args: &Value, key: &str) -> Result<Option<u64>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .map(Some)
            .with_context(|| format!("`{key}` must be a whole number")),
    }
}

/// Reads and parses the note.
pub fn load(vault: &Path, note: &str) -> Result<(PathBuf, Document)> {
    let file = SuitePaths::new(vault).resolve(note)?;
    let text = std::fs::read_to_string(&file).with_context(|| format!("{note} cannot be read"))?;
    Ok((file, Document::parse(&text)))
}

fn refused(e: PatchError) -> anyhow::Error {
    match e {
        PatchError::StaleBlock { index, .. } => anyhow::anyhow!(
            "block {index} changed since you read it; read the note again and retry"
        ),
        PatchError::StaleRevision { .. } => {
            anyhow::anyhow!("the note changed since it was read; read it again and retry")
        }
        other => anyhow::anyhow!("that edit was refused: {other}"),
    }
}

/// Applies `ops` to `doc` as one patch, refused whole if any address is stale.
pub fn patch(doc: &Document, ops: Vec<Op>) -> Result<Document> {
    apply(
        doc,
        &Patch {
            base_revision: Some(doc.revision()),
            ops,
        },
    )
    .map_err(refused)
}

/// The address of node `index`, which must still hash to `hash` when one is given.
pub fn addr(doc: &Document, index: usize, hash: Option<&str>) -> Result<Addr> {
    let at = doc
        .addr(index)
        .with_context(|| format!("there is no block {index}"))?;
    if let Some(h) = hash
        && h != at.hash
    {
        bail!("block {index} changed since you read it; read the note again and retry");
    }
    Ok(at)
}

/// Where a new block goes: after the block `after_index` (which must still hash to `hash`), or
/// at the end of the note. Returns the address to insert after and the new block's number.
pub fn place_after(doc: &Document, args: &Value) -> Result<(Option<Addr>, usize)> {
    match opt_u64(args, "after_index")? {
        Some(i) => Ok((
            Some(addr(doc, i as usize, Some(req_str(args, "hash")?))?),
            i as usize + 1,
        )),
        None => Ok((
            doc.nodes().len().checked_sub(1).and_then(|i| doc.addr(i)),
            doc.nodes().len(),
        )),
    }
}

/// `1 row`, `3 rows`.
pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

// ---------------------------------------------------------------- the preview token

fn secret() -> &'static [u8; 32] {
    static SECRET: OnceLock<[u8; 32]> = OnceLock::new();
    SECRET.get_or_init(|| {
        // RandomState is seeded from the operating system for every instance, so a process's
        // secret cannot be worked out from outside it.
        let mut out = [0u8; 32];
        for chunk in out.chunks_mut(8) {
            let mut h = RandomState::new().build_hasher();
            h.write_u64(chunk.len() as u64);
            chunk.copy_from_slice(&h.finish().to_le_bytes());
        }
        out
    })
}

/// The request with the keys that do not change what it does taken out, and its keys in order.
pub fn canonical(request: &Value) -> String {
    fn sorted(v: &Value) -> Value {
        match v {
            Value::Object(m) => {
                let mut keys: Vec<&String> = m.keys().collect();
                keys.sort();
                Value::Array(
                    keys.into_iter()
                        .map(|k| json!([k, sorted(&m[k])]))
                        .collect(),
                )
            }
            Value::Array(a) => Value::Array(a.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    let mut r = request.clone();
    if let Value::Object(m) = &mut r {
        for k in ["token", "focus", "dry_run"] {
            m.remove(k);
        }
        // The identity keys the runtime sets are not part of what was asked.
        m.retain(|k, _| !k.starts_with("_hq_"));
    }
    sorted(&r).to_string()
}

/// The token that lets `request` be applied to the note at `revision`: it fits only that request
/// on only that revision, and only in this process.
pub fn token_for(revision: &str, request: &Value) -> String {
    let mut h = Sha256::new();
    h.update(secret());
    h.update(revision.as_bytes());
    h.update([0]);
    h.update(canonical(request).as_bytes());
    hex::encode(h.finalize())[..24].to_string()
}

// ---------------------------------------------------------------- focus

/// What the owner has selected, passed through by a UI. Plain input the model may read; it never
/// says who is calling. Checked against the note so a stale selection is caught, then echoed.
pub fn check_focus(vault: &Path, focus: &Value) -> Result<Value> {
    let note = focus
        .get("note")
        .and_then(Value::as_str)
        .context("`focus.note` is needed when `focus` is given")?;
    let (_, doc) =
        load(vault, note).with_context(|| format!("`focus.note` {note} is not a note"))?;
    let mut out = json!({ "note": note });
    if let Some(block) = opt_u64(focus, "block")? {
        let node = doc
            .node(block as usize)
            .with_context(|| format!("`focus.block` {block} is not a block of {note}"))?;
        out["block"] = json!(block);
        out["block_hash"] = json!(node.hash());
    }
    for key in ["row", "col"] {
        if let Some(n) = opt_u64(focus, key)? {
            out[key] = json!(n);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_fits_one_request_on_one_revision() {
        let a = json!({"action": "delete_rows", "note": "n.md", "row": 3});
        let t = token_for("rev1", &a);
        assert_eq!(t, token_for("rev1", &a));
        assert_ne!(t, token_for("rev2", &a), "another revision");
        let b = json!({"action": "delete_rows", "note": "n.md", "row": 4});
        assert_ne!(t, token_for("rev1", &b), "another request");
        // The token itself, the focus and the runtime's keys are not part of the request.
        let with = json!({"action": "delete_rows", "note": "n.md", "row": 3, "token": t,
            "focus": {"note": "n.md"}, "_hq_caller_session": "s"});
        assert_eq!(t, token_for("rev1", &with));
        // Key order does not matter.
        let reordered = json!({"row": 3, "note": "n.md", "action": "delete_rows"});
        assert_eq!(t, token_for("rev1", &reordered));
    }

    #[test]
    fn a_focus_must_name_a_note_and_a_block_that_exist() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("Notebooks")).unwrap();
        std::fs::write(d.path().join("Notebooks/a.md"), "# A\n\nText.\n").unwrap();
        let ok = check_focus(
            d.path(),
            &json!({"note": "Notebooks/a.md", "block": 1, "row": 2, "col": 0}),
        )
        .unwrap();
        assert_eq!(ok["block"], 1);
        assert_eq!(ok["row"], 2);
        assert!(ok["block_hash"].as_str().unwrap().len() >= 8);
        assert!(check_focus(d.path(), &json!({"note": "Notebooks/missing.md"})).is_err());
        assert!(check_focus(d.path(), &json!({"note": "Notebooks/a.md", "block": 9})).is_err());
        assert!(check_focus(d.path(), &json!({"block": 1})).is_err());
        assert!(check_focus(d.path(), &json!({"note": "../x.md"})).is_err());
    }
}
