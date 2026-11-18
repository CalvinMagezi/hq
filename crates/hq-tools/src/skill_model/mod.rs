//! What a skill is, independent of how it is stored, edited or audited: the SKILL.md parser
//! (`parse`), the validator (`validate`), skill fixtures (`fixtures`), hint matching (`hint_match`) and the registry of skills HQ ships (`shipped`).
//!
//! These sit below `skills`, `skill_audit`, `skill_edit` and `skill_manage_tool` so the audit
//! trail can parse and validate a skill without reaching up into the module that loads them.
//! `skills` re-exports every public item here under its old name.

pub mod fixtures;
pub mod hint_match;
pub mod parse;
pub mod shipped;
pub mod validate;
