//! Reading office files into what the HQ Suite keeps: a fidelity tier, a loss report, the
//! markdown size, and the pictures inside.
//!
//! This crate stands on `hq-convert` (the importer) and adds what the suite needs beyond it:
//! it looks inside a document's package for media, which the importer only counts, and it
//! projects the fidelity tier from the importer's report. Mapping the importer's blocks into
//! hqmd with typed blocks (milestone M2) lands here too.

pub mod media;
pub mod project;

pub use media::{ImageFormat, MediaItem, MediaScan, for_each_media, sniff_image};
pub use project::{Failure, FailureKind, Projection, class_of, project};
