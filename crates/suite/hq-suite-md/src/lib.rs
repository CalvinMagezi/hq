//! hqmd: HQ-compatible markdown with typed fenced blocks.
//!
//! A note is valid CommonMark with GitHub tables, so any viewer shows something readable. The
//! suite adds typed blocks (`sheet`, `tile`, `plot`, `chart`) as fenced code blocks with a header
//! of `type key=value ...`, and a few flat frontmatter keys. Everything else is left alone.
//!
//! The model keeps the source text of every block, so parsing and printing an unedited note
//! returns the same bytes, and an edit changes only the blocks it names. See
//! `docs/suite/HQMD.md` for the language and `crates/suite/CLAUDE.md` for the rules this crate
//! lives under: no I/O, no HQ crates, nothing here reads a path or a clock.

pub mod block;
pub mod comment;
pub mod doc;
pub mod frontmatter;
pub mod hash;
pub mod info;
pub mod meta;
pub mod patch;
pub mod reference;
pub mod sheet;
pub mod validate;

pub use block::{BlockError, BlockType, Problem, Severity, TypedBlock};
pub use comment::{Comment, CommentState, comments, strip_comments};
pub use doc::{Document, MAX_DOC_BYTES, Node, NodeKind};
pub use frontmatter::Scalar;
pub use info::{Attr, Info, InfoError};
pub use meta::{Fidelity, OriginalState, SuiteKind, SuiteMeta};
pub use patch::{Addr, Op, Patch, PatchError, apply};
pub use reference::{Cell, RangeRef, safe_rel_path};
pub use sheet::{ColType, Grid};
pub use validate::{Diagnostic, has_errors, validate};
