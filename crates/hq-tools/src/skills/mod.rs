//! Skill loading — parse SKILL.md files, list/load skills as tools.

pub mod builtin;
mod evidence_proposal;
mod hints;
mod proposals;
mod security;
mod telemetry;
mod tools;

pub use crate::skill_model::fixtures::{
    FIXTURES_FILE, Fixtures, candidate_failures, check_text, load_fixtures, skill_failures,
};
pub use crate::skill_model::parse::*;
pub use crate::skill_model::validate::*;
pub use evidence_proposal::{PROPOSAL_FILE, render_proposal, write_evidence_proposal};
pub use hints::*;
pub use proposals::*;
pub use security::*;
pub use telemetry::log_skill_load;
pub use tools::*;

#[cfg(test)]
mod tests;
