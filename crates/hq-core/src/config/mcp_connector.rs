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
}

/// The connector's origins once checked: lowercase, no trailing slash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorOrigins {
    pub public: String,
    pub consent: String,
}

impl ConnectorOrigins {
    /// The protected resource Claude is given tokens for.
    pub fn resource(&self) -> String {
        format!("{}/mcp", self.public)
    }
}

impl McpConnectorConfig {
    /// The checked origins, `None` when the connector is off, or why it cannot be turned on.
    pub fn origins(
        &self,
        web_auth_token: Option<&str>,
    ) -> Result<Option<ConnectorOrigins>, String> {
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
        Ok(Some(ConnectorOrigins { public, consent }))
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
    if url.host_str().is_none() {
        return Err(format!("{raw:?} has no host"));
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
        }
    }

    #[test]
    fn off_by_default_and_off_needs_nothing() {
        assert_eq!(McpConnectorConfig::default().origins(None), Ok(None));
    }

    #[test]
    fn on_needs_a_web_token_and_a_public_url() {
        let c = cfg(Some("https://hq.example.com"), None);
        assert!(c.origins(None).unwrap_err().contains("web_auth_token"));
        assert!(
            c.origins(Some("  "))
                .unwrap_err()
                .contains("web_auth_token")
        );
        assert!(
            cfg(None, None)
                .origins(Some("t"))
                .unwrap_err()
                .contains("public_url")
        );
    }

    #[test]
    fn origins_are_normalised_and_consent_defaults_to_public() {
        let c = cfg(Some("https://HQ.example.com/"), None);
        let o = c.origins(Some("t")).unwrap().unwrap();
        assert_eq!(o.public, "https://hq.example.com");
        assert_eq!(o.consent, o.public);
        assert_eq!(o.resource(), "https://hq.example.com/mcp");

        let split = cfg(
            Some("https://hq.example.com"),
            Some("https://hq.example.ts.net:8443"),
        );
        let o = split.origins(Some("t")).unwrap().unwrap();
        assert_eq!(o.consent, "https://hq.example.ts.net:8443");
    }

    #[test]
    fn anything_but_a_bare_https_origin_is_refused() {
        for bad in [
            "http://hq.example.com",
            "https://hq.example.com/mcp",
            "https://hq.example.com/?a=1",
            "https://hq.example.com/#x",
            "https://user:pw@hq.example.com",
            "hq.example.com",
        ] {
            assert!(
                cfg(Some(bad), None).origins(Some("t")).is_err(),
                "public {bad}"
            );
            assert!(
                cfg(Some("https://hq.example.com"), Some(bad))
                    .origins(Some("t"))
                    .is_err(),
                "authorize {bad}"
            );
        }
    }
}
