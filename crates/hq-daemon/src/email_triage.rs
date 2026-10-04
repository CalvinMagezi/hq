//! Inbox Lieutenant — email-triage handling for the agent-worker.
//!
//! When an `email.totriage` event is reasoned, the LLM returns a structured
//! CLASSIFICATION / SUMMARY / DRAFT block. For reply-worthy mail we stash a
//! pending send (token → draft + original message id) and Nudge the user with
//! the draft + a `send <token>` prompt. The Telegram reply handler (in hq-relay)
//! resolves the token and calls `email_send::send_via_company`, which sends
//! the reply in-process through the company's gws connector.

use hq_core::mailbox;
use crate::notif_gate::parse_event_header;
use anyhow::Result;
use chrono::{DateTime, Utc};
use hq_core::types::{MailboxMessage, MailboxMessageType};
use serde::{Deserialize, Serialize};
use std::path::Path;

const PENDING_FILE: &str = "_system/pending-emails.json";
const TTL_HOURS: i64 = 24;

/// Serializes `take_pending`'s read-modify-write against the pending-emails
/// file. Telegram and Discord relays both run as tasks in the same `hq start
/// all` process and can race a rapid double-click/double-send on the same
/// token; without this, two concurrent reads could both see the token still
/// present and both fire the send. A `std::sync::Mutex` (not tokio's) is
/// correct here since `take_pending` is itself synchronous file I/O.
static TAKE_PENDING_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// True if this mailbox message is an email-triage event.
pub fn is_email_triage(msg: &MailboxMessage) -> bool {
    parse_event_header(&msg.content)
        .map(|h| h.kind == "email.totriage" || h.source == "gmail")
        .unwrap_or(false)
}

#[derive(Debug, PartialEq)]
pub struct Triage {
    pub needs_reply: bool,
    pub summary: String,
    pub draft: Option<String>,
}

/// Parse the LLM's structured triage output. Defensive: a missing DRAFT (or a
/// "fyi" classification) means no reply is drafted.
pub fn parse_triage(llm: &str) -> Triage {
    let section = |name: &str| -> Option<String> {
        let lower = llm.to_lowercase();
        let key = format!("{}:", name.to_lowercase());
        let start = lower.find(&key)? + key.len();
        // value runs to the next ALL-CAPS section header or end.
        let rest = &llm[start..];
        let end = ["CLASSIFICATION:", "SUMMARY:", "DRAFT:"]
            .iter()
            .filter_map(|h| {
                let p = rest.to_uppercase().find(h)?;
                if p > 0 { Some(p) } else { None }
            })
            .min()
            .unwrap_or(rest.len());
        let val = rest[..end].trim().to_string();
        if val.is_empty() { None } else { Some(val) }
    };

    let classification = section("CLASSIFICATION").unwrap_or_default().to_lowercase();
    let summary =
        section("SUMMARY").unwrap_or_else(|| llm.trim().lines().next().unwrap_or("").to_string());
    let draft = section("DRAFT").filter(|d| {
        let l = d.trim().to_lowercase();
        !l.is_empty() && l != "n/a" && l != "none"
    });

    let needs_reply = (classification.contains("needs-reply")
        || classification.contains("needs reply"))
        && draft.is_some()
        || (classification.is_empty() && draft.is_some());

    Triage {
        needs_reply,
        summary,
        draft,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingEmail {
    pub token: String,
    pub message_id: String,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub subject: String,
    pub draft: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub company_id: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PendingStore {
    #[serde(default)]
    items: Vec<PendingEmail>,
}

impl PendingStore {
    fn load(vault_path: &Path, now: DateTime<Utc>) -> Self {
        let mut s: PendingStore = std::fs::read_to_string(vault_path.join(PENDING_FILE))
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default();
        // prune expired
        s.items
            .retain(|p| now.signed_duration_since(p.created_at).num_hours() < TTL_HOURS);
        s
    }
    fn save(&self, vault_path: &Path) -> Result<()> {
        let path = vault_path.join(PENDING_FILE);
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// Store a pending send.
pub fn add_pending(vault_path: &Path, pe: PendingEmail) -> Result<()> {
    let mut store = PendingStore::load(vault_path, Utc::now());
    store.items.retain(|p| p.token != pe.token);
    store.items.push(pe);
    store.save(vault_path)
}

/// Take (remove + return) a pending send by token. None if missing/expired.
/// Consuming and race-safe within one process: two concurrent calls for the
/// same token can never both return `Some` (see `TAKE_PENDING_LOCK`).
pub fn take_pending(vault_path: &Path, token: &str) -> Option<PendingEmail> {
    let _guard = TAKE_PENDING_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut store = PendingStore::load(vault_path, Utc::now());
    let idx = store.items.iter().position(|p| p.token == token)?;
    let pe = store.items.remove(idx);
    let _ = store.save(vault_path);
    Some(pe)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EmailAction {
    Send,
    Skip,
}

/// Resolve a Send/Skip action for `token`, consuming its pending entry via
/// `take_pending`. Send resolves the company's gws email backend
/// via `email_send::send_via_company` and sends in-process — no workflow engine
/// involved. Shared by the Telegram and Discord relay bots so both call the same
/// resolution logic instead of duplicating it.
///
/// Returns `None` if no pending email matches `token` (already resolved, or expired) —
/// callers decide how to surface that: Telegram falls through to its next command
/// parser (unchanged pre-extraction behavior), Discord's button handler replies with
/// an explicit "already handled" message since there's no next parser to fall through to.
pub async fn resolve_email_action(
    vault_path: &Path,
    token: &str,
    action: EmailAction,
    revision: Option<String>,
) -> Option<String> {
    let pe = take_pending(vault_path, token)?;
    let reply = match action {
        EmailAction::Skip => format!("Skipped — \"{}\" won't be sent.", pe.subject),
        EmailAction::Send => {
            let body = revision.unwrap_or(pe.draft);
            // Falls back to the configured default company for legacy pending emails.
            let company_id = if pe.company_id.is_empty() {
                hq_core::config::HqConfig::load()
                    .ok()
                    .map(|c| c.default_company)
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "default".to_string())
            } else {
                pe.company_id.clone()
            };
            let subject = format!("Re: {}", pe.subject);
            match crate::email_send::send_via_company(&company_id, &pe.from, &subject, &body)
                .await
            {
                Ok(()) => "✅ Sent.".to_string(),
                Err(e) => format!("⚠️ Send failed: {e}"),
            }
        }
    };
    Some(reply)
}

/// Parse a Telegram reply like "send a1b2c3", "skip a1b2c3", or
/// "<revised text>\nsend a1b2c3". Returns (action, token, optional revision).
pub fn parse_email_command(text: &str) -> Option<(EmailAction, String, Option<String>)> {
    // The last match wins so a revision can precede "send <token>".
    let (action, token, before) = crate::parse_token_command(
        text,
        &[("send", EmailAction::Send), ("skip", EmailAction::Skip)],
    )?;
    let revision = (action == EmailAction::Send && !before.is_empty()).then_some(before);
    Some((action, token, revision))
}

/// Build the structured triage prompt the agent worker sends to the LLM router.
/// The reasoning lives here: the mail-ingest layer only forwards the raw email fields.
/// The model must answer in the exact CLASSIFICATION / SUMMARY / DRAFT shape
/// `parse_triage` expects, or the event falls back to a quiet FYI.
///
/// `identity` is `(contact_name, role, company_name)`. Pass `None` to use the
/// generic operator fallback. Callers are responsible for loading config and
/// resolving the correct company before calling this function.
pub fn build_prompt(msg: &MailboxMessage, identity: Option<(&str, &str, &str)>) -> String {
    let (contact_name, role, co_name) =
        identity.unwrap_or(("the operator", "operator", "the company"));

    let from = msg
        .meta
        .get("from")
        .map(String::as_str)
        .unwrap_or("(unknown sender)");
    let subject = msg
        .meta
        .get("subject")
        .map(String::as_str)
        .unwrap_or("(no subject)");
    let body = msg
        .meta
        .get("snippet")
        .or_else(|| msg.meta.get("body"))
        .map(String::as_str)
        .unwrap_or("(no preview available)");

    format!(
        "You are HQ's inbox triage, acting for {contact_name} ({role}, {co_name}).\n\n\
         An email arrived:\n\
         From: {from}\n\
         Subject: {subject}\n\
         Body: {body}\n\n\
         Decide whether it needs a reply from {contact_name}, then answer in EXACTLY this \
         format and nothing else:\n\
         CLASSIFICATION: needs-reply OR fyi\n\
         SUMMARY: <one short line>\n\
         DRAFT:\n\
         <if needs-reply, a complete ready-to-send reply in {contact_name}'s voice — direct, \
         warm, concise, no placeholders or square brackets; if fyi, write exactly: n/a>"
    )
}

/// Handle a reasoned email-triage event: stash a pending send + Nudge the user
/// for reply-worthy mail, or emit a quiet FYI line. Returns Ok(()) always
/// (failures are logged, not fatal).
pub async fn handle(
    vault_path: &Path,
    msg: &MailboxMessage,
    llm_output: &str,
    decisions: Option<&std::sync::Arc<hq_llm::decision::Decisions>>,
) -> Result<()> {
    let t = parse_triage(llm_output);
    let from = msg.meta.get("from").cloned().unwrap_or_default();
    let subject = msg
        .meta
        .get("subject")
        .cloned()
        .unwrap_or_else(|| "(no subject)".into());
    let company_id = msg.meta.get("company_id").cloned().unwrap_or_default();

    if t.needs_reply
        && let (Some(draft), Some(message_id)) =
            (t.draft.clone(), msg.meta.get("messageId").cloned())
        {
            let token = msg.id.chars().take(6).collect::<String>();
            add_pending(
                vault_path,
                PendingEmail {
                    token: token.clone(),
                    message_id,
                    from: from.clone(),
                    subject: subject.clone(),
                    draft: draft.clone(),
                    created_at: Utc::now(),
                    company_id: company_id.clone(),
                },
            )?;
            let company_label = if company_id.is_empty() {
                String::new()
            } else {
                format!("[{company_id}] ")
            };
            let text = format!(
                "{company_label}Reply needed — {from}\nRe: {subject}\n\n{}\n\nDraft:\n---\n{draft}\n---\nReply `send {token}` to send as-is, `skip {token}` to ignore, or type your own version then `send {token}`.",
                t.summary
            );
            let mut out = mailbox::new_message(
                "email-triage",
                "relay",
                MailboxMessageType::Nudge,
                Some("Email needs reply"),
                &text,
                None,
            );
            // Carry the token so the relay renders tap-to-send/skip keyboard pills
            // instead of making the user type `send <token>`.
            out.meta.insert("email_token".to_string(), token.clone());
            // Same-day actionable: a reply-needed prompt with send/skip
            // tokens, not a status ping.
            out.meta.insert(
                hq_core::mailbox::META_INTERRUPT.to_string(),
                "true".to_string(),
            );
            mailbox::send_message(vault_path, &out)?;

            // Surface the reply-needed item on the value bus (best-effort).
            // message_id was moved into PendingEmail above, so re-read it for the dedup key.
            let vi = hq_core::types::ValueItem::new(
                "email-triage",
                hq_core::types::ValueKind::ActionNeeded,
                format!("Reply needed — {from}"),
                t.summary.clone(),
            )
            .with_dedup_key(msg.meta.get("messageId").cloned().unwrap_or_default());
            let _ = hq_db::value_items::emit_at(vault_path, &vi);
            return Ok(());
        }

    // FYI (or no draft/message id): quiet one-liner, unless it is not worth the owner's attention.
    // The event note is already written by the caller, so a suppressed FYI is still on record.
    if crate::email_gate::suppress_fyi(decisions, &from, &subject).await {
        return Ok(());
    }
    let text = format!("📧 FYI — {from} · {subject}\n{}", t.summary);
    // Its own producer so the relay can route FYIs to Discord's #fyi (FR-026).
    let out = mailbox::new_message(
        "email-fyi",
        "relay",
        MailboxMessageType::Direct,
        Some("Email FYI"),
        &text,
        None,
    );
    mailbox::send_message(vault_path, &out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn ev_with_meta(pairs: &[(&str, &str)]) -> MailboxMessage {
        let mut m = mailbox::new_message(
            "ingest",
            "agent-worker",
            MailboxMessageType::Direct,
            None,
            "[hq-event flow=email-triage source=gmail kind=email.totriage]\n\nbody",
            None,
        );
        for (k, v) in pairs {
            m.meta.insert(k.to_string(), v.to_string());
        }
        m
    }

    /// Regression for the Discord double-click TOCTOU flagged in review: two
    /// threads racing `take_pending` on the same token must never both
    /// observe `Some` — that would mean the same email gets sent twice.
    #[test]
    fn take_pending_is_race_safe_against_concurrent_callers() {
        let tmp = TempDir::new().unwrap();
        let vault = tmp.path().to_path_buf();
        add_pending(
            &vault,
            PendingEmail {
                token: "racetok1".to_string(),
                message_id: "m1".to_string(),
                from: "a@b.com".to_string(),
                subject: "s".to_string(),
                draft: "d".to_string(),
                created_at: Utc::now(),
                company_id: String::new(),
            },
        )
        .unwrap();

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let vault = vault.clone();
                std::thread::spawn(move || take_pending(&vault, "racetok1").is_some())
            })
            .collect();
        let hits: usize = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|got| *got)
            .count();
        assert_eq!(hits, 1, "exactly one caller should ever observe Some");
    }

    #[test]
    fn build_prompt_includes_email_fields_and_format() {
        let msg = ev_with_meta(&[
            ("from", "Jane Doe <jane@acme.example>"),
            ("subject", "Revised Q3 timeline"),
            ("snippet", "Can you send the updated dates?"),
        ]);
        let p = build_prompt(&msg, None);
        assert!(p.contains("Jane Doe <jane@acme.example>"));
        assert!(p.contains("Revised Q3 timeline"));
        assert!(p.contains("Can you send the updated dates?"));
        assert!(p.contains("CLASSIFICATION: needs-reply OR fyi"));
        assert!(p.contains("DRAFT:"));
    }

    #[test]
    fn build_prompt_tolerates_missing_fields() {
        let msg = ev_with_meta(&[]);
        let p = build_prompt(&msg, None);
        assert!(p.contains("(unknown sender)"));
        assert!(p.contains("(no subject)"));
        assert!(p.contains("(no preview available)"));
    }

    #[test]
    fn parses_structured_triage() {
        let out = "CLASSIFICATION: needs-reply\nSUMMARY: She wants the revised timeline.\nDRAFT:\nHi Jane, here's the revised timeline...";
        let t = parse_triage(out);
        assert!(t.needs_reply);
        assert_eq!(t.summary, "She wants the revised timeline.");
        assert_eq!(
            t.draft.as_deref(),
            Some("Hi Jane, here's the revised timeline...")
        );
    }

    #[test]
    fn fyi_has_no_draft() {
        let t = parse_triage("CLASSIFICATION: fyi\nSUMMARY: Newsletter, no action.\nDRAFT: n/a");
        assert!(!t.needs_reply);
        assert!(t.draft.is_none());
    }

    #[test]
    fn revision_with_case_changing_unicode_keeps_offsets() {
        // Full Unicode lowercasing grows "İ" by a byte, which used to shift the slice.
        let (action, token, revision) = parse_email_command("İstanbul it is\nsend a1b2c3").unwrap();
        assert_eq!(action, EmailAction::Send);
        assert_eq!(token, "a1b2c3");
        assert_eq!(revision.as_deref(), Some("İstanbul it is"));
    }

    #[test]
    fn parses_send_skip_and_revision() {
        assert_eq!(
            parse_email_command("send a1b2c3"),
            Some((EmailAction::Send, "a1b2c3".to_string(), None))
        );
        assert_eq!(
            parse_email_command("skip a1b2c3"),
            Some((EmailAction::Skip, "a1b2c3".to_string(), None))
        );
        assert_eq!(
            parse_email_command("Actually tell her Tuesday works.\nsend a1b2c3"),
            Some((
                EmailAction::Send,
                "a1b2c3".to_string(),
                Some("Actually tell her Tuesday works.".to_string())
            ))
        );
        assert!(parse_email_command("just a normal message").is_none());
    }

    #[test]
    fn pending_store_roundtrip_and_take() {
        let tmp = TempDir::new().unwrap();
        add_pending(
            tmp.path(),
            PendingEmail {
                token: "abc123".into(),
                message_id: "msg-1".into(),
                from: "j@co.com".into(),
                subject: "hi".into(),
                draft: "the draft".into(),
                created_at: Utc::now(),
                company_id: String::new(),
            },
        )
        .unwrap();
        let pe = take_pending(tmp.path(), "abc123").unwrap();
        assert_eq!(pe.message_id, "msg-1");
        assert_eq!(pe.draft, "the draft");
        // gone after take
        assert!(take_pending(tmp.path(), "abc123").is_none());
    }

    #[test]
    fn build_prompt_uses_provided_identity() {
        use chrono::Utc;
        use hq_core::types::{MailboxMessage, MailboxMessageType};
        let mut meta = std::collections::HashMap::new();
        meta.insert("from".into(), "client@example.com".into());
        meta.insert("subject".into(), "Invoice query".into());
        meta.insert("snippet".into(), "Please send the invoice for April".into());
        let msg = MailboxMessage {
            id: "test-id".into(),
            timestamp: Utc::now(),
            from: "ingest".into(),
            to: "agent-worker".into(),
            msg_type: MailboxMessageType::Direct,
            subject: None,
            content: String::new(),
            job_id: None,
            meta,
        };
        let prompt = build_prompt(&msg, Some(("Jane Smith", "CFO", "Acme Corp")));
        assert!(prompt.contains("Jane Smith"));
        assert!(prompt.contains("CFO"));
        assert!(prompt.contains("Acme Corp"));
        assert!(!prompt.contains("Alice Example"));
        assert!(prompt.contains("CLASSIFICATION:"));
    }

    #[test]
    fn build_prompt_falls_back_when_no_identity() {
        use chrono::Utc;
        use hq_core::types::{MailboxMessage, MailboxMessageType};
        let mut meta = std::collections::HashMap::new();
        meta.insert("from".into(), "x@y.com".into());
        meta.insert("subject".into(), "Hello".into());
        let msg = MailboxMessage {
            id: "test-id".into(),
            timestamp: Utc::now(),
            from: "ingest".into(),
            to: "agent-worker".into(),
            msg_type: MailboxMessageType::Direct,
            subject: None,
            content: String::new(),
            job_id: None,
            meta,
        };
        let prompt = build_prompt(&msg, None);
        assert!(prompt.contains("the operator"));
        assert!(prompt.contains("CLASSIFICATION:"));
    }

    #[tokio::test]
    async fn resolve_email_action_returns_none_when_no_pending_matches() {
        let tmp = TempDir::new().unwrap();
        let reply = resolve_email_action(tmp.path(), "nope1234", EmailAction::Skip, None).await;
        assert!(reply.is_none());
    }

    #[tokio::test]
    async fn resolve_email_action_skip_replies_and_consumes_pending() {
        let tmp = TempDir::new().unwrap();
        add_pending(
            tmp.path(),
            PendingEmail {
                token: "abc123".into(),
                message_id: "msg-1".into(),
                from: "j@co.com".into(),
                subject: "Q3 timeline".into(),
                draft: "the draft".into(),
                created_at: Utc::now(),
                company_id: String::new(),
            },
        )
        .unwrap();

        let reply = resolve_email_action(tmp.path(), "abc123", EmailAction::Skip, None).await;
        assert_eq!(
            reply.as_deref(),
            Some("Skipped — \"Q3 timeline\" won't be sent.")
        );

        // Consumed by take_pending inside resolve_email_action — a second call finds nothing.
        let second = resolve_email_action(tmp.path(), "abc123", EmailAction::Skip, None).await;
        assert!(second.is_none());
    }

    #[test]
    fn pending_email_round_trips_company_id() {
        let tmp = TempDir::new().unwrap();
        let pe = PendingEmail {
            token: "abc123".into(),
            message_id: "msg-1".into(),
            from: "x@y.com".into(),
            subject: "Hello".into(),
            draft: "Hi there".into(),
            created_at: Utc::now(),
            company_id: "acme".into(),
        };
        add_pending(tmp.path(), pe.clone()).unwrap();
        let got = take_pending(tmp.path(), "abc123").unwrap();
        assert_eq!(got.company_id, "acme");
    }
}
