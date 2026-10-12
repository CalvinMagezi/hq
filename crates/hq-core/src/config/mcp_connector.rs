use serde::{Deserialize, Serialize};

/// Lets a person add this HQ to claude.ai as a custom connector, over OAuth.
/// Off by default. Plan and threat model: `docs/plans/claude-connector.md`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpConnectorConfig {
    #[serde(default)]
    pub enabled: bool,

    /// The https origin Claude's servers reach, for example "https://hq.example.com". The
    /// protected resource is this origin plus `/mcp`.
    #[serde(default)]
    pub public_url: Option<String>,

    /// A private https origin, such as a tailnet name, that serves the consent page to the
    /// owner's browser. Unset: the consent page is on `public_url`.
    #[serde(default)]
    pub authorize_url: Option<String>,

    /// The scope preselected on the consent page: `read_only` (the default), `tasks` or
    /// `full`. The owner can still pick another one there.
    #[serde(default)]
    pub default_scope: Option<String>,
}

/// Grant scopes a consent page may offer, narrowest first.
pub const CONNECTOR_SCOPES: &[&str] = &["read_only", "tasks", "full"];

/// The connector's settings once checked. Origins are lowercase, with no trailing slash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorSettings {
    pub public: String,
    pub consent: String,
    pub default_scope: String,
}

impl ConnectorSettings {
    /// The protected resource Claude is given tokens for.
    pub fn resource(&self) -> String {
        format!("{}/mcp", self.public)
    }
}

impl McpConnectorConfig {
    /// The checked settings, `None` when the connector is off, or why it cannot be turned on.
    pub fn settings(
        &self,
        web_auth_token: Option<&str>,
    ) -> Result<Option<ConnectorSettings>, String> {
        if !self.enabled {
            return Ok(None);
        }
        if web_auth_token.is_none_or(|t| t.trim().is_empty()) {
            return Err(
                "mcp_connector needs web_auth_token: the consent page signs the owner in with it"
                    .into(),
            );
        }
        let raw_public = self
            .public_url
            .as_deref()
            .ok_or("mcp_connector.public_url is not set")?;
        let public =
            https_origin(raw_public).map_err(|e| format!("mcp_connector.public_url: {e}"))?;
        let consent = match self.authorize_url.as_deref() {
            Some(u) => https_origin(u).map_err(|e| format!("mcp_connector.authorize_url: {e}"))?,
            None => public.clone(),
        };
        let default_scope = self
            .default_scope
            .as_deref()
            .unwrap_or(CONNECTOR_SCOPES[0])
            .to_string();
        if !CONNECTOR_SCOPES.contains(&default_scope.as_str()) {
            return Err(format!(
                "mcp_connector.default_scope must be one of {CONNECTOR_SCOPES:?}"
            ));
        }
        Ok(Some(ConnectorSettings {
            public,
            consent,
            default_scope,
        }))
    }
}

/// An https origin with no path, query, fragment or credentials, normalised for comparison.
fn https_origin(raw: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(raw.trim()).map_err(|e| format!("{raw:?} is not a URL ({e})"))?;
    if url.scheme() != "https" {
        return Err(format!("{raw:?} must use https"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!("{raw:?} must not carry credentials"));
    }
    if url.query().is_some() || url.fragment().is_some() || url.path() != "/" {
        return Err(format!(
            "{raw:?} must be an origin, with no path, query or fragment"
        ));
    }
    let host_ok = url.host_str().is_some_and(|h| {
        h.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-.:[]".contains(&b))
    });
    if !host_ok {
        return Err(format!("{raw:?} needs a plain host name or address"));
    }
    Ok(url.as_str().trim_end_matches('/').to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(public: Option<&str>, authorize: Option<&str>) -> McpConnectorConfig {
        McpConnectorConfig {
            enabled: true,
            public_url: public.map(Into::into),
            authorize_url: authorize.map(Into::into),
            default_scope: None,
        }
    }

    #[test]
    fn off_by_default_and_off_needs_nothing() {
        assert_eq!(McpConnectorConfig::default().settings(None), Ok(None));
    }

    #[test]
    fn on_needs_a_web_token_and_a_public_url() {
        let c = cfg(Some("https://hq.example.com"), None);
        assert!(c.settings(None).unwrap_err().contains("web_auth_token"));
        assert!(
            c.settings(Some("  "))
                .unwrap_err()
                .contains("web_auth_token")
        );
        assert!(
            cfg(None, None)
                .settings(Some("t"))
                .unwrap_err()
                .contains("public_url")
        );
    }

    #[test]
    fn origins_are_normalised_and_consent_defaults_to_public() {
        let c = cfg(Some("https://HQ.example.com/"), None);
        let o = c.settings(Some("t")).unwrap().unwrap();
        assert_eq!(o.public, "https://hq.example.com");
        assert_eq!(o.consent, o.public);
        assert_eq!(o.resource(), "https://hq.example.com/mcp");
        assert_eq!(o.default_scope, "read_only");

        let split = cfg(
            Some("https://hq.example.com"),
            Some("https://hq.example.ts.net:8443"),
        );
        let o = split.settings(Some("t")).unwrap().unwrap();
        assert_eq!(o.consent, "https://hq.example.ts.net:8443");
    }

    #[test]
    fn the_default_scope_must_be_a_known_one() {
        let mut c = cfg(Some("https://hq.example.com"), None);
        c.default_scope = Some("full".into());
        assert_eq!(
            c.settings(Some("t")).unwrap().unwrap().default_scope,
            "full"
        );
        c.default_scope = Some("handoff".into());
        assert!(c.settings(Some("t")).unwrap_err().contains("default_scope"));
    }

    #[test]
    fn anything_but_a_bare_https_origin_is_refused() {
        for bad in [
            "http://hq.example.com",
            "https://hq.example.com/mcp",
            "https://hq.example.com/?a=1",
            "https://hq.example.com/#x",
            "https://user:pw@hq.example.com",
            "https://hq\"x.example.com",
            "hq.example.com",
        ] {
            assert!(
                cfg(Some(bad), None).settings(Some("t")).is_err(),
                "public {bad}"
            );
            assert!(
                cfg(Some("https://hq.example.com"), Some(bad))
                    .settings(Some("t"))
                    .is_err(),
                "authorize {bad}"
            );
        }
    }
}
