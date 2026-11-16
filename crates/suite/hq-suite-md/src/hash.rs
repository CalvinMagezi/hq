//! Short content hashes. A block is addressed by its position plus the hash of its text, so an
//! edit made against an out-of-date view fails instead of landing on the wrong block.

use sha2::{Digest, Sha256};

/// Hex characters in a block hash: 48 bits, plenty to tell two versions of one block apart.
pub const BLOCK_HASH_LEN: usize = 12;
/// Hex characters in a revision id (the hash of a whole document).
pub const REVISION_LEN: usize = 16;

fn hex_prefix(bytes: &[u8], len: usize) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = hex::encode(digest);
    out.truncate(len);
    out
}

/// Hash of one block's source text (without the blank lines around it).
pub fn block_hash(src: &str) -> String {
    hex_prefix(src.as_bytes(), BLOCK_HASH_LEN)
}

/// Revision id of a whole document: changes with any byte.
pub fn revision_of(text: &str) -> String {
    hex_prefix(text.as_bytes(), REVISION_LEN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_are_fixed_and_hashes_differ() {
        assert_eq!(block_hash("a").len(), BLOCK_HASH_LEN);
        assert_eq!(revision_of("a").len(), REVISION_LEN);
        assert_ne!(block_hash("a"), block_hash("b"));
        assert_eq!(block_hash("same"), block_hash("same"));
    }
}
