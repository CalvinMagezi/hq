//! Shares one in-flight read between concurrent callers and keeps a successful
//! result for a very short time, so overlapping browser polls of one session
//! cost a single herdr (ssh) invocation.

use anyhow::{Result, anyhow};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Most distinct (session, lines) reads tracked at once.
const MAX_ENTRIES: usize = 64;
/// Long enough to absorb overlapping polls, short enough to look live.
const FRESH_FOR: Duration = Duration::from_secs(1);

type Shared = Result<Value, String>;

struct Flight {
    done: Mutex<Option<Shared>>,
    ready: Condvar,
}

enum Slot {
    InFlight(Arc<Flight>),
    Fresh(Instant, Value),
}

pub(super) struct Coalescer {
    slots: Mutex<HashMap<(String, usize), Slot>>,
    fresh_for: Duration,
    /// Bumped by `forget`; a read that started before a bump is not cached.
    epoch: AtomicU64,
}

/// Resolves the flight even if the loader panics, so waiters never hang.
struct Resolve<'a> {
    owner: &'a Coalescer,
    key: (String, usize),
    flight: Arc<Flight>,
    outcome: Option<Shared>,
    started_epoch: u64,
    cacheable: bool,
}

impl Drop for Resolve<'_> {
    fn drop(&mut self) {
        let outcome = self
            .outcome
            .take()
            .unwrap_or_else(|| Err("screen read aborted".into()));
        {
            let mut slots = self.owner.slots.lock().unwrap_or_else(|e| e.into_inner());
            let current = self.owner.epoch.load(Ordering::SeqCst) == self.started_epoch;
            match &outcome {
                Ok(v) if self.cacheable && current => {
                    slots.insert(self.key.clone(), Slot::Fresh(Instant::now(), v.clone()));
                }
                _ => {
                    slots.remove(&self.key);
                }
            }
        }
        *self.flight.done.lock().unwrap_or_else(|e| e.into_inner()) = Some(outcome);
        self.flight.ready.notify_all();
    }
}

impl Coalescer {
    pub(super) fn new() -> Self {
        Self::with_ttl(FRESH_FOR)
    }

    fn with_ttl(fresh_for: Duration) -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
            fresh_for,
            epoch: AtomicU64::new(0),
        }
    }

    /// Drops what is held for a session whose screen just changed (a send, a
    /// stop, a resume), and keeps reads already in flight from being cached.
    pub(super) fn forget(&self, session_id: &str) {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        self.epoch.fetch_add(1, Ordering::SeqCst);
        slots.retain(|(id, _), s| id != session_id || matches!(s, Slot::InFlight(_)));
    }

    pub(super) fn get(
        &self,
        session_id: &str,
        lines: usize,
        load: impl FnOnce() -> Result<Value>,
        cacheable: impl Fn(&Value) -> bool,
    ) -> Result<Value> {
        let key = (session_id.to_string(), lines);
        let started_epoch;
        let flight = {
            let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
            match slots.get(&key) {
                Some(Slot::Fresh(at, v)) if at.elapsed() < self.fresh_for => return Ok(v.clone()),
                Some(Slot::InFlight(f)) => {
                    let f = f.clone();
                    drop(slots);
                    return wait(&f);
                }
                _ => {}
            }
            let fresh_for = self.fresh_for;
            slots.retain(|_, s| match s {
                Slot::Fresh(at, _) => at.elapsed() < fresh_for,
                Slot::InFlight(_) => true,
            });
            if slots.len() >= MAX_ENTRIES {
                drop(slots);
                return load();
            }
            let flight = Arc::new(Flight {
                done: Mutex::new(None),
                ready: Condvar::new(),
            });
            slots.insert(key.clone(), Slot::InFlight(flight.clone()));
            started_epoch = self.epoch.load(Ordering::SeqCst);
            flight
        };
        let mut guard = Resolve {
            owner: self,
            key,
            flight,
            outcome: None,
            started_epoch,
            cacheable: false,
        };
        let result = load();
        guard.cacheable = result.as_ref().is_ok_and(&cacheable);
        guard.outcome = Some(match &result {
            Ok(v) => Ok(v.clone()),
            Err(e) => Err(format!("{e:#}")),
        });
        result
    }
}

fn wait(flight: &Flight) -> Result<Value> {
    let mut done = flight.done.lock().unwrap_or_else(|e| e.into_inner());
    while done.is_none() {
        done = flight.ready.wait(done).unwrap_or_else(|e| e.into_inner());
    }
    match done.as_ref().expect("checked above") {
        Ok(v) => Ok(v.clone()),
        Err(msg) => Err(anyhow!("{msg}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::AtomicUsize;

    fn yes(_: &Value) -> bool {
        true
    }

    #[test]
    fn concurrent_reads_share_one_load() {
        let c = Arc::new(Coalescer::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (c, calls, barrier) = (c.clone(), calls.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    c.get(
                        "s1",
                        200,
                        || {
                            calls.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(150));
                            Ok(json!({"n": 1}))
                        },
                        yes,
                    )
                    .unwrap()
                })
            })
            .collect();
        for h in handles {
            assert_eq!(h.join().unwrap(), json!({"n": 1}));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_result_is_reused_within_the_ttl_then_reloaded() {
        let c = Coalescer::with_ttl(Duration::from_millis(80));
        let calls = AtomicUsize::new(0);
        let load = || Ok(json!(calls.fetch_add(1, Ordering::SeqCst)));
        assert_eq!(c.get("s", 1, load, yes).unwrap(), json!(0));
        assert_eq!(c.get("s", 1, load, yes).unwrap(), json!(0));
        std::thread::sleep(Duration::from_millis(120));
        assert_eq!(c.get("s", 1, load, yes).unwrap(), json!(1));
        assert_eq!(c.get("other", 1, load, yes).unwrap(), json!(2));
    }

    #[test]
    fn a_failed_read_is_shared_with_waiters_but_never_cached() {
        let c = Arc::new(Coalescer::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let handles: Vec<_> = (0..3)
            .map(|_| {
                let (c, calls, barrier) = (c.clone(), calls.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    c.get(
                        "s",
                        5,
                        || {
                            calls.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(100));
                            Err(anyhow!("no such session"))
                        },
                        yes,
                    )
                    .unwrap_err()
                    .to_string()
                })
            })
            .collect();
        for h in handles {
            assert!(h.join().unwrap().contains("no such session"));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let again = c.get("s", 5, || Ok(json!("ok")), yes).unwrap();
        assert_eq!(again, json!("ok"));
    }

    #[test]
    fn the_table_stays_bounded() {
        let c = Coalescer::new();
        for i in 0..(MAX_ENTRIES * 3) {
            c.get(&format!("s{i}"), 1, || Ok(json!(i)), yes).unwrap();
        }
        assert!(c.slots.lock().unwrap().len() <= MAX_ENTRIES);
    }

    #[test]
    fn a_panicking_load_does_not_wedge_the_key() {
        let c = Arc::new(Coalescer::new());
        let c2 = c.clone();
        let r = std::thread::spawn(move || c2.get("s", 1, || panic!("boom"), yes)).join();
        assert!(r.is_err());
        assert_eq!(c.get("s", 1, || Ok(json!(1)), yes).unwrap(), json!(1));
    }

    #[test]
    fn a_result_the_caller_marks_uncacheable_is_not_kept() {
        let c = Coalescer::new();
        let calls = AtomicUsize::new(0);
        let load = || Ok(json!({"source": "snapshot", "n": calls.fetch_add(1, Ordering::SeqCst)}));
        let live = |v: &Value| v["source"] == "live";
        c.get("s", 1, load, live).unwrap();
        c.get("s", 1, load, live).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn forget_drops_the_cache_and_keeps_an_overlapping_read_from_being_cached() {
        let c = Arc::new(Coalescer::new());
        c.get("s", 1, || Ok(json!(1)), yes).unwrap();
        c.forget("s");
        assert_eq!(c.get("s", 1, || Ok(json!(2)), yes).unwrap(), json!(2));
        c.forget("other");
        assert_eq!(c.get("s", 1, || Ok(json!(3)), yes).unwrap(), json!(2));

        let (c2, started) = (c.clone(), Arc::new(std::sync::Barrier::new(2)));
        let started2 = started.clone();
        c.forget("s");
        let t = std::thread::spawn(move || {
            c2.get(
                "s",
                9,
                || {
                    started2.wait();
                    std::thread::sleep(Duration::from_millis(100));
                    Ok(json!("pre-send"))
                },
                yes,
            )
            .unwrap()
        });
        started.wait();
        c.forget("s");
        t.join().unwrap();
        assert_eq!(
            c.get("s", 9, || Ok(json!("post-send")), yes).unwrap(),
            json!("post-send")
        );
    }
}
