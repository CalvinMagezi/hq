//! Peak memory of this process, where the system will tell us.

/// Asks the kernel to reset the process's peak resident size to its current size, so the next
/// reading covers only what runs after this call. Linux and Android only; `false` elsewhere or
/// where the file is not writable.
pub fn reset_peak_rss() -> bool {
    std::fs::write("/proc/self/clear_refs", "5").is_ok()
}

/// The process's peak resident size since start or the last reset, in KiB.
pub fn peak_rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|n| n.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_the_peak_never_panics_and_is_plausible_where_supported() {
        if let Some(kib) = peak_rss_kib() {
            assert!(kib > 100, "a running test process is bigger than {kib} KiB");
            let _ = reset_peak_rss();
        }
    }
}
