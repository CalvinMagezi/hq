use serde::{Deserialize, Serialize};

/// Web Push to the PWA: the server sends notifications to browsers that subscribed, so they
/// arrive on a phone or desktop even when HQ is closed. Off by default. Docs:
/// `docs/WEB_PUSH.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebPushConfig {
    #[serde(default)]
    pub enabled: bool,

    /// Contact for the push services, a `mailto:` or `https:` URL (RFC 8292). Apple's push
    /// service refuses pushes without one. Unset: the first `https` entry of
    /// `web_allowed_origins`, else a placeholder that works with Google's and Mozilla's.
    #[serde(default)]
    pub subject: Option<String>,

    /// Push every new inbox item. Off: only items an agent sent with `notify_owner` and
    /// items that wait on an answer (approvals) are pushed; the rest stay in the inbox.
    #[serde(default)]
    pub push_all: bool,

    /// Notifications agents may send through `notify_owner` per rolling hour, so a looping
    /// agent cannot flood a phone.
    #[serde(default = "default_max_agent_per_hour")]
    pub max_agent_per_hour: u32,

    /// Push services to accept subscriptions for besides the built-in ones (Google, Apple,
    /// Mozilla, Microsoft). A host name; its subdomains are allowed too.
    #[serde(default)]
    pub extra_push_hosts: Vec<String>,
}

fn default_max_agent_per_hour() -> u32 {
    20
}

impl Default for WebPushConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            subject: None,
            push_all: false,
            max_agent_per_hour: default_max_agent_per_hour(),
            extra_push_hosts: Vec::new(),
        }
    }
}

/// The push services the major browsers subscribe through. A subscription whose endpoint is
/// anywhere else is refused, so the server never posts to an address a client made up.
pub const PUSH_SERVICE_HOSTS: &[&str] = &[
    // Chrome, Edge on Android, Opera, Samsung Internet, Brave.
    "fcm.googleapis.com",
    // Safari on macOS and iOS (web.push.apple.com).
    "push.apple.com",
    // Firefox (updates.push.services.mozilla.com).
    "push.services.mozilla.com",
    // Edge on Windows (wns2-*.notify.windows.com).
    "notify.windows.com",
];

impl WebPushConfig {
    /// Whether a subscription endpoint on `host` may be used.
    pub fn push_host_allowed(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        PUSH_SERVICE_HOSTS
            .iter()
            .copied()
            .chain(self.extra_push_hosts.iter().map(String::as_str))
            .map(|h| h.trim().trim_end_matches('.').to_ascii_lowercase())
            .any(|h| h.contains('.') && (host == h || host.ends_with(&format!(".{h}"))))
    }

    /// Every host a push can go to, for the egress report.
    pub fn hosts(&self) -> Vec<String> {
        PUSH_SERVICE_HOSTS
            .iter()
            .map(|h| h.to_string())
            .chain(self.extra_push_hosts.iter().cloned())
            .collect()
    }

    /// The VAPID subject to sign with.
    pub fn effective_subject(&self, web_allowed_origins: &[String]) -> String {
        if let Some(s) = self.subject.as_deref().map(str::trim)
            && !s.is_empty()
        {
            return s.to_string();
        }
        web_allowed_origins
            .iter()
            .map(|o| o.trim().trim_end_matches('/'))
            .find(|o| o.starts_with("https://"))
            .map(str::to_string)
            .unwrap_or_else(|| "mailto:hq@localhost".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_push_services_and_listed_hosts_are_allowed() {
        let mut cfg = WebPushConfig::default();
        for ok in [
            "fcm.googleapis.com",
            "web.push.apple.com",
            "updates.push.services.mozilla.com",
            "wns2-par02p.notify.windows.com",
            "FCM.googleapis.com.",
        ] {
            assert!(cfg.push_host_allowed(ok), "{ok}");
        }
        for bad in [
            "localhost",
            "127.0.0.1",
            "evilfcm.googleapis.com.example.com",
            "notfcm.googleapis.co",
            "push.example.com",
            "apple.com",
        ] {
            assert!(!cfg.push_host_allowed(bad), "{bad}");
        }
        cfg.extra_push_hosts = vec!["push.example.com".into(), "com".into()];
        assert!(cfg.push_host_allowed("eu.push.example.com"));
        assert!(
            !cfg.push_host_allowed("anything.com"),
            "a bare label never acts as a suffix"
        );
    }

    #[test]
    fn subject_prefers_the_setting_then_an_https_origin() {
        let mut cfg = WebPushConfig::default();
        let origins = vec![
            "http://localhost:4747".to_string(),
            "https://hq.example.ts.net/".to_string(),
        ];
        assert_eq!(cfg.effective_subject(&origins), "https://hq.example.ts.net");
        assert_eq!(cfg.effective_subject(&[]), "mailto:hq@localhost");
        cfg.subject = Some("mailto:admin@example.com".into());
        assert_eq!(cfg.effective_subject(&origins), "mailto:admin@example.com");
    }
}
