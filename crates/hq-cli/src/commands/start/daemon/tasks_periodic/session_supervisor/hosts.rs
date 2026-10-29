//! A host with running sessions that stops answering (a laptop that slept, a machine off the
//! network) says nothing about its sessions, so the sweep leaves them as they are. Left at
//! that, nobody hears about it: a team waits for hours on a closed lid. Here each silent host
//! opens an outage, the outage raises one "not answering" alert once it has lasted
//! `agent_host.unreachable_alert_minutes`, and the host's first answer retires that alert and
//! says it is back. Outages are rows (`hq_db::host_outages`), so a restart repeats nothing.

use super::*;
use hq_db::host_outages;
use hq_tools::harness_session::HostPoll;

/// Sessions named in an alert; the rest are counted.
const NAMED_SESSIONS: usize = 6;

/// Brings the outages up to date with what the hosts said this sweep. `after` is how long a
/// host may be silent before the alert (`None`: never alert, but still track). Logged, never
/// raised: the sweep's other work does not depend on it.
pub(super) fn watch_hosts(
    db: &Database,
    polled: &HostPoll,
    rows: &[registry::HarnessSessionRow],
    after: Option<Duration>,
    now: i64,
) {
    for (host, answer) in polled {
        let result = match answer {
            Ok(_) => host_answered(db, host, rows, now),
            Err(detail) => host_silent(db, host, detail, rows, after, now),
        };
        if let Err(e) = result {
            tracing::warn!(%host, error = %e, "session-supervisor: host outage not recorded");
        }
    }
    // A host whose sessions were all stopped while it was silent is no longer polled. Its
    // outage ends quietly: nothing waits on it, and whether it is back is unknown.
    match host_outages::list(db) {
        Ok(open) => {
            for outage in open.iter().filter(|o| !polled.contains_key(&o.host)) {
                if let Ok(Some(ended)) = host_outages::end(db, &outage.host)
                    && let Some(item) = &ended.alert_item_id
                {
                    let _ = hq_db::value_items::retire(db, item);
                }
            }
        }
        Err(e) => tracing::warn!(error = %e, "session-supervisor: host outages not listed"),
    }
}

fn host_silent(
    db: &Database,
    host: &str,
    detail: &str,
    rows: &[registry::HarnessSessionRow],
    after: Option<Duration>,
    now: i64,
) -> Result<()> {
    let outage = host_outages::note_silent(db, host, &first_line(detail), now)?;
    let Some(after) = after else { return Ok(()) };
    if outage.alert_item_id.is_some() || now - outage.since < after.as_secs() as i64 {
        return Ok(());
    }
    let item = ValueItem::new(
        "session-supervisor",
        ValueKind::ActionNeeded,
        format!("Host '{host}' is not answering"),
        format!(
            "Host {host} has not answered for {} minutes, and {} on it cannot go on until it does: {}.\nA laptop that slept or left the network does this. Wake it, or check `hq host status` on it. HQ says when it answers again. `agent_host.keep_awake: while_sessions` keeps a machine awake while sessions run on it.\nLast error: {}",
            outage.minutes(now),
            sessions_phrase(rows, host),
            sessions_list(rows, host),
            outage.detail,
        ),
    )
    .with_dedup_key(format!("host-unreachable-{host}-{}", outage.since));
    // The claim makes the alert one per outage, however many sweeps overlap.
    if host_outages::claim_alert(db, host, outage.since, &item.id)? {
        hq_db::value_items::emit(db, &item)?;
        for row in rows.iter().filter(|r| r.host == host) {
            record_event(
                db,
                &row.id,
                "host_unreachable",
                serde_json::json!({ "host": host, "minutes": outage.minutes(now) }),
            );
        }
        tracing::info!(%host, minutes = outage.minutes(now), "session-supervisor: host not answering, alert sent");
    }
    Ok(())
}

fn host_answered(
    db: &Database,
    host: &str,
    rows: &[registry::HarnessSessionRow],
    now: i64,
) -> Result<()> {
    let Some(outage) = host_outages::end(db, host)? else {
        return Ok(());
    };
    // A silence too short to alert about ends without a word.
    let Some(item) = &outage.alert_item_id else {
        return Ok(());
    };
    hq_db::value_items::retire(db, item)?;
    let back = ValueItem::new(
        "session-supervisor",
        ValueKind::Fyi,
        format!("Host '{host}' is answering again"),
        format!(
            "Host {host} answered after {} minutes of silence. {} on it: {}.",
            outage.minutes(now),
            capitalised(&sessions_phrase(rows, host)),
            sessions_list(rows, host),
        ),
    )
    .with_dedup_key(format!("host-back-{host}-{}", outage.since));
    hq_db::value_items::emit(db, &back)?;
    for row in rows.iter().filter(|r| r.host == host) {
        record_event(
            db,
            &row.id,
            "host_back",
            serde_json::json!({ "host": host, "minutes": outage.minutes(now) }),
        );
    }
    tracing::info!(%host, minutes = outage.minutes(now), "session-supervisor: host answering again");
    Ok(())
}

/// "1 session" or "3 sessions".
fn sessions_phrase(rows: &[registry::HarnessSessionRow], host: &str) -> String {
    match rows.iter().filter(|r| r.host == host).count() {
        1 => "1 session".to_string(),
        n => format!("{n} sessions"),
    }
}

fn capitalised(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// The host's sessions by label and id, the first few named.
fn sessions_list(rows: &[registry::HarnessSessionRow], host: &str) -> String {
    let on_host: Vec<_> = rows.iter().filter(|r| r.host == host).collect();
    let mut named: Vec<String> = on_host
        .iter()
        .take(NAMED_SESSIONS)
        .map(|r| format!("'{}' {}", session_label(r), r.id))
        .collect();
    if on_host.len() > NAMED_SESSIONS {
        named.push(format!("{} more", on_host.len() - NAMED_SESSIONS));
    }
    named.join(", ")
}

/// Errors from ssh can run to many lines; the start of the first says what happened.
fn first_line(detail: &str) -> String {
    const MAX: usize = 300;
    let line = detail.lines().next().unwrap_or("").trim();
    if line.len() <= MAX {
        return line.to_string();
    }
    format!("{}…", &line[..line.floor_char_boundary(MAX)])
}
