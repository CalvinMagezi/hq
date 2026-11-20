//! `sandbox_denials`: what the caller's own egress proxy refused lately, so a `sandbox_request`
//! names exactly the host and port. File and process denials are not collected here: they show
//! in the helper's own tool errors, which name the path.

use crate::agent_host::EgressDecision;
use crate::registry::HqTool;
use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_db::Database;
use serde_json::{Value, json};
use std::sync::Arc;

const MAX_SHOWN: usize = 20;

/// The refused requests among `decisions`, once each, newest last, at most [`MAX_SHOWN`].
pub(super) fn refused(decisions: &[EgressDecision]) -> Vec<Value> {
    let mut seen: Vec<(&str, u16)> = Vec::new();
    for d in decisions.iter().filter(|d| !d.allowed) {
        if !seen.contains(&(d.host.as_str(), d.port)) {
            seen.push((d.host.as_str(), d.port));
        }
    }
    let skip = seen.len().saturating_sub(MAX_SHOWN);
    seen.into_iter()
        .skip(skip)
        .map(|(host, port)| {
            let reason = decisions
                .iter()
                .rev()
                .find(|d| !d.allowed && d.host == host && d.port == port)
                .map(|d| d.reason.as_str())
                .unwrap_or_default();
            json!({ "host": host, "port": port, "reason": reason })
        })
        .collect()
}

pub struct SandboxDenialsTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for SandboxDenialsTool {
    fn name(&self) -> &str {
        "sandbox_denials"
    }

    fn description(&self) -> &str {
        "List the network hosts and ports your sandbox's egress proxy refused recently, so a sandbox_request can name exactly what you need. Paths and local ports are not listed here: your tool errors name them."
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    fn category(&self) -> &str {
        "harness"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let Some(session) = super::caller_session(&args).map(str::to_string) else {
            bail!("only a session HQ launched can ask what its sandbox refused");
        };
        let db = self.db.clone();
        let (decisions, cannot) = tokio::task::spawn_blocking(move || -> Result<_> {
            let row = super::control::get_row(&db, &session)?;
            let host = crate::agent_host::host(Some(&row.host))?;
            let found = host.egress_decisions(&row.agent_name);
            Ok((found.clone().unwrap_or_default(), found.is_none()))
        })
        .await??;
        Ok(json!({
            "egress_refused": refused(&decisions),
            "available": !cannot,
            "next": "Call sandbox_request with capability egress and the host (host:port when not 443). For a path or a local port, use the path or port named in your error.",
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(host: &str, port: u16, allowed: bool, reason: &str) -> EgressDecision {
        EgressDecision {
            host: host.into(),
            port,
            allowed,
            reason: reason.into(),
        }
    }

    #[test]
    fn only_refusals_are_listed_once_each_with_the_latest_reason() {
        let log = vec![
            d("pub.dev", 443, false, "not allowed"),
            d("api.example.org", 443, true, ""),
            d("pub.dev", 443, false, "not in the allowlist"),
            d("dl.example.org", 8443, false, "not allowed"),
        ];
        let out = refused(&log);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["host"], "pub.dev");
        assert_eq!(out[0]["reason"], "not in the allowlist");
        assert_eq!(out[1]["port"], 8443);
    }

    #[test]
    fn a_long_log_is_cut_to_the_newest_few() {
        let log: Vec<_> = (0..40)
            .map(|i| d(&format!("h{i}.example.org"), 443, false, "no"))
            .collect();
        let out = refused(&log);
        assert_eq!(out.len(), MAX_SHOWN);
        assert_eq!(out.last().unwrap()["host"], "h39.example.org");
    }
}
