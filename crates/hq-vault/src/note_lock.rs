//! Per-note striped lock shared by every writer and reviewer of a vault note.
//!
//! Callers take [`lock_note`] across read, compare, keep, write, and provenance record so no
//! other writer or restore can slip in between those steps.
//!
//! Lock order: always take the note lock before any database connection, never hold two note
//! locks at once, and never hold a [`NoteGuard`] across an `.await` (run async tool
//! read-modify-write steps inside `tokio::task::spawn_blocking` or a synchronous block).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, MutexGuard};

/// How many locks the notes share. A path always maps to the same one, so two writers of one note
/// meet; unrelated notes may share a lock, which only makes them wait a moment.
const NOTE_LOCKS: usize = 32;

#[cfg(debug_assertions)]
std::thread_local! {
    static HELD_STRIPES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Guard returned by [`lock_note`]. Holds the stripe mutex until dropped.
pub struct NoteGuard {
    _guard: MutexGuard<'static, ()>,
    #[cfg(debug_assertions)]
    slot: usize,
}

impl Drop for NoteGuard {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        HELD_STRIPES.with(|held| {
            held.set(held.get() & !(1u32 << self.slot));
        });
    }
}

/// Held while a note is read, compared, kept, written, and recorded, so nothing slips in between
/// those steps.
pub fn lock_note(rel: &str) -> NoteGuard {
    static LOCKS: [Mutex<()>; NOTE_LOCKS] = [const { Mutex::new(()) }; NOTE_LOCKS];
    let slot = stripe_of(rel);
    #[cfg(debug_assertions)]
    HELD_STRIPES.with(|held| {
        let mask = 1u32 << slot;
        debug_assert!(
            held.get() & mask == 0,
            "re-entrant note_lock on stripe {slot} for '{rel}'"
        );
        held.set(held.get() | mask);
    });
    let guard = LOCKS[slot]
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    NoteGuard {
        _guard: guard,
        #[cfg(debug_assertions)]
        slot,
    }
}

fn stripe_of(rel: &str) -> usize {
    let mut hasher = DefaultHasher::new();
    // `a//b` and `./a/b` name the same file, so they must pick the same lock.
    for part in rel.split('/').filter(|p| !p.is_empty() && *p != ".") {
        part.hash(&mut hasher);
    }
    (hasher.finish() % NOTE_LOCKS as u64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::time::Duration;

    #[test]
    fn spellings_of_one_path_share_a_lock() {
        let _held = lock_note("Notebooks/Inbox/a.md");
        let (tx, rx) = std::sync::mpsc::channel();
        let other = std::thread::spawn(move || {
            let _again = lock_note("./Notebooks//Inbox/a.md");
            tx.send(()).unwrap();
        });
        // The second spelling must wait for the first; if it got another lock it would not.
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
        drop(_held);
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        other.join().unwrap();
    }

    #[test]
    fn two_notes_sharing_a_stripe_both_finish_without_deadlock() {
        let first = "Notebooks/Inbox/stripe-a.md";
        let target_stripe = stripe_of(first);
        let second = (0..10_000)
            .map(|i| format!("Notebooks/Inbox/stripe-{i}.md"))
            .find(|p| p != first && stripe_of(p) == target_stripe)
            .expect("32 stripes guarantee a collision within 10k paths");

        let barrier = Arc::new(Barrier::new(2));
        let b1 = barrier.clone();
        let b2 = barrier.clone();

        let t1 = std::thread::spawn(move || {
            b1.wait();
            for _ in 0..50 {
                let _g = lock_note(first);
                std::thread::yield_now();
            }
        });
        let t2 = std::thread::spawn(move || {
            b2.wait();
            for _ in 0..50 {
                let _g = lock_note(&second);
                std::thread::yield_now();
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "re-entrant note_lock")]
    fn reentering_same_stripe_panics_in_debug() {
        let _first = lock_note("Notebooks/Inbox/reentry.md");
        let _second = lock_note("Notebooks/Inbox/reentry.md");
    }
}
