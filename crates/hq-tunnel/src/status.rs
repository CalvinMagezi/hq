//! What the peer tells the app: a small JSON status file, replaced atomically, and a bounded
//! event log. Neither is a network listener, and neither holds a key or any traffic. The status
//! file is how a screen shows "what it did" without the app asking the peer anything.

use crate::fsutil::{PRIVATE_FILE_MODE, private_file, temp_path, write_private_atomic};
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const STATUS_VERSION: u32 = 1;
pub const MAX_EVENT_LOG_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub version: u32,
    /// `up` while the session is fresh and no handshake has gone unanswered for 30 seconds.
    /// `connecting` before the first handshake or while one is unanswered for under 30 seconds,
    /// `down` after that, even if an older session is still fresh (the hub does not answer). `idle` when the
    /// session has aged out and nothing is in flight (a tunnel with no keepalive left alone).
    /// `stopped` on exit.
    pub state: &'static str,
    pub address: String,
    pub endpoint: String,
    pub started_unix: u64,
    pub updated_unix: u64,
    /// The most seconds the peer can go without rewriting this file while it runs. An idle one
    /// sleeps until its next keepalive, so a reader should call the peer stopped only when
    /// `updated_unix` is older than this plus a margin, not after a fixed few seconds.
    pub max_age_s: u64,
    pub last_handshake_age_s: Option<u64>,
    pub connections: usize,
    pub accepted: u64,
    pub denied: u64,
    pub dropped: u64,
    pub reach_opened: u64,
    pub udp_tx_bytes: u64,
    pub udp_rx_bytes: u64,
    /// Seconds the CPU has been suspended since the peer started (boot time minus monotonic time).
    pub suspended_s: f64,
    /// How many times the loop has woken since it started; an idle tunnel adds about one per
    /// keepalive, so a number that climbs fast is a loop that is spinning.
    pub wakeups: u64,
    /// Resident memory in kilobytes and open descriptors, where `/proc` says (Linux and Android).
    pub rss_kb: Option<u64>,
    pub open_fds: Option<u64>,
    pub error: Option<String>,
}

/// Resident memory of this process in kilobytes, from `/proc/self/statm`.
pub fn process_rss_kb() -> Option<u64> {
    let statm = fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    // SAFETY: sysconf has no preconditions.
    let page = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok()?;
    Some(pages * page / 1024)
}

/// How many descriptors this process has open, counted from `/proc/self/fd`.
pub fn open_fd_count() -> Option<u64> {
    let entries = fs::read_dir("/proc/self/fd").ok()?.count() as u64;
    // The listing holds a descriptor of its own.
    Some(entries.saturating_sub(1))
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Writes `status` to `path` through a temporary file and a rename, so a reader never sees half.
/// The directory must be private to the app: a rename into a shared one could be raced.
pub fn write_atomic(path: &Path, status: &Status) -> io::Result<()> {
    let mut text = serde_json::to_string_pretty(status).map_err(io::Error::other)?;
    text.push('\n');
    write_private_atomic(path, text.as_bytes())
}

/// Lines of `unix-time text`, newest last. When the file passes its limit the older half goes.
pub struct EventLog {
    path: PathBuf,
    max_bytes: u64,
}

impl EventLog {
    pub fn new(path: PathBuf, max_bytes: u64) -> Self {
        Self { path, max_bytes }
    }

    pub fn append(&self, text: &str) {
        let line = format!("{} {}\n", unix_now(), text.replace('\n', " "));
        let result = (|| {
            if self.size_after_clearing_a_link() + line.len() as u64 > self.max_bytes {
                self.trim()?;
            }
            let mut file = OpenOptions::new()
                .append(true)
                .create(true)
                .mode(PRIVATE_FILE_MODE)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&self.path)?;
            file.write_all(line.as_bytes())
        })();
        // A full disk must not stop the tunnel, and there is nowhere safe to report it.
        let _ = result;
    }

    /// A link at the path is removed, never followed, so the log cannot be pointed at another file.
    fn size_after_clearing_a_link(&self) -> u64 {
        match fs::symlink_metadata(&self.path) {
            Ok(m) if m.file_type().is_symlink() => {
                let _ = fs::remove_file(&self.path);
                0
            }
            Ok(m) => m.len(),
            Err(_) => 0,
        }
    }

    fn trim(&self) -> io::Result<()> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&self.path)?;
        let cap = self.max_bytes * 2;
        let mut bytes = Vec::new();
        file.take(cap).read_to_end(&mut bytes)?;
        // This log never grows past its limit, so a file this large was put there by someone else.
        if bytes.len() as u64 >= cap {
            bytes.clear();
        }
        // Bytes, not a string: cutting at the middle of the file must not need a character boundary.
        let from_half = &bytes[bytes.len() / 2..];
        let rest = from_half
            .iter()
            .position(|&b| b == b'\n')
            .map_or(&[][..], |at| &from_half[at + 1..]);
        let tmp = temp_path(&self.path);
        let mut file = private_file(&tmp)?;
        file.write_all(rest)?;
        fs::rename(&tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn sample() -> Status {
        Status {
            version: STATUS_VERSION,
            state: "up",
            address: "10.77.0.4".into(),
            endpoint: "192.0.2.1:51820".into(),
            started_unix: 1,
            updated_unix: 2,
            max_age_s: 30,
            last_handshake_age_s: Some(5),
            connections: 1,
            accepted: 2,
            denied: 0,
            dropped: 0,
            reach_opened: 0,
            udp_tx_bytes: 10,
            udp_rx_bytes: 20,
            suspended_s: 0.0,
            wakeups: 0,
            rss_kb: None,
            open_fds: None,
            error: None,
        }
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn the_process_can_report_its_memory_and_descriptors() {
        assert!(process_rss_kb().unwrap() > 100);
        // Other tests open and close descriptors on their own threads, so count a crowd of them
        // rather than expect an exact difference.
        const HELD: u64 = 200;
        let before = open_fd_count().unwrap();
        let held: Vec<_> = (0..HELD)
            .map(|_| fs::File::open("/proc/self/statm").unwrap())
            .collect();
        assert!(open_fd_count().unwrap() >= before + HELD / 2);
        drop(held);
    }

    #[test]
    fn the_status_file_is_private_whole_and_versioned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("status.json");
        write_atomic(&path, &sample()).unwrap();
        write_atomic(
            &path,
            &Status {
                state: "down",
                ..sample()
            },
        )
        .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["version"], 1);
        assert_eq!(json["state"], "down");
        let mode = fs::metadata(&path).unwrap().permissions();
        assert_eq!(mode.mode() & 0o777, 0o600);
        assert!(!dir.path().join("status.json.tmp").exists());
    }

    #[test]
    fn a_link_at_the_path_is_replaced_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("precious");
        fs::write(&target, "keep me").unwrap();
        let path = dir.path().join("status.json");
        std::os::unix::fs::symlink(&target, dir.path().join("status.json.tmp")).unwrap();
        write_atomic(&path, &sample()).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep me");
        assert!(fs::read_to_string(&path).unwrap().contains("\"state\""));
    }

    #[test]
    fn a_link_at_the_event_log_is_replaced_and_what_it_pointed_at_is_never_read_or_written() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("precious");
        let big = "line of the file the link points at\n".repeat(2_000);
        fs::write(&target, &big).unwrap();
        let path = dir.path().join("events.log");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        EventLog::new(path.clone(), 1_000).append("hello");
        assert_eq!(fs::read_to_string(&target).unwrap(), big);
        assert!(
            !fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("line of the file"), "the target was read");
        assert!(
            text.ends_with(" hello\n") && text.lines().count() == 1,
            "{text}"
        );
    }

    #[test]
    fn a_log_far_past_its_limit_is_started_again_not_read_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.log");
        fs::write(
            &path,
            "an old line that someone else put here\n".repeat(2_000),
        )
        .unwrap();
        EventLog::new(path.clone(), 1_000).append("hello");
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            text.len() < 100 && text.contains("hello"),
            "{} bytes",
            text.len()
        );
    }

    #[test]
    fn trimming_works_on_a_log_that_holds_more_than_ascii() {
        let dir = tempfile::tempdir().unwrap();
        let log = EventLog::new(dir.path().join("events.log"), 400);
        for n in 0..60 {
            log.append(&format!("naïve événement {n} ✓"));
        }
        let text = fs::read_to_string(dir.path().join("events.log")).unwrap();
        assert!(text.len() <= 400 && text.contains("événement"));
    }

    #[test]
    fn the_event_log_stays_under_its_limit_and_keeps_the_newest_lines() {
        let dir = tempfile::tempdir().unwrap();
        let log = EventLog::new(dir.path().join("events.log"), 2_000);
        for n in 0..500 {
            log.append(&format!("event number {n}"));
        }
        let text = fs::read_to_string(dir.path().join("events.log")).unwrap();
        assert!(text.len() <= 2_000, "{}", text.len());
        assert!(text.lines().last().unwrap().ends_with("event number 499"));
        assert!(
            text.lines().all(|l| l.contains("event number")),
            "no half line is kept"
        );
        assert!(!text.contains("event number 0\n"));
    }
}
