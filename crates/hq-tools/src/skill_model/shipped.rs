//! The registry half of the skills HQ ships in its binary: which built-ins exist, which are
//! active in this process, and the text embedded for each. It depends on nothing but the config, so
//! the skill parser, the validator and the audit trail can all use it without reaching up into
//! `skills`. The part that writes the vault mirror (`sync_builtin_skills`) stays in
//! `skills::builtin`, which re-exports everything here under the same names.

use std::fs;
use std::path::Path;
use std::sync::RwLock;

use anyhow::{Result, bail};
use hq_core::config::HqConfig;
use sha2::{Digest, Sha256};

pub(crate) const SKILL_FILE: &str = "SKILL.md";

/// One shipped skill: its name, every file it is made of, and when it applies.
pub struct Shipped {
    pub name: &'static str,
    pub files: &'static [(&'static str, &'static str)],
    /// Whether this install can use the skill; one that cannot keeps it out of every prompt.
    pub applies: fn(&HqConfig) -> bool,
}

/// The skill `calendar_create_event` refuses to run without.
pub const CALENDAR_CREATION_SKILL: &str = "calendar-creation";

/// The skill `calendar_list_events` refuses to run without.
pub const CALENDAR_READING_SKILL: &str = "calendar-reading";

/// The skill `calendar_update_event` and `calendar_delete_event` refuse to run without.
pub const CALENDAR_EDITING_SKILL: &str = "calendar-editing";

/// The skill `contacts_search` refuses to run without.
pub const CONTACTS_LOOKUP_SKILL: &str = "contacts-lookup";

/// The skill `finance_import_messages` refuses to run without.
pub const FINANCE_IMPORT_SKILL: &str = "finance-import";

/// The set is decided once at start, and GitHub can be connected any time after, so the worker
/// is always offered. Whether it acts is `dispatch::allowed_repos`.
fn every_install(_: &HqConfig) -> bool {
    true
}

/// Every built-in skill. A test checks this list against `assets/skills/`.
pub const BUILTIN_SKILLS: &[Shipped] = &[
    Shipped {
        name: "edge-coding-worker",
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/edge-coding-worker/SKILL.md"),
        )],
        applies: every_install,
    },
    Shipped {
        name: "hq-task-brief",
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/hq-task-brief/SKILL.md"),
        )],
        applies: every_install,
    },
    Shipped {
        name: "hq-engineering-core",
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/hq-engineering-core/SKILL.md"),
        )],
        applies: every_install,
    },
    Shipped {
        name: "hq-engineering-edge",
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/hq-engineering-edge/SKILL.md"),
        )],
        applies: every_install,
    },
    Shipped {
        name: "hq-wayfinder",
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/hq-wayfinder/SKILL.md"),
        )],
        applies: every_install,
    },
    Shipped {
        name: "hq-diagnose",
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/hq-diagnose/SKILL.md"),
        )],
        applies: every_install,
    },
    Shipped {
        name: "hq-review-loop",
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/hq-review-loop/SKILL.md"),
        )],
        applies: every_install,
    },
    Shipped {
        name: CALENDAR_CREATION_SKILL,
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/calendar-creation/SKILL.md"),
        )],
        applies: every_install,
    },
    Shipped {
        name: CALENDAR_READING_SKILL,
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/calendar-reading/SKILL.md"),
        )],
        applies: every_install,
    },
    Shipped {
        name: CALENDAR_EDITING_SKILL,
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/calendar-editing/SKILL.md"),
        )],
        applies: every_install,
    },
    Shipped {
        name: "office-authoring",
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/office-authoring/SKILL.md"),
        )],
        applies: every_install,
    },
    Shipped {
        name: "local-price-research",
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/local-price-research/SKILL.md"),
        )],
        applies: every_install,
    },
    Shipped {
        name: CONTACTS_LOOKUP_SKILL,
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/contacts-lookup/SKILL.md"),
        )],
        applies: every_install,
    },
    Shipped {
        name: FINANCE_IMPORT_SKILL,
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/finance-import/SKILL.md"),
        )],
        applies: every_install,
    },
];

/// The active built-ins, set once from the config the process loaded. `None` until
/// [`configure`] runs: nothing is active, so a library user or a test sees no built-in by default.
static ACTIVE: RwLock<Option<Vec<&'static str>>> = RwLock::new(None);

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Per-thread override for tests, so each test decides what is active without a global.
    static TEST_ACTIVE: std::cell::RefCell<Option<Vec<&'static str>>> =
        const { std::cell::RefCell::new(None) };
}

/// Decides which built-ins this process offers, from the config it loaded. Returns the names in
/// `skills.builtin.off` that no built-in has, which are most likely typos.
pub fn configure(config: &HqConfig) -> Vec<String> {
    let active = active_for(config);
    *ACTIVE.write().unwrap_or_else(|e| e.into_inner()) = Some(active);
    unknown_off(config)
}

/// The built-ins `config` makes active.
pub fn active_for(config: &HqConfig) -> Vec<&'static str> {
    let off = &config.skills.builtin.off;
    BUILTIN_SKILLS
        .iter()
        .filter(|s| (s.applies)(config) && !off.iter().any(|o| o.eq_ignore_ascii_case(s.name)))
        .map(|s| s.name)
        .collect()
}

pub(crate) fn unknown_off(config: &HqConfig) -> Vec<String> {
    let off = &config.skills.builtin.off;
    off.iter()
        .filter(|o| shipped(o).is_none())
        .cloned()
        .collect()
}

/// Sets what is active for the calling thread until the returned guard is dropped. For tests,
/// here and in crates that build sessions (feature `test-support`).
#[cfg(any(test, feature = "test-support"))]
pub fn set_active_for_this_thread(names: Option<&[&'static str]>) -> ActiveGuard {
    let previous = TEST_ACTIVE.with(|t| t.replace(names.map(<[&str]>::to_vec)));
    ActiveGuard(previous)
}

/// Restores the calling thread's previous override when dropped.
#[cfg(any(test, feature = "test-support"))]
pub struct ActiveGuard(Option<Vec<&'static str>>);

#[cfg(any(test, feature = "test-support"))]
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        let previous = self.0.take();
        TEST_ACTIVE.with(|t| *t.borrow_mut() = previous);
    }
}

fn active_names() -> Vec<&'static str> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(names) = TEST_ACTIVE.with(|t| t.borrow().clone()) {
        return names;
    }
    ACTIVE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_default()
}

/// Names compare without case: on a case-insensitive file system `Edge-Coding-Worker` is the
/// same folder as `edge-coding-worker`, so a variant must never escape the built-in rules.
fn shipped(name: &str) -> Option<&'static Shipped> {
    BUILTIN_SKILLS
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(name))
}

/// Whether `name` is an active built-in: loaded from HQ's text and closed to every change.
pub fn is_active(name: &str) -> bool {
    shipped(name).is_some_and(|s| active_names().contains(&s.name))
}

/// The active built-in's own spelling of `name`.
pub fn canonical_name(name: &str) -> Option<&'static str> {
    shipped(name).filter(|s| is_active(s.name)).map(|s| s.name)
}

/// The active built-ins, in the order they ship.
pub fn active_builtins() -> impl Iterator<Item = &'static Shipped> {
    let active = active_names();
    BUILTIN_SKILLS
        .iter()
        .filter(move |s| active.contains(&s.name))
}

/// The embedded text of one of an active built-in's files.
pub fn shipped_file(name: &str, file: &str) -> Option<&'static str> {
    canonical_name(name)?;
    shipped(name)?
        .files
        .iter()
        .find(|(f, _)| *f == file)
        .map(|(_, text)| *text)
}

/// SHA-256 of an active built-in's embedded SKILL.md: what a session actually ran.
pub fn shipped_hash(name: &str) -> Option<String> {
    shipped_file(name, SKILL_FILE).map(|text| hex::encode(Sha256::digest(text.as_bytes())))
}

/// A skill file as a session should see it: the embedded text for an active built-in, never what
/// its vault copy holds; the vault file for any other skill.
pub fn read_skill_file(skills_dir: &Path, name: &str, file: &str) -> Option<String> {
    if is_active(name) {
        return shipped_file(name, file).map(str::to_string);
    }
    fs::read_to_string(skills_dir.join(name).join(file)).ok()
}

/// Refuses a change to an active built-in, saying what to do instead.
pub fn refuse_builtin(name: &str, action: &str) -> Result<()> {
    if let Some(canonical) = canonical_name(name) {
        bail!(
            "{canonical} ships with HQ, so it cannot be {action}; to change it, copy it under \
             another name, or add {canonical} to skills.builtin.off in the config to use your own"
        );
    }
    Ok(())
}
