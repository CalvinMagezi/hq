//! Notification gate — decides drop/digest/urgent for relay messages by reading
//! learned `notif.<source>.<kind>` preferences from `_system/USER.md`.
//!
//! Called by both relay pollers (Telegram + Discord) right before they forward a
//! mailbox message, so every producer (the mail-ingest webhooks, native
//! producers, agents) is gated at the one point
//! where the user is actually told. Fail-safe: any
//! error yields `Digest` (keep), never a silent `Drop`.

use crate::user_model::{UserModel, UserTrait, load_user_model, save_user_model};
use chrono::Utc;
use hq_core::types::{MailboxMessage, MailboxMessageType};
use serde_json::json;
use std::path::Path;

/// Confidence floor below which a learned preference is ignored (default keep).
const GATE_CONFIDENCE: f64 = 0.6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventHeader {
    pub source: String,
    pub kind: String,
}

/// Parse the leading `[… key=value …]` header producers embed as the first line.
/// The mail-ingest webhooks and native producers use explicit `source=`/`kind=`;
/// other `[<tag> id=… …]` headers map to source=`<tag>`, kind=`<id>`.
pub fn parse_event_header(content: &str) -> Option<EventHeader> {
    let line = content.lines().next()?.trim();
    if !line.starts_with('[') {
        return None;
    }
    let end = line.find(']')?;
    let inner = &line[1..end];

    let mut tag = "";
    let mut kv: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for (i, tok) in inner.split_whitespace().enumerate() {
        if i == 0 && !tok.contains('=') {
            tag = tok;
            continue;
        }
        if let Some((k, v)) = tok.split_once('=') {
            kv.insert(k, v);
        }
    }

    if let (Some(source), Some(kind)) = (kv.get("source"), kv.get("kind")) {
        return Some(EventHeader {
            source: source.to_string(),
            kind: kind.to_string(),
        });
    }
    if !tag.is_empty() {
        let kind = kv.get("id").copied().unwrap_or("event").to_string();
        return Some(EventHeader {
            source: tag.to_string(),
            kind,
        });
    }
    None
}

/// The `notif.<source>.<kind>` trait key a message maps to, if it carries a
/// parseable header. Used to attach reaction buttons that teach this exact trait.
pub fn trait_key_for(content: &str) -> Option<String> {
    parse_event_header(content).map(|h| format!("notif.{}.{}", h.source, h.kind))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDecision {
    Drop,
    Digest,
    Urgent,
}

use hq_core::mailbox::META_INTERRUPT;

/// Decide how a relay-bound message should be delivered. Pure read of USER.md.
/// Fail-safe: any parse/load problem yields `Digest` (keep), never `Drop`.
pub fn gate_decision(vault_path: &Path, msg: &MailboxMessage) -> GateDecision {
    // A Nudge skips the learned gate only when its producer explicitly asked
    // for an interrupt; a plain Nudge is gated like any other notification.
    let interrupt_nudge = msg.msg_type == MailboxMessageType::Nudge
        && msg.meta.get(META_INTERRUPT).map(String::as_str) == Some("true");
    if interrupt_nudge {
        log_decision(vault_path, msg, GateDecision::Urgent, "interrupt-tagged");
        return GateDecision::Urgent;
    }

    let Some(h) = parse_event_header(&msg.content) else {
        log_decision(vault_path, msg, GateDecision::Digest, "no-header");
        return GateDecision::Digest;
    };

    let key = format!("notif.{}.{}", h.source, h.kind);
    let model = match load_user_model(vault_path) {
        Ok(m) => m,
        Err(_) => {
            log_decision(vault_path, msg, GateDecision::Digest, "model-load-failed");
            return GateDecision::Digest;
        }
    };

    let decision = match model.traits.get(&key) {
        Some(t) if t.confidence >= GATE_CONFIDENCE => match t.value.as_str() {
            Some("noise") => GateDecision::Drop,
            Some("urgent") => GateDecision::Urgent,
            _ => GateDecision::Digest,
        },
        _ => GateDecision::Digest,
    };
    log_decision(vault_path, msg, decision, &key);
    decision
}

fn log_decision(vault_path: &Path, msg: &MailboxMessage, decision: GateDecision, reason: &str) {
    let (source, kind) = parse_event_header(&msg.content)
        .map(|h| (h.source, h.kind))
        .unwrap_or_else(|| ("-".into(), "-".into()));
    let label = match decision {
        GateDecision::Drop => "drop",
        GateDecision::Digest => "digest",
        GateDecision::Urgent => "urgent",
    };
    let entry = json!({
        "ts": Utc::now().to_rfc3339(),
        "from": msg.from,
        "source": source,
        "kind": kind,
        "decision": label,
        "reason": reason,
        "msg_id": msg.id,
    });
    let dir = vault_path.join("_system");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("notif-gate-log.jsonl");
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
    {
        let _ = writeln!(f, "{}", entry);
    }
    rotate_log_if_large(&path);
}

/// Cap the gate log: when it exceeds ~5 MB, rewrite keeping only the most recent
/// lines, so the log cannot grow without bound.
fn rotate_log_if_large(path: &Path) {
    const MAX_BYTES: u64 = 5 * 1024 * 1024;
    const KEEP_LINES: usize = 5000;
    let over = std::fs::metadata(path)
        .map(|m| m.len() > MAX_BYTES)
        .unwrap_or(false);
    if !over {
        return;
    }
    if let Ok(content) = std::fs::read_to_string(path) {
        let lines: Vec<&str> = content.lines().collect();
        let start = lines.len().saturating_sub(KEEP_LINES);
        let kept = lines[start..].join("\n");
        let _ = std::fs::write(path, format!("{kept}\n"));
    }
}

/// (key, value, confidence) defaults. Confidence is deliberately low (0.3) so
/// explicit user replies (conf 0.9) override them via `merge()`.
const DEFAULT_NOTIF_TRAITS: &[(&str, &str, f64)] = &[
    ("notif.github.ci.failed", "urgent", 0.3),
    ("notif.github.dependabot", "noise", 0.3),
    ("notif.github.pr.merged", "signal", 0.3),
];

/// Seed defaults the first time around; never clobbers a higher-confidence user
/// value because it goes through `UserModel::merge`.
pub fn seed_default_notif_traits(vault_path: &Path) -> anyhow::Result<()> {
    let mut model = load_user_model(vault_path).unwrap_or_else(|_| UserModel::default());
    let now = Utc::now();
    let mut changed = false;
    for (key, value, conf) in DEFAULT_NOTIF_TRAITS {
        changed |= model.merge(
            key,
            UserTrait {
                value: json!(value),
                confidence: *conf,
                last_updated: Some(now),
            },
        );
    }
    if changed {
        save_user_model(vault_path, &model)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user_model::UserModel;
    use tempfile::TempDir;

    fn msg(content: &str, t: MailboxMessageType) -> MailboxMessage {
        hq_core::mailbox::new_message("ingest", "relay", t, None, content, None)
    }

    #[test]
    fn parses_event_header() {
        let c = "[hq-event flow=ci-status source=github kind=ci.failed]\n\nmain is red";
        let h = parse_event_header(c).unwrap();
        assert_eq!(h.source, "github");
        assert_eq!(h.kind, "ci.failed");
    }

    #[test]
    fn parses_tag_and_id_header_as_source_and_kind() {
        let c = "[digest id=daily role=explorer]\n\nreview notes";
        let h = parse_event_header(c).unwrap();
        assert_eq!(h.source, "digest");
        assert_eq!(h.kind, "daily");
    }

    #[test]
    fn no_header_returns_none() {
        assert!(parse_event_header("just a plain message").is_none());
    }

    #[test]
    fn trait_key_for_builds_namespaced_key() {
        let c = "[hq-event flow=ci source=github kind=ci.failed]\n\nred";
        assert_eq!(trait_key_for(c).as_deref(), Some("notif.github.ci.failed"));
        assert!(trait_key_for("plain message").is_none());
    }

    #[test]
    fn interrupt_tagged_nudge_is_urgent() {
        let tmp = TempDir::new().unwrap();
        let mut m = msg("anything", MailboxMessageType::Nudge);
        m.meta.insert(META_INTERRUPT.to_string(), "true".to_string());
        assert_eq!(gate_decision(tmp.path(), &m), GateDecision::Urgent);
    }

    #[test]
    fn interrupt_tagged_nudge_logs_its_own_reason() {
        // The pre-FR-001b bug logged "type-exempt"; a distinct reason keeps
        // the fix verifiable from the log alone.
        let tmp = TempDir::new().unwrap();
        let mut nudge = msg("nudge", MailboxMessageType::Nudge);
        nudge.meta.insert(META_INTERRUPT.to_string(), "true".to_string());
        gate_decision(tmp.path(), &nudge);

        let log = std::fs::read_to_string(tmp.path().join("_system/notif-gate-log.jsonl")).unwrap();
        assert!(log.contains("\"reason\":\"interrupt-tagged\""), "{log}");
        assert!(!log.contains("type-exempt"), "{log}");
    }

    #[test]
    fn untagged_nudge_falls_through_to_the_learned_gate() {
        // FR-001b: a Nudge is no longer unconditionally urgent — a
        // machine-generated status ping that happens to use the Nudge type
        // for delivery speed must not read as urgent just because of that.
        let tmp = TempDir::new().unwrap();
        let d = gate_decision(tmp.path(), &msg("anything", MailboxMessageType::Nudge));
        assert_eq!(d, GateDecision::Digest);
    }

    #[test]
    fn unknown_event_defaults_to_digest() {
        let tmp = TempDir::new().unwrap();
        let d = gate_decision(
            tmp.path(),
            &msg(
                "[hq-event flow=x source=foo kind=bar]\n\nhi",
                MailboxMessageType::Direct,
            ),
        );
        assert_eq!(d, GateDecision::Digest);
    }

    #[test]
    fn confident_noise_is_dropped() {
        let tmp = TempDir::new().unwrap();
        let mut m = UserModel::default();
        m.merge(
            "notif.github.dependabot",
            UserTrait {
                value: json!("noise"),
                confidence: 0.9,
                last_updated: Some(Utc::now()),
            },
        );
        save_user_model(tmp.path(), &m).unwrap();
        let d = gate_decision(
            tmp.path(),
            &msg(
                "[hq-event flow=gh source=github kind=dependabot]\n\nbump",
                MailboxMessageType::Direct,
            ),
        );
        assert_eq!(d, GateDecision::Drop);
    }

    #[test]
    fn confident_urgent_is_urgent() {
        let tmp = TempDir::new().unwrap();
        let mut m = UserModel::default();
        m.merge(
            "notif.github.ci.failed",
            UserTrait {
                value: json!("urgent"),
                confidence: 0.9,
                last_updated: Some(Utc::now()),
            },
        );
        save_user_model(tmp.path(), &m).unwrap();
        let d = gate_decision(
            tmp.path(),
            &msg(
                "[hq-event flow=ci source=github kind=ci.failed]\n\nred",
                MailboxMessageType::Direct,
            ),
        );
        assert_eq!(d, GateDecision::Urgent);
    }

    #[test]
    fn low_confidence_noise_still_kept() {
        let tmp = TempDir::new().unwrap();
        let mut m = UserModel::default();
        m.merge(
            "notif.github.dependabot",
            UserTrait {
                value: json!("noise"),
                confidence: 0.4,
                last_updated: Some(Utc::now()),
            },
        );
        save_user_model(tmp.path(), &m).unwrap();
        let d = gate_decision(
            tmp.path(),
            &msg(
                "[hq-event flow=gh source=github kind=dependabot]\n\nbump",
                MailboxMessageType::Direct,
            ),
        );
        assert_eq!(d, GateDecision::Digest);
    }

    #[test]
    fn decisions_are_logged() {
        let tmp = TempDir::new().unwrap();
        gate_decision(
            tmp.path(),
            &msg(
                "[hq-event flow=x source=foo kind=bar]\n\nhi",
                MailboxMessageType::Direct,
            ),
        );
        let log = std::fs::read_to_string(tmp.path().join("_system/notif-gate-log.jsonl")).unwrap();
        assert!(log.contains("\"decision\":\"digest\""));
        assert!(log.contains("\"source\":\"foo\""));
    }

    #[test]
    fn seeds_defaults_without_clobbering_user_values() {
        let tmp = TempDir::new().unwrap();
        let mut m = UserModel::default();
        m.merge(
            "notif.github.dependabot",
            UserTrait {
                value: json!("signal"),
                confidence: 0.95,
                last_updated: Some(Utc::now()),
            },
        );
        save_user_model(tmp.path(), &m).unwrap();

        seed_default_notif_traits(tmp.path()).unwrap();
        let after = load_user_model(tmp.path()).unwrap();
        assert_eq!(
            after
                .traits
                .get("notif.github.dependabot")
                .unwrap()
                .value
                .as_str(),
            Some("signal")
        );
        assert_eq!(
            after
                .traits
                .get("notif.github.ci.failed")
                .unwrap()
                .value
                .as_str(),
            Some("urgent")
        );
    }
}
