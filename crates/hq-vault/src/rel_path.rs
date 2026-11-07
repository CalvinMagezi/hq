//! One reading of a vault-relative path, shared by every rule that keys on a vault folder.
//!
//! A folder rule that compares raw strings can be walked around by spelling the same path
//! another way: `./_system/SOUL.md`, `skills//x/SKILL.md`, `.//./_commands/x.md`, or
//! `_System/SOUL.md` on a case-insensitive volume all open the protected file. Parsing the
//! path into components first, and folding the top folder's name, closes those spellings in
//! one place. The vault note tools, the reorganization tools and the connector gateway all
//! classify paths through [`RelPath`].

use anyhow::{Result, bail};
use std::path::{Component, Path};

/// Folders that hold HQ's identity, threads and databases. Note tools never write here.
pub const PRIVATE_FOLDERS: &[&str] = &["_system", "_threads", "_data"];

/// Folders whose files HQ loads as instructions or acts on as messages: skills (matched by
/// hint and loaded into sessions), slash commands, agent definitions and agent mailboxes.
/// Each has a managed path with its own approval and checks (`skill_manage`,
/// `slash_command_manage`, the mailbox tools), so generic note tools do not write here.
pub const INSTRUCTION_FOLDERS: &[&str] =
    &["skills", "_skills", "_commands", "agents", "_mailboxes"];

/// The folder that holds keys, the database and other machine state. Note tools do not read it.
pub const DATA_FOLDER: &str = "_data";

/// A vault-relative path as its components: no `.`, no empty segments, no `..`, no root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelPath {
    parts: Vec<String>,
}

impl RelPath {
    /// Parse `raw` as a path inside the vault. The empty string and `.` are the vault root.
    ///
    /// Refused: a parent step (`..`), an absolute path or drive prefix, a NUL byte, and a
    /// backslash. A backslash is a separator on Windows and an ordinary character elsewhere,
    /// so the same text would name two different files depending on the machine.
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.contains('\0') {
            bail!("invalid vault path: contains a NUL byte");
        }
        if raw.contains('\\') {
            bail!("invalid vault path '{raw}': use / to separate folders");
        }
        let mut parts = Vec::new();
        for c in Path::new(raw).components() {
            match c {
                Component::CurDir => {}
                Component::Normal(p) => parts.push(p.to_string_lossy().into_owned()),
                Component::ParentDir => {
                    bail!("invalid vault path '{raw}': must stay inside the vault, no '..'")
                }
                Component::RootDir | Component::Prefix(_) => {
                    bail!("invalid vault path '{raw}': must be relative to the vault")
                }
            }
        }
        Ok(Self { parts })
    }

    /// Whether this is the vault root itself.
    pub fn is_root(&self) -> bool {
        self.parts.is_empty()
    }

    /// The path in its one spelling, `a/b/c.md` (empty for the root).
    pub fn normalized(&self) -> String {
        self.parts.join("/")
    }

    /// The top-level folder (or file) name folded the way a case-insensitive volume compares
    /// it, so `_System`, `SKILLS` and `Agents` match their lowercase rule.
    pub fn top(&self) -> Option<String> {
        self.parts.first().map(|p| fold(p))
    }

    /// The entry of `folders` this path is, or sits inside, if any.
    pub fn top_in<'a>(&self, folders: &[&'a str]) -> Option<&'a str> {
        let top = self.top()?;
        folders.iter().copied().find(|f| *f == top)
    }
}

/// Fold a name for comparison with a lowercase folder rule. Beyond Unicode lowercasing (which
/// maps the Kelvin sign to `k`), the long s folds to `s` as case-insensitive volumes fold it,
/// and trailing dots and spaces go because Windows drops them when it opens a path.
fn fold(name: &str) -> String {
    name.to_lowercase()
        .replace('\u{17f}', "s")
        .trim_end_matches(['.', ' '])
        .to_string()
}

/// The protected folder `raw` is in, or `Err` when it cannot be read as a vault path at all.
/// Callers that gate on the answer treat `Err` as protected.
pub fn protected_folder<'a>(raw: &str, folders: &[&'a str]) -> Result<Option<&'a str>> {
    Ok(RelPath::parse(raw)?.top_in(folders))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spellings_of_one_path_parse_alike() {
        for raw in [
            "_system/SOUL.md",
            "./_system/SOUL.md",
            ".//./_system/SOUL.md",
            "_system//SOUL.md",
            "_system/./SOUL.md",
            "_system/SOUL.md/",
        ] {
            assert_eq!(RelPath::parse(raw).unwrap().normalized(), "_system/SOUL.md");
        }
    }

    #[test]
    fn traversal_roots_and_backslashes_are_refused() {
        for raw in [
            "../x.md",
            "a/../../x.md",
            "a/../b.md",
            "/etc/passwd",
            "skills\\x\\SKILL.md",
            "a\0b",
        ] {
            assert!(RelPath::parse(raw).is_err(), "{raw}");
        }
        // Two dots inside a name are not a parent step.
        assert!(RelPath::parse("Notebooks/a..b.md").is_ok());
    }

    #[test]
    fn the_root_has_no_top_folder() {
        for raw in ["", ".", "./"] {
            let p = RelPath::parse(raw).unwrap();
            assert!(p.is_root() && p.top().is_none(), "{raw:?}");
        }
    }

    #[test]
    fn top_folders_match_however_they_are_cased() {
        let cases = [
            ("_System/SOUL.md", Some("_system")),
            ("./SKILLS/x/SKILL.md", Some("skills")),
            ("Agents/coder.md", Some("agents")),
            ("s\u{212a}ills/x/SKILL.md", Some("skills")),
            ("_\u{17f}ystem/SOUL.md", Some("_system")),
            ("skills./x/SKILL.md", Some("skills")),
            ("_mailboxes", Some("_mailboxes")),
            ("Notebooks/skills/x.md", None),
            ("skills.md", None),
            ("Inbox/agents.md", None),
        ];
        let all: Vec<&str> = PRIVATE_FOLDERS
            .iter()
            .chain(INSTRUCTION_FOLDERS)
            .copied()
            .collect();
        for (raw, want) in cases {
            assert_eq!(protected_folder(raw, &all).unwrap(), want, "{raw}");
        }
    }
}
