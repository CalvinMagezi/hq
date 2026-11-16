//! Which sites a session that has read outside text may fetch without asking again. Only the
//! owner's approvals are written here. Trust is per exact host, so one person's page on a shared
//! host does not cover the others. The owner may also name a domain by hand
//! (`fetch_trusted_domains`): it covers the host and its subdomains in every chat, and is only
//! ever written by the owner's own routes, never by a tool a model can call.

use anyhow::Result;
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

/// Approved fetches of one site in one chat that allow the rest of that site in that chat.
const APPROVALS_FOR_WHOLE_SITE: i64 = 3;
/// Different chats that must have approved a site before it stops asking in new chats.
const CHATS_FOR_TRUST: i64 = 3;

/// Records one approved fetch. True when the site is now allowed throughout this chat.
pub fn record_approval(conn: &Connection, host: &str, scope: &str) -> Result<bool> {
    count_allowed_once(conn, host)?;
    conn.execute(
        "INSERT INTO fetch_site_grants (host, scope, approvals, updated_at) VALUES (?1, ?2, 1, ?3)
         ON CONFLICT(host, scope) DO UPDATE SET approvals = approvals + 1,
           updated_at = excluded.updated_at",
        params![host, scope, Utc::now().to_rfc3339()],
    )?;
    conn.execute(
        "UPDATE fetch_site_grants SET whole_site = 1
         WHERE host = ?1 AND scope = ?2 AND approvals >= ?3",
        params![host, scope, APPROVALS_FOR_WHOLE_SITE],
    )?;
    Ok(whole_site(conn, host, scope)?)
}

/// The owner chose to allow the whole site in this chat.
pub fn allow_whole_site(conn: &Connection, host: &str, scope: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO fetch_site_grants (host, scope, approvals, whole_site, updated_at)
         VALUES (?1, ?2, 1, 1, ?3)
         ON CONFLICT(host, scope) DO UPDATE SET approvals = approvals + 1, whole_site = 1,
           updated_at = excluded.updated_at",
        params![host, scope, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

fn whole_site(conn: &Connection, host: &str, scope: &str) -> rusqlite::Result<bool> {
    Ok(conn
        .query_row(
            "SELECT whole_site FROM fetch_site_grants WHERE host = ?1 AND scope = ?2",
            params![host, scope],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        == Some(1))
}

/// How many times the owner answered "Allow once" for `host`, in every chat. "Allow all" and a
/// decline never change it, and only the owner's inbox answers write it, so a helper's own
/// prompts never count. The apps offer "Allow all for this site" once it reaches two; the server
/// does not enforce that (`approve_site` works at any count).
pub fn allowed_once_for_host(conn: &Connection, host: &str) -> Result<i64> {
    Ok(conn
        .query_row(
            "SELECT count FROM site_allowed_once WHERE host = ?1",
            params![host],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0))
}

fn count_allowed_once(conn: &Connection, host: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO site_allowed_once (host, count) VALUES (?1, 1)
         ON CONFLICT(host) DO UPDATE SET count = count + 1",
        params![host],
    )?;
    Ok(())
}

/// A declined fetch undoes what this chat had earned for the site.
pub fn forget_in_chat(conn: &Connection, host: &str, scope: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM fetch_site_grants WHERE host = ?1 AND scope = ?2",
        params![host, scope],
    )?;
    Ok(())
}

pub fn is_allowed(conn: &Connection, host: &str, scope: &str) -> Result<bool> {
    if whole_site(conn, host, scope)? || domain_trusted(conn, host)? {
        return Ok(true);
    }
    let chats: i64 = conn.query_row(
        "SELECT COUNT(*) FROM fetch_site_grants WHERE host = ?1",
        params![host],
        |r| r.get(0),
    )?;
    Ok(chats >= CHATS_FOR_TRUST)
}

/// Sites that no longer ask in new chats, for the owner to review.
pub fn trusted(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT host FROM fetch_site_grants GROUP BY host HAVING COUNT(*) >= ?1 ORDER BY host",
    )?;
    let rows = stmt.query_map(params![CHATS_FOR_TRUST], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The owner removed a site: it must be earned again. A domain the owner added by hand goes too.
pub fn forget(conn: &Connection, host: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM fetch_site_grants WHERE host = ?1",
        params![host],
    )?;
    conn.execute(
        "DELETE FROM fetch_trusted_domains WHERE domain = ?1",
        params![normalize_domain(host)],
    )?;
    Ok(())
}

/// Lowercase, no surrounding space, no trailing dot, no leading `www.` (the fetch check drops it
/// too), so one spelling is stored and compared.
fn normalize_domain(raw: &str) -> String {
    let lower = raw.trim().to_lowercase();
    let host = lower.trim_end_matches('.');
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

/// A domain the owner may add: dotted labels of letters, digits and hyphens (a non-ASCII name is
/// kept as given), never an address, a port, a path or a login.
fn valid_domain(domain: &str) -> bool {
    let labels: Vec<&str> = domain.split('.').collect();
    let label_ok = |l: &&str| {
        !l.is_empty()
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars().all(|c| c == '-' || c.is_alphanumeric())
    };
    labels.len() >= 2
        && labels.iter().all(label_ok)
        && labels
            .last()
            .is_some_and(|tld| !tld.chars().all(|c| c.is_ascii_digit()))
}

/// True when `host` is `domain` or a subdomain of it. Whole labels only, so `badexample.com` and
/// `example.com.evil.test` do not match `example.com`.
fn host_in_domain(host: &str, domain: &str) -> bool {
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|rest| rest.ends_with('.'))
}

/// Whether `host` (already parsed out of the address, so a login in front of it never gets here)
/// falls under a domain the owner added.
fn domain_trusted(conn: &Connection, host: &str) -> Result<bool> {
    let host = host.trim_end_matches('.').to_lowercase();
    let domains = trusted_domains(conn)?;
    Ok(domains.iter().any(|d| host_in_domain(&host, d)))
}

/// Domains the owner added by hand, for the owner to review.
pub fn trusted_domains(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT domain FROM fetch_trusted_domains ORDER BY domain")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The owner trusts `domain` and its subdomains. Returns the stored spelling, or an error naming
/// what is wrong. Called only from the owner's routes and commands, never from a tool.
pub fn add_trusted_domain(conn: &Connection, raw: &str) -> Result<String> {
    let domain = normalize_domain(raw);
    if !valid_domain(&domain) {
        anyhow::bail!(
            "not a domain name: use something like example.com, with no scheme, port or path"
        );
    }
    conn.execute(
        "INSERT INTO fetch_trusted_domains (domain, added_at) VALUES (?1, ?2)
         ON CONFLICT(domain) DO NOTHING",
        params![domain, Utc::now().to_rfc3339()],
    )?;
    Ok(domain)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_db(check: impl FnOnce(&Connection) -> Result<()>) {
        crate::Database::open_memory()
            .unwrap()
            .with_conn(check)
            .unwrap();
    }

    #[test]
    fn three_approvals_allow_the_site_in_that_chat_only() {
        with_db(|c| {
            assert!(!record_approval(c, "a.example", "chat-1")?);
            assert!(!record_approval(c, "a.example", "chat-1")?);
            assert!(!is_allowed(c, "a.example", "chat-1")?);
            assert!(record_approval(c, "a.example", "chat-1")?);
            assert!(is_allowed(c, "a.example", "chat-1")?);
            assert!(!is_allowed(c, "a.example", "chat-2")?);
            assert!(!is_allowed(c, "b.example", "chat-1")?);
            Ok(())
        });
    }

    #[test]
    fn allowed_once_counts_only_plain_answers_and_a_decline_or_allow_all_never_change_it() {
        with_db(|c| {
            assert_eq!(allowed_once_for_host(c, "a.example")?, 0);
            record_approval(c, "a.example", "chat-1")?;
            record_approval(c, "a.example", "chat-1")?;
            assert_eq!(allowed_once_for_host(c, "a.example")?, 2);
            // A decline clears the chat's grant, not what the owner already allowed.
            forget_in_chat(c, "a.example", "chat-1")?;
            assert_eq!(allowed_once_for_host(c, "a.example")?, 2);
            // "Allow all" in another chat is not an allow-once answer.
            allow_whole_site(c, "a.example", "chat-2")?;
            assert_eq!(allowed_once_for_host(c, "a.example")?, 2);
            allow_whole_site(c, "b.example", "chat-A")?;
            assert_eq!(allowed_once_for_host(c, "b.example")?, 0);
            record_approval(c, "a.example", "chat-3")?;
            assert_eq!(allowed_once_for_host(c, "a.example")?, 3);
            Ok(())
        });
    }

    #[test]
    fn a_site_approved_in_three_chats_stops_asking_until_removed() {
        with_db(|c| {
            for chat in ["chat-1", "chat-2"] {
                record_approval(c, "a.example", chat)?;
            }
            assert!(!is_allowed(c, "a.example", "chat-new")?);
            assert!(trusted(c)?.is_empty());
            record_approval(c, "a.example", "chat-3")?;
            assert!(is_allowed(c, "a.example", "chat-new")?);
            assert_eq!(trusted(c)?, ["a.example"]);
            forget(c, "a.example")?;
            assert!(!is_allowed(c, "a.example", "chat-new")?);
            Ok(())
        });
    }

    #[test]
    fn allowing_the_whole_site_counts_at_once_and_a_decline_undoes_it() {
        with_db(|c| {
            allow_whole_site(c, "a.example", "chat-1")?;
            assert!(is_allowed(c, "a.example", "chat-1")?);
            forget_in_chat(c, "a.example", "chat-1")?;
            assert!(!is_allowed(c, "a.example", "chat-1")?);
            Ok(())
        });
    }

    #[test]
    fn a_trusted_domain_covers_itself_and_its_subdomains_by_whole_labels() {
        with_db(|c| {
            assert_eq!(add_trusted_domain(c, " Example.COM. ")?, "example.com");
            let allowed = |h: &str| is_allowed(c, h, "chat-1").unwrap();
            assert!(allowed("example.com"));
            assert!(allowed("docs.example.com"));
            assert!(allowed("a.b.example.com"));
            assert!(allowed("EXAMPLE.com"));
            assert!(allowed("example.com."));
            assert!(!allowed("badexample.com"));
            assert!(!allowed("example.com.evil.test"));
            assert!(!allowed("example.com@evil.test"));
            assert!(!allowed("evil.test"));
            assert!(!allowed("com"));
            Ok(())
        });
    }

    #[test]
    fn only_plain_domains_can_be_added_and_removing_one_restores_asking() {
        with_db(|c| {
            for bad in [
                "example",
                "",
                "https://example.com",
                "example.com:8080",
                "example.com/x",
                "user@example.com",
                "1.2.3.4",
                "a..b",
                "-a.com",
                "a b.com",
                "*.example.com",
            ] {
                assert!(add_trusted_domain(c, bad).is_err(), "{bad} must be refused");
            }
            assert_eq!(add_trusted_domain(c, "www.example.com")?, "example.com");
            assert_eq!(add_trusted_domain(c, "bücher.example")?, "bücher.example");
            assert_eq!(trusted_domains(c)?, ["bücher.example", "example.com"]);
            assert!(is_allowed(c, "example.com", "chat-1")?);
            forget(c, "example.com")?;
            assert!(!is_allowed(c, "example.com", "chat-1")?);
            assert_eq!(trusted_domains(c)?, ["bücher.example"]);
            Ok(())
        });
    }
}
