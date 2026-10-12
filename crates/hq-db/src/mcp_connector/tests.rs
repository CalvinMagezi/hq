use super::*;
use crate::Database;

const NOW: i64 = 1_800_000_000;
const CLIENT: &str = "https://claude.ai/oauth/mcp-oauth-client-metadata";
const REDIRECT: &str = "https://claude.ai/api/mcp/auth_callback";
const RESOURCE: &str = "https://hq.example.com/mcp";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

fn new_request() -> NewRequest<'static> {
    NewRequest {
        client_id: CLIENT,
        client_name: "Claude",
        redirect_uri: REDIRECT,
        state: Some("xyz"),
        code_challenge: CHALLENGE,
        resource: RESOURCE,
        scope: "hq offline_access",
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

/// A request approved with `scope`, returning its code.
fn approved(c: &Connection, scope: GrantScope) -> String {
    let id = create_request(c, &new_request(), NOW).unwrap();
    let req = take_request(c, &id, NOW).unwrap().unwrap();
    issue_code(c, &req, scope, "Claude", NOW).unwrap()
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
fn a_request_is_decided_once_and_expires() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        let id = create_request(c, &new_request(), NOW)?;
        let req = get_request(c, &id, NOW)?.unwrap();
        assert_eq!(req.state.as_deref(), Some("xyz"));
        assert!(take_request(c, &id, NOW)?.is_some());
        assert!(take_request(c, &id, NOW)?.is_none(), "taken twice");

        let late = create_request(c, &new_request(), NOW)?;
        assert!(get_request(c, &late, NOW + REQUEST_TTL_SECS)?.is_none());
        assert!(take_request(c, &late, NOW + REQUEST_TTL_SECS)?.is_none());
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
        let grants = list_grants(c, true)?;
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
        arm(c, NOW + 600)?;
        assert_eq!(revoke_all(c, "kill switch", NOW)?, 2);
        assert_eq!(armed_until(c, NOW)?, None);
        for pair in [b, d] {
            assert!(grant_for_access_token(c, &pair.access_token, NOW)?.is_none());
        }
        assert!(list_grants(c, false)?.is_empty());
        assert_eq!(list_grants(c, true)?.len(), 3);
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
