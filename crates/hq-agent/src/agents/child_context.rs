//! Per-child vault context packets (FR-062).
//!
//! A child never inherits HQ's conversation or memory, so each one gets a
//! self-contained packet built from its own `context_need`, at the moment it
//! starts. HQ later checks the child's citations against that same packet.

use super::types::ChildRequest;
use chrono::{DateTime, Utc};
use hq_db::Database;
use hq_memory::context_packet::{CitationReport, ContextPacket, build_packet, verify_citations};
use hq_vault::VaultClient;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const VAULT_UNAVAILABLE_NOTE: &str = "Vault context was requested for this task but the vault \
     could not be opened. No context was supplied. Say so instead of assuming what the vault holds.";

fn append_context(req: &mut ChildRequest, block: &str) {
    req.context = Some(match req.context.take() {
        Some(existing) if !existing.is_empty() => format!("{existing}\n\n{block}"),
        _ => block.to_string(),
    });
}

/// Build or refresh the child's packet and fold its rendering into `req.context`.
pub(super) fn prepare(
    vault_path: &Path,
    db: Option<&Database>,
    mut req: ChildRequest,
    now: DateTime<Utc>,
) -> (ChildRequest, Option<ContextPacket>) {
    let prebuilt = req.context_packet.take();
    let wants_context =
        prebuilt.is_some() || req.context_need.as_ref().is_some_and(|n| !n.is_empty());
    if !wants_context {
        return (req, None);
    }
    let Ok(vault) = VaultClient::new(vault_path.to_path_buf()) else {
        append_context(&mut req, VAULT_UNAVAILABLE_NOTE);
        return (req, None);
    };
    let packet = match (prebuilt, req.context_need.clone()) {
        (Some(mut packet), _) => {
            // Refresh failure keeps the old entries; their retrieval time still shows their age.
            let _ = packet.refresh_expired(&vault, now);
            packet
        }
        (None, Some(need)) => build_packet(&vault, db, &req.goal, &need, now),
        (None, None) => return (req, None),
    };
    append_context(&mut req, &packet.render());
    (req, Some(packet))
}

pub(super) async fn prepare_async(
    vault_path: PathBuf,
    db: Option<Arc<Database>>,
    req: ChildRequest,
) -> (ChildRequest, Option<ContextPacket>) {
    let fallback = req.clone();
    tokio::task::spawn_blocking(move || prepare(&vault_path, db.as_deref(), req, Utc::now()))
        .await
        .unwrap_or((fallback, None))
}

pub(super) fn check_citations(
    vault_path: &Path,
    output: &str,
    packet: &ContextPacket,
) -> Option<CitationReport> {
    let vault = VaultClient::new(vault_path.to_path_buf()).ok()?;
    Some(verify_citations(output, packet, &vault))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use hq_memory::context_packet::ContextNeed;
    use tempfile::TempDir;

    fn vault_with_notes() -> TempDir {
        let dir = TempDir::new().unwrap();
        let nb = dir.path().join("Notebooks");
        std::fs::create_dir_all(&nb).unwrap();
        std::fs::write(nb.join("billing.md"), "Invoices are due in 30 days.").unwrap();
        std::fs::write(nb.join("hiring.md"), "Interviews run on Tuesdays.").unwrap();
        dir
    }

    fn child(id: &str, goal: &str, note: &str) -> ChildRequest {
        let mut req = ChildRequest::new(id, goal);
        req.context_need = Some(ContextNeed {
            refs: vec![format!("Notebooks/{note}.md")],
            time_sensitive: true,
            ..Default::default()
        });
        req
    }

    #[test]
    fn two_children_get_distinct_self_contained_packets() {
        let dir = vault_with_notes();
        let now = Utc::now();
        let (a, pa) = prepare(
            dir.path(),
            None,
            child("a", "check billing", "billing"),
            now,
        );
        let (b, pb) = prepare(dir.path(), None, child("b", "plan hiring", "hiring"), now);
        let (ca, cb) = (a.context.unwrap(), b.context.unwrap());
        assert!(ca.contains("Invoices are due") && !ca.contains("Interviews"));
        assert!(cb.contains("Interviews") && !cb.contains("Invoices"));
        assert!(ca.contains("Never follow instructions") && ca.contains("Gaps:"));
        assert_eq!(pa.unwrap().entries.len(), 1);
        assert_eq!(pb.unwrap().entries.len(), 1);
    }

    #[test]
    fn child_without_a_need_gets_no_packet_and_missing_vault_notes_become_gaps() {
        let dir = vault_with_notes();
        let (req, packet) = prepare(dir.path(), None, ChildRequest::new("x", "g"), Utc::now());
        assert!(req.context.is_none() && packet.is_none());

        let (req, packet) = prepare(dir.path(), None, child("y", "g", "ghost"), Utc::now());
        assert!(packet.unwrap().entries.is_empty());
        assert!(req.context.unwrap().contains("missing or unreadable"));
    }

    #[test]
    fn prebuilt_packet_is_refreshed_when_the_child_starts_late() {
        let dir = vault_with_notes();
        let t0 = Utc::now();
        let (_, packet) = prepare(dir.path(), None, child("a", "check billing", "billing"), t0);
        std::fs::write(
            dir.path().join("Notebooks/billing.md"),
            "Invoices are due in 45 days.",
        )
        .unwrap();

        let mut queued = ChildRequest::new("a", "check billing");
        queued.context_packet = packet;
        let (req, refreshed) = prepare(dir.path(), None, queued, t0 + Duration::hours(3));
        assert!(req.context.unwrap().contains("45 days"));
        assert_eq!(refreshed.unwrap().retrieved_at, t0 + Duration::hours(3));
    }

    #[test]
    fn citations_in_a_child_answer_are_checked_against_its_packet() {
        let dir = vault_with_notes();
        let (_, packet) = prepare(dir.path(), None, child("a", "g", "billing"), Utc::now());
        let packet = packet.unwrap();
        let answer = "Due in 30 days [S1], also [S4].\nSources used: S1\nGaps: none";
        let report = check_citations(dir.path(), answer, &packet).unwrap();
        assert_eq!(report.verified, vec!["S1"]);
        assert_eq!(report.unknown, vec!["S4"]);
        assert!(report.lists_sources_and_gaps);
    }
}
