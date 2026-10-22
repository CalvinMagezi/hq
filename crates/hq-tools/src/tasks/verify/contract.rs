//! What a task says "done" means, and a hash to tell whether it changed.

use hq_db::tasks::Task;
use sha2::{Digest, Sha256};

/// The acceptance criteria written into a task's description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contract {
    pub criteria: Vec<String>,
    /// Hex SHA-256 of the criteria, in order. Equal criteria always hash equal.
    pub hash: String,
}

impl Contract {
    /// A task that states no criteria can be assessed, not checked.
    pub fn is_assessed_only(&self) -> bool {
        self.criteria.is_empty()
    }
}

/// The contract a description states, using the same `Acceptance` section the context packets read.
pub fn contract_of(description: &str) -> Contract {
    let criteria = hq_memory::context_task::acceptance_of(description);
    let mut hasher = Sha256::new();
    for c in &criteria {
        hasher.update(c.as_bytes());
        hasher.update(b"\n");
    }
    let hash = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Contract { criteria, hash }
}

/// `contract_of` for a task.
pub fn task_contract(task: &Task) -> Contract {
    contract_of(&task.description)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn criteria_come_from_the_acceptance_section_and_hash_stably() {
        let d = "Why.\n\nAcceptance:\n- tests pass\n- docs updated\n\nNotes.";
        let c = contract_of(d);
        assert_eq!(c.criteria, ["tests pass", "docs updated"]);
        assert_eq!(c.hash, contract_of(d).hash);
        assert_eq!(c.hash.len(), 64);
        assert!(!c.is_assessed_only());
    }

    #[test]
    fn a_change_to_the_criteria_changes_the_hash_and_other_text_does_not() {
        let a = contract_of("Do it.\n\nAcceptance:\n- one\n- two\n\nx");
        let reworded = contract_of("Do it differently.\n\nAcceptance:\n- one\n- two\n\ny");
        let weaker = contract_of("Do it.\n\nAcceptance:\n- one\n\nx");
        assert_eq!(a.hash, reworded.hash, "only the criteria are the contract");
        assert_ne!(a.hash, weaker.hash);
    }

    #[test]
    fn no_criteria_means_assessed_only() {
        let c = contract_of("Just a description.");
        assert!(c.is_assessed_only());
        assert_eq!(c, contract_of("Another with none."));
    }
}
