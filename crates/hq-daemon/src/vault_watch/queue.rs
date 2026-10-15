//! The pipeline's decision logic: what to re-check, and when.
//!
//! A pure state machine. Time is passed in, so debouncing, retries and overflow are tested
//! without sleeping. Events only mark a path or folder as "look again"; nothing here knows or
//! trusts what an event said happened, because the consumer reads the file when it acts.

use std::collections::HashMap;
use std::time::Duration;

use tokio::time::Instant;

use super::normalize::WatchEvent;

/// A unit of work for the consumer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Work {
    /// Reconcile one note against the file as it is now.
    Note(String),
    /// Reconcile everything under a folder.
    Folder(String),
    /// Scan the whole of `Notebooks/`.
    Everything,
}

#[derive(Debug, Clone, Copy)]
pub struct EngineConfig {
    /// A path is read once this long has passed with no further events for it.
    pub debounce: Duration,
    /// A path that never goes quiet (something appending to a note) is still read after this.
    pub max_wait: Duration,
    /// Distinct paths and folders held at once. Beyond this the queue gives up on tracking them
    /// individually and asks for a full scan.
    pub capacity: usize,
    /// Tries for one piece of work before it is left to the scheduled scan.
    pub max_attempts: u32,
    /// First retry delay; doubles on each failure.
    pub retry_base: Duration,
    /// Full scans are never closer together than this (it doubles while scans keep failing).
    pub scan_gap: Duration,
}

impl EngineConfig {
    pub fn with_debounce(debounce: Duration) -> Self {
        Self {
            debounce,
            max_wait: (debounce * 10).max(Duration::from_secs(2)),
            capacity: 4096,
            max_attempts: 4,
            retry_base: Duration::from_secs(2),
            scan_gap: Duration::from_secs(5),
        }
    }
}

/// What `ingest` did with an event, for the counters.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Ingest {
    pub queued: u32,
    /// Events merged into a path that was already waiting.
    pub coalesced: u32,
    /// The queue was full and was replaced by a request for a full scan.
    pub queue_overflow: bool,
}

/// What `fail` decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retry {
    At(Instant),
    /// Out of attempts; the scheduled scan is the floor.
    GaveUp,
}

#[derive(Debug)]
struct Entry {
    first: Instant,
    last: Instant,
    attempts: u32,
    not_before: Option<Instant>,
}

impl Entry {
    fn ready_at(&self, cfg: &EngineConfig) -> Instant {
        let quiet = (self.last + cfg.debounce).min(self.first + cfg.max_wait);
        self.not_before.map_or(quiet, |t| t.max(quiet))
    }
}

#[derive(Debug)]
pub struct Engine {
    cfg: EngineConfig,
    pending: HashMap<Work, Entry>,
    scan_requested: bool,
    last_scan: Option<Instant>,
    scan_failures: u32,
}

fn is_under(path: &str, folder: &str) -> bool {
    path.strip_prefix(folder)
        .is_some_and(|rest| rest.starts_with('/'))
}

fn depth(path: &str) -> usize {
    path.matches('/').count()
}

impl Engine {
    pub fn new(cfg: EngineConfig) -> Self {
        Self {
            cfg,
            pending: HashMap::new(),
            scan_requested: false,
            last_scan: None,
            scan_failures: 0,
        }
    }

    /// Number of paths and folders waiting.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Ask for a full scan: something was lost, or the watcher has just started.
    pub fn request_scan(&mut self) {
        self.scan_requested = true;
    }

    fn touch(&mut self, work: Work, now: Instant, out: &mut Ingest) {
        if let Some(entry) = self.pending.get_mut(&work) {
            entry.last = now;
            out.coalesced += 1;
            return;
        }
        if self.pending.len() >= self.cfg.capacity {
            self.scan_requested = true;
            out.queue_overflow = true;
            return;
        }
        self.pending.insert(
            work,
            Entry {
                first: now,
                last: now,
                attempts: 0,
                not_before: None,
            },
        );
        out.queued += 1;
    }

    pub fn ingest(&mut self, event: WatchEvent, now: Instant) -> Ingest {
        let mut out = Ingest::default();
        match event {
            WatchEvent::Created(p) | WatchEvent::Changed(p) | WatchEvent::Deleted(p) => {
                self.touch(Work::Note(p), now, &mut out)
            }
            WatchEvent::Renamed { from, to } => {
                self.touch(Work::Note(from), now, &mut out);
                self.touch(Work::Note(to), now, &mut out);
            }
            WatchEvent::Folder(p) => self.touch(Work::Folder(p), now, &mut out),
            WatchEvent::Overflow => self.scan_requested = true,
        }
        if out.queue_overflow {
            // Everything waiting is covered by the scan that is now requested.
            self.pending.clear();
        }
        out
    }

    fn scan_gap(&self) -> Duration {
        self.cfg.scan_gap * 2u32.pow(self.scan_failures.min(6))
    }

    fn scan_ready_at(&self, now: Instant) -> Instant {
        self.last_scan.map_or(now, |t| t + self.scan_gap())
    }

    /// Take the work that is ready at `now`. A full scan comes alone: it covers everything
    /// waiting. Otherwise folders come first, shallowest first, and notes inside a ready folder
    /// are dropped because the folder's reconcile covers them.
    pub fn take_due(&mut self, now: Instant) -> Vec<(Work, u32)> {
        if self.scan_requested && now >= self.scan_ready_at(now) {
            self.scan_requested = false;
            self.last_scan = Some(now);
            self.pending.clear();
            return vec![(Work::Everything, 0)];
        }

        let mut folders: Vec<String> = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        for (work, entry) in &self.pending {
            if entry.ready_at(&self.cfg) > now {
                continue;
            }
            match work {
                Work::Folder(p) => folders.push(p.clone()),
                Work::Note(p) => notes.push(p.clone()),
                Work::Everything => {}
            }
        }
        folders.sort_by(|a, b| (depth(a), a).cmp(&(depth(b), b)));
        let mut roots: Vec<String> = Vec::new();
        for folder in folders {
            if !roots.iter().any(|root| is_under(&folder, root)) {
                roots.push(folder);
            }
        }
        notes.sort();

        let mut due = Vec::new();
        for root in &roots {
            let attempts = self
                .pending
                .remove(&Work::Folder(root.clone()))
                .map_or(0, |e| e.attempts);
            due.push((Work::Folder(root.clone()), attempts));
        }
        // Folders nested inside a ready folder, ready or not, and notes inside any ready folder.
        self.pending.retain(|work, _| match work {
            Work::Folder(p) | Work::Note(p) => !roots.iter().any(|root| is_under(p, root)),
            Work::Everything => true,
        });
        for note in notes {
            if let Some(entry) = self.pending.remove(&Work::Note(note.clone())) {
                due.push((Work::Note(note), entry.attempts));
            }
        }
        due
    }

    /// Record a failure of `work`, which was tried `attempts` times before this. It is queued
    /// again after a doubling delay, or given up on.
    pub fn fail(&mut self, work: Work, attempts: u32, now: Instant) -> Retry {
        if work == Work::Everything {
            self.scan_failures += 1;
            self.scan_requested = true;
            return Retry::At(self.scan_ready_at(now));
        }
        let attempts = attempts + 1;
        if attempts >= self.cfg.max_attempts {
            return Retry::GaveUp;
        }
        let at = now + self.cfg.retry_base * 2u32.pow(attempts - 1);
        // A newer event for the same path may already be waiting; keep the earlier try count.
        let entry = self.pending.entry(work).or_insert(Entry {
            first: now,
            last: now,
            attempts,
            not_before: None,
        });
        entry.attempts = entry.attempts.max(attempts);
        entry.not_before = Some(at);
        Retry::At(at)
    }

    /// A full scan finished; scans may follow at the normal pace again.
    pub fn scan_succeeded(&mut self) {
        self.scan_failures = 0;
    }

    /// When `take_due` will next have something, or `None` when idle.
    pub fn next_wakeup(&self, now: Instant) -> Option<Instant> {
        let scan = self.scan_requested.then(|| self.scan_ready_at(now));
        let queued = self.pending.values().map(|e| e.ready_at(&self.cfg)).min();
        match (scan, queued) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEBOUNCE: Duration = Duration::from_millis(300);

    fn engine() -> (Engine, Instant) {
        (
            Engine::new(EngineConfig::with_debounce(DEBOUNCE)),
            Instant::now(),
        )
    }

    fn changed(p: &str) -> WatchEvent {
        WatchEvent::Changed(p.into())
    }

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    fn works(due: Vec<(Work, u32)>) -> Vec<Work> {
        due.into_iter().map(|(w, _)| w).collect()
    }

    #[test]
    fn a_burst_of_saves_is_read_once_after_it_goes_quiet() {
        let (mut e, t0) = engine();
        let mut coalesced = 0;
        for i in 0..50 {
            coalesced += e.ingest(changed("Notebooks/a.md"), at(t0, i * 5)).coalesced;
        }
        assert_eq!(coalesced, 49);
        assert_eq!(e.pending(), 1);
        // 50 saves over 250 ms: not quiet yet at 400 ms, quiet 300 ms after the last (245 ms).
        assert!(e.take_due(at(t0, 400)).is_empty());
        assert_eq!(
            works(e.take_due(at(t0, 546))),
            vec![Work::Note("Notebooks/a.md".into())]
        );
        assert_eq!(e.pending(), 0);
    }

    #[test]
    fn nothing_is_due_before_the_debounce_has_passed() {
        let (mut e, t0) = engine();
        e.ingest(changed("Notebooks/a.md"), t0);
        assert!(e.take_due(at(t0, 299)).is_empty());
        assert_eq!(e.take_due(at(t0, 300)).len(), 1);
    }

    #[test]
    fn a_path_that_never_goes_quiet_is_still_read() {
        let (mut e, t0) = engine();
        // An event every 100 ms for 4 seconds keeps pushing the quiet point away.
        for i in 0..40 {
            e.ingest(changed("Notebooks/log.md"), at(t0, i * 100));
        }
        let max_wait = EngineConfig::with_debounce(DEBOUNCE).max_wait;
        assert_eq!(max_wait, Duration::from_secs(3));
        assert_eq!(e.take_due(at(t0, 3000)).len(), 1);
    }

    #[test]
    fn duplicate_and_out_of_order_events_collapse_to_one_look() {
        let (mut e, t0) = engine();
        e.ingest(WatchEvent::Deleted("Notebooks/a.md".into()), t0);
        e.ingest(WatchEvent::Created("Notebooks/a.md".into()), at(t0, 10));
        e.ingest(WatchEvent::Deleted("Notebooks/a.md".into()), at(t0, 20));
        assert_eq!(
            works(e.take_due(at(t0, 400))),
            vec![Work::Note("Notebooks/a.md".into())]
        );
    }

    #[test]
    fn a_rename_looks_at_both_ends() {
        let (mut e, t0) = engine();
        e.ingest(
            WatchEvent::Renamed {
                from: "Notebooks/a.md".into(),
                to: "Notebooks/b.md".into(),
            },
            t0,
        );
        assert_eq!(
            works(e.take_due(at(t0, 400))),
            vec![
                Work::Note("Notebooks/a.md".into()),
                Work::Note("Notebooks/b.md".into())
            ]
        );
    }

    #[test]
    fn each_path_waits_for_its_own_quiet_period() {
        let (mut e, t0) = engine();
        e.ingest(changed("Notebooks/P/a.md"), t0);
        e.ingest(changed("Notebooks/Q/b.md"), t0);
        e.ingest(WatchEvent::Folder("Notebooks/P/deep".into()), t0);
        e.ingest(WatchEvent::Folder("Notebooks/P".into()), at(t0, 250));
        // The event for folder P is 100 ms old at 350 ms, so it is not ready; the rest are, and
        // none of them sits inside a folder that is ready yet, so each runs by itself.
        let first = works(e.take_due(at(t0, 350)));
        assert_eq!(
            first,
            vec![
                Work::Folder("Notebooks/P/deep".into()),
                Work::Note("Notebooks/P/a.md".into()),
                Work::Note("Notebooks/Q/b.md".into()),
            ]
        );
        let second = works(e.take_due(at(t0, 600)));
        assert_eq!(second, vec![Work::Folder("Notebooks/P".into())]);
    }

    #[test]
    fn when_a_folder_and_its_notes_are_ready_together_only_the_folder_runs() {
        let (mut e, t0) = engine();
        e.ingest(changed("Notebooks/P/a.md"), t0);
        e.ingest(changed("Notebooks/P/sub/b.md"), t0);
        e.ingest(WatchEvent::Folder("Notebooks/P/sub".into()), t0);
        e.ingest(WatchEvent::Folder("Notebooks/P".into()), t0);
        e.ingest(changed("Notebooks/PX/c.md"), t0);
        let due = works(e.take_due(at(t0, 400)));
        assert_eq!(
            due,
            vec![
                Work::Folder("Notebooks/P".into()),
                Work::Note("Notebooks/PX/c.md".into())
            ]
        );
        assert_eq!(e.pending(), 0);
    }

    #[test]
    fn an_overflow_event_replaces_everything_with_one_full_scan() {
        let (mut e, t0) = engine();
        e.ingest(changed("Notebooks/a.md"), t0);
        e.ingest(WatchEvent::Overflow, at(t0, 10));
        let due = e.take_due(at(t0, 20));
        assert_eq!(works(due), vec![Work::Everything]);
        assert_eq!(e.pending(), 0, "the scan covers what was waiting");
        assert!(e.take_due(at(t0, 10_000)).is_empty());
    }

    #[test]
    fn a_full_queue_asks_for_a_scan_instead_of_dropping_events() {
        let mut cfg = EngineConfig::with_debounce(DEBOUNCE);
        cfg.capacity = 3;
        let mut e = Engine::new(cfg);
        let t0 = Instant::now();
        for i in 0..3 {
            assert!(
                !e.ingest(changed(&format!("Notebooks/{i}.md")), t0)
                    .queue_overflow
            );
        }
        let over = e.ingest(changed("Notebooks/3.md"), t0);
        assert!(over.queue_overflow);
        assert_eq!(works(e.take_due(t0)), vec![Work::Everything]);
        // A path already waiting never overflows the queue.
        let mut e = Engine::new(cfg);
        for i in 0..3 {
            e.ingest(changed(&format!("Notebooks/{i}.md")), t0);
        }
        assert!(!e.ingest(changed("Notebooks/1.md"), t0).queue_overflow);
    }

    #[test]
    fn full_scans_are_spaced_out() {
        let (mut e, t0) = engine();
        e.request_scan();
        assert_eq!(works(e.take_due(t0)), vec![Work::Everything]);
        e.scan_succeeded();
        e.request_scan();
        assert!(e.take_due(at(t0, 4_999)).is_empty());
        assert_eq!(e.next_wakeup(at(t0, 100)), Some(at(t0, 5_000)));
        assert_eq!(works(e.take_due(at(t0, 5_000))), vec![Work::Everything]);
    }

    #[test]
    fn a_failing_scan_backs_off() {
        let (mut e, t0) = engine();
        e.request_scan();
        e.take_due(t0);
        // First failure: next try one gap (5 s) later... doubling to 10 s.
        assert_eq!(e.fail(Work::Everything, 0, t0), Retry::At(at(t0, 10_000)));
        assert!(e.take_due(at(t0, 9_999)).is_empty());
        assert_eq!(works(e.take_due(at(t0, 10_000))), vec![Work::Everything]);
        e.scan_succeeded();
        e.request_scan();
        assert_eq!(e.next_wakeup(at(t0, 10_001)), Some(at(t0, 15_000)));
    }

    #[test]
    fn a_failed_note_is_retried_with_doubling_delays_then_left_to_the_scan() {
        let (mut e, t0) = engine();
        let work = Work::Note("Notebooks/a.md".into());
        e.ingest(changed("Notebooks/a.md"), t0);
        let due = e.take_due(at(t0, 400));
        assert_eq!(due, vec![(work.clone(), 0)]);

        // Try 1 fails: retry in 2 s.
        let now = at(t0, 400);
        assert_eq!(
            e.fail(work.clone(), 0, now),
            Retry::At(now + Duration::from_secs(2))
        );
        assert!(e.take_due(now + Duration::from_millis(1999)).is_empty());
        let due = e.take_due(now + Duration::from_secs(2));
        assert_eq!(due, vec![(work.clone(), 1)]);

        // Try 2 fails: 4 s. Try 3 fails: 8 s.
        let now = now + Duration::from_secs(2);
        assert_eq!(
            e.fail(work.clone(), 1, now),
            Retry::At(now + Duration::from_secs(4))
        );
        let now = now + Duration::from_secs(4);
        assert_eq!(e.take_due(now), vec![(work.clone(), 2)]);
        assert_eq!(
            e.fail(work.clone(), 2, now),
            Retry::At(now + Duration::from_secs(8))
        );
        let now = now + Duration::from_secs(8);
        assert_eq!(e.take_due(now), vec![(work.clone(), 3)]);
        // The fourth failure is the last.
        assert_eq!(e.fail(work, 3, now), Retry::GaveUp);
        assert_eq!(e.pending(), 0);
    }

    #[test]
    fn a_new_event_while_a_retry_waits_does_not_reset_the_backoff() {
        let (mut e, t0) = engine();
        let work = Work::Note("Notebooks/a.md".into());
        e.ingest(changed("Notebooks/a.md"), t0);
        e.take_due(at(t0, 400));
        let now = at(t0, 400);
        e.fail(work.clone(), 0, now);
        e.ingest(changed("Notebooks/a.md"), now + Duration::from_millis(500));
        // Still held back until the retry time, even though the new event has gone quiet.
        assert!(e.take_due(now + Duration::from_millis(1900)).is_empty());
        assert_eq!(e.take_due(now + Duration::from_secs(2)), vec![(work, 1)]);
    }

    #[test]
    fn next_wakeup_is_the_earliest_of_everything_waiting() {
        let (mut e, t0) = engine();
        assert_eq!(e.next_wakeup(t0), None);
        e.ingest(changed("Notebooks/a.md"), t0);
        e.ingest(changed("Notebooks/b.md"), at(t0, 100));
        assert_eq!(e.next_wakeup(t0), Some(at(t0, 300)));
        e.request_scan();
        assert_eq!(e.next_wakeup(t0), Some(t0));
    }
}
