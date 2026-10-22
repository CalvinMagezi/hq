//! The range of a device's keepalive, shared by the command that sets it and the parser in
//! `hq-tunnel` that reads it back, so the two cannot drift apart.

/// Below this a device would wake its radio constantly.
pub const KEEPALIVE_MIN_SECS: u16 = 10;
/// Above this a carrier closes the path before the next packet.
pub const KEEPALIVE_MAX_SECS: u16 = 300;

/// Whether `secs` is a keepalive a device may be given: 0 for none, or within the range.
pub fn is_allowed(secs: u16) -> bool {
    secs == 0 || (KEEPALIVE_MIN_SECS..=KEEPALIVE_MAX_SECS).contains(&secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_or_within_the_range_is_allowed() {
        for secs in [0, KEEPALIVE_MIN_SECS, 25, KEEPALIVE_MAX_SECS] {
            assert!(is_allowed(secs), "{secs}");
        }
        for secs in [1, KEEPALIVE_MIN_SECS - 1, KEEPALIVE_MAX_SECS + 1, u16::MAX] {
            assert!(!is_allowed(secs), "{secs}");
        }
    }
}
