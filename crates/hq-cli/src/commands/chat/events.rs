use crate::render::{self, Theme};
use hq_agent::session::AgentSession;
use hq_core::middleware::unstructured_llm_error_looks_transient;
use hq_core::types::SessionEvent;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Print streamed session events for the interactive chat, and track the
/// model and token totals that `/cost` and `/tokens` report.
pub(super) fn wire_event_rendering(
    session: &mut AgentSession,
    theme: Theme,
    first_token: Arc<AtomicBool>,
    last_model: Arc<Mutex<String>>,
    total_tokens: Arc<Mutex<(u64, u64)>>,
) {
    session.on_event(move |event| {
        match event {
            SessionEvent::TextDelta(text) => {
                if first_token.load(Ordering::Relaxed) {
                    // Stop spinner: set flag false first, then clear line terminal-width wide
                    first_token.store(false, Ordering::Relaxed);
                    let w = crossterm::terminal::size().map(|(w, _)| w as usize).unwrap_or(80);
                    print!("\r{}\r", " ".repeat(w));
                    print!(
                        "\x1b[{sec}m hq>\x1b[0m ",
                        sec = render::fg(&theme.secondary),
                    );
                }
                print!("{}", text);
                let _ = io::stdout().flush();
            }
            SessionEvent::ToolStart { tool_name, .. } => {
                println!(
                    "\n{}",
                    render::render_tool_start(&tool_name, &theme)
                );
                let _ = io::stdout().flush();
            }
            SessionEvent::ToolEnd {
                tool_name,
                tool_call_id: _,
                result,
            } => {
                let formatted = render::format_tool_result(&tool_name, &result);
                println!(
                    "\x1b[{dim}m{formatted}\x1b[0m",
                    dim = render::fg(&theme.text_dim),
                );
            }
            SessionEvent::ToolProgress {
                tool_name,
                tool_call_id: _,
                message,
            } => {
                println!(
                    "{}",
                    render::render_tool_progress(&tool_name, &message, &theme)
                );
            }
            SessionEvent::Compaction {
                old_messages,
                new_messages,
            } => {
                println!(
                    "{}",
                    render::render_compaction(old_messages, new_messages, &theme)
                );
            }
            SessionEvent::Error(msg) => {
                // Suppress stream chunk errors that are transient (retried internally)
                if !unstructured_llm_error_looks_transient(&msg) {
                    println!(
                        "{}",
                        render::render_error(&msg, &theme)
                    );
                }
            }
            SessionEvent::CostUpdate { model, input_tokens, output_tokens, .. } => {
                if let Ok(mut m) = last_model.lock() {
                    *m = model;
                }
                if let Ok(mut tok) = total_tokens.lock() {
                    tok.0 += input_tokens;
                    tok.1 += output_tokens;
                }
            }
            SessionEvent::RetryAttempt { attempt, max_retries, delay_ms, error } => {
                let short: String = error.chars().take(60).collect();
                println!(
                    "\x1b[{warn}m  \u{21bb} Retry {attempt}/{max_retries} in {delay_ms}ms: {short}\u{2026}\x1b[0m",
                    warn = render::fg(&theme.warning),
                );
            }
            SessionEvent::ContextOverflowRecovery => {
                println!(
                    "\x1b[{muted}m  \u{21bb} Context overflow \u{2014} compacting\u{2026}\x1b[0m",
                    muted = render::fg(&theme.text_muted),
                );
            }
            SessionEvent::PlanModeEntered { plan_file } => {
                println!(
                    "\x1b[{info}m  \u{1f4cb} Plan mode: {plan_file}\x1b[0m",
                    info = render::fg(&theme.info),
                );
            }
            SessionEvent::PlanModeExited { plan_file } => {
                println!(
                    "\x1b[{success}m  \u{2713} Plan ready: {plan_file}\x1b[0m",
                    success = render::fg(&theme.success),
                );
            }
            SessionEvent::SubagentCompleted { agent_type, harness, result_preview } => {
                println!(
                    "\x1b[{muted}m  \u{21b3} {agent_type} ({harness}): {result_preview}\x1b[0m",
                    muted = render::fg(&theme.text_muted),
                );
            }
            SessionEvent::BudgetExhausted { spent, budget } => {
                println!(
                    "\x1b[{warn}m  \u{26a0} Budget exhausted: ${spent:.4} of ${budget:.4}\x1b[0m",
                    warn = render::fg(&theme.warning),
                );
            }
            _ => {}
        }
    });
}
