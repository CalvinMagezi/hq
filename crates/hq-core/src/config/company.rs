use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyConfig {
    pub id: String,
    pub name: String,
    pub identity: CompanyIdentity,
    /// Vault-relative prefix, e.g. "Notebooks/Companies/acme"
    pub vault_prefix: String,
    #[serde(default)]
    pub listeners: Vec<ListenerDef>,
    #[serde(default)]
    pub connectors: Vec<ConnectorBinding>,
    #[serde(default)]
    pub budget: CompanyBudget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListenerDef {
    pub id: String,
    pub kind: String,
    pub path: String,
    #[serde(default)]
    pub secret_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorBinding {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub secret_ref: Option<String>,
}

impl ConnectorBinding {
    pub fn is_singleton(&self) -> bool {
        matches!(self.kind.as_str(), "gws" | "github" | "vercel" | "convex")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyIdentity {
    pub contact_name: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyBudget {
    #[serde(default)]
    pub monthly_usd: f64,
    #[serde(default = "default_alert_at")]
    pub alert_at: f64,
}

fn default_alert_at() -> f64 {
    0.8
}

impl Default for CompanyBudget {
    fn default() -> Self {
        Self {
            monthly_usd: 0.0,
            alert_at: default_alert_at(),
        }
    }
}

impl CompanyConfig {
    pub fn pending_emails_path(&self, vault_path: &Path) -> PathBuf {
        vault_path.join(&self.vault_prefix).join("PendingEmails")
    }
}

/// Look up a company by id from a slice. Returns `None` if not found.
pub fn company_by_id<'a>(companies: &'a [CompanyConfig], id: &str) -> Option<&'a CompanyConfig> {
    companies.iter().find(|c| c.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_companies() -> Vec<CompanyConfig> {
        vec![
            CompanyConfig {
                id: "northwind".into(),
                name: "Northwind Logistics".into(),
                identity: CompanyIdentity {
                    contact_name: "Alice Example".into(),
                    role: "CTO".into(),
                },
                vault_prefix: "Notebooks/Companies/northwind".into(),
                listeners: vec![],
                connectors: vec![],
                budget: CompanyBudget {
                    monthly_usd: 50.0,
                    alert_at: 0.8,
                },
            },
            CompanyConfig {
                id: "acme".into(),
                name: "AcmeCorp".into(),
                identity: CompanyIdentity {
                    contact_name: "Alice Example".into(),
                    role: "Director".into(),
                },
                vault_prefix: "Notebooks/Companies/acme".into(),
                listeners: vec![],
                connectors: vec![],
                budget: CompanyBudget {
                    monthly_usd: 30.0,
                    alert_at: 0.8,
                },
            },
        ]
    }

    #[test]
    fn deserializes_listeners_and_connectors() {
        let yaml = r#"
id: northwind
name: Northwind Logistics Limited
identity:
  contact_name: Alice Example
  role: CTO
vault_prefix: Notebooks/Companies/northwind
listeners:
  - id: gmail-inbox
    kind: email
    path: /trigger/northwind/email-triage
    secret_ref: hq-northwind-listener
connectors:
  - name: gmail
    kind: gws
    secret_ref: null
  - name: zoho-mail
    kind: imap_smtp
    secret_ref: hq-northwind-zoho
flows:
  email-reply: inbox-lieutenant-send
  invoice-send: northwind-invoice
"#;
        let c: CompanyConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(c.listeners.len(), 1);
        assert_eq!(c.listeners[0].path, "/trigger/northwind/email-triage");
        assert_eq!(c.connectors.len(), 2);
        assert_eq!(c.connectors[1].kind, "imap_smtp");
        assert_eq!(
            c.connectors[1].secret_ref.as_deref(),
            Some("hq-northwind-zoho")
        );
    }

    #[test]
    fn singleton_connector_detection() {
        let binding = ConnectorBinding {
            name: "gmail".into(),
            kind: "gws".into(),
            secret_ref: None,
        };
        assert!(binding.is_singleton());
        let switchable = ConnectorBinding {
            name: "zoho-mail".into(),
            kind: "imap_smtp".into(),
            secret_ref: Some("hq-x-zoho".into()),
        };
        assert!(!switchable.is_singleton());
    }

    #[test]
    fn company_by_id_found() {
        let companies = two_companies();
        let c = company_by_id(&companies, "acme").unwrap();
        assert_eq!(c.identity.role, "Director");
    }

    #[test]
    fn company_by_id_not_found() {
        let companies = two_companies();
        assert!(company_by_id(&companies, "unknown").is_none());
    }

    #[test]
    fn vault_paths_derived_correctly() {
        use std::path::Path;
        let companies = two_companies();
        let c = company_by_id(&companies, "acme").unwrap();
        let vault = Path::new("/vault");
        assert_eq!(
            c.pending_emails_path(vault),
            Path::new("/vault/Notebooks/Companies/acme/PendingEmails")
        );
    }
}
