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

    /// The connector's own TLS, for a device that a public forwarder reaches with the raw TLS
    /// bytes (`hq edge`) instead of an ingress that ends TLS for it.
    #[serde(default)]
    pub tls: ConnectorTlsConfig,
}

/// Lets HQ end TLS itself for the connector's public name and get its certificate through
/// ACME. Off by default. Design and threat model: `docs/security/CONNECTOR_TLS.md`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnectorTlsConfig {
    #[serde(default)]
    pub enabled: bool,

    /// Where certificates come from: `letsencrypt` (the default), `letsencrypt-staging`, or the
    /// https URL of another ACME directory.
    #[serde(default)]
    pub acme_directory: Option<String>,

    /// An address the CA may write to about the certificate. Optional; leave it unset to give
    /// the CA none.
    #[serde(default)]
    pub contact_email: Option<String>,

    /// Agreement to the CA's subscriber agreement, which registering an ACME account accepts on
    /// the owner's behalf. Let's Encrypt's is at <https://letsencrypt.org/repository/>. HQ
    /// registers no account until this is `true`.
    #[serde(default)]
    pub agree_to_ca_terms: bool,

    /// Where this machine takes the connector's connections from the bridge. Unset: a unix
    /// socket in a directory only this user can enter (`<vault>/_data/connector_door/door.sock`).
    /// An absolute path (or `unix:/path`) names another socket. A loopback `ip:port` with a fixed
    /// port is allowed as an opt-in, but any local process can bind a port first, so another user
    /// or app on the machine could take the forwarded connections (including the CA's validation
    /// of the certificate); use it only on a machine with one user. The process that holds the
    /// forwarder's streams (`hq bridge serve` or `run`, a separate process on a phone) connects
    /// here, and nothing else should.
    #[serde(default)]
    pub door_listen: Option<String>,
}

/// The directory (under `<vault>/_data`) and file the connector's TLS door listens on when
/// `door_listen` is unset. The bridge's forwarder resolves the same default, so a registered
/// device needs no setting for it.
pub const DOOR_DIR: &str = "connector_door";
pub const DOOR_SOCKET_FILE: &str = "door.sock";

/// Where the connector's TLS door takes the bridge's streams, as configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoorListen {
    /// A unix socket in `<vault>/_data/connector_door/`, a directory only this user can enter.
    Default,
    /// A unix socket at this absolute path, in a directory this user owns and others cannot write.
    Unix(std::path::PathBuf),
    /// A loopback address with a fixed port. Weaker: see `McpConnectorConfig::tls.door_listen`.
    Tcp(std::net::SocketAddr),
}

/// A door address with the vault known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoorTarget {
    Unix(std::path::PathBuf),
    Tcp(std::net::SocketAddr),
}

impl DoorListen {
    /// Reads `mcp_connector.tls.door_listen` or `--edge-door`: nothing means the default, a path
    /// starting with `/` (or `unix:`) a unix socket, anything else a loopback `ip:port` with a
    /// fixed port.
    pub fn parse(text: Option<&str>) -> Result<Self, String> {
        let text = text.map(str::trim).filter(|t| !t.is_empty());
        let Some(text) = text else {
            return Ok(Self::Default);
        };
        if let Some(path) = text
            .strip_prefix("unix:")
            .or(text.starts_with('/').then_some(text))
        {
            let path = std::path::PathBuf::from(path);
            if !path.is_absolute() || path.file_name().is_none() {
                return Err(format!("{text:?} is not an absolute socket path"));
            }
            return Ok(Self::Unix(path));
        }
        let addr: std::net::SocketAddr = text.parse().map_err(|_| {
            format!("{text:?} is not a socket path (starting with /) or a loopback ip:port")
        })?;
        if !addr.ip().is_loopback() || addr.port() == 0 {
            return Err(format!(
                "{text:?} must be a loopback address with a fixed port, or a socket path"
            ));
        }
        Ok(Self::Tcp(addr))
    }

    /// The address to listen on or connect to, with the vault's `_data` directory filled in.
    pub fn resolve(&self, vault: &std::path::Path) -> DoorTarget {
        match self {
            Self::Default => {
                DoorTarget::Unix(vault.join("_data").join(DOOR_DIR).join(DOOR_SOCKET_FILE))
            }
            Self::Unix(path) => DoorTarget::Unix(path.clone()),
            Self::Tcp(addr) => DoorTarget::Tcp(*addr),
        }
    }
}

/// The ACME directory a bare `letsencrypt` means.
pub const ACME_DIRECTORY_LETSENCRYPT: &str = "https://acme-v02.api.letsencrypt.org/directory";
/// The one `letsencrypt-staging` means: untrusted certificates, far higher limits.
pub const ACME_DIRECTORY_LETSENCRYPT_STAGING: &str =
    "https://acme-staging-v02.api.letsencrypt.org/directory";

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

/// What the connector's TLS needs once checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorTlsSettings {
    /// The name the certificate is for and the only TLS server name answered.
    pub host: String,
    /// `host`, plus `:port` when the public origin names one: what a request's `Host` must be.
    pub authority: String,
    pub directory_url: String,
    pub contact_email: Option<String>,
    /// Where the TLS door takes the bridge's streams.
    pub door_listen: DoorListen,
}

impl McpConnectorConfig {
    /// The checked TLS settings, `None` when connector TLS is off, or why it cannot be turned
    /// on. The certificate name is the public origin's host, so there is no second setting to
    /// keep in step with it.
    pub fn tls_settings(
        &self,
        web_auth_token: Option<&str>,
    ) -> Result<Option<ConnectorTlsSettings>, String> {
        if !self.tls.enabled {
            return Ok(None);
        }
        let connector = self
            .settings(web_auth_token)?
            .ok_or("mcp_connector.tls needs mcp_connector.enabled")?;
        if !self.tls.agree_to_ca_terms {
            return Err(
                "mcp_connector.tls needs agree_to_ca_terms: true, because getting a certificate \
                 registers an account that accepts the CA's subscriber agreement"
                    .into(),
            );
        }
        let url = reqwest::Url::parse(&connector.public)
            .map_err(|e| format!("mcp_connector.public_url: {e}"))?;
        // The CA validates the name on port 443 only, and a name ending in a dot never matches
        // the server name a client sends.
        if url.port().is_some() {
            return Err("mcp_connector.tls needs public_url on the default https port".into());
        }
        let host = match url.domain() {
            Some(d) if d.contains('.') && !d.ends_with('.') => d.to_string(),
            _ => {
                return Err(
                    "mcp_connector.tls needs public_url to name a domain: a certificate is \
                     issued for a DNS name, not an address or a single label"
                        .into(),
                );
            }
        };
        let authority = match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.clone(),
        };
        let directory_url = match self.tls.acme_directory.as_deref().map(str::trim) {
            None | Some("") | Some("letsencrypt") => ACME_DIRECTORY_LETSENCRYPT.to_string(),
            Some("letsencrypt-staging") => ACME_DIRECTORY_LETSENCRYPT_STAGING.to_string(),
            Some(other) => {
                let parsed = reqwest::Url::parse(other)
                    .map_err(|e| format!("mcp_connector.tls.acme_directory: {e}"))?;
                if parsed.scheme() != "https" {
                    return Err("mcp_connector.tls.acme_directory must be https".into());
                }
                other.to_string()
            }
        };
        let contact_email = self
            .tls
            .contact_email
            .as_deref()
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .map(|e| contact_address(e).map(str::to_string))
            .transpose()?;
        let door_listen = DoorListen::parse(self.tls.door_listen.as_deref())
            .map_err(|e| format!("mcp_connector.tls.door_listen: {e}"))?;
        Ok(Some(ConnectorTlsSettings {
            host,
            authority,
            directory_url,
            contact_email,
            door_listen,
        }))
    }

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

/// A plain mailbox address: it goes into an ACME `mailto:` contact, so nothing that could add a
/// header or a second contact gets through.
fn contact_address(raw: &str) -> Result<&str, String> {
    let plain = raw
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"@.-_+".contains(&b));
    let (local, domain) = raw.split_once('@').unwrap_or_default();
    if plain && !local.is_empty() && domain.contains('.') && !domain.contains('@') {
        Ok(raw)
    } else {
        Err("mcp_connector.tls.contact_email is not a plain email address".into())
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
            ..Default::default()
        }
    }

    fn with_tls(public: &str, tls: ConnectorTlsConfig) -> McpConnectorConfig {
        McpConnectorConfig {
            tls,
            ..cfg(Some(public), None)
        }
    }

    fn tls_on() -> ConnectorTlsConfig {
        ConnectorTlsConfig {
            enabled: true,
            agree_to_ca_terms: true,
            ..Default::default()
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

    #[test]
    fn tls_is_off_unless_asked_for_and_asks_for_the_connector_first() {
        assert_eq!(
            cfg(Some("https://hq.example.com"), None).tls_settings(Some("t")),
            Ok(None)
        );
        let off = McpConnectorConfig {
            enabled: false,
            ..with_tls("https://hq.example.com", tls_on())
        };
        assert!(
            off.tls_settings(Some("t"))
                .unwrap_err()
                .contains("mcp_connector.enabled")
        );
    }

    #[test]
    fn the_certificate_name_is_the_public_origins_host() {
        let c = with_tls("https://Dev1.Edge.Example.com", tls_on());
        let t = c.tls_settings(Some("t")).unwrap().unwrap();
        assert_eq!(t.host, "dev1.edge.example.com");
        assert_eq!(t.authority, "dev1.edge.example.com");
        assert_eq!(t.directory_url, ACME_DIRECTORY_LETSENCRYPT);
        assert_eq!(t.contact_email, None);
    }

    #[test]
    fn the_door_is_a_private_socket_unless_a_loopback_port_is_asked_for() {
        let c = with_tls("https://hq.example.com", tls_on());
        assert_eq!(
            c.tls_settings(Some("t")).unwrap().unwrap().door_listen,
            DoorListen::Default
        );
        let vault = std::path::Path::new("/vault");
        assert_eq!(
            DoorListen::Default.resolve(vault),
            DoorTarget::Unix("/vault/_data/connector_door/door.sock".into())
        );
        for (door, expected) in [
            (
                "/run/hq/door.sock",
                Some(DoorListen::Unix("/run/hq/door.sock".into())),
            ),
            (
                "unix:/run/hq/door.sock",
                Some(DoorListen::Unix("/run/hq/door.sock".into())),
            ),
            (
                "127.0.0.1:7001",
                Some(DoorListen::Tcp("127.0.0.1:7001".parse().unwrap())),
            ),
            (
                "[::1]:7001",
                Some(DoorListen::Tcp("[::1]:7001".parse().unwrap())),
            ),
            ("0.0.0.0:5679", None),
            ("192.168.1.9:5679", None),
            ("[::]:5679", None),
            ("127.0.0.1:0", None),
            ("localhost:5679", None),
            ("127.0.0.1", None),
            ("relative/door.sock", None),
            ("unix:relative.sock", None),
            ("/", None),
        ] {
            let tls = ConnectorTlsConfig {
                door_listen: Some(door.into()),
                ..tls_on()
            };
            let got = with_tls("https://hq.example.com", tls)
                .tls_settings(Some("t"))
                .map(|t| t.map(|t| t.door_listen));
            match expected {
                Some(want) => assert_eq!(got, Ok(Some(want)), "{door}"),
                None => assert!(got.is_err(), "{door}: {got:?}"),
            }
        }
        // An empty setting is the default, as everywhere else.
        assert_eq!(DoorListen::parse(Some("  ")), Ok(DoorListen::Default));
    }

    #[test]
    fn the_ca_is_only_asked_once_its_terms_are_agreed_to() {
        let mut tls = tls_on();
        tls.agree_to_ca_terms = false;
        let err = with_tls("https://hq.example.com", tls)
            .tls_settings(Some("t"))
            .unwrap_err();
        assert!(err.contains("agree_to_ca_terms"), "{err}");
    }

    #[test]
    fn a_name_the_ca_cannot_validate_or_a_client_cannot_match_is_refused() {
        for bad in ["https://hq.example.com:8443", "https://hq.example.com./"] {
            assert!(
                with_tls(bad, tls_on()).tls_settings(Some("t")).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_certificate_cannot_be_asked_for_an_address_or_a_bare_label() {
        for bad in [
            "https://192.0.2.7",
            "https://[2001:db8::1]",
            "https://localhost",
            "https://hq",
        ] {
            let err = with_tls(bad, tls_on()).tls_settings(Some("t")).unwrap_err();
            assert!(err.contains("domain"), "{bad}: {err}");
        }
    }

    #[test]
    fn the_directory_is_a_known_name_or_an_https_url_and_the_contact_is_plain() {
        let mut tls = tls_on();
        tls.acme_directory = Some("letsencrypt-staging".into());
        tls.contact_email = Some("owner@example.com".into());
        let t = with_tls("https://hq.example.com", tls)
            .tls_settings(Some("t"))
            .unwrap()
            .unwrap();
        assert_eq!(t.directory_url, ACME_DIRECTORY_LETSENCRYPT_STAGING);
        assert_eq!(t.contact_email.as_deref(), Some("owner@example.com"));

        let mut own = tls_on();
        own.acme_directory = Some("https://acme.example.org/dir".into());
        let t = with_tls("https://hq.example.com", own)
            .tls_settings(Some("t"))
            .unwrap()
            .unwrap();
        assert_eq!(t.directory_url, "https://acme.example.org/dir");

        for bad_dir in ["http://acme.example.org/dir", "not a url"] {
            let mut tls = tls_on();
            tls.acme_directory = Some(bad_dir.into());
            assert!(
                with_tls("https://hq.example.com", tls)
                    .tls_settings(Some("t"))
                    .is_err(),
                "{bad_dir}"
            );
        }
        for bad_mail in [
            "a@b",
            "a@b.c,d@e.f",
            "a b@c.d",
            "a@b.c\r\nBcc: x@y.z",
            "@b.c",
        ] {
            let mut tls = tls_on();
            tls.contact_email = Some(bad_mail.into());
            assert!(
                with_tls("https://hq.example.com", tls)
                    .tls_settings(Some("t"))
                    .is_err(),
                "{bad_mail:?}"
            );
        }
    }
}
