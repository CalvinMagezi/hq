//! Live session-progress state shared between the session event subscriber and the
//! relay ticker tasks.
//!
//! `ActivityFeed` accumulates a rolling, human-readable log of everything the agent
//! does (tool calls + arg/result previews, reasoning, sub-agent completions, retries,
//! compaction, running cost) so the relay can render full live visibility into the
//! native hq harness. The session `on_event` callback (sync `Fn`) writes here; the
//! ticker reads and edits the Telegram message on a throttled cadence. Discord's
//! ticker reads only the turn and last tool.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Maximum activity lines kept in the rolling buffer (older lines drop off the top).
const MAX_LINES: usize = 14;
/// Characters of trailing reasoning shown live as the "thinking" line.
const REASONING_TAIL: usize = 200;

/// Rolling live-activity state for one running session.
#[derive(Default)]
pub struct ActivityFeed {
    pub turn: u32,
    /// Completed activity lines, oldest first.
    lines: VecDeque<String>,
    /// Tool currently executing (name + arg preview), shown as the active line.
    active_tool: Option<String>,
    /// Name of the most recently started tool, kept after it ends. Discord's
    /// one-line ticker shows it.
    pub last_tool: String,
    /// Accumulated reasoning for the current burst; rendered as a trailing preview.
    reasoning: String,
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub model: String,
    /// Set whenever state changes so the ticker only edits when there's something new.
    pub dirty: bool,
}

pub type SharedActivityFeed = Arc<Mutex<ActivityFeed>>;

pub fn new_activity_feed() -> SharedActivityFeed {
    Arc::new(Mutex::new(ActivityFeed::default()))
}

fn first_line(s: &str, max: usize) -> String {
    let line = s
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let truncated: String = line.chars().take(max).collect();
    if line.chars().count() > max {
        format!("{truncated}\u{2026}")
    } else {
        truncated
    }
}

impl ActivityFeed {
    fn push(&mut self, line: String) {
        if self.lines.len() >= MAX_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
        self.dirty = true;
    }

    pub fn on_turn(&mut self, turn: u32) {
        self.turn = turn;
        // A turn boundary ends the current reasoning burst.
        self.reasoning.clear();
        self.dirty = true;
    }

    pub fn on_tool_start(&mut self, name: &str) {
        self.last_tool.clear();
        self.last_tool.push_str(name);
        self.active_tool = Some(format!("\u{1F527} {name}"));
        self.reasoning.clear();
        self.dirty = true;
    }

    pub fn on_tool_progress(&mut self, name: &str, message: &str) {
        self.active_tool = Some(format!("\u{1F527} {name} {}", first_line(message, 90)));
        self.dirty = true;
    }

    pub fn on_tool_end(&mut self, name: &str, result: &str) {
        let preview = first_line(result, 70);
        let line = if preview.is_empty() {
            format!("\u{2713} {name}")
        } else {
            format!("\u{2713} {name} \u{2192} {preview}")
        };
        self.push(line);
        self.active_tool = None;
    }

    pub fn on_reasoning(&mut self, delta: &str) {
        self.reasoning.push_str(delta);
        self.dirty = true;
    }

    pub fn on_subagent(&mut self, agent_type: &str, harness: &str, preview: &str) {
        self.push(format!(
            "\u{1F91D} {agent_type} via {harness} \u{2192} {}",
            first_line(preview, 60)
        ));
    }

    pub fn on_retry(&mut self, attempt: u32, max: u32, error: &str) {
        self.push(format!(
            "\u{1F501} retry {attempt}/{max}: {}",
            first_line(error, 50)
        ));
    }

    pub fn on_compaction(&mut self, old: usize, new: usize) {
        self.push(format!(
            "\u{1F5DC} compacted context {old}\u{2192}{new} msgs"
        ));
    }

    pub fn on_error(&mut self, msg: &str) {
        self.push(format!("\u{26A0} {}", first_line(msg, 70)));
    }

    pub fn on_cost(&mut self, total_usd: f64, input: u64, output: u64, model: &str) {
        self.cost_usd = total_usd;
        self.input_tokens = input;
        self.output_tokens = output;
        if !model.is_empty() {
            self.model = model.to_string();
        }
        self.dirty = true;
    }

    /// Render the full feed into a Telegram-safe plain-text block (<= ~4096 chars).
    pub fn render(&self, elapsed_secs: u64) -> String {
        let mins = elapsed_secs / 60;
        let secs = elapsed_secs % 60;
        let time = if mins > 0 {
            format!("{mins}m {secs}s")
        } else {
            format!("{secs}s")
        };

        let mut header = String::from("\u{1F9E0} ");
        if !self.model.is_empty() {
            header.push_str(&self.model);
            header.push_str(" \u{B7} ");
        }
        if self.turn > 0 {
            header.push_str(&format!("turn {} \u{B7} ", self.turn));
        }
        header.push_str(&time);
        if self.cost_usd > 0.0 {
            header.push_str(&format!(" \u{B7} ${:.4}", self.cost_usd));
        }
        if self.input_tokens + self.output_tokens > 0 {
            header.push_str(&format!(
                " \u{B7} {}\u{2192}{} tok",
                fmt_k(self.input_tokens),
                fmt_k(self.output_tokens)
            ));
        }

        let mut out = String::with_capacity(512);
        out.push_str(&header);
        out.push('\n');
        for line in &self.lines {
            out.push_str(line);
            out.push('\n');
        }
        if let Some(active) = &self.active_tool {
            out.push_str(active);
            out.push('\n');
        }
        let reasoning = self.reasoning.trim();
        if !reasoning.is_empty() {
            let tail: String = reasoning
                .chars()
                .rev()
                .take(REASONING_TAIL)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            out.push_str("\u{1F4AD} ");
            out.push_str(tail.trim());
        }
        // Hard cap well under Telegram's 4096 limit.
        if out.chars().count() > 3500 {
            out = out.chars().take(3500).collect();
        }
        out
    }
}

fn fmt_k(n: u64) -> String {
    if n >= 1000 {
        format!("{}k", n / 1000)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_renders_tools_and_header() {
        let mut f = ActivityFeed::default();
        f.on_cost(0.0042, 12345, 3210, "deepseek-v4-pro");
        f.on_turn(2);
        f.on_tool_start("vault_search");
        f.on_tool_progress("vault_search", "\u{201C}Value Bus\u{201D}");
        f.on_tool_end("vault_search", "4 results\nNotebooks/...");
        assert_eq!(f.last_tool, "vault_search", "Discord's ticker keeps the last tool after it ends");
        f.on_subagent("researcher", "hq", "Value Bus fixes effort-without-payoff");
        let r = f.render(83);
        assert!(r.contains("deepseek-v4-pro"));
        assert!(r.contains("turn 2"));
        assert!(r.contains("1m 23s"));
        assert!(r.contains("$0.0042"));
        assert!(r.contains("\u{2713} vault_search \u{2192} 4 results"));
        assert!(r.contains("researcher via hq"));
    }

    #[test]
    fn rolling_buffer_caps_lines() {
        let mut f = ActivityFeed::default();
        for i in 0..40 {
            f.on_tool_end("read_file", &format!("file{i}"));
        }
        assert!(f.lines.len() <= MAX_LINES);
        let r = f.render(5);
        assert!(r.contains("file39"));
        assert!(!r.contains("file0 "));
    }

    #[test]
    fn reasoning_shows_trailing_tail() {
        let mut f = ActivityFeed::default();
        f.on_reasoning("I will first search the vault for the relevant note");
        let r = f.render(3);
        assert!(r.contains("\u{1F4AD}"));
        assert!(r.contains("relevant note"));
    }
}
