use super::*;

const RESOURCE: &str = "https://hq.example.com/mcp";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

fn query(overrides: &[(&str, Option<&str>)]) -> HashMap<String, String> {
    let mut q: HashMap<String, String> = [
        ("response_type", "code"),
        (
            "client_id",
            "https://claude.ai/oauth/mcp-oauth-client-metadata",
        ),
        ("redirect_uri", "https://claude.ai/api/mcp/auth_callback"),
        ("code_challenge", CHALLENGE),
        ("code_challenge_method", "S256"),
        ("state", "abc"),
        ("scope", "hq offline_access"),
        ("resource", RESOURCE),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    for (k, v) in overrides {
        match v {
            Some(v) => q.insert(k.to_string(), v.to_string()),
            None => q.remove(*k),
        };
    }
    q
}

#[test]
fn only_claude_callbacks_and_loopback_callbacks_are_redirect_targets() {
    for ok in [
        "https://claude.ai/api/mcp/auth_callback",
        "https://claude.com/api/mcp/auth_callback",
        "http://localhost:3118/callback",
        "http://127.0.0.1:55001/callback",
        "http://localhost/callback",
    ] {
        assert!(redirect_allowed(ok), "{ok}");
    }
    for bad in [
        "https://claude.ai/api/mcp/auth_callback/",
        "https://claude.ai/api/mcp/auth_callback?x=1",
        "https://evil.example/api/mcp/auth_callback",
        "http://claude.ai/api/mcp/auth_callback",
        "https://localhost:3118/callback",
        "http://localhost:3118/other",
        "http://localhost.evil.example/callback",
        "http://192.168.1.2:3118/callback",
        "http://user@localhost:3118/callback",
        "http://localhost:3118/callback?next=x",
    ] {
        assert!(!redirect_allowed(bad), "{bad}");
    }
}

#[test]
fn a_well_formed_request_passes() {
    let q = query(&[]);
    let c = check(&q, RESOURCE).unwrap();
    assert_eq!(c.state, Some("abc"));
    assert_eq!(c.scope, "hq offline_access");
    let bare = query(&[("resource", None), ("state", None), ("scope", None)]);
    let c = check(&bare, RESOURCE).unwrap();
    assert_eq!(c.scope, crate::oauth::SCOPE);
}

#[test]
fn each_broken_field_is_refused() {
    let long_state = "s".repeat(MAX_STATE_LEN + 1);
    let cases: Vec<Vec<(&str, Option<&str>)>> = vec![
        vec![("response_type", Some("token"))],
        vec![("client_id", None)],
        vec![("redirect_uri", Some("https://evil.example/cb"))],
        vec![("redirect_uri", None)],
        vec![("code_challenge_method", Some("plain"))],
        vec![("code_challenge_method", None)],
        vec![("code_challenge", Some("short"))],
        vec![(
            "code_challenge",
            Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw+cM"),
        )],
        vec![("code_challenge", None)],
        vec![("resource", Some("https://other.example.com/mcp"))],
        vec![("state", Some(long_state.as_str()))],
    ];
    for overrides in cases {
        let q = query(&overrides);
        assert!(check(&q, RESOURCE).is_err(), "{overrides:?}");
    }
}

#[test]
fn loopback_redirects_match_the_listed_ones_on_any_port() {
    // As listed in Claude Code's published client document.
    let listed = ["http://localhost/callback", "http://127.0.0.1/callback"];
    let any = |requested: &str| listed.iter().any(|l| redirect_matches(l, requested));
    assert!(any("http://localhost:3118/callback"));
    assert!(any("http://127.0.0.1:55001/callback"));
    assert!(any("http://localhost/callback"));
    assert!(!any("http://localhost:3118/other"));
    assert!(!any("http://localhost:3118/callback?x=1"));
    assert!(!redirect_matches(
        "http://localhost/callback",
        "http://127.0.0.1:3118/callback"
    ));
    assert!(!redirect_matches(
        "https://claude.ai/api/mcp/auth_callback",
        "https://claude.ai:8443/api/mcp/auth_callback"
    ));
    assert!(redirect_matches(
        "https://claude.ai/api/mcp/auth_callback",
        "https://claude.ai/api/mcp/auth_callback"
    ));
}
