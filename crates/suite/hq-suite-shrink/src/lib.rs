//! Shrinking documents and media into the small record, and measuring how much that saves.
//!
//! Today this crate holds the picture codecs with the pixel and memory caps around them, and
//! the measurement behind `hq suite measure`. The shrink pipeline itself (plan, journal, apply,
//! verify, undo) arrives with milestone M1.

pub mod codec;
pub mod measure;
pub mod probe;

pub use codec::{CodecError, Encoder};
pub use measure::{MeasureError, MeasureReport, Settings, measure};
