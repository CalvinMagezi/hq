//! Shrinking documents and media into the small record, and measuring how much that saves.
//!
//! The picture codecs with the pixel and memory caps around them, the measurement behind
//! `hq suite measure`, and the pipeline that shrinks one file: prepare, confirm, undo, recover.

pub mod codec;
pub mod measure;
pub mod pipeline;
mod place;
pub mod probe;

pub use codec::{CodecError, Encoder};
pub use measure::{MeasureError, MeasureReport, Settings, measure};
pub use pipeline::{
    Faults, ImagePolicy, NoFaults, Notes, Options, OriginalAt, Outcome, Preview, Recovered,
    Refusal, ShrinkError, Shrinker, set_original_state,
};
