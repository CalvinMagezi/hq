//! A fingerprint of what a memory says, for spotting the same words again.

use sha2::{Digest, Sha256};

/// A hash of `text` with case, punctuation and spacing ignored, so "Acme delivers on Thursdays."
/// and "acme  delivers on thursdays" match. It is a fingerprint of the exact sentence, not of
/// its meaning: a reworded sentence has a different one.
pub fn content_hash(text: &str) -> String {
    let normalised: String = text
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    hex::encode(&Sha256::digest(normalised.as_bytes())[..16])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_punctuation_and_spacing_do_not_change_it() {
        let a = content_hash("Acme Feeds delivers on Thursdays.");
        assert_eq!(a, content_hash("  acme feeds  delivers on THURSDAYS!! "));
        assert_eq!(a.len(), 32);
    }

    #[test]
    fn different_words_are_different() {
        assert_ne!(
            content_hash("Acme Feeds delivers on Thursdays"),
            content_hash("Acme Feeds delivers on Tuesdays")
        );
        assert_ne!(content_hash("a"), content_hash("b"));
    }

    #[test]
    fn text_in_other_scripts_is_not_flattened_to_nothing() {
        assert_ne!(
            content_hash("Mwasuze otya"),
            content_hash("Habari ya asubuhi")
        );
        assert_ne!(content_hash("café"), content_hash("cafe"));
    }
}
