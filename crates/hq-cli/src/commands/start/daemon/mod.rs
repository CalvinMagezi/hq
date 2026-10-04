//! Daemon scheduler — interval-based task execution with 5-second tick.
//!
//! Phase 1 hardening (inspired by Claude Code's Kairos daemon):
//! - Instance lock: prevents double-fire from concurrent daemons
//! - Graceful shutdown: clean exit via ShutdownSignal with status update
//! - Task timeouts: hung tasks get killed, tick continues

pub mod helpers;
pub mod tasks_fast;
pub mod tasks_periodic;
pub mod tasks_slow;

use anyhow::{Context, Result};
use hq_agent::shutdown::ShutdownSignal;
use hq_core::config::HqConfig;
use hq_daemon::instance_lock::DaemonLock;
use hq_daemon::task_state::TaskState;
use hq_db::Database;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

use helpers::ensure_dir;

/// Tick interval for the main daemon loop.
const TICK_INTERVAL: Duration = Duration::from_secs(5);

// ─── DaemonTask definition ───────────────────────────────────

pub struct DaemonTask {
    pub name: &'static str,
    pub interval: Duration,
    pub timeout: Duration,
    pub last_run: tokio::time::Instant,
    pub run_count: u64,
    pub error_count: u64,
    pub timeout_count: u64,
    /// Runs that did no work because nothing is configured for this task to
    /// act on (e.g. no email backend declared) — distinct from `error_count`,
    /// which is for a run that actually failed. See `TaskUnconfigured`.
    pub skipped_count: u64,
    pub last_error: Option<String>,
}

impl DaemonTask {
    pub fn new(name: &'static str, interval: Duration, timeout: Duration) -> Self {
        Self {
            name,
            interval,
            timeout,
            // Set last_run to past so all tasks fire on first tick
            last_run: tokio::time::Instant::now() - interval - Duration::from_secs(1),
            run_count: 0,
            error_count: 0,
            timeout_count: 0,
            skipped_count: 0,
            last_error: None,
        }
    }

    /// Convenience: create with default timeout based on interval tier.
    pub fn with_default_timeout(name: &'static str, interval: Duration) -> Self {
        let timeout = hq_core::middleware::default_timeout_for_interval(&interval);
        Self::new(name, interval, timeout)
    }

    /// Create with a startup delay so the first run happens after `startup_delay` rather than immediately.
    /// Useful for Ollama-heavy tasks that would otherwise all load models simultaneously on daemon start.
    pub fn delayed(name: &'static str, interval: Duration, startup_delay: Duration) -> Self {
        let timeout = hq_core::middleware::default_timeout_for_interval(&interval);
        Self {
            name,
            interval,
            timeout,
            last_run: tokio::time::Instant::now() - interval + startup_delay,
            run_count: 0,
            error_count: 0,
            timeout_count: 0,
            skipped_count: 0,
            last_error: None,
        }
    }
}

/// Sentinel error a task returns to report "did no work because nothing is
/// configured for it to act on" — distinct from a real failure. The daemon
/// tick loop downcasts for this instead of counting it as `error_count`.
#[derive(Debug)]
pub struct TaskUnconfigured(pub String);

impl std::fmt::Display for TaskUnconfigured {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for TaskUnconfigured {}

// ─── Main daemon loop ────────────────────────────────────────

/// How often to check for config changes (seconds).
const CONFIG_RELOAD_INTERVAL_SECS: u64 = 60;

/// How often task state is persisted (seconds), rather than on every tick.
const STATE_SAVE_INTERVAL_SECS: u64 = 60;

pub async fn run_daemon(
    config: &HqConfig,
    vault: Arc<hq_vault::VaultClient>,
    db: Arc<Database>,
    shutdown: ShutdownSignal,
) -> Result<()> {
    let vault_path = vault.vault_path().to_path_buf();
    let started_at = chrono::Utc::now().to_rfc3339();
    let mut config = config.clone();
    let mut reloader = ConfigReloader::new();

    // Prevents double-fire from concurrent daemons.
    let lock = DaemonLock::try_acquire(&vault_path)
        .context("failed to check daemon lock")?
        .context("another daemon is already running — only one instance allowed")?;

    run_startup_hooks(&config, &vault_path, &db);
    let mut tasks = default_tasks();
    let (mut task_state, missed) = load_task_state(&vault_path, &tasks);
    let mut save_ticker = TickCounter::every_secs(STATE_SAVE_INTERVAL_SECS);

    write_cron_schedule(&vault_path, &tasks);
    write_daemon_status(&vault_path, &tasks, &started_at);
    info!(
        tasks = tasks.len(),
        missed,
        "daemon: scheduler running ({} tasks, {}s tick, {} missed)",
        tasks.len(),
        TICK_INTERVAL.as_secs(),
        missed,
    );

    let mut interval = tokio::time::interval(TICK_INTERVAL);
    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Err(e) = lock.heartbeat() {
                    warn!("daemon: lock heartbeat failed: {e}");
                }
                run_due_tasks(&mut tasks, &mut task_state, &vault_path, &db, &config).await;
                if save_ticker.fire()
                    && let Err(e) = task_state.save(&vault_path)
                {
                    warn!("daemon: failed to save task state: {e}");
                }
                reloader.poll(&mut config);
                write_daemon_status(&vault_path, &tasks, &started_at);
                write_daemon_metrics(&vault_path, &tasks, &started_at);
            }
            _ = shutdown.wait() => {
                info!("daemon: shutdown signal received, cleaning up");
                break;
            }
        }
    }

    task_state.mark_shutdown();
    let _ = task_state.save(&vault_path);
    write_daemon_status_stopped(&vault_path, &tasks, &started_at);
    lock.release();
    info!("daemon: stopped cleanly");
    Ok(())
}

/// Counts ticks and fires once per `n` of them.
struct TickCounter {
    ticks: u64,
    every: u64,
}

impl TickCounter {
    fn every_secs(secs: u64) -> Self {
        Self {
            ticks: 0,
            every: (secs / TICK_INTERVAL.as_secs()).max(1),
        }
    }

    fn fire(&mut self) -> bool {
        self.ticks += 1;
        if self.ticks < self.every {
            return false;
        }
        self.ticks = 0;
        true
    }
}

/// Hot-reload: reloads the config when its file's mtime changes.
struct ConfigReloader {
    path: std::path::PathBuf,
    mtime: Option<std::time::SystemTime>,
    ticker: TickCounter,
}

impl ConfigReloader {
    fn new() -> Self {
        let path = HqConfig::config_read_path();
        let mtime = file_mtime(&path);
        Self {
            path,
            mtime,
            ticker: TickCounter::every_secs(CONFIG_RELOAD_INTERVAL_SECS),
        }
    }

    fn poll(&mut self, config: &mut HqConfig) {
        if !self.ticker.fire() {
            return;
        }
        let current = file_mtime(&self.path);
        if current == self.mtime {
            return;
        }
        match HqConfig::load() {
            Err(e) => warn!("daemon: config reload failed: {e}"),
            Ok(new_config) => {
                *config = new_config;
                self.mtime = current;
                info!("daemon: config reloaded from disk");
            }
        }
    }
}

fn file_mtime(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

/// One-shot startup work and the loops that run beside the scheduler.
fn run_startup_hooks(config: &HqConfig, vault_path: &Path, db: &Database) {
    ensure_dir(&vault_path.join("_system"));

    // Merge-safe: never clobbers higher-confidence user or learner values.
    if let Err(e) = hq_daemon::notif_gate::seed_default_notif_traits(vault_path) {
        warn!(error = %e, "notif-gate: seeding defaults failed");
    }

    // Read once: changing `decisions:` needs a daemon restart, unlike the hot-reloaded config.
    hq_llm::decision::init(&config.decisions, vault_path);

    // Before any relay listener can deliver new work: fresh stranded rows get an
    // FYI via the relay mailbox, rows past the age ceiling are interrupted silently.
    match hq_daemon::turn_reconciler::reconcile_stranded_turns(
        vault_path,
        db,
        config.relay.background_turn_max_days,
    ) {
        Ok(stats) if stats.stranded > 0 => info!(
            stranded = stats.stranded,
            notified = stats.notified,
            stale = stats.stale,
            "turn-reconciler: stranded background turns marked interrupted"
        ),
        Ok(_) => {}
        Err(e) => warn!(error = %e, "turn-reconciler: startup reconciliation failed"),
    }

    // Its own loop, so slow scheduler tasks (consolidation, embeddings) can't
    // delay inbound event handling; drains the agent-worker mailbox every 20s.
    tokio::spawn(hq_daemon::agent_worker::run_agent_worker_loop(
        vault_path.to_path_buf(),
    ));

    // Keeps _system/MACHINE.md current so every system prompt can state which
    // CLIs this host actually has, instead of the agent guessing.
    hq_daemon::spawn_machine_profile_loop(vault_path.to_path_buf());
}

/// Every scheduled task; the timeout is derived from the interval tier.
fn default_tasks() -> Vec<DaemonTask> {
    vec![
        // Every 1 minute (fast tier — 30s timeout)
        DaemonTask::with_default_timeout("expire-approvals", Duration::from_secs(60)),
        DaemonTask::with_default_timeout("session-supervisor", Duration::from_secs(60)),
        DaemonTask::with_default_timeout("copilot-usage", Duration::from_secs(60)),
        DaemonTask::with_default_timeout("subagent-supervisor", Duration::from_secs(60)),
        // Every 5 minutes (periodic tier — 2min timeout)
        DaemonTask::with_default_timeout("heartbeat", Duration::from_secs(300)),
        // Value Bus delivery: rank + deliver pending value items every 5 minutes
        DaemonTask::with_default_timeout("value-bus-deliver", Duration::from_secs(300)),
        // Time-gated fallback for gws-backed companies: /hooks/gmail (Pub/Sub
        // push) is primary but requires one-time external setup
        // (docs/runbooks/gmail-pubsub-ingress.md); this tick covers inboxes
        // without it configured. 15 min, not tighter, to stay light on Gmail
        // API quota — see gmail_ingest::poll_and_enqueue.
        DaemonTask::with_default_timeout("email-poll", Duration::from_secs(900)),
        // Every 30 minutes — Ollama-heavy tasks staggered to avoid simultaneous model loading at startup
        DaemonTask::delayed(
            "memory-consolidation",
            Duration::from_secs(1800),
            Duration::from_secs(600),
        ),
        DaemonTask::delayed(
            "embeddings",
            Duration::from_secs(1800),
            Duration::from_secs(300),
        ),
        DaemonTask::delayed(
            "inbox-triage",
            Duration::from_secs(1800),
            Duration::from_secs(720),
        ),
        DaemonTask::with_default_timeout("disk-watchdog", Duration::from_secs(21600)),
        DaemonTask::with_default_timeout("vault-health", Duration::from_secs(21600)),
        DaemonTask::with_default_timeout("thread-log-rotation", Duration::from_secs(21600)),
        DaemonTask::with_default_timeout("memory-forgetting", Duration::from_secs(86400)),
        DaemonTask::with_default_timeout("vault-cleanup", Duration::from_secs(86400)),
        DaemonTask::with_default_timeout("db-vacuum", Duration::from_secs(604800)), // weekly
        DaemonTask::with_default_timeout("vault-cap-enforcer", Duration::from_secs(21600)), // 6 hours
        // Enforce the background-turn age ceiling between restarts (idempotent)
        DaemonTask::with_default_timeout("turn-reconcile", Duration::from_secs(21600)), // every 6h
    ]
}

/// Load persisted task state, log tasks missed during downtime, and mark startup.
/// Returns the state and how many tasks were missed.
fn load_task_state(vault_path: &Path, tasks: &[DaemonTask]) -> (TaskState, usize) {
    let mut task_state = TaskState::load(vault_path);
    let task_intervals: Vec<(&str, Duration)> =
        tasks.iter().map(|t| (t.name, t.interval)).collect();
    let missed = task_state.find_missed_tasks(&task_intervals);
    if !missed.is_empty() {
        info!(
            count = missed.len(),
            "daemon: detected missed tasks during downtime"
        );
        for (name, overdue_secs) in &missed {
            let hours = *overdue_secs as f64 / 3600.0;
            info!(task = %name, overdue_hours = format!("{hours:.1}"), "daemon: missed task — will fire on first tick");
        }
    }
    task_state.mark_startup();
    let _ = task_state.save(vault_path);
    (task_state, missed.len())
}

/// Run every task whose interval has elapsed, one after another.
async fn run_due_tasks(
    tasks: &mut [DaemonTask],
    task_state: &mut TaskState,
    vault_path: &Path,
    db: &Database,
    config: &HqConfig,
) {
    let now = tokio::time::Instant::now();
    for task in tasks.iter_mut() {
        if now.duration_since(task.last_run) < task.interval {
            continue;
        }
        task.last_run = now;
        debug!(task = task.name, "daemon: running task");
        let start = std::time::Instant::now();
        let outcome =
            tokio::time::timeout(task.timeout, dispatch_task(task.name, vault_path, db, config))
                .await;
        record_outcome(task, task_state, outcome, start.elapsed().as_millis() as u64);
    }
}

/// Fold one task run into its counters: success, unconfigured skip, error or timeout.
fn record_outcome(
    task: &mut DaemonTask,
    task_state: &mut TaskState,
    outcome: Result<Result<()>, tokio::time::error::Elapsed>,
    duration_ms: u64,
) {
    match outcome {
        Err(_) => {
            task.timeout_count += 1;
            warn!(
                task = task.name,
                timeout_secs = task.timeout.as_secs(),
                "daemon: task timed out"
            );
            task.last_error = Some(format!("timed out after {}s", task.timeout.as_secs()));
        }
        Ok(Err(e)) => match e.downcast_ref::<TaskUnconfigured>() {
            Some(unconfigured) => {
                task.skipped_count += 1;
                debug!(
                    task = task.name,
                    reason = %unconfigured.0,
                    duration_ms,
                    "daemon: task unconfigured, not an error"
                );
                task.last_error = Some(unconfigured.0.clone());
            }
            None => {
                task.error_count += 1;
                let err_msg = format!("{e:#}");
                warn!(task = task.name, error = %err_msg, duration_ms, "daemon: task error");
                task.last_error = Some(err_msg);
            }
        },
        Ok(Ok(())) => {
            task.run_count += 1;
            task_state.mark_run(task.name);
            debug!(task = task.name, duration_ms, "daemon: task completed");
        }
    }
}

// ─── Task dispatch ───────────────────────────────────────────

async fn dispatch_task(
    task_name: &str,
    vault_path: &Path,
    db: &Database,
    config: &HqConfig,
) -> Result<()> {
    match task_name {
        // Fast cycle (1 min)
        "expire-approvals" => tasks_fast::run_expire_approvals(vault_path).await,
        "value-bus-deliver" => hq_daemon::run_value_bus_delivery(vault_path, db).await,

        // Periodic (5 min — 1 hour)
        "heartbeat" => tasks_periodic::run_heartbeat(vault_path).await,
        "subagent-supervisor" => tasks_periodic::run_subagent_supervisor(db),
        "session-supervisor" => {
            tasks_periodic::session_supervisor::run_session_supervisor(vault_path, db, config).await
        }
        "email-poll" => tasks_periodic::run_email_poll(vault_path, config).await,
        "memory-consolidation" => {
            if config.dream.enabled {
                tasks_periodic::run_memory_consolidation(vault_path, db).await
            } else {
                Ok(())
            }
        }
        "embeddings" => tasks_periodic::run_embeddings(vault_path, db, config).await,
        "inbox-triage" => {
            if config.dream.enabled {
                tasks_periodic::run_inbox_triage(vault_path, config).await
            } else {
                Ok(())
            }
        }
        "disk-watchdog" => tasks_periodic::run_disk_watchdog(vault_path, config).await,
        "copilot-usage" => tasks_periodic::run_copilot_usage(db, config).await,


        // Slow cycle (6h — daily)
        "vault-health" => tasks_slow::run_vault_health(vault_path, db).await,
        "memory-forgetting" => tasks_slow::run_memory_forgetting(vault_path, db).await,
        "thread-log-rotation" => tasks_slow::run_thread_log_rotation(vault_path).await,
        "vault-cleanup" => tasks_slow::run_vault_cleanup(vault_path, db).await,
        "db-vacuum" => tasks_slow::run_db_vacuum(vault_path, db).await,
        "vault-cap-enforcer" => tasks_slow::run_vault_cap_enforcer(vault_path, db).await,
        "turn-reconcile" => tasks_periodic::run_turn_reconcile(vault_path, db, config).await,








        unknown => {
            warn!(task = unknown, "daemon: unknown task");
            Ok(())
        }
    }
}

// ─── Status file writers ─────────────────────────────────────

fn format_interval(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

pub fn write_cron_schedule(vault_path: &Path, tasks: &[DaemonTask]) {
    let sys_dir = vault_path.join("_system");
    ensure_dir(&sys_dir);
    let now = chrono::Utc::now().to_rfc3339();
    let mut md = format!(
        "---\ngenerated_at: {now}\ntask_count: {}\nruntime: rust\n---\n\n\
         # Cron Schedule\n\nGenerated at {now}\n\n\
         | # | Task | Interval | Timeout |\n|---|------|----------|--------|\n",
        tasks.len()
    );
    for (i, t) in tasks.iter().enumerate() {
        md.push_str(&format!(
            "| {} | `{}` | {} | {} |\n",
            i + 1,
            t.name,
            format_interval(t.interval.as_secs()),
            format_interval(t.timeout.as_secs()),
        ));
    }
    let _ = std::fs::write(sys_dir.join("CRON-SCHEDULE.md"), md);
}

pub fn write_daemon_status(vault_path: &Path, tasks: &[DaemonTask], started_at: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    let total_runs: u64 = tasks.iter().map(|t| t.run_count).sum();
    let total_errors: u64 = tasks.iter().map(|t| t.error_count).sum();
    let total_timeouts: u64 = tasks.iter().map(|t| t.timeout_count).sum();
    let total_skipped: u64 = tasks.iter().map(|t| t.skipped_count).sum();
    let mut md = format!(
        "---\nstatus: running\nstarted_at: {started_at}\nlast_tick: {now}\n\
         runtime: rust\ntask_count: {}\ntotal_runs: {total_runs}\n\
         total_errors: {total_errors}\ntotal_timeouts: {total_timeouts}\ntotal_skipped: {total_skipped}\n---\n\n\
         # Daemon Status\n\nStarted: {started_at}  \n\
         Last tick: {now}  \nTasks: {} | Runs: {total_runs} | Errors: {total_errors} | Timeouts: {total_timeouts} | Skipped: {total_skipped}\n\n\
         | Task | Interval | Timeout | Runs | Errors | Timeouts | Skipped | Last Error |\n\
         |------|----------|---------|------|--------|----------|---------|------------|\n",
        tasks.len(),
        tasks.len()
    );
    for t in tasks {
        let err_str = t.last_error.as_deref().unwrap_or("-");
        let err_display = if err_str.len() > 60 {
            &err_str[..60]
        } else {
            err_str
        };
        md.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} | {} | {} |\n",
            t.name,
            format_interval(t.interval.as_secs()),
            format_interval(t.timeout.as_secs()),
            t.run_count,
            t.error_count,
            t.timeout_count,
            t.skipped_count,
            err_display
        ));
    }
    let _ = std::fs::write(vault_path.join("DAEMON-STATUS.md"), md);
}

/// Write structured JSON metrics for programmatic consumption (by MCP health tools).
fn write_daemon_metrics(vault_path: &Path, tasks: &[DaemonTask], started_at: &str) {
    use serde_json::{Map, Value, json};

    let total_runs: u64 = tasks.iter().map(|t| t.run_count).sum();
    let total_errors: u64 = tasks.iter().map(|t| t.error_count).sum();
    let total_timeouts: u64 = tasks.iter().map(|t| t.timeout_count).sum();

    let mut task_map = Map::new();
    for t in tasks {
        task_map.insert(
            t.name.to_string(),
            json!({
                "runs": t.run_count,
                "errors": t.error_count,
                "timeouts": t.timeout_count,
                "skipped": t.skipped_count,
                "interval_secs": t.interval.as_secs(),
                "timeout_secs": t.timeout.as_secs(),
                "last_error": t.last_error,
            }),
        );
    }

    let metrics = json!({
        "started_at": started_at,
        "last_tick": chrono::Utc::now().to_rfc3339(),
        "total_runs": total_runs,
        "total_errors": total_errors,
        "total_timeouts": total_timeouts,
        "task_count": tasks.len(),
        "tasks": Value::Object(task_map),
    });

    let sys_dir = vault_path.join("_system");
    let tmp = sys_dir.join(".daemon-metrics.json.tmp");
    let target = sys_dir.join(".daemon-metrics.json");
    if let Ok(json_str) = serde_json::to_string_pretty(&metrics) {
        let _ = std::fs::write(&tmp, json_str);
        let _ = std::fs::rename(&tmp, &target);
    }
}

fn write_daemon_status_stopped(vault_path: &Path, tasks: &[DaemonTask], started_at: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    let total_runs: u64 = tasks.iter().map(|t| t.run_count).sum();
    let total_errors: u64 = tasks.iter().map(|t| t.error_count).sum();
    let total_timeouts: u64 = tasks.iter().map(|t| t.timeout_count).sum();
    let total_skipped: u64 = tasks.iter().map(|t| t.skipped_count).sum();
    let md = format!(
        "---\nstatus: stopped\nstarted_at: {started_at}\nstopped_at: {now}\n\
         runtime: rust\ntask_count: {}\ntotal_runs: {total_runs}\n\
         total_errors: {total_errors}\ntotal_timeouts: {total_timeouts}\ntotal_skipped: {total_skipped}\n---\n\n\
         # Daemon Status\n\nStarted: {started_at}  \nStopped: {now}  \n\
         Tasks: {} | Runs: {total_runs} | Errors: {total_errors} | Timeouts: {total_timeouts} | Skipped: {total_skipped}\n",
        tasks.len(),
        tasks.len()
    );
    let _ = std::fs::write(vault_path.join("DAEMON-STATUS.md"), md);
}
