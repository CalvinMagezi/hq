//! OAuth state for the claude.ai connector (`docs/plans/claude-connector.md`): the owner's
//! arming window, pending consent requests, single-use codes, grants and tokens. Secrets are
//! returned once and stored only as SHA-256. Callers pass `now` in unix seconds.

use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};
use sha2::{Digest, Sha256};

/// Prefixes that make a leaked secret easy to recognise and scan for.
pub const ACCESS_PREFIX: &str = "hqo_";
pub const REFRESH_PREFIX: &str = "hqr_";
pub const CODE_PREFIX: &str = "hqc_";

pub const ACCESS_TTL_SECS: i64 = 60 * 60;
pub const REFRESH_TTL_SECS: i64 = 30 * 24 * 60 * 60;
/// A grant ends this long after the owner approved it, however often it is refreshed.
pub const GRANT_MAX_SECS: i64 = 90 * 24 * 60 * 60;
pub const REQUEST_TTL_SECS: i64 = 10 * 60;
pub const CODE_TTL_SECS: i64 = 60;
/// A refresh token presented again this soon after its use is a client retrying a refresh,
/// not a thief: refused, but the grant survives.
pub const REUSE_GRACE_SECS: i64 = 60;

/// What a grant may reach, mapped by the web server onto the gateway's allowlists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantScope {
    Full,
    ReadOnly,
    Tasks,
}

impl GrantScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::ReadOnly => "read_only",
            Self::Tasks => "tasks",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "full" => Some(Self::Full),
            "read_only" => Some(Self::ReadOnly),
            "tasks" => Some(Self::Tasks),
            _ => None,
        }
    }
}

fn hash(secret: &str) -> String {
    Sha256::digest(secret.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn random_secret(prefix: &str) -> String {
    format!(
        "{prefix}{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Opens the window for connecting a new client until `until`.
pub fn arm(conn: &Connection, until: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO mcp_connector_arming (id, armed_until) VALUES (1, ?1)
         ON CONFLICT(id) DO UPDATE SET armed_until = excluded.armed_until",
        params![until],
    )?;
    Ok(())
}

pub fn disarm(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM mcp_connector_arming", [])?;
    Ok(())
}

/// When the window closes, if it is open at `now`.
pub fn armed_until(conn: &Connection, now: i64) -> Result<Option<i64>> {
    let until: Option<i64> = conn
        .query_row(
            "SELECT armed_until FROM mcp_connector_arming WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(until.filter(|u| *u > now))
}

/// A validated authorization request, before the owner decides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRequest<'a> {
    pub client_id: &'a str,
    pub client_name: &'a str,
    pub redirect_uri: &'a str,
    pub state: Option<&'a str>,
    pub code_challenge: &'a str,
    pub resource: &'a str,
    pub scope: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthRequest {
    pub id: String,
    pub client_id: String,
    pub client_name: String,
    pub redirect_uri: String,
    pub state: Option<String>,
    pub code_challenge: String,
    pub resource: String,
    pub scope: String,
    pub expires_at: i64,
}

const REQUEST_COLS: &str =
    "id, client_id, client_name, redirect_uri, state, code_challenge, resource, scope, expires_at";

fn request_from_row(r: &Row) -> rusqlite::Result<AuthRequest> {
    Ok(AuthRequest {
        id: r.get(0)?,
        client_id: r.get(1)?,
        client_name: r.get(2)?,
        redirect_uri: r.get(3)?,
        state: r.get(4)?,
        code_challenge: r.get(5)?,
        resource: r.get(6)?,
        scope: r.get(7)?,
        expires_at: r.get(8)?,
    })
}

/// Stores a request for the consent page and returns its id.
pub fn create_request(conn: &Connection, req: &NewRequest, now: i64) -> Result<String> {
    purge_expired(conn, now)?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    conn.execute(
        &format!(
            "INSERT INTO mcp_oauth_requests ({REQUEST_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)"
        ),
        params![
            id,
            req.client_id,
            req.client_name,
            req.redirect_uri,
            req.state,
            req.code_challenge,
            req.resource,
            req.scope,
            now + REQUEST_TTL_SECS
        ],
    )?;
    Ok(id)
}

/// A request that has not expired.
pub fn get_request(conn: &Connection, id: &str, now: i64) -> Result<Option<AuthRequest>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {REQUEST_COLS} FROM mcp_oauth_requests WHERE id = ?1 AND expires_at > ?2"
            ),
            params![id, now],
            request_from_row,
        )
        .optional()?)
}

/// Removes a request and returns it if it was still valid, so a decision is made once.
pub fn take_request(conn: &Connection, id: &str, now: i64) -> Result<Option<AuthRequest>> {
    let req = get_request(conn, id, now)?;
    let removed = conn.execute("DELETE FROM mcp_oauth_requests WHERE id = ?1", params![id])?;
    Ok(req.filter(|_| removed == 1))
}

/// The owner's approval of a request: a single-use code for the client to exchange.
pub fn issue_code(
    conn: &Connection,
    req: &AuthRequest,
    grant_scope: GrantScope,
    label: &str,
    now: i64,
) -> Result<String> {
    let code = random_secret(CODE_PREFIX);
    conn.execute(
        "INSERT INTO mcp_oauth_codes (code_hash, client_id, client_name, redirect_uri,
            code_challenge, resource, scope, grant_scope, label, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            hash(&code),
            req.client_id,
            req.client_name,
            req.redirect_uri,
            req.code_challenge,
            req.resource,
            req.scope,
            grant_scope.as_str(),
            label,
            now + CODE_TTL_SECS
        ],
    )?;
    Ok(code)
}

/// Tokens handed to a client. The secrets exist only here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    pub scope: String,
    pub grant_id: String,
}

/// What the token endpoint checks a code against. `challenge` is the S256 of the verifier
/// the client sent, computed by the caller.
#[derive(Debug, Clone, Copy)]
pub struct CodeExchange<'a> {
    pub code: &'a str,
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
    pub challenge: &'a str,
    pub resource: Option<&'a str>,
}

struct CodeRow {
    client_id: String,
    client_name: String,
    redirect_uri: String,
    code_challenge: String,
    resource: String,
    scope: String,
    grant_scope: String,
    label: String,
    expires_at: i64,
    grant_id: Option<String>,
}

fn code_row(conn: &Connection, code_hash: &str) -> Result<Option<CodeRow>> {
    Ok(conn
        .query_row(
            "SELECT client_id, client_name, redirect_uri, code_challenge, resource, scope,
                    grant_scope, label, expires_at, grant_id
             FROM mcp_oauth_codes WHERE code_hash = ?1",
            params![code_hash],
            |r| {
                Ok(CodeRow {
                    client_id: r.get(0)?,
                    client_name: r.get(1)?,
                    redirect_uri: r.get(2)?,
                    code_challenge: r.get(3)?,
                    resource: r.get(4)?,
                    scope: r.get(5)?,
                    grant_scope: r.get(6)?,
                    label: r.get(7)?,
                    expires_at: r.get(8)?,
                    grant_id: r.get(9)?,
                })
            },
        )
        .optional()?)
}

/// Exchanges a code for tokens, or `None` for anything wrong with it. The code is spent by
/// the first attempt, right or wrong; a second use revokes the grant the first one made.
pub fn redeem_code(conn: &Connection, ex: &CodeExchange, now: i64) -> Result<Option<TokenPair>> {
    let code_hash = hash(ex.code);
    let Some(row) = code_row(conn, &code_hash)? else {
        return Ok(None);
    };
    let claimed = conn.execute(
        "UPDATE mcp_oauth_codes SET used = 1 WHERE code_hash = ?1 AND used = 0",
        params![code_hash],
    )?;
    if claimed == 0 {
        if let Some(grant_id) = &row.grant_id {
            revoke_grant(conn, grant_id, "authorization code used twice", now)?;
        }
        return Ok(None);
    }
    let matches = row.expires_at > now
        && row.client_id == ex.client_id
        && row.redirect_uri == ex.redirect_uri
        && row.code_challenge == ex.challenge
        && ex.resource.is_none_or(|r| r == row.resource);
    if !matches {
        return Ok(None);
    }
    let tx = conn.unchecked_transaction()?;
    let grant_id = create_grant(&tx, &row, now)?;
    tx.execute(
        "UPDATE mcp_oauth_codes SET grant_id = ?2 WHERE code_hash = ?1",
        params![code_hash, grant_id],
    )?;
    let pair = issue_tokens(&tx, &grant_id, &row.scope, now)?;
    tx.commit()?;
    Ok(Some(pair))
}

fn create_grant(conn: &Connection, row: &CodeRow, now: i64) -> Result<String> {
    if GrantScope::parse(&row.grant_scope).is_none() {
        bail!("unknown grant scope {:?}", row.grant_scope);
    }
    let id = format!("grant-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    conn.execute(
        "INSERT INTO mcp_oauth_grants (id, client_id, client_name, label, grant_scope, scope,
            resource, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            row.client_id,
            row.client_name,
            row.label,
            row.grant_scope,
            row.scope,
            row.resource,
            now,
            now + GRANT_MAX_SECS
        ],
    )?;
    Ok(id)
}

fn issue_tokens(conn: &Connection, grant_id: &str, scope: &str, now: i64) -> Result<TokenPair> {
    let grant_expires: i64 = conn.query_row(
        "SELECT expires_at FROM mcp_oauth_grants WHERE id = ?1",
        params![grant_id],
        |r| r.get(0),
    )?;
    let access = random_secret(ACCESS_PREFIX);
    let refresh = random_secret(REFRESH_PREFIX);
    let insert = "INSERT INTO mcp_oauth_tokens (token_hash, grant_id, kind, expires_at)
                  VALUES (?1, ?2, ?3, ?4)";
    let access_expires = (now + ACCESS_TTL_SECS).min(grant_expires);
    conn.execute(
        insert,
        params![hash(&access), grant_id, "access", access_expires],
    )?;
    let refresh_expires = (now + REFRESH_TTL_SECS).min(grant_expires);
    conn.execute(
        insert,
        params![hash(&refresh), grant_id, "refresh", refresh_expires],
    )?;
    Ok(TokenPair {
        access_token: access,
        refresh_token: refresh,
        expires_in: access_expires - now,
        scope: scope.to_string(),
        grant_id: grant_id.to_string(),
    })
}

/// How a refresh attempt ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refresh {
    Issued(TokenPair),
    Refused,
    /// A spent refresh token came back after the grace: the grant is revoked.
    Reused {
        grant_id: String,
    },
}

struct RefreshRow {
    grant_id: String,
    expires_at: i64,
    rotated_at: Option<i64>,
    client_id: String,
    resource: String,
    scope: String,
    revoked: bool,
    grant_expires: i64,
}

fn refresh_row(conn: &Connection, token_hash: &str) -> Result<Option<RefreshRow>> {
    Ok(conn
        .query_row(
            "SELECT t.grant_id, t.expires_at, t.rotated_at, g.client_id, g.resource, g.scope,
                    g.revoked_at IS NOT NULL, g.expires_at
             FROM mcp_oauth_tokens t JOIN mcp_oauth_grants g ON g.id = t.grant_id
             WHERE t.token_hash = ?1 AND t.kind = 'refresh'",
            params![token_hash],
            |r| {
                Ok(RefreshRow {
                    grant_id: r.get(0)?,
                    expires_at: r.get(1)?,
                    rotated_at: r.get(2)?,
                    client_id: r.get(3)?,
                    resource: r.get(4)?,
                    scope: r.get(5)?,
                    revoked: r.get(6)?,
                    grant_expires: r.get(7)?,
                })
            },
        )
        .optional()?)
}

/// Spends a refresh token for a new pair.
pub fn refresh(
    conn: &Connection,
    token: &str,
    client_id: &str,
    resource: Option<&str>,
    now: i64,
) -> Result<Refresh> {
    let token_hash = hash(token);
    let Some(row) = refresh_row(conn, &token_hash)? else {
        return Ok(Refresh::Refused);
    };
    if let Some(rotated) = row.rotated_at {
        if now - rotated <= REUSE_GRACE_SECS || row.revoked {
            return Ok(Refresh::Refused);
        }
        revoke_grant(conn, &row.grant_id, "refresh token used twice", now)?;
        return Ok(Refresh::Reused {
            grant_id: row.grant_id,
        });
    }
    let usable = !row.revoked
        && row.expires_at > now
        && row.grant_expires > now
        && row.client_id == client_id
        && resource.is_none_or(|r| r == row.resource);
    if !usable {
        return Ok(Refresh::Refused);
    }
    let tx = conn.unchecked_transaction()?;
    let claimed = tx.execute(
        "UPDATE mcp_oauth_tokens SET rotated_at = ?2 WHERE token_hash = ?1 AND rotated_at IS NULL",
        params![token_hash, now],
    )?;
    if claimed == 0 {
        return Ok(Refresh::Refused);
    }
    let pair = issue_tokens(&tx, &row.grant_id, &row.scope, now)?;
    tx.commit()?;
    Ok(Refresh::Issued(pair))
}

/// The grant an access token was issued under, when the token and grant are live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantAccess {
    pub grant_id: String,
    pub grant_scope: GrantScope,
    pub label: String,
}

/// Resolves a presented access token and records the grant as used.
pub fn grant_for_access_token(
    conn: &Connection,
    token: &str,
    now: i64,
) -> Result<Option<GrantAccess>> {
    if !token.starts_with(ACCESS_PREFIX) {
        return Ok(None);
    }
    let found: Option<(String, String, String)> = conn
        .query_row(
            "SELECT g.id, g.grant_scope, g.label
             FROM mcp_oauth_tokens t JOIN mcp_oauth_grants g ON g.id = t.grant_id
             WHERE t.token_hash = ?1 AND t.kind = 'access' AND t.expires_at > ?2
               AND g.revoked_at IS NULL AND g.expires_at > ?2",
            params![hash(token), now],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((grant_id, scope, label)) = found else {
        return Ok(None);
    };
    let Some(grant_scope) = GrantScope::parse(&scope) else {
        return Ok(None);
    };
    conn.execute(
        "UPDATE mcp_oauth_grants SET last_used_at = ?2 WHERE id = ?1",
        params![grant_id, now],
    )?;
    Ok(Some(GrantAccess {
        grant_id,
        grant_scope,
        label,
    }))
}

/// Revokes the grant a token belongs to (RFC 7009). False when the token is unknown.
pub fn revoke_token(conn: &Connection, token: &str, now: i64) -> Result<bool> {
    let grant_id: Option<String> = conn
        .query_row(
            "SELECT grant_id FROM mcp_oauth_tokens WHERE token_hash = ?1",
            params![hash(token)],
            |r| r.get(0),
        )
        .optional()?;
    match grant_id {
        Some(id) => revoke_grant(conn, &id, "revoked by the client", now),
        None => Ok(false),
    }
}

/// Ends a grant and deletes its tokens. False when it was unknown or already revoked.
pub fn revoke_grant(conn: &Connection, grant_id: &str, reason: &str, now: i64) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE mcp_oauth_grants SET revoked_at = ?2, revoked_reason = ?3
         WHERE id = ?1 AND revoked_at IS NULL",
        params![grant_id, now, reason],
    )?;
    conn.execute(
        "DELETE FROM mcp_oauth_tokens WHERE grant_id = ?1 AND kind = 'access'",
        params![grant_id],
    )?;
    Ok(changed == 1)
}

/// Revokes every live grant and closes the arming window. Returns how many were revoked.
pub fn revoke_all(conn: &Connection, reason: &str, now: i64) -> Result<usize> {
    let ids: Vec<String> = conn
        .prepare("SELECT id FROM mcp_oauth_grants WHERE revoked_at IS NULL")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in &ids {
        revoke_grant(conn, id, reason, now)?;
    }
    disarm(conn)?;
    Ok(ids.len())
}

/// A grant as the owner sees it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Grant {
    pub id: String,
    pub client_id: String,
    pub client_name: String,
    pub label: String,
    pub grant_scope: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub last_used_at: Option<i64>,
    pub revoked_at: Option<i64>,
    pub revoked_reason: Option<String>,
}

/// Grants, newest first. Revoked ones only when asked for.
pub fn list_grants(conn: &Connection, include_revoked: bool) -> Result<Vec<Grant>> {
    let sql = format!(
        "SELECT id, client_id, client_name, label, grant_scope, created_at, expires_at,
                last_used_at, revoked_at, revoked_reason
         FROM mcp_oauth_grants {} ORDER BY created_at DESC, rowid DESC",
        if include_revoked {
            ""
        } else {
            "WHERE revoked_at IS NULL"
        }
    );
    let rows = conn
        .prepare(&sql)?
        .query_map([], |r| {
            Ok(Grant {
                id: r.get(0)?,
                client_id: r.get(1)?,
                client_name: r.get(2)?,
                label: r.get(3)?,
                grant_scope: r.get(4)?,
                created_at: r.get(5)?,
                expires_at: r.get(6)?,
                last_used_at: r.get(7)?,
                revoked_at: r.get(8)?,
                revoked_reason: r.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// Drops requests, codes and tokens that can no longer be used.
pub fn purge_expired(conn: &Connection, now: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM mcp_oauth_requests WHERE expires_at <= ?1",
        params![now],
    )?;
    conn.execute(
        "DELETE FROM mcp_oauth_codes WHERE expires_at <= ?1 - ?2",
        params![now, GRANT_MAX_SECS],
    )?;
    conn.execute(
        "DELETE FROM mcp_oauth_tokens WHERE expires_at <= ?1 - ?2",
        params![now, REUSE_GRACE_SECS],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
