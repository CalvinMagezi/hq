//! Native email send for triage replies. A company's email goes out through
//! the `gws` CLI; any other mail setup is the user's own gws configuration.

use anyhow::{Context, Result, bail};
use hq_core::config::HqConfig;
use tokio::process::Command;

/// Check that `company_id` exists and declares a `gws` connector. Pure: no
/// network or subprocess I/O, so it is testable without a real `gws` binary.
fn check_company(company_id: &str, config: &HqConfig) -> Result<()> {
    let company = config
        .companies
        .iter()
        .find(|c| c.id == company_id)
        .with_context(|| format!("unknown company '{company_id}'"))?;
    if !company.connectors.iter().any(|b| b.kind == "gws") {
        bail!("company '{company_id}' declares no email backend (a gws connector)");
    }
    Ok(())
}

/// Send a reply on behalf of `company_id` through its `gws` connector.
pub async fn send_via_company(company_id: &str, to: &str, subject: &str, body: &str) -> Result<()> {
    let config = HqConfig::load().context("failed to load HqConfig")?;
    check_company(company_id, &config)?;
    let output = Command::new(hq_core::paths::resolve_gws_binary())
        .args(["gmail", "+send", "--to", to, "--subject", subject, "--body", body])
        .output()
        .await
        .context("failed to spawn gws")?;
    if !output.status.success() {
        bail!("gws gmail +send failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::config::{CompanyConfig, CompanyIdentity, ConnectorBinding};

    fn company(id: &str, connectors: Vec<ConnectorBinding>) -> CompanyConfig {
        CompanyConfig {
            id: id.into(),
            name: id.into(),
            identity: CompanyIdentity {
                contact_name: "Alex".into(),
                role: "Owner".into(),
            },
            vault_prefix: format!("Notebooks/Companies/{id}"),
            listeners: vec![],
            connectors,
        }
    }

    fn config_with(companies: Vec<CompanyConfig>) -> HqConfig {
        
        HqConfig {
            companies,
            ..Default::default()
        }
    }

    #[test]
    fn an_unknown_company_is_rejected() {
        let err = check_company("ghost", &config_with(vec![])).unwrap_err();
        assert!(err.to_string().contains("unknown company"), "{err}");
    }

    #[test]
    fn a_company_with_no_gws_connector_is_rejected() {
        let config = config_with(vec![company(
            "northwind",
            vec![ConnectorBinding {
                name: "mail".into(),
                kind: "imap_smtp".into(),
                secret_ref: None,
            }],
        )]);
        let err = check_company("northwind", &config).unwrap_err();
        assert!(err.to_string().contains("no email backend"), "{err}");
    }

    #[test]
    fn a_gws_company_passes() {
        let config = config_with(vec![company(
            "northwind",
            vec![ConnectorBinding {
                name: "gmail".into(),
                kind: "gws".into(),
                secret_ref: None,
            }],
        )]);
        check_company("northwind", &config).unwrap();
    }
}
