//! Skill loading — parse SKILL.md files, list/load skills as tools.

mod evidence_proposal;
mod fixtures;
mod hints;
mod parse;
mod proposals;
mod security;
mod telemetry;
mod tools;
mod validate;

pub use evidence_proposal::{PROPOSAL_FILE, render_proposal, write_evidence_proposal};
pub use fixtures::{
    FIXTURES_FILE, Fixtures, candidate_failures, check_text, load_fixtures, skill_failures,
};
pub use hints::*;
pub use parse::*;
pub use proposals::*;
pub use security::*;
pub use telemetry::log_skill_load;
pub use tools::*;
pub use validate::*;

#[cfg(test)]
mod tests;
