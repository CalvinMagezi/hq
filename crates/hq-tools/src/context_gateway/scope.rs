use anyhow::{Result, bail};
use hq_vault::rel_path::{INSTRUCTION_FOLDERS, PRIVATE_FOLDERS, RelPath};

use super::{ScopeCheck, SessionGrant};

/// A vault-relative path in its one spelling. Parsing refuses traversal, absolute paths, the
/// vault root and every folder HQ keeps private or loads as instructions, so a value of this
/// type is always safe to hand to a vault read.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceId(String);

impl SourceId {
    pub fn parse(raw: &str) -> Result<Self> {
        let path = RelPath::parse(raw)?;
        if path.is_root() {
            bail!("the vault root is not a source");
        }
        if path.top_in(PRIVATE_FOLDERS).is_some() || path.top_in(INSTRUCTION_FOLDERS).is_some() {
            bail!("protected folder");
        }
        Ok(Self(path.normalized()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn is_inside(&self, folder: &SourceId) -> bool {
        self.0 == folder.0
            || self
                .0
                .strip_prefix(folder.0.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    }
}

/// A source is readable only inside one of the grant's folders. The match is on whole path
/// components, so `Notebooks/Projects/A` does not open `Notebooks/Projects/AB`.
pub struct PrefixScope;

impl ScopeCheck for PrefixScope {
    fn permits(&self, grant: &SessionGrant, id: &SourceId) -> bool {
        grant
            .scope
            .allow_prefixes
            .iter()
            .any(|folder| id.is_inside(folder))
    }
}
