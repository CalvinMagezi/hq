use super::*;
use crate::Database;

const NOW: i64 = 1_800_000_000;
const CLIENT: &str = "https://claude.ai/oauth/mcp-oauth-client-metadata";
const REDIRECT: &str = "https://claude.ai/api/mcp/auth_callback";
const RESOURCE: &str = "https://hq.example.com/mcp";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
const BINDER: &str = "browser-cookie";

fn new_request() -> NewRequest<'static> {
    NewRequest {
        client_id: CLIENT,
        client_name: "Claude",
        redirect_uri: REDIRECT,
        state: Some("xyz"),
        code_challenge: CHALLENGE,
        resource: RESOURCE,
        scope: "hq offline_access",
        profile: Profile::Claude,
    }
}

fn exchange(code: &str) -> CodeExchange<'_> {
    CodeExchange {
        code,
        client_id: CLIENT,
        redirect_uri: REDIRECT,
        challenge: CHALLENGE,
        resource: Some(RESOURCE),
    }
}

/// Stores a request while armed, returning its id.
fn pending(c: &Connection) -> String {
    match create_request(c, &new_request(), BINDER, NOW).unwrap() {
        Pending::Created(id) => id,
        other => panic!("not stored: {other:?}"),
    }
}

/// A request approved with `scope`, returning its code.
fn approved(c: &Connection, scope: GrantScope) -> String {
    arm(c, NOW + 600).unwrap();
    let id = pending(c);
    match approve_request(c, &id, BINDER, scope, "Claude", NOW).unwrap() {
        Approval::Issued { code, .. } => code,
        other => panic!("not approved: {other:?}"),
    }
}

fn connected(c: &Connection) -> TokenPair {
    let code = approved(c, GrantScope::Full);
    redeem_code(c, &exchange(&code), NOW).unwrap().unwrap()
}

#[test]
fn the_arming_window_opens_closes_and_expires() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        assert_eq!(armed_until(c, NOW)?, None);
        arm(c, NOW + 600)?;
        assert_eq!(armed_until(c, NOW)?, Some(NOW + 600));
        assert_eq!(armed_until(c, NOW + 600)?, None, "closed at the deadline");
        arm(c, NOW + 60)?;
        disarm(c)?;
        assert_eq!(armed_until(c, NOW)?, None);
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_request_needs_the_window_is_decided_once_and_expires() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        assert_eq!(
            create_request(c, &new_request(), BINDER, NOW)?,
            Pending::NotArmed
        );
        arm(c, NOW + 600)?;
        let id = pending(c);
        let req = get_request(c, &id, BINDER, NOW)?.unwrap();
        assert_eq!(req.state.as_deref(), Some("xyz"));
        assert_eq!((req.created_at, req.armed_until), (NOW, NOW + 600));
        assert!(deny_request(c, &id, BINDER, NOW)?.is_some());
        assert!(deny_request(c, &id, BINDER, NOW)?.is_none(), "taken twice");

        let late = pending(c);
        assert!(get_request(c, &late, BINDER, NOW + REQUEST_TTL_SECS)?.is_none());
        Ok(())
    })
    .unwrap();
}

#[test]
fn only_the_browser_that_started_a_request_can_see_or_decide_it() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        arm(c, NOW + 600)?;
        let id = pending(c);
        assert!(get_request(c, &id, "another-browser", NOW)?.is_none());
        assert!(deny_request(c, &id, "another-browser", NOW)?.is_none());
        assert_eq!(
            approve_request(c, &id, "another-browser", GrantScope::Full, "x", NOW)?,
            Approval::Gone
        );
        assert!(
            get_request(c, &id, BINDER, NOW)?.is_some(),
            "still there for its owner"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_request_from_an_earlier_window_cannot_be_approved() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        arm(c, NOW + 600)?;
        let stale = pending(c);
        close_window(c)?;
        assert!(
            get_request(c, &stale, BINDER, NOW)?.is_none(),
            "closing drops pending requests"
        );
        arm(c, NOW + 600)?;
        let id = pending(c);
        arm(c, NOW + 900)?;
        assert_eq!(
            approve_request(c, &id, BINDER, GrantScope::Full, "x", NOW)?,
            Approval::Gone,
            "re-arming starts a new window"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn an_open_window_takes_a_bounded_number_of_requests() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        arm(c, NOW + 600)?;
        for _ in 0..MAX_PENDING_REQUESTS {
            pending(c);
        }
        assert_eq!(
            create_request(c, &new_request(), BINDER, NOW)?,
            Pending::Full
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_code_becomes_tokens_for_the_grant_scope_chosen() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        let code = approved(c, GrantScope::Tasks);
        assert!(code.starts_with(CODE_PREFIX));
        let pair = redeem_code(c, &exchange(&code), NOW)?.unwrap();
        assert!(pair.access_token.starts_with(ACCESS_PREFIX));
        assert!(pair.refresh_token.starts_with(REFRESH_PREFIX));
        assert_eq!(pair.expires_in, ACCESS_TTL_SECS);
        assert_eq!(pair.scope, "hq offline_access");
        let access = grant_for_access_token(c, &pair.access_token, NOW)?.unwrap();
        assert_eq!(access.grant_scope, GrantScope::Tasks);
        assert_eq!(access.grant_id, pair.grant_id);
        assert!(grant_for_access_token(c, &pair.refresh_token, NOW)?.is_none());
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_code_is_refused_on_any_mismatch_and_spent_by_the_attempt() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        let mismatches: [fn(CodeExchange) -> CodeExchange; 4] = [
            |e| CodeExchange {
                client_id: "https://evil.example/cimd",
                ..e
            },
            |e| CodeExchange {
                redirect_uri: "https://evil.example/cb",
                ..e
            },
            |e| CodeExchange {
                challenge: "wrong",
                ..e
            },
            |e| CodeExchange {
                resource: Some("https://other.example.com/mcp"),
                ..e
            },
        ];
        for bend in mismatches {
            let code = approved(c, GrantScope::Full);
            assert!(redeem_code(c, &bend(exchange(&code)), NOW)?.is_none());
            assert!(
                redeem_code(c, &exchange(&code), NOW)?.is_none(),
                "a failed attempt spends the code"
            );
        }
        let code = approved(c, GrantScope::Full);
        let no_resource = CodeExchange {
            resource: None,
            ..exchange(&code)
        };
        assert!(redeem_code(c, &no_resource, NOW)?.is_some());
        let late = approved(c, GrantScope::Full);
        assert!(redeem_code(c, &exchange(&late), NOW + CODE_TTL_SECS)?.is_none());
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_reused_code_revokes_the_grant_it_produced() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        let code = approved(c, GrantScope::Full);
        let pair = redeem_code(c, &exchange(&code), NOW)?.unwrap();
        assert!(redeem_code(c, &exchange(&code), NOW + 5)?.is_none());
        assert!(grant_for_access_token(c, &pair.access_token, NOW + 5)?.is_none());
        let grants = list_grants(c, true, NOW)?;
        assert_eq!(
            grants[0].revoked_reason.as_deref(),
            Some("authorization code used twice")
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn refresh_rotates_and_the_old_token_dies() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        let first = connected(c);
        let Refresh::Issued(second) =
            refresh(c, &first.refresh_token, CLIENT, Some(RESOURCE), NOW + 10)?
        else {
            panic!("refresh refused");
        };
        assert_ne!(second.refresh_token, first.refresh_token);
        assert_eq!(second.grant_id, first.grant_id);
        assert!(grant_for_access_token(c, &second.access_token, NOW + 10)?.is_some());
        assert_eq!(
            refresh(c, &first.refresh_token, CLIENT, None, NOW + 20)?,
            Refresh::Refused,
            "a retry inside the grace is refused but harmless"
        );
        assert!(grant_for_access_token(c, &second.access_token, NOW + 20)?.is_some());
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_refresh_token_replayed_after_the_grace_revokes_the_grant() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        let first = connected(c);
        let Refresh::Issued(second) = refresh(c, &first.refresh_token, CLIENT, None, NOW)? else {
            panic!("refresh refused");
        };
        let later = NOW + REUSE_GRACE_SECS + 1;
        assert_eq!(
            refresh(c, &first.refresh_token, CLIENT, None, later)?,
            Refresh::Reused {
                grant_id: first.grant_id.clone()
            }
        );
        assert!(grant_for_access_token(c, &second.access_token, later)?.is_none());
        assert_eq!(
            refresh(c, &second.refresh_token, CLIENT, None, later)?,
            Refresh::Refused
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn refresh_is_refused_for_the_wrong_client_resource_or_an_expired_grant() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        let pair = connected(c);
        let rt = &pair.refresh_token;
        assert_eq!(
            refresh(c, rt, "https://evil.example/c", None, NOW)?,
            Refresh::Refused
        );
        assert_eq!(
            refresh(c, rt, CLIENT, Some("https://other.example.com/mcp"), NOW)?,
            Refresh::Refused
        );
        assert_eq!(
            refresh(c, rt, CLIENT, None, NOW + GRANT_MAX_SECS)?,
            Refresh::Refused
        );
        assert_eq!(
            refresh(c, "hqr_unknown", CLIENT, None, NOW)?,
            Refresh::Refused
        );
        assert!(matches!(
            refresh(c, rt, CLIENT, None, NOW)?,
            Refresh::Issued(_)
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn access_tokens_expire_and_die_with_their_grant() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        let pair = connected(c);
        assert!(grant_for_access_token(c, &pair.access_token, NOW + ACCESS_TTL_SECS)?.is_none());
        let other = connected(c);
        assert!(revoke_grant(c, &other.grant_id, "owner", NOW)?);
        assert!(
            !revoke_grant(c, &other.grant_id, "owner", NOW)?,
            "already revoked"
        );
        assert!(grant_for_access_token(c, &other.access_token, NOW)?.is_none());
        assert_eq!(
            refresh(c, &other.refresh_token, CLIENT, None, NOW)?,
            Refresh::Refused
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn revoking_by_token_or_all_at_once_ends_grants_and_disarms() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        let a = connected(c);
        assert!(revoke_token(c, &a.refresh_token, NOW)?);
        assert!(!revoke_token(c, "hqo_unknown", NOW)?);
        assert!(grant_for_access_token(c, &a.access_token, NOW)?.is_none());

        let (b, d) = (connected(c), connected(c));
        let unexchanged = approved(c, GrantScope::Full);
        arm(c, NOW + 600)?;
        let waiting = pending(c);
        assert_eq!(revoke_all(c, "kill switch", NOW)?, 2);
        assert!(
            redeem_code(c, &exchange(&unexchanged), NOW)?.is_none(),
            "an approved code not yet exchanged dies with the switch"
        );
        assert!(get_request(c, &waiting, BINDER, NOW)?.is_none());
        assert_eq!(armed_until(c, NOW)?, None);
        for pair in [b, d] {
            assert!(grant_for_access_token(c, &pair.access_token, NOW)?.is_none());
        }
        assert!(list_grants(c, false, NOW)?.is_empty());
        assert_eq!(list_grants(c, true, NOW)?.len(), 3);
        Ok(())
    })
    .unwrap();
}

#[test]
fn an_approval_needs_the_window_and_closes_it() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        arm(c, NOW + 600)?;
        let (first, second) = (pending(c), pending(c));
        disarm(c)?;
        assert_eq!(
            approve_request(c, &first, BINDER, GrantScope::Full, "x", NOW)?,
            Approval::NotArmed
        );
        arm(c, NOW + 600)?;
        assert!(matches!(
            approve_request(c, &first, BINDER, GrantScope::Full, "x", NOW)?,
            Approval::Issued { .. }
        ));
        assert_eq!(
            approve_request(c, &second, BINDER, GrantScope::Full, "x", NOW)?,
            Approval::NotArmed,
            "one arming admits one client"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn an_expired_grant_is_not_listed_as_live() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        connected(c);
        let later = NOW + GRANT_MAX_SECS;
        assert_eq!(list_grants(c, false, NOW)?.len(), 1);
        assert!(list_grants(c, false, later)?.is_empty());
        assert_eq!(list_grants(c, true, later)?.len(), 1);
        assert_eq!(revoke_all(c, "off", later)?, 0);
        Ok(())
    })
    .unwrap();
}

#[test]
fn only_hashes_are_stored() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        let pair = connected(c);
        let stored: Vec<String> = c
            .prepare("SELECT token_hash FROM mcp_oauth_tokens")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        assert_eq!(stored.len(), 2);
        for secret in [&pair.access_token, &pair.refresh_token] {
            assert!(!stored.contains(secret));
            assert!(stored.contains(&hash(secret)));
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn the_call_log_counts_per_grant_and_lists_newest_first() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        record_call(c, "grant-a", "vault_search", "ok", "h1", NOW)?;
        record_call(c, "grant-a", "task_create", "ok", "h2", NOW + 30)?;
        record_call(c, "grant-b", "vault_read", "refused", "h3", NOW + 40)?;
        assert_eq!(calls_since(c, "grant-a", NOW)?, 2);
        assert_eq!(calls_since(c, "grant-a", NOW + 1)?, 1);
        let all = recent_calls(c, None, 10)?;
        assert_eq!(all[0].tool, "vault_read");
        let mine = recent_calls(c, Some("grant-a"), 10)?;
        assert_eq!(mine.len(), 2);
        assert_eq!(mine[0].tool, "task_create");
        record_call(
            c,
            "grant-b",
            "vault_read",
            "ok",
            "h4",
            NOW + 30 + AUDIT_RETENTION_SECS,
        )?;
        let left = recent_calls(c, None, 10)?;
        assert_eq!(left.len(), 2, "a write drops what aged out: {left:?}");
        Ok(())
    })
    .unwrap();
}

const GEMINI_REDIRECT: &str = "https://oauth-redirect.googleusercontent.com/r/hq-test";
const GEMINI_RESOURCE: &str = "https://hq.example.com/mcp/gemini/v1";

/// A Gemini client, approved and exchanged with `scope` asked for on the consent page.
fn gemini_connected(c: &Connection, scope: GrantScope) -> (NewClient, TokenPair) {
    let client = create_client(c, "Gemini test", Profile::Gemini, NOW).unwrap();
    arm(c, NOW + 600).unwrap();
    let req = NewRequest {
        client_id: &client.id,
        client_name: "Gemini",
        redirect_uri: GEMINI_REDIRECT,
        state: None,
        code_challenge: CHALLENGE,
        resource: GEMINI_RESOURCE,
        scope: "hq",
        profile: Profile::Gemini,
    };
    let Pending::Created(id) = create_request(c, &req, BINDER, NOW).unwrap() else {
        panic!("not stored");
    };
    let Approval::Issued { code, .. } =
        approve_request(c, &id, BINDER, scope, "Gemini", NOW).unwrap()
    else {
        panic!("not approved");
    };
    let ex = CodeExchange {
        code: &code,
        client_id: &client.id,
        redirect_uri: GEMINI_REDIRECT,
        challenge: CHALLENGE,
        resource: Some(GEMINI_RESOURCE),
    };
    let pair = redeem_code(c, &ex, NOW).unwrap().unwrap();
    (client, pair)
}

#[test]
fn a_gemini_grant_is_read_only_whatever_the_owner_picked_and_names_its_profile() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        let (_, pair) = gemini_connected(c, GrantScope::Full);
        let access = grant_for_access_token(c, &pair.access_token, NOW)?.unwrap();
        assert_eq!(access.grant_scope, GrantScope::ReadOnly);
        assert_eq!(access.profile, Profile::Gemini);
        let claude = connected(c);
        let access = grant_for_access_token(c, &claude.access_token, NOW)?.unwrap();
        assert_eq!(
            access.grant_scope,
            GrantScope::Full,
            "Claude keeps the owner's pick"
        );
        assert_eq!(access.profile, Profile::Claude);
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_gemini_refresh_keeps_its_token_and_reuse_never_revokes_the_grant() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        let (client, pair) = gemini_connected(c, GrantScope::ReadOnly);
        let later = NOW + REUSE_GRACE_SECS * 10;
        for at in [later, later + REUSE_GRACE_SECS * 10] {
            let Refresh::Issued(next) = refresh(
                c,
                &pair.refresh_token,
                &client.id,
                Some(GEMINI_RESOURCE),
                at,
            )?
            else {
                panic!("refused at {at}");
            };
            assert_eq!(next.refresh_token, pair.refresh_token);
            assert_ne!(next.access_token, pair.access_token);
            assert!(grant_for_access_token(c, &next.access_token, at)?.is_some());
        }
        assert_eq!(
            refresh(c, &pair.refresh_token, CLIENT, None, later)?,
            Refresh::Refused,
            "another client id"
        );
        let end = NOW + GRANT_MAX_SECS;
        assert_eq!(
            refresh(c, &pair.refresh_token, &client.id, None, end)?,
            Refresh::Refused,
            "never past the grant's end"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn claude_refresh_tokens_still_rotate() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        let pair = connected(c);
        let Refresh::Issued(next) = refresh(c, &pair.refresh_token, CLIENT, None, NOW)? else {
            panic!("refused");
        };
        assert_ne!(next.refresh_token, pair.refresh_token);
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_client_secret_authenticates_only_its_live_client() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        let a = create_client(c, "a", Profile::Gemini, NOW)?;
        let b = create_client(c, "b", Profile::Gemini, NOW)?;
        assert!(a.id.starts_with(CLIENT_PREFIX) && a.secret.starts_with(CLIENT_SECRET_PREFIX));
        assert!(authenticate_client(c, &a.id, &a.secret)?);
        assert!(!authenticate_client(c, &a.id, &b.secret)?);
        assert!(!authenticate_client(c, &a.id, "")?);
        assert!(!authenticate_client(c, "hqcl_unknown", &a.secret)?);
        assert!(create_client(c, "x", Profile::Claude, NOW).is_err());
        let stored: String = c.query_row(
            "SELECT secret_hash FROM mcp_oauth_clients WHERE id = ?1",
            params![a.id],
            |r| r.get(0),
        )?;
        assert!(!stored.contains(&a.secret), "only the digest is kept");
        Ok(())
    })
    .unwrap();
}

#[test]
fn revoking_a_client_ends_its_grants_and_its_secret_only() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        let (client, pair) = gemini_connected(c, GrantScope::ReadOnly);
        let claude = connected(c);
        assert!(revoke_client(c, &client.id, NOW)?);
        assert!(!revoke_client(c, &client.id, NOW)?, "already revoked");
        assert!(!authenticate_client(c, &client.id, &client.secret)?);
        assert!(live_client(c, &client.id)?.is_none());
        assert!(grant_for_access_token(c, &pair.access_token, NOW)?.is_none());
        assert_eq!(
            refresh(c, &pair.refresh_token, &client.id, None, NOW)?,
            Refresh::Refused
        );
        assert!(grant_for_access_token(c, &claude.access_token, NOW)?.is_some());
        assert_eq!(list_clients(c)?.len(), 1);
        Ok(())
    })
    .unwrap();
}

#[test]
fn rows_from_before_profiles_read_as_claude() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO mcp_oauth_grants (id, client_id, client_name, label, grant_scope, scope,
                resource, created_at, expires_at)
             VALUES ('grant-old', ?1, 'Claude', 'Claude', 'read_only', 'hq', ?2, ?3, ?4)",
            params![CLIENT, RESOURCE, NOW, NOW + GRANT_MAX_SECS],
        )?;
        let grants = list_grants(c, false, NOW)?;
        assert_eq!(grants[0].profile, "claude");
        Ok(())
    })
    .unwrap();
}
