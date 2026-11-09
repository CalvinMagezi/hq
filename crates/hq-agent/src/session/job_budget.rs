//! One spending limit for a coding job, shared by its turn and every child that turn starts.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Why a job's money ran out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStop {
    Budget,
    Credits,
}

/// Where a job's spending is kept, so a killed run has already paid for what it used.
pub trait JobLedger: Send + Sync {
    fn add_spend(&self, usd: f64);
    fn stopped(&self, why: JobStop);
}

pub struct JobBudget {
    spent: Mutex<f64>,
    credit_ceiling: Option<f64>,
    credits_reached: AtomicBool,
    ledger: Arc<dyn JobLedger>,
}

impl std::fmt::Debug for JobBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobBudget")
            .field("spent", &self.spent())
            .field("credit_ceiling", &self.credit_ceiling)
            .finish_non_exhaustive()
    }
}

impl JobBudget {
    /// `spent` is what earlier attempts already used.
    pub fn new(spent: f64, credit_ceiling: Option<f64>, ledger: Arc<dyn JobLedger>) -> Self {
        Self {
            spent: Mutex::new(spent),
            credit_ceiling,
            credits_reached: AtomicBool::new(false),
            ledger,
        }
    }

    pub fn has_credit_ceiling(&self) -> bool {
        self.credit_ceiling.is_some()
    }

    fn guard(&self) -> std::sync::MutexGuard<'_, f64> {
        self.spent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn spent(&self) -> f64 {
        *self.guard()
    }

    pub(super) fn add(&self, usd: f64) {
        *self.guard() += usd;
        self.ledger.add_spend(usd);
    }

    /// The account-wide counter after a step. Other Copilot use moves it too, so this can stop a
    /// job early but cannot let other use push it past its cap.
    pub(super) fn credit_reading(&self, used: f64) {
        if self.credit_ceiling.is_some_and(|ceiling| used >= ceiling) {
            self.credits_reached.store(true, Ordering::SeqCst);
        }
    }

    /// Why the job must stop now, recording it for the dispatcher. `budget` is the job's limit.
    pub(super) fn stop_now(&self, budget: Option<f64>) -> Option<JobStop> {
        let why = if self.credits_reached.load(Ordering::SeqCst) {
            JobStop::Credits
        } else if budget.is_some_and(|limit| self.spent() >= limit) {
            JobStop::Budget
        } else {
            return None;
        };
        self.ledger.stopped(why);
        Some(why)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Book {
        added: Mutex<f64>,
        stops: Mutex<Vec<JobStop>>,
    }

    impl JobLedger for Book {
        fn add_spend(&self, usd: f64) {
            *self.added.lock().unwrap() += usd;
        }
        fn stopped(&self, why: JobStop) {
            self.stops.lock().unwrap().push(why);
        }
    }

    #[test]
    fn earlier_attempts_count_and_each_call_is_written_down() {
        let book = Arc::new(Book::default());
        let job = JobBudget::new(0.40, None, book.clone());
        assert_eq!(job.stop_now(Some(0.50)), None);
        job.add(0.10);
        assert!((*book.added.lock().unwrap() - 0.10).abs() < 1e-9);
        assert_eq!(job.stop_now(Some(0.50)), Some(JobStop::Budget));
        assert_eq!(*book.stops.lock().unwrap(), vec![JobStop::Budget]);
    }

    #[test]
    fn the_credit_ceiling_stops_a_job_with_money_left() {
        let job = JobBudget::new(0.0, Some(150.0), Arc::new(Book::default()));
        job.credit_reading(149.0);
        assert_eq!(job.stop_now(Some(5.0)), None);
        job.credit_reading(150.0);
        assert_eq!(job.stop_now(Some(5.0)), Some(JobStop::Credits));
    }
}
