//! Where the HQ Suite meets HQ.
//!
//! This is the only suite crate that may know the rest of HQ: it will register the `office_*`
//! tools, read the `suite:` section of the config, schedule the governor's daemon task, describe
//! the device actions and serve the REST routes for the web app. Today it carries the
//! `hq suite measure` command. Every other suite crate stays free of HQ so the suite can be
//! lifted out; `tests/boundary.rs` keeps it that way.

pub mod api;
pub mod author;
pub mod blocks;
pub mod calc;
pub mod charts;
pub mod comments;
pub mod dash_ops;
pub mod docs;
pub mod edit;
pub mod edit_plan;
pub mod filter;
pub mod governor;
pub mod measure;
pub mod peer;
pub mod preview;
pub mod restore;
pub mod sheet_ops;
pub mod sheets;
pub mod shrink;
pub mod sinks;
pub mod slide_ops;
pub mod text;
