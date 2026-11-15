//! Bounds on `harness_session_spawn` calls that join no team: how many such sessions may run,
//! how many may start per hour, and whether a used-up hard budget stops a start. Team starts
//! have their own limits (`a2a::check_tree_limits`).

use std::sync::Arc;

use anyhow::{Result, bail};
use hq_core::config::{Budget, BudgetAction, BudgetScope, BudgetsConfig, SpawnLimitsConfig};
use hq_core::types::{ValueItem, ValueKind};
use hq_db::Database;
use hq_db::harness_sessions_registry as registry;
use hq_db::usage_ledger::{period_bounds, scope_spend};
use tracing::warn;

use crate::a2a::RATE_WINDOW_MINUTES;

/// Starts that passed the check and have not yet written their session row. The lock is held only
/// to check and add one, never across a folder approval or a launch, so a slow start cannot
/// stall another one.
static PENDING: std::sync::Mutex<usize> = std::sync::Mutex::new(0);

/// A slot claimed by a start that passed the limits. Dropped once the start's row is written
/// (or the start failed), after which the row itself is what counts.
pub struct Reservation(());

impl Drop for Reservation {
    fn drop(&mut self) {
        let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        *pending = pending.saturating_sub(1);
    }
}

/// Checks the limits counting starts still in flight, and claims a slot atomically.
pub fn reserve(
    db: &Arc<Database>,
    limits: &SpawnLimitsConfig,
    budgets: &BudgetsConfig,
    now: i64,
) -> Result<Reservation> {
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    check_with_pending(db, limits, budgets, now, *pending)?;
    *pending += 1;
    Ok(Reservation(()))
}

const SECS_PER_HOUR: i64 = 3600;
const HARNESS_ORIGIN: &str = "harness";
const ALERT_SOURCE: &str = "spawn-limits";

/// `check_with_pending` with no starts in flight, for tests.
#[cfg(test)]
fn check(
    db: &Arc<Database>,
    limits: &SpawnLimitsConfig,
    budgets: &BudgetsConfig,
    now: i64,
) -> Result<()> {
    check_with_pending(db, limits, budgets, now, 0)
}

fn check_with_pending(
    db: &Arc<Database>,
    limits: &SpawnLimitsConfig,
    budgets: &BudgetsConfig,
    now: i64,
    pending: usize,
) -> Result<()> {
    let refusal = match refusal(db, limits, budgets, now, pending) {
        Ok(None) => return Ok(()),
        Ok(Some(r)) => r,
        Err(e) => return Err(e),
    };
    notify(db, &refusal);
    bail!("{}", refusal.message);
}

struct Refusal {
    message: String,
    title: String,
    dedup_key: String,
}

fn refusal(
    db: &Arc<Database>,
    limits: &SpawnLimitsConfig,
    budgets: &BudgetsConfig,
    now: i64,
    pending: usize,
) -> Result<Option<Refusal>> {
    let pending = pending as i64;
    let hour = now / SECS_PER_HOUR;
    let (live, recent) = db.with_conn(|c| {
        Ok((
            registry::count_active_unparented(c)? + pending,
            registry::count_unparented_started_since(c, RATE_WINDOW_MINUTES)? + pending,
        ))
    })?;
    let max_live = limits.live_sessions();
    if live >= max_live as i64 {
        return Ok(Some(Refusal {
            message: format!(
                "{live} sessions outside a team are already active (at most {max_live}, agent_host.spawn_limits.max_live_sessions); stop one or wait for one to finish"
            ),
            title: "Session start refused: too many running".into(),
            dedup_key: format!("spawn-limit:live:{hour}"),
        }));
    }
    let per_hour = limits.spawns_per_hour();
    if recent >= per_hour {
        return Ok(Some(Refusal {
            message: format!(
                "rate limit: at most {per_hour} sessions outside a team may start per {RATE_WINDOW_MINUTES} minutes (agent_host.spawn_limits.max_spawns_per_hour)"
            ),
            title: "Session start refused: hourly limit reached".into(),
            dedup_key: format!("spawn-limit:rate:{hour}"),
        }));
    }
    exhausted_budget(db, limits, budgets, now)
}

/// A hard budget that covers harness spend and is used up. Soft budgets never stop a start.
fn exhausted_budget(
    db: &Arc<Database>,
    limits: &SpawnLimitsConfig,
    budgets: &BudgetsConfig,
    now: i64,
) -> Result<Option<Refusal>> {
    for budget in budgets.enforceable().into_iter().filter(|b| {
        b.action == BudgetAction::Block && covers_harness(&b.scope, limits.count_harness_in_global)
    }) {
        let (start, end) = period_bounds(budget.period, now);
        let spent = spent_with_harness(db, budget, start, end)?;
        if spent < budget.limit_usd {
            continue;
        }
        let resets = chrono::DateTime::from_timestamp(end, 0).map_or_else(
            || end.to_string(),
            |t| t.format("%Y-%m-%d %H:%M UTC").to_string(),
        );
        return Ok(Some(Refusal {
            message: format!(
                "budget '{}' is used up (${spent:.2} of ${:.2}); no session starts until it resets at {resets}",
                budget.name, budget.limit_usd
            ),
            title: format!("Session start refused: budget '{}' is used up", budget.name),
            dedup_key: format!("spawn-limit:budget:{}:{start}", budget.name),
        }));
    }
    Ok(None)
}

fn covers_harness(scope: &BudgetScope, count_global: bool) -> bool {
    match scope {
        BudgetScope::Origin(o) => o == HARNESS_ORIGIN,
        BudgetScope::Global => count_global,
        BudgetScope::Provider(_) | BudgetScope::Model(_) => false,
    }
}

/// The ledger leaves harness calls out of a global sum, so a global budget adds them back.
fn spent_with_harness(db: &Arc<Database>, budget: &Budget, start: i64, end: i64) -> Result<f64> {
    db.with_conn(|c| {
        let mut spent = scope_spend(c, &budget.scope, start, end)?;
        if budget.scope == BudgetScope::Global {
            let harness = BudgetScope::Origin(HARNESS_ORIGIN.into());
            spent += scope_spend(c, &harness, start, end)?;
        }
        Ok(spent)
    })
}

fn notify(db: &Database, r: &Refusal) {
    let item = ValueItem::new(
        ALERT_SOURCE,
        ValueKind::Fyi,
        r.title.clone(),
        r.message.clone(),
    )
    .with_dedup_key(r.dedup_key.clone());
    if let Err(e) = hq_db::value_items::emit(db, &item) {
        warn!(error = %e, "spawn refusal could not be recorded in the inbox");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::harness_sessions_registry::{NewSession, Placement};
    use hq_db::task_outcomes::{TaskOutcome, insert};

    const NOW_HOUR_SECS: i64 = 1_800_000_000;

    fn add_session(db: &Database, id: &str) {
        db.with_conn(|c| {
            registry::insert(
                c,
                &NewSession {
                    id,
                    harness: "claude-code",
                    label: "",
                    cwd: "/tmp",
                    mission_id: None,
                    placement: Placement {
                        host: "local",
                        agent_name: id,
                        workspace_id: "w",
                        pane_id: "p",
                    },
                    purpose: None,
                },
            )
        })
        .unwrap();
    }

    fn limits(live: u32, per_hour: u32) -> SpawnLimitsConfig {
        SpawnLimitsConfig {
            max_live_sessions: live,
            max_spawns_per_hour: per_hour,
            count_harness_in_global: false,
        }
    }

    fn spend(db: &Database, origin: &str, usd: f64, at: i64) {
        let mut o = TaskOutcome::now("s", 0, "m", "p", "chat");
        o.cost_usd = usd;
        o.origin = origin.into();
        o.recorded_at = at;
        db.with_conn(|c| insert(c, &o)).unwrap();
    }

    fn budgets(yaml: &str) -> BudgetsConfig {
        serde_yaml::from_str(yaml).unwrap()
    }

    fn inbox_count(db: &Database) -> usize {
        hq_db::value_items::list_by_state(db, hq_core::types::ValueState::Pending)
            .unwrap()
            .iter()
            .filter(|i| i.source_task == ALERT_SOURCE)
            .count()
    }

    #[test]
    fn default_config_lets_a_normal_start_through() {
        let db = Arc::new(Database::open_memory().unwrap());
        add_session(&db, "a");
        let d = SpawnLimitsConfig::default();
        check(&db, &d, &BudgetsConfig::default(), NOW_HOUR_SECS).unwrap();
    }

    #[test]
    fn live_cap_refuses_at_the_cap_and_passes_below_it() {
        let db = Arc::new(Database::open_memory().unwrap());
        add_session(&db, "a");
        check(
            &db,
            &limits(2, 100),
            &BudgetsConfig::default(),
            NOW_HOUR_SECS,
        )
        .unwrap();
        add_session(&db, "b");
        let err = check(
            &db,
            &limits(2, 100),
            &BudgetsConfig::default(),
            NOW_HOUR_SECS,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("max_live_sessions"), "{err}");
    }

    #[test]
    fn idle_and_done_sessions_do_not_count_toward_the_live_cap_but_working_ones_do() {
        let db = Arc::new(Database::open_memory().unwrap());
        for (id, status) in [("a", "idle"), ("b", "done"), ("c", "idle")] {
            add_session(&db, id);
            db.with_conn(|c| registry::set_seen(c, id, status)).unwrap();
        }
        check(
            &db,
            &limits(1, 100),
            &BudgetsConfig::default(),
            NOW_HOUR_SECS,
        )
        .unwrap();
        for (id, status) in [("w", "working"), ("x", "blocked")] {
            add_session(&db, id);
            db.with_conn(|c| registry::set_seen(c, id, status)).unwrap();
        }
        assert!(
            check(
                &db,
                &limits(2, 100),
                &BudgetsConfig::default(),
                NOW_HOUR_SECS
            )
            .is_err()
        );
        assert!(
            check(
                &db,
                &limits(3, 100),
                &BudgetsConfig::default(),
                NOW_HOUR_SECS
            )
            .is_ok()
        );
    }

    #[test]
    fn hourly_count_includes_ended_sessions_and_resets_with_the_window() {
        let db = Arc::new(Database::open_memory().unwrap());
        add_session(&db, "a");
        add_session(&db, "b");
        db.with_conn(|c| registry::set_status_exited_if_running(c, "a").map(|_| ()))
            .unwrap();
        db.with_conn(|c| registry::set_status_exited_if_running(c, "b").map(|_| ()))
            .unwrap();
        let err = check(
            &db,
            &limits(10, 2),
            &BudgetsConfig::default(),
            NOW_HOUR_SECS,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("max_spawns_per_hour"), "{err}");
        // Two hours later the same rows are outside the window.
        db.with_conn(|c| {
            c.execute(
                "UPDATE harness_sessions SET created_at = datetime('now', '-2 hours')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        check(
            &db,
            &limits(10, 2),
            &BudgetsConfig::default(),
            NOW_HOUR_SECS,
        )
        .unwrap();
    }

    #[test]
    fn team_members_do_not_count_against_the_outside_cap() {
        let db = Arc::new(Database::open_memory().unwrap());
        add_session(&db, "root");
        add_session(&db, "kid");
        db.with_conn(|c| {
            c.execute(
                "UPDATE harness_sessions SET parent_session_id = 'root' WHERE id = 'kid'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        check(
            &db,
            &limits(2, 100),
            &BudgetsConfig::default(),
            NOW_HOUR_SECS,
        )
        .unwrap();
    }

    #[test]
    fn two_simultaneous_starts_at_the_boundary_let_exactly_one_pass() {
        let db = Arc::new(Database::open_memory().unwrap());
        let gate = Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let (db, gate) = (db.clone(), gate.clone());
                std::thread::spawn(move || {
                    gate.wait();
                    reserve(
                        &db,
                        &limits(1, 100),
                        &BudgetsConfig::default(),
                        NOW_HOUR_SECS,
                    )
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    }

    #[test]
    fn a_dropped_reservation_frees_its_slot() {
        let db = Arc::new(Database::open_memory().unwrap());
        let (l, b) = (limits(1, 100), BudgetsConfig::default());
        let held = reserve(&db, &l, &b, NOW_HOUR_SECS).unwrap();
        assert!(reserve(&db, &l, &b, NOW_HOUR_SECS).is_err());
        drop(held);
        reserve(&db, &l, &b, NOW_HOUR_SECS).unwrap();
    }

    const HARNESS_HARD: &str = "budgets:\n  - {name: coders, scope: 'origin:harness', period: day, limit_usd: 5, action: block}\n";
    const HARNESS_SOFT: &str = "budgets:\n  - {name: coders, scope: 'origin:harness', period: day, limit_usd: 5, action: notify}\n";

    #[test]
    fn a_used_up_hard_harness_budget_refuses_and_names_it() {
        let db = Arc::new(Database::open_memory().unwrap());
        spend(&db, "harness", 6.0, NOW_HOUR_SECS);
        let err = check(&db, &limits(10, 100), &budgets(HARNESS_HARD), NOW_HOUR_SECS)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("'coders'") && err.contains("resets at"),
            "{err}"
        );
    }

    #[test]
    fn soft_or_unexhausted_budgets_do_not_refuse() {
        let db = Arc::new(Database::open_memory().unwrap());
        spend(&db, "harness", 6.0, NOW_HOUR_SECS);
        check(&db, &limits(10, 100), &budgets(HARNESS_SOFT), NOW_HOUR_SECS).unwrap();
        let fresh = Arc::new(Database::open_memory().unwrap());
        spend(&fresh, "harness", 1.0, NOW_HOUR_SECS);
        check(
            &fresh,
            &limits(10, 100),
            &budgets(HARNESS_HARD),
            NOW_HOUR_SECS,
        )
        .unwrap();
    }

    #[test]
    fn a_global_budget_counts_harness_spend_only_when_asked() {
        let db = Arc::new(Database::open_memory().unwrap());
        spend(&db, "harness", 6.0, NOW_HOUR_SECS);
        let global = budgets(
            "budgets:\n  - {name: all, scope: global, period: day, limit_usd: 5, action: block}\n",
        );
        check(&db, &limits(10, 100), &global, NOW_HOUR_SECS).unwrap();
        let mut on = limits(10, 100);
        on.count_harness_in_global = true;
        assert!(check(&db, &on, &global, NOW_HOUR_SECS).is_err());
    }

    #[test]
    fn repeated_refusals_file_one_inbox_item_per_limit_and_period() {
        let db = Arc::new(Database::open_memory().unwrap());
        add_session(&db, "a");
        for _ in 0..3 {
            assert!(
                check(
                    &db,
                    &limits(1, 100),
                    &BudgetsConfig::default(),
                    NOW_HOUR_SECS
                )
                .is_err()
            );
        }
        assert_eq!(inbox_count(&db), 1);
        // A different limit is its own item.
        db.with_conn(|c| registry::set_status_exited_if_running(c, "a").map(|_| ()))
            .unwrap();
        assert!(check(&db, &limits(5, 1), &BudgetsConfig::default(), NOW_HOUR_SECS).is_err());
        assert_eq!(inbox_count(&db), 2);
    }
}
