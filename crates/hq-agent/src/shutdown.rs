//! Graceful shutdown infrastructure — signal handling, cleanup registry, failsafe timer.
//!
//! Inspired by Claude Code's layered shutdown:
//! 1. Signal received (SIGINT/SIGTERM)
//! 2. Run all registered cleanup handlers in reverse order
//! 3. Failsafe timer force-exits if cleanup hangs

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tracing::{info, warn};

/// A registered cleanup function.
type CleanupFn = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

/// Registry of cleanup functions that run on graceful shutdown.
///
/// Handlers run in reverse registration order (LIFO), matching the pattern
/// of "last resource opened is first cleaned up."
pub struct CleanupRegistry {
    handlers: Vec<CleanupFn>,
}

impl CleanupRegistry {
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }

    /// Register a cleanup handler that will run on shutdown.
    pub fn register(
        &mut self,
        handler: impl FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + 'static,
    ) {
        self.handlers.push(Box::new(handler));
    }

    /// Run all handlers in reverse order with a failsafe timeout.
    pub async fn run_all(self, failsafe_timeout: Duration) {
        let count = self.handlers.len();
        if count == 0 {
            return;
        }

        info!(count, "running shutdown cleanup handlers");

        let cleanup = async {
            // Run in reverse (LIFO) order
            for (i, handler) in self.handlers.into_iter().rev().enumerate() {
                let fut = handler();
                match tokio::time::timeout(Duration::from_secs(2), fut).await {
                    Ok(()) => {
                        info!(handler = count - i, "cleanup handler completed");
                    }
                    Err(_) => {
                        warn!(
                            handler = count - i,
                            "cleanup handler timed out (2s), skipping"
                        );
                    }
                }
            }
        };

        match tokio::time::timeout(failsafe_timeout, cleanup).await {
            Ok(()) => info!("all cleanup handlers completed"),
            Err(_) => warn!(
                timeout_secs = failsafe_timeout.as_secs(),
                "cleanup failsafe timer expired, forcing exit"
            ),
        }
    }
}

impl Default for CleanupRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared shutdown signal that can be checked from multiple tasks.
#[derive(Clone)]
pub struct ShutdownSignal {
    notify: Arc<Notify>,
    triggered: Arc<std::sync::atomic::AtomicBool>,
}

impl ShutdownSignal {
    pub fn new() -> Self {
        Self {
            notify: Arc::new(Notify::new()),
            triggered: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Trigger the shutdown signal.
    pub fn trigger(&self) {
        self.triggered
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// Check if shutdown has been triggered (non-blocking).
    pub fn is_triggered(&self) -> bool {
        self.triggered.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Wait until shutdown is triggered.
    pub async fn wait(&self) {
        if self.is_triggered() {
            return;
        }
        self.notify.notified().await;
    }
}

impl Default for ShutdownSignal {
    fn default() -> Self {
        Self::new()
    }
}

/// Install OS signal handlers (SIGINT, SIGTERM) that trigger the shutdown signal.
///
/// Returns a future that resolves when a signal is received.
/// Call this once at the top level of your application.
pub async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut sigint = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");

        tokio::select! {
            _ = sigint.recv() => {
                info!("received SIGINT, initiating graceful shutdown");
            }
            _ = sigterm.recv() => {
                info!("received SIGTERM, initiating graceful shutdown");
            }
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
        info!("received Ctrl+C, initiating graceful shutdown");
    }
}
