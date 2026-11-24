//! What the egress proxy needs of a sandboxed agent: its rules from the plan, and the environment
//! beyond the usual variables.

use crate::egress::Rule;
use crate::sandbox::Allow;

/// Java and Gradle ignore the proxy variables, so a build under the strict profile cannot resolve
/// a host even when the owner allowed it. Every JVM reads this one, so they use the sandbox's proxy.
pub(crate) const JAVA_OPTIONS_ENV: &str = "JAVA_TOOL_OPTIONS";

/// `JAVA_TOOL_OPTIONS` with the sandbox's egress proxy added to whatever the host already sets.
pub(crate) fn java_proxy_options(port: u16, existing: Option<&str>) -> String {
    let proxy = format!(
        "-Dhttp.proxyHost=127.0.0.1 -Dhttp.proxyPort={port} -Dhttps.proxyHost=127.0.0.1 \
         -Dhttps.proxyPort={port} -Dhttp.nonProxyHosts="
    );
    match existing.map(str::trim).filter(|e| !e.is_empty()) {
        Some(e) => format!("{e} {proxy}"),
        None => proxy,
    }
}

pub(crate) fn rules(allow: &[Allow]) -> Vec<Rule> {
    allow
        .iter()
        .map(|a| {
            let mut rule = Rule::new(&a.host);
            rule.ports = a.ports.clone();
            rule.port_ranges = a.port_ranges.clone();
            rule.allow_private = a.private;
            rule
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_is_pointed_at_the_sandbox_proxy_and_keeps_what_the_host_set() {
        let plain = java_proxy_options(4321, None);
        assert!(
            plain.contains("-Dhttps.proxyHost=127.0.0.1")
                && plain.contains("-Dhttps.proxyPort=4321")
        );
        assert!(
            plain.contains("-Dhttp.nonProxyHosts="),
            "nothing bypasses the proxy"
        );
        let kept = java_proxy_options(4321, Some("  -Xmx2g "));
        assert!(kept.starts_with("-Xmx2g -Dhttp.proxyHost="), "{kept}");
        assert_eq!(java_proxy_options(1, Some("")), java_proxy_options(1, None));
    }
}
