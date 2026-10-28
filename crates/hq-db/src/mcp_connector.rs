//! OAuth state for the claude.ai connector (`docs/plans/claude-connector.md`): the owner's
//! arming window, pending consent requests, single-use codes, grants and tokens. Secrets are
//! returned once and stored only as SHA-256. Callers pass `now` in unix seconds.

use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};
use sha2::{Digest, Sha256};

/// Prefixes that make a leaked secret easy to recognise and scan for.
pub const ACCESS_PREFIX: &str = "hqo_";
pub const REFRESH_PREFIX: &str = "hqr_";
pub const CODE_PREFIX: &str = "hqc_";
/// Owner-created confidential clients and their secrets. A CIMD client id is an https URL, so
/// it can never start with this.
pub const CLIENT_PREFIX: &str = "hqcl_";
pub const CLIENT_SECRET_PREFIX: &str = "hqs_";

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

/// Which consumer a request, code or grant belongs to. Each has its own endpoint and rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    Claude,
    Gemini,
}

impl Profile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Gemini => "gemini",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(Self::Claude),
            "gemini" => Some(Self::Gemini),
            _ => None,
        }
    }

    /// Whether this profile's refresh tokens are spent on use. Google's clients keep one
    /// refresh token for the life of a link, so a rotated one would read as theft.
    fn rotates_refresh_tokens(self) -> bool {
        self == Self::Claude
    }

    /// The scope a grant may have, whatever the owner picked: Gemini is read-only for now.
    fn clamp(self, scope: GrantScope) -> GrantScope {
        match self {
            Self::Claude => scope,
            Self::Gemini => GrantScope::ReadOnly,
        }
    }
}

fn hash(secret: &str) -> String {
    Sha256::digest(secret.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A write transaction taken up front, so a read inside it cannot be overtaken by another
/// writer before this one decides (the code, approval and kill-switch paths).
fn immediate(conn: &Connection) -> Result<Transaction<'_>> {
    Ok(Transaction::new_unchecked(
        conn,
        TransactionBehavior::Immediate,
    )?)
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

/// The owner closing the window: nothing new can start and no pending request can be approved.
pub fn close_window(conn: &Connection) -> Result<()> {
    let tx = immediate(conn)?;
    disarm(&tx)?;
    tx.execute("DELETE FROM mcp_oauth_requests", [])?;
    tx.commit()?;
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
    pub profile: Profile,
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
    pub created_at: i64,
    pub expires_at: i64,
    /// The arming window it arrived in; a request from an earlier window cannot be approved.
    pub armed_until: i64,
    pub profile: Profile,
}

const REQUEST_COLS: &str = "id, client_id, client_name, redirect_uri, state, code_challenge, \
                            resource, scope, created_at, expires_at, armed_until, profile";

/// Requests waiting at once, so an open window cannot be flooded.
pub const MAX_PENDING_REQUESTS: i64 = 20;

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
        created_at: r.get(8)?,
        expires_at: r.get(9)?,
        armed_until: r.get(10)?,
        profile: profile_column(r, 11)?,
    })
}

/// A profile column; an unknown value is a corrupt row, not a Claude one.
fn profile_column(r: &Row, idx: usize) -> rusqlite::Result<Profile> {
    let raw: String = r.get(idx)?;
    Profile::parse(&raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            idx,
            rusqlite::types::Type::Text,
            format!("unknown connector profile {raw:?}").into(),
        )
    })
}

/// How storing a request ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pending {
    /// Stored under this id, for the consent page.
    Created(String),
    NotArmed,
    /// Too many requests are already waiting.
    Full,
}

/// Stores a request for the consent page. `binder` is a secret held by the browser that
/// started it (an HttpOnly cookie), so only that browser can see or decide the request.
pub fn create_request(
    conn: &Connection,
    req: &NewRequest,
    binder: &str,
    now: i64,
) -> Result<Pending> {
    let tx = immediate(conn)?;
    purge_expired(&tx, now)?;
    let Some(armed) = armed_until(&tx, now)? else {
        return Ok(Pending::NotArmed);
    };
    let waiting: i64 = tx.query_row("SELECT COUNT(*) FROM mcp_oauth_requests", [], |r| r.get(0))?;
    if waiting >= MAX_PENDING_REQUESTS {
        return Ok(Pending::Full);
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    tx.execute(
        &format!(
            "INSERT INTO mcp_oauth_requests ({REQUEST_COLS}, binder_hash)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)"
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
            now,
            now + REQUEST_TTL_SECS,
            armed,
            req.profile.as_str(),
            hash(binder)
        ],
    )?;
    tx.commit()?;
    Ok(Pending::Created(id))
}

/// A live request, for the browser holding its binder only.
pub fn get_request(
    conn: &Connection,
    id: &str,
    binder: &str,
    now: i64,
) -> Result<Option<AuthRequest>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {REQUEST_COLS} FROM mcp_oauth_requests
                 WHERE id = ?1 AND binder_hash = ?2 AND expires_at > ?3"
            ),
            params![id, hash(binder), now],
            request_from_row,
        )
        .optional()?)
}

/// Removes a request and returns it if it was still valid, so a decision is made once.
fn take_request(
    conn: &Connection,
    id: &str,
    binder: &str,
    now: i64,
) -> Result<Option<AuthRequest>> {
    let req = get_request(conn, id, binder, now)?;
    if req.is_none() {
        return Ok(None);
    }
    let removed = conn.execute("DELETE FROM mcp_oauth_requests WHERE id = ?1", params![id])?;
    Ok(req.filter(|_| removed == 1))
}

/// The owner's refusal: the request is gone and the client is told so.
pub fn deny_request(
    conn: &Connection,
    id: &str,
    binder: &str,
    now: i64,
) -> Result<Option<AuthRequest>> {
    take_request(conn, id, binder, now)
}

/// How an owner's approval ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Approval {
    /// The code for the client, and the request it answers.
    Issued {
        request: Box<AuthRequest>,
        code: String,
    },
    /// The arming window had closed.
    NotArmed,
    /// The request expired, was already answered, belongs to another browser or to an
    /// earlier window.
    Gone,
}

/// The owner's approval of a pending request. One approval per arming: the window closes in
/// the same transaction, so two approvals at once cannot both get in.
pub fn approve_request(
    conn: &Connection,
    id: &str,
    binder: &str,
    grant_scope: GrantScope,
    label: &str,
    now: i64,
) -> Result<Approval> {
    let tx = immediate(conn)?;
    let Some(armed) = armed_until(&tx, now)? else {
        return Ok(Approval::NotArmed);
    };
    let Some(request) = take_request(&tx, id, binder, now)? else {
        return Ok(Approval::Gone);
    };
    if request.armed_until != armed {
        tx.commit()?;
        return Ok(Approval::Gone);
    }
    let grant_scope = request.profile.clamp(grant_scope);
    let code = issue_code(&tx, &request, grant_scope, label, now)?;
    disarm(&tx)?;
    tx.commit()?;
    Ok(Approval::Issued {
        request: Box::new(request),
        code,
    })
}

/// A single-use code for an approved request.
fn issue_code(
    conn: &Connection,
    req: &AuthRequest,
    grant_scope: GrantScope,
    label: &str,
    now: i64,
) -> Result<String> {
    let code = random_secret(CODE_PREFIX);
    conn.execute(
        "INSERT INTO mcp_oauth_codes (code_hash, client_id, client_name, redirect_uri,
            code_challenge, resource, scope, grant_scope, label, expires_at, profile)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
            now + CODE_TTL_SECS,
            req.profile.as_str()
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
    profile: String,
}

fn code_row(conn: &Connection, code_hash: &str) -> Result<Option<CodeRow>> {
    Ok(conn
        .query_row(
            "SELECT client_id, client_name, redirect_uri, code_challenge, resource, scope,
                    grant_scope, label, expires_at, grant_id, profile
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
                    profile: r.get(10)?,
                })
            },
        )
        .optional()?)
}

/// Exchanges a code for tokens, or `None` for anything wrong with it. The code is spent by
/// the first attempt, right or wrong; a second use revokes the grant the first one made. One
/// transaction covers it all, so a failure leaves the code unspent and two uses never race.
pub fn redeem_code(conn: &Connection, ex: &CodeExchange, now: i64) -> Result<Option<TokenPair>> {
    let code_hash = hash(ex.code);
    // An unknown code is refused on a plain read, so junk from the public token endpoint never
    // takes the database's write lock.
    if code_row(conn, &code_hash)?.is_none() {
        return Ok(None);
    }
    let tx = immediate(conn)?;
    let Some(row) = code_row(&tx, &code_hash)? else {
        return Ok(None);
    };
    let claimed = tx.execute(
        "UPDATE mcp_oauth_codes SET used = 1 WHERE code_hash = ?1 AND used = 0",
        params![code_hash],
    )?;
    if claimed == 0 {
        if let Some(grant_id) = &row.grant_id {
            revoke_grant(&tx, grant_id, "authorization code used twice", now)?;
        }
        tx.commit()?;
        return Ok(None);
    }
    let matches = row.expires_at > now
        && row.client_id == ex.client_id
        && row.redirect_uri == ex.redirect_uri
        && row.code_challenge == ex.challenge
        && ex.resource.is_none_or(|r| r == row.resource);
    if !matches {
        tx.commit()?;
        return Ok(None);
    }
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
    if Profile::parse(&row.profile).is_none() {
        bail!("unknown connector profile {:?}", row.profile);
    }
    let id = format!("grant-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    conn.execute(
        "INSERT INTO mcp_oauth_grants (id, client_id, client_name, label, grant_scope, scope,
            resource, created_at, expires_at, profile)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            row.client_id,
            row.client_name,
            row.label,
            row.grant_scope,
            row.scope,
            row.resource,
            now,
            now + GRANT_MAX_SECS,
            row.profile
        ],
    )?;
    Ok(id)
}

fn grant_expires_at(conn: &Connection, grant_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT expires_at FROM mcp_oauth_grants WHERE id = ?1",
        params![grant_id],
        |r| r.get(0),
    )?)
}

const INSERT_TOKEN: &str = "INSERT INTO mcp_oauth_tokens (token_hash, grant_id, kind, expires_at)
                            VALUES (?1, ?2, ?3, ?4)";

/// A new access token for the grant, and when it expires.
fn issue_access(
    conn: &Connection,
    grant_id: &str,
    grant_expires: i64,
    now: i64,
) -> Result<(String, i64)> {
    let access = random_secret(ACCESS_PREFIX);
    let access_expires = (now + ACCESS_TTL_SECS).min(grant_expires);
    conn.execute(
        INSERT_TOKEN,
        params![hash(&access), grant_id, "access", access_expires],
    )?;
    Ok((access, access_expires))
}

/// A refresh for a profile that keeps its refresh token: a new access token, the same refresh
/// token with its expiry pushed out, never past the grant's end.
fn renew_access(
    conn: &Connection,
    grant_id: &str,
    refresh_token: &str,
    scope: &str,
    now: i64,
) -> Result<TokenPair> {
    let grant_expires = grant_expires_at(conn, grant_id)?;
    let (access, access_expires) = issue_access(conn, grant_id, grant_expires, now)?;
    conn.execute(
        "UPDATE mcp_oauth_tokens SET expires_at = ?2 WHERE token_hash = ?1 AND kind = 'refresh'",
        params![
            hash(refresh_token),
            (now + REFRESH_TTL_SECS).min(grant_expires)
        ],
    )?;
    Ok(TokenPair {
        access_token: access,
        refresh_token: refresh_token.to_string(),
        expires_in: access_expires - now,
        scope: scope.to_string(),
        grant_id: grant_id.to_string(),
    })
}

fn issue_tokens(conn: &Connection, grant_id: &str, scope: &str, now: i64) -> Result<TokenPair> {
    let grant_expires = grant_expires_at(conn, grant_id)?;
    let (access, access_expires) = issue_access(conn, grant_id, grant_expires, now)?;
    let refresh = random_secret(REFRESH_PREFIX);
    let insert = INSERT_TOKEN;
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
    profile: String,
}

fn refresh_row(conn: &Connection, token_hash: &str) -> Result<Option<RefreshRow>> {
    Ok(conn
        .query_row(
            "SELECT t.grant_id, t.expires_at, t.rotated_at, g.client_id, g.resource, g.scope,
                    g.revoked_at IS NOT NULL, g.expires_at, g.profile
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
                    profile: r.get(8)?,
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
    let Some(profile) = Profile::parse(&row.profile).filter(|_| usable) else {
        return Ok(Refresh::Refused);
    };
    if !profile.rotates_refresh_tokens() {
        let tx = conn.unchecked_transaction()?;
        let pair = renew_access(&tx, &row.grant_id, token, &row.scope, now)?;
        tx.commit()?;
        return Ok(Refresh::Issued(pair));
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
    pub profile: Profile,
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
    let found: Option<(String, String, String, String)> = conn
        .query_row(
            "SELECT g.id, g.grant_scope, g.label, g.profile
             FROM mcp_oauth_tokens t JOIN mcp_oauth_grants g ON g.id = t.grant_id
             WHERE t.token_hash = ?1 AND t.kind = 'access' AND t.expires_at > ?2
               AND g.revoked_at IS NULL AND g.expires_at > ?2",
            params![hash(token), now],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((grant_id, scope, label, profile)) = found else {
        return Ok(None);
    };
    let (Some(grant_scope), Some(profile)) = (GrantScope::parse(&scope), Profile::parse(&profile))
    else {
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
        profile,
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

/// The kill switch: revokes every live grant, cancels approved codes not yet exchanged and
/// pending requests, and closes the window, in one transaction so an exchange in flight either
/// finishes first (and is revoked) or finds its code gone. Returns how many grants it revoked.
pub fn revoke_all(conn: &Connection, reason: &str, now: i64) -> Result<usize> {
    let tx = immediate(conn)?;
    let ids: Vec<String> = tx
        .prepare("SELECT id FROM mcp_oauth_grants WHERE revoked_at IS NULL AND expires_at > ?1")?
        .query_map(params![now], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in &ids {
        revoke_grant(&tx, id, reason, now)?;
    }
    tx.execute("DELETE FROM mcp_oauth_codes WHERE used = 0", [])?;
    tx.execute("DELETE FROM mcp_oauth_requests", [])?;
    disarm(&tx)?;
    tx.commit()?;
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
    pub profile: String,
}

/// Grants, newest first: the live ones, or every grant ever made when `include_ended`.
pub fn list_grants(conn: &Connection, include_ended: bool, now: i64) -> Result<Vec<Grant>> {
    let sql = format!(
        "SELECT id, client_id, client_name, label, grant_scope, created_at, expires_at,
                last_used_at, revoked_at, revoked_reason, profile
         FROM mcp_oauth_grants {} ORDER BY created_at DESC, rowid DESC",
        if include_ended {
            "WHERE ?1 IS NOT NULL"
        } else {
            "WHERE revoked_at IS NULL AND expires_at > ?1"
        }
    );
    let rows = conn
        .prepare(&sql)?
        .query_map(params![now], |r| {
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
                profile: r.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// An owner-created confidential client as the owner sees it. The secret is never kept.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Client {
    pub id: String,
    pub label: String,
    pub profile: String,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
}

/// A new client's id and secret. The secret exists only here: show it once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewClient {
    pub id: String,
    pub secret: String,
}

/// Creates a confidential client for `profile`. Only Gemini uses them so far.
pub fn create_client(
    conn: &Connection,
    label: &str,
    profile: Profile,
    now: i64,
) -> Result<NewClient> {
    if profile != Profile::Gemini {
        bail!("only the gemini profile uses registered clients");
    }
    let id = format!("{CLIENT_PREFIX}{}", uuid::Uuid::new_v4().simple());
    let secret = random_secret(CLIENT_SECRET_PREFIX);
    conn.execute(
        "INSERT INTO mcp_oauth_clients (id, label, profile, secret_hash, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, label, profile.as_str(), hash(&secret), now],
    )?;
    Ok(NewClient { id, secret })
}

/// The live client `id` names, without checking a secret: for the authorization request,
/// where the client is identified but not yet authenticated.
pub fn live_client(conn: &Connection, id: &str) -> Result<Option<Client>> {
    Ok(conn
        .query_row(
            "SELECT id, label, profile, created_at, revoked_at FROM mcp_oauth_clients
             WHERE id = ?1 AND revoked_at IS NULL",
            params![id],
            client_from_row,
        )
        .optional()?)
}

fn client_from_row(r: &Row) -> rusqlite::Result<Client> {
    Ok(Client {
        id: r.get(0)?,
        label: r.get(1)?,
        profile: r.get(2)?,
        created_at: r.get(3)?,
        revoked_at: r.get(4)?,
    })
}

/// Whether `secret` is the live client `id`'s. Both sides are SHA-256 digests compared in
/// constant time, so the comparison says nothing about how close a guess was.
pub fn authenticate_client(conn: &Connection, id: &str, secret: &str) -> Result<bool> {
    let stored: Option<String> = conn
        .query_row(
            "SELECT secret_hash FROM mcp_oauth_clients WHERE id = ?1 AND revoked_at IS NULL",
            params![id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(stored) = stored else {
        return Ok(false);
    };
    let presented = hash(secret);
    let same = stored.len() == presented.len()
        && stored
            .bytes()
            .zip(presented.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0;
    Ok(same)
}

/// Clients, newest first, revoked ones included.
pub fn list_clients(conn: &Connection) -> Result<Vec<Client>> {
    let rows = conn
        .prepare(
            "SELECT id, label, profile, created_at, revoked_at FROM mcp_oauth_clients
             ORDER BY created_at DESC, rowid DESC",
        )?
        .query_map([], client_from_row)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// Revokes a client and every grant it holds, in one transaction. False when it was unknown or
/// already revoked.
pub fn revoke_client(conn: &Connection, id: &str, now: i64) -> Result<bool> {
    let tx = immediate(conn)?;
    let changed = tx.execute(
        "UPDATE mcp_oauth_clients SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
        params![id, now],
    )?;
    let grants: Vec<String> = tx
        .prepare("SELECT id FROM mcp_oauth_grants WHERE client_id = ?1 AND revoked_at IS NULL")?
        .query_map(params![id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for grant in &grants {
        revoke_grant(&tx, grant, "client revoked by the owner", now)?;
    }
    tx.execute(
        "DELETE FROM mcp_oauth_codes WHERE client_id = ?1 AND used = 0",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM mcp_oauth_requests WHERE client_id = ?1",
        params![id],
    )?;
    tx.commit()?;
    Ok(changed == 1)
}

/// How long the call log is kept.
pub const AUDIT_RETENTION_SECS: i64 = 30 * 24 * 60 * 60;

/// One logged call.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AuditEntry {
    pub grant_id: String,
    pub tool: String,
    pub outcome: String,
    pub args_sha256: String,
    pub at: i64,
}

/// Logs a call: the tool, how it ended, and a hash of its arguments, never the arguments.
/// Each write also drops what has aged out, so the log keeps 30 days without a separate task.
pub fn record_call(
    conn: &Connection,
    grant_id: &str,
    tool: &str,
    outcome: &str,
    args_sha256: &str,
    now: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO mcp_connector_audit (grant_id, tool, outcome, args_sha256, at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![grant_id, tool, outcome, args_sha256, now],
    )?;
    purge_expired(conn, now)
}

/// Calls a grant made since `since`, for its rate limit.
pub fn calls_since(conn: &Connection, grant_id: &str, since: i64) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM mcp_connector_audit WHERE grant_id = ?1 AND at >= ?2",
        params![grant_id, since],
        |r| r.get(0),
    )?)
}

/// The newest logged calls, for one grant or all of them.
pub fn recent_calls(
    conn: &Connection,
    grant_id: Option<&str>,
    limit: i64,
) -> Result<Vec<AuditEntry>> {
    let rows = conn
        .prepare(
            "SELECT grant_id, tool, outcome, args_sha256, at FROM mcp_connector_audit
             WHERE ?1 IS NULL OR grant_id = ?1 ORDER BY at DESC, id DESC LIMIT ?2",
        )?
        .query_map(params![grant_id, limit], |r| {
            Ok(AuditEntry {
                grant_id: r.get(0)?,
                tool: r.get(1)?,
                outcome: r.get(2)?,
                args_sha256: r.get(3)?,
                at: r.get(4)?,
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
    conn.execute(
        "DELETE FROM mcp_connector_audit WHERE at <= ?1 - ?2",
        params![now, AUDIT_RETENTION_SECS],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
