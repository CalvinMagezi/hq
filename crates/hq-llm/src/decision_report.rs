//! Summarizes the decision log so the effect of each gate can be checked without
//! reading raw JSONL: how often it ran, what it did, what it cost, and what it hid.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::path::Path;

const LOG_DIR: &str = "_system/decision-shadow";
const MAX_EXAMPLES: usize = 8;
const EXAMPLE_CHARS: usize = 110;
/// Upper edges of the score buckets, ascending; the last bucket is open ended.
const BUCKET_EDGES: [f64; 4] = [0.15, 0.30, 0.50, 0.80];
const HIDING_ACTIONS: [&str; 2] = ["skipped", "suppressed"];

#[derive(Default)]
struct SiteStats {
    calls: u32,
    errors: u32,
    actions: BTreeMap<String, u32>,
    buckets: [u32; BUCKET_EDGES.len() + 1],
    latencies_ms: Vec<u64>,
    cost: f64,
    models: BTreeSet<String>,
    hidden: Vec<String>,
}

/// Plain-text report over the last `days` days, optionally for a single site.
pub fn report(vault_path: &Path, days: u32, site_filter: Option<&str>) -> String {
    let dir = vault_path.join(LOG_DIR);
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(i64::from(days)))
        .format("%Y-%m-%d")
        .to_string();
    let mut sites: BTreeMap<String, SiteStats> = BTreeMap::new();
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            p.extension().is_some_and(|e| e == "jsonl") && stem >= cutoff.as_str()
        })
        .collect();
    files.sort();
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        for entry in text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
            let site = entry["site"].as_str().unwrap_or("?").to_string();
            if site_filter.is_none_or(|f| f == site) {
                add(sites.entry(site).or_default(), &entry);
            }
        }
    }
    render(&sites, days)
}

fn add(stats: &mut SiteStats, entry: &Value) {
    stats.calls += 1;
    if entry.get("error").is_some() {
        stats.errors += 1;
    }
    if let Some(action) = entry["action"].as_str() {
        *stats.actions.entry(action.to_string()).or_default() += 1;
        if HIDING_ACTIONS.contains(&action) {
            stats.hidden.push(example(entry));
        }
    }
    if let Some(score) = score_of(entry) {
        let bucket = BUCKET_EDGES.iter().position(|edge| score < *edge);
        stats.buckets[bucket.unwrap_or(BUCKET_EDGES.len())] += 1;
    }
    if let Some(ms) = entry["latency_ms"].as_u64() {
        stats.latencies_ms.push(ms);
    }
    stats.cost += entry["cost"].as_f64().unwrap_or(0.0);
    if let Some(model) = entry["model"].as_str() {
        stats.models.insert(model.to_string());
    }
}

/// Enforced records carry `score`; shadow records carry the raw answers.
fn score_of(entry: &Value) -> Option<f64> {
    if let Some(score) = entry["score"].as_f64() {
        return Some(score);
    }
    entry["answers"]
        .as_object()?
        .values()
        .find_map(|a| (a["type"] == "noul").then(|| a["noul"].as_f64()).flatten())
}

fn example(entry: &Value) -> String {
    let ts = entry["ts"].as_str().unwrap_or("?").get(5..16).unwrap_or("?");
    let score = score_of(entry).map_or("?".to_string(), |s| format!("{s:.2}"));
    let what = match (entry["incumbent"]["from"].as_str(), entry["incumbent"]["subject"].as_str()) {
        (Some(from), Some(subject)) => format!("{from} | {subject}"),
        _ => entry["excerpt"].as_str().unwrap_or("").replace('\n', " "),
    };
    let category = entry["category"].as_str().unwrap_or("");
    let line = format!("{ts}  {score}  {category:<18} {what}");
    line.chars().take(EXAMPLE_CHARS + 30).collect()
}

fn percentile(sorted: &[u64], pct: usize) -> u64 {
    sorted
        .get((sorted.len().saturating_sub(1) * pct) / 100)
        .copied()
        .unwrap_or(0)
}

fn render(sites: &BTreeMap<String, SiteStats>, days: u32) -> String {
    let mut out = format!("Decision log, last {days} day(s)\n");
    if sites.is_empty() {
        out.push_str("\nNo records. Either decisions are disabled or nothing has reached a gate yet.\n");
        return out;
    }
    for (site, s) in sites {
        let mut latencies = s.latencies_ms.clone();
        latencies.sort_unstable();
        let _ = writeln!(out, "\n{site}");
        let _ = writeln!(out, "  calls {}  errors {}  cost ${:.5}", s.calls, s.errors, s.cost);
        if !s.actions.is_empty() {
            let actions: Vec<String> = s.actions.iter().map(|(a, n)| format!("{a} {n}")).collect();
            let _ = writeln!(out, "  outcomes: {}", actions.join(", "));
        }
        if !latencies.is_empty() {
            let _ = writeln!(
                out,
                "  latency ms: p50 {}  p95 {}",
                percentile(&latencies, 50),
                percentile(&latencies, 95)
            );
        }
        let labels = ["<0.15", "0.15-0.30", "0.30-0.50", "0.50-0.80", ">=0.80"];
        let scores: Vec<String> = labels
            .iter()
            .zip(s.buckets)
            .map(|(label, n)| format!("{label} {n}"))
            .collect();
        let _ = writeln!(out, "  scores: {}", scores.join("  "));
        if !s.models.is_empty() {
            let models: Vec<&str> = s.models.iter().map(String::as_str).collect();
            let _ = writeln!(out, "  model builds: {}", models.join(", "));
        }
        if !s.hidden.is_empty() {
            let _ = writeln!(out, "  most recent hidden ({} total):", s.hidden.len());
            for line in s.hidden.iter().rev().take(MAX_EXAMPLES) {
                let _ = writeln!(out, "    {line}");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_log(dir: &Path, lines: &[Value]) {
        let log = dir.join(LOG_DIR);
        std::fs::create_dir_all(&log).unwrap();
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let body: Vec<String> = lines.iter().map(Value::to_string).collect();
        std::fs::write(log.join(format!("{today}.jsonl")), body.join("\n")).unwrap();
    }

    #[test]
    fn summarizes_enforced_and_shadow_sites_and_lists_what_was_hidden() {
        let dir = tempfile::tempdir().unwrap();
        write_log(
            dir.path(),
            &[
                serde_json::json!({"ts":"2026-09-18T10:00:00Z","site":"email_fyi","action":"suppressed",
                    "score":0.09,"category":"promo_newsletter","model":"m1","cost":0.00002,"latency_ms":400,
                    "incumbent":{"from":"Glovo","subject":"Top Sellers"}}),
                serde_json::json!({"ts":"2026-09-18T11:00:00Z","site":"email_fyi","action":"forwarded",
                    "score":0.84,"category":"work_or_admin","model":"m1","cost":0.00002,"latency_ms":900,
                    "incumbent":{"from":"","subject":"Tax Certificate"}}),
                serde_json::json!({"ts":"2026-09-18T12:00:00Z","site":"cognition_insight",
                    "answers":{"is_durable":{"type":"noul","noul":0.05}},"model":"m1","cost":0.00002,
                    "latency_ms":500,"incumbent":{"decision":"keep"}}),
                serde_json::json!({"ts":"2026-09-18T12:01:00Z","site":"cognition_insight","error":"timeout"}),
            ],
        );
        let text = report(dir.path(), 7, None);
        assert!(text.contains("email_fyi"));
        assert!(text.contains("outcomes: forwarded 1, suppressed 1"));
        assert!(text.contains("Glovo | Top Sellers"));
        assert!(!text.contains("Tax Certificate"), "forwarded mail is not listed as hidden");
        assert!(text.contains("cognition_insight"));
        assert!(text.contains("errors 1"));
        assert!(text.contains("<0.15 1"));

        let only_email = report(dir.path(), 7, Some("email_fyi"));
        assert!(!only_email.contains("cognition_insight"));
    }

    #[test]
    fn an_empty_log_says_so() {
        let dir = tempfile::tempdir().unwrap();
        assert!(report(dir.path(), 7, None).contains("No records"));
    }
}
