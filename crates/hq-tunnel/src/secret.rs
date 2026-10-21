//! A 32 byte secret that is wiped on drop and never printed.

use zeroize::Zeroize;

#[derive(Clone)]
pub struct Secret32([u8; 32]);

impl Secret32 {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub(crate) fn bytes(&self) -> [u8; 32] {
        self.0
    }
}

impl Drop for Secret32 {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for Secret32 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret32(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_is_redacted_in_debug_output() {
        let secret = Secret32::new([0xab; 32]);
        let shown = format!("{secret:?} {:?}", Some(&secret));
        assert!(!shown.contains("ab") && !shown.contains("171"), "{shown}");
    }
}
