//! Whether a paired hub's web forward is held by HQ's own bridge (`hq bridge serve`, often a
//! launchd service) rather than this app. The window's sign-in goes to whoever holds the port,
//! so it is trusted only when HQ's record in `~/.hq/run` names the process, that process started
//! before the record was written (a reused pid did not), and the OS says the process listens on
//! exactly that address. Anything that cannot be checked counts as no.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use hq_bridge::holder;

/// How long a full check (the listener lookup) is reused; the process is re-checked every time.
const FULL_CHECK_EVERY: Duration = Duration::from_secs(1);
/// A lookup that takes longer than this counts as no, so the proxy never hangs on it.
#[cfg(target_os = "macos")]
const LOOKUP_LIMIT: Duration = Duration::from_secs(2);

/// What the OS can say about a process. A seam so the rules are tested without real processes.
pub trait Processes: Send + Sync {
    fn started(&self, pid: u32) -> Option<SystemTime>;
    fn listens_on(&self, pid: u32, addr: SocketAddr) -> bool;
}

/// The process HQ's record names, when every rule holds for `listen`.
pub fn holder_of(
    record: &Path,
    listen: SocketAddr,
    os: &dyn Processes,
) -> Option<(u32, SystemTime)> {
    let rec = holder::read(record).ok()?;
    let written = holder::written_at(record).ok()?;
    if !rec.listens.contains(&listen) || !listen.ip().is_loopback() {
        return None;
    }
    let started = os.started(rec.pid)?;
    (started <= written && os.listens_on(rec.pid, listen)).then_some((rec.pid, started))
}

#[derive(Default)]
struct Checked {
    at: Option<Instant>,
    listen: Option<SocketAddr>,
    holder: Option<(u32, SystemTime)>,
}

/// The check behind Open HQ and the proxy gate, cached so a page load does not run the OS
/// lookup for every request.
#[derive(Clone)]
pub struct BridgeTrust {
    record: PathBuf,
    os: Arc<dyn Processes>,
    checked: Arc<Mutex<Checked>>,
}

impl BridgeTrust {
    pub fn new(hq_dir: &Path, os: Arc<dyn Processes>) -> Self {
        Self {
            record: holder::record_path(hq_dir),
            os,
            checked: Arc::default(),
        }
    }

    /// For this machine: the real OS lookups on macOS; elsewhere nothing is ever trusted.
    pub fn for_this_machine(hq_dir: &Path) -> Self {
        Self::new(hq_dir, Arc::new(SystemProcesses))
    }

    pub fn holds(&self, listen: SocketAddr) -> bool {
        let mut checked = self.checked.lock().unwrap();
        let fresh = checked.listen == Some(listen)
            && checked.at.is_some_and(|at| at.elapsed() < FULL_CHECK_EVERY);
        if fresh {
            // The same process must still be there: a pid that ended or was reused is not.
            return checked
                .holder
                .is_some_and(|(pid, started)| self.os.started(pid) == Some(started));
        }
        checked.holder = holder_of(&self.record, listen, self.os.as_ref());
        checked.at = Some(Instant::now());
        checked.listen = Some(listen);
        checked.holder.is_some()
    }
}

pub struct SystemProcesses;

#[cfg(target_os = "macos")]
impl Processes for SystemProcesses {
    fn started(&self, pid: u32) -> Option<SystemTime> {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        // SAFETY: `info` is a writable buffer of exactly `size` bytes for PROC_PIDTBSDINFO.
        let got = unsafe {
            libc::proc_pidinfo(
                pid as libc::c_int,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size,
            )
        };
        (got == size).then(|| {
            SystemTime::UNIX_EPOCH
                + Duration::from_secs(info.pbi_start_tvsec)
                + Duration::from_micros(info.pbi_start_tvusec)
        })
    }

    fn listens_on(&self, pid: u32, addr: SocketAddr) -> bool {
        const LSOF: &str = "/usr/sbin/lsof";
        const POLL: Duration = Duration::from_millis(20);
        let child = std::process::Command::new(LSOF)
            .args(["-a", "-nP", "-p", &pid.to_string()])
            .arg(format!("-iTCP@{addr}"))
            .args(["-sTCP:LISTEN", "-Fn"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn();
        let Ok(mut child) = child else { return false };
        let deadline = Instant::now() + LOOKUP_LIMIT;
        while matches!(child.try_wait(), Ok(None)) {
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            std::thread::sleep(POLL);
        }
        let Ok(out) = child.wait_with_output() else {
            return false;
        };
        let wanted = format!("n{addr}");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|line| line == wanted)
    }
}

#[cfg(not(target_os = "macos"))]
impl Processes for SystemProcesses {
    fn started(&self, _: u32) -> Option<SystemTime> {
        None
    }
    fn listens_on(&self, _: u32, _: SocketAddr) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        started: Option<SystemTime>,
        listens: Option<SocketAddr>,
    }

    impl Processes for Fake {
        fn started(&self, _: u32) -> Option<SystemTime> {
            self.started
        }
        fn listens_on(&self, _: u32, addr: SocketAddr) -> bool {
            self.listens == Some(addr)
        }
    }

    fn listen() -> SocketAddr {
        "127.0.0.1:47480".parse().unwrap()
    }

    fn long_ago() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1)
    }

    fn check(home: &Path, fake: Fake) -> bool {
        holder_of(&holder::record_path(home), listen(), &fake).is_some()
    }

    #[test]
    fn hqs_bridge_is_trusted_only_when_every_rule_holds() {
        let home = tempfile::TempDir::new().unwrap();
        let ok = || Fake {
            started: Some(long_ago()),
            listens: Some(listen()),
        };
        assert!(!check(home.path(), ok()), "no record");

        holder::write(home.path(), vec![listen()]).unwrap();
        assert!(check(home.path(), ok()));
        assert!(
            !check(
                home.path(),
                Fake {
                    started: None,
                    ..ok()
                }
            ),
            "the process ended"
        );
        let later = SystemTime::now() + Duration::from_secs(60);
        assert!(
            !check(
                home.path(),
                Fake {
                    started: Some(later),
                    ..ok()
                }
            ),
            "a reused pid"
        );
        assert!(
            !check(
                home.path(),
                Fake {
                    listens: None,
                    ..ok()
                }
            ),
            "it does not hold the port"
        );
        let other: SocketAddr = "127.0.0.1:47481".parse().unwrap();
        assert!(
            !check(
                home.path(),
                Fake {
                    listens: Some(other),
                    ..ok()
                }
            ),
            "another address"
        );

        holder::write(home.path(), vec![other]).unwrap();
        assert!(!check(home.path(), ok()), "the record names another port");
    }

    #[cfg(unix)]
    #[test]
    fn a_record_others_could_write_is_not_trusted() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::TempDir::new().unwrap();
        holder::write(home.path(), vec![listen()]).unwrap();
        let path = holder::record_path(home.path());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        let fake = Fake {
            started: Some(long_ago()),
            listens: Some(listen()),
        };
        assert!(!check(home.path(), fake));
    }

    #[test]
    fn a_cached_yes_ends_with_the_process() {
        let home = tempfile::TempDir::new().unwrap();
        holder::write(home.path(), vec![listen()]).unwrap();
        struct Ends(Mutex<bool>);
        impl Processes for Ends {
            fn started(&self, _: u32) -> Option<SystemTime> {
                (!*self.0.lock().unwrap()).then(long_ago)
            }
            fn listens_on(&self, _: u32, _: SocketAddr) -> bool {
                true
            }
        }
        let os = Arc::new(Ends(Mutex::new(false)));
        let trust = BridgeTrust::new(home.path(), os.clone());
        assert!(trust.holds(listen()));
        *os.0.lock().unwrap() = true;
        assert!(
            !trust.holds(listen()),
            "the cached answer is dropped once the process is gone"
        );
    }

    /// The real lookups on macOS: this test process listens and names itself in the record.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_real_lookup_finds_a_listener_this_process_holds() {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = socket.local_addr().unwrap();
        let home = tempfile::TempDir::new().unwrap();
        holder::write(home.path(), vec![addr]).unwrap();
        let trust = BridgeTrust::for_this_machine(home.path());
        assert!(trust.holds(addr));
        let free: SocketAddr = "127.0.0.1:1".parse().unwrap();
        assert!(!SystemProcesses.listens_on(std::process::id(), free));
    }
}
