use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

use crate::provider::LlmError;

use super::LlmRouter;
use super::types::{RELIABILITY_WINDOW, TaskHint};

/// Try to extract an LlmError from an anyhow::Error for smart classification.
pub(super) fn classify_anyhow_error(error: &anyhow::Error) -> LlmError {
    if let Some(llm_err) = error.downcast_ref::<LlmError>() {
        return match llm_err {
            LlmError::RateLimit { retry_after } => LlmError::RateLimit {
                retry_after: *retry_after,
            },
            LlmError::Overloaded => LlmError::Overloaded,
            LlmError::Auth { status, message } => LlmError::Auth {
                status: *status,
                message: message.clone(),
            },
            LlmError::ServerError { status, message } => LlmError::ServerError {
                status: *status,
                message: message.clone(),
            },
            LlmError::ContextOverflow { message } => LlmError::ContextOverflow {
                message: message.clone(),
            },
            LlmError::Network(msg) => LlmError::Network(msg.clone()),
            LlmError::Other(e) => LlmError::Other(anyhow::anyhow!("{}", e)),
        };
    }

    let msg = error.to_string().to_lowercase();
    if msg.contains("rate limit") || msg.contains("429") || msg.contains("too many requests") {
        LlmError::RateLimit { retry_after: None }
    } else if msg.contains("overloaded") || msg.contains("529") {
        LlmError::Overloaded
    } else if msg.contains("unauthorized") || msg.contains("403") || msg.contains("401") {
        LlmError::Auth {
            status: 401,
            message: msg,
        }
    } else if msg.contains("timeout") || msg.contains("connection") {
        LlmError::Network(msg)
    } else {
        LlmError::Other(anyhow::anyhow!("{}", error))
    }
}

/// Short label for telemetry's `error_class` column. Mirrors the `LlmError`
/// variants but flattened to a stable string the leaderboard queries can group.
pub(super) fn classify_error_for_telemetry(error: &anyhow::Error) -> String {
    match classify_anyhow_error(error) {
        LlmError::RateLimit { .. } => "rate_limit".into(),
        LlmError::Overloaded => "overloaded".into(),
        LlmError::Auth { .. } => "auth".into(),
        LlmError::ServerError { .. } => "server_error".into(),
        LlmError::ContextOverflow { .. } => "context_overflow".into(),
        LlmError::Network(_) => "network".into(),
        LlmError::Other(_) => "other".into(),
    }
}

/// Tracks the health and performance of a single provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealth {
    /// When this provider can be retried after a rate limit.
    #[serde(skip)]
    pub cooldown_until: Option<Instant>,
    /// Number of consecutive failures (resets on success).
    pub consecutive_failures: u32,
    /// Total requests sent to this provider.
    pub total_requests: u64,
    /// Total failed requests.
    pub total_failures: u64,
    /// Total tokens consumed (input + output).
    pub total_tokens: u64,
    /// Sliding window of recent outcomes per task type (true = success).
    /// Only the last RELIABILITY_WINDOW outcomes affect reliability scoring.
    pub task_window: [Vec<bool>; 6], // indexed by TaskHint
    /// Backoff exponent for exponential backoff (resets on success).
    pub backoff_exponent: u32,
    /// Exponential moving average of response latency (milliseconds).
    /// Used to penalize slow providers in scoring.
    pub avg_latency_ms: f64,
    /// Tokens consumed in the current daily period.
    pub daily_tokens_used: u64,
    /// When the daily token counter resets.
    #[serde(skip)]
    pub daily_reset_at: Option<Instant>,
    /// Daily token soft cap (0 = unlimited). When daily_tokens_used > 80% of this,
    /// the provider's score is penalized. At 100%, it's treated like rate-limited.
    pub daily_token_limit: u64,
}

impl Default for ProviderHealth {
    fn default() -> Self {
        Self {
            cooldown_until: None,
            consecutive_failures: 0,
            total_requests: 0,
            total_failures: 0,
            total_tokens: 0,
            task_window: [
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ],
            backoff_exponent: 0,
            avg_latency_ms: 0.0,
            daily_tokens_used: 0,
            daily_reset_at: None,
            daily_token_limit: 0,
        }
    }
}

impl ProviderHealth {
    /// Whether this provider is currently in a cooldown period.
    pub fn is_cooling_down(&self) -> bool {
        self.cooldown_until
            .map(|until| Instant::now() < until)
            .unwrap_or(false)
    }

    /// Get current daily tokens, accounting for reset.
    fn check_daily_tokens_current(&self) -> u64 {
        if let Some(reset_at) = self.daily_reset_at
            && Instant::now() >= reset_at
        {
            return 0; // Period expired, tokens would reset
        }
        self.daily_tokens_used
    }

    /// Ratio of daily budget consumed (0.0 to 1.0+). Returns 0.0 if no limit.
    pub fn daily_budget_ratio(&self) -> f64 {
        if self.daily_token_limit == 0 {
            return 0.0;
        }
        self.check_daily_tokens_current() as f64 / self.daily_token_limit as f64
    }

    /// Record a successful request with observed latency.
    pub fn record_success(&mut self, task: TaskHint, tokens: u64, latency: Duration) {
        self.total_requests += 1;
        self.total_tokens += tokens;
        self.consecutive_failures = 0;
        self.backoff_exponent = 0;
        self.cooldown_until = None;

        // Sliding window: push success, trim to RELIABILITY_WINDOW
        let idx = task as usize;
        self.task_window[idx].push(true);
        if self.task_window[idx].len() > RELIABILITY_WINDOW {
            self.task_window[idx].remove(0);
        }

        // Daily token tracking
        self.maybe_reset_daily();
        self.daily_tokens_used += tokens;

        // Exponential moving average of latency (alpha = 0.3)
        let ms = latency.as_millis() as f64;
        if self.avg_latency_ms == 0.0 {
            self.avg_latency_ms = ms;
        } else {
            self.avg_latency_ms = self.avg_latency_ms * 0.7 + ms * 0.3;
        }
    }

    /// Record a failed request and apply appropriate cooldown.
    pub fn record_failure(&mut self, task: TaskHint, error: &LlmError) {
        self.total_requests += 1;
        self.total_failures += 1;
        self.consecutive_failures += 1;

        // Sliding window: push failure
        let idx = task as usize;
        self.task_window[idx].push(false);
        if self.task_window[idx].len() > RELIABILITY_WINDOW {
            self.task_window[idx].remove(0);
        }

        match error {
            LlmError::RateLimit { retry_after } => {
                let cooldown = retry_after.unwrap_or_else(|| {
                    // Exponential backoff: 2^exp seconds, max 60s
                    let secs = 2u64.saturating_pow(self.backoff_exponent).min(60);
                    Duration::from_secs(secs)
                });
                self.cooldown_until = Some(Instant::now() + cooldown);
                self.backoff_exponent = (self.backoff_exponent + 1).min(6);
            }
            LlmError::Overloaded => {
                // Short cooldown for overloaded
                self.cooldown_until = Some(Instant::now() + Duration::from_secs(5));
            }
            LlmError::Auth { .. } => {
                // Auth errors won't fix themselves. Long cooldown.
                self.cooldown_until = Some(Instant::now() + Duration::from_secs(3600));
            }
            LlmError::ServerError { .. } | LlmError::Network(_) => {
                // Transient: short backoff
                let secs = 2u64.saturating_pow(self.backoff_exponent.min(4));
                self.cooldown_until = Some(Instant::now() + Duration::from_secs(secs));
                self.backoff_exponent = (self.backoff_exponent + 1).min(5);
            }
            _ => {
                // Unknown errors: don't cooldown, might be request-specific
            }
        }
    }

    /// Success rate for a specific task type (0.0 to 1.0).
    /// Uses a sliding window of the last RELIABILITY_WINDOW outcomes.
    /// Returns 0.5 (neutral) if no history for this task type.
    /// With fewer than MIN_SAMPLES samples, blends observed rate with neutral
    /// to prevent a single success from spiking reliability to 100%.
    pub fn task_success_rate(&self, task: TaskHint) -> f64 {
        const MIN_SAMPLES: usize = 5;
        let idx = task as usize;
        let window = &self.task_window[idx];
        if window.is_empty() {
            0.5 // No data, neutral score
        } else {
            let successes = window.iter().filter(|&&ok| ok).count();
            let observed = successes as f64 / window.len() as f64;
            if window.len() < MIN_SAMPLES {
                // Blend: (observed * samples + 0.5 * remaining) / MIN_SAMPLES
                let weight = window.len() as f64 / MIN_SAMPLES as f64;
                observed * weight + 0.5 * (1.0 - weight)
            } else {
                observed
            }
        }
    }

    /// Reset daily token counter if the 24h period has elapsed.
    fn maybe_reset_daily(&mut self) {
        if let Some(reset_at) = self.daily_reset_at
            && Instant::now() >= reset_at
        {
            self.daily_tokens_used = 0;
            self.daily_reset_at = Some(Instant::now() + Duration::from_secs(86400));
        } else if self.daily_reset_at.is_none() {
            // First token usage: start the 24h clock
            self.daily_reset_at = Some(Instant::now() + Duration::from_secs(86400));
        }
    }
}

impl LlmRouter {
    /// Get a snapshot of provider health stats (for diagnostics).
    /// Returns: (name, total_requests, total_failures, total_tokens, not cooling down, avg_latency_ms)
    #[cfg(test)]
    pub fn health_snapshot(&self) -> Vec<(String, u64, u64, u64, bool, f64)> {
        let health = self.health.lock().unwrap();
        health
            .iter()
            .map(|(name, h)| {
                (
                    name.clone(),
                    h.total_requests,
                    h.total_failures,
                    h.total_tokens,
                    !h.is_cooling_down(),
                    h.avg_latency_ms,
                )
            })
            .collect()
    }
}
