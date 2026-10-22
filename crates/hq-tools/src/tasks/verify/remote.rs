//! Read-only access to GitHub for verification, behind a trait so collectors are tested on
//! canned answers and never touch the network.

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;

/// A diff and whether it was cut short.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    pub text: String,
    pub truncated: bool,
}

/// What verification reads from GitHub. Every method only reads.
#[async_trait]
pub trait GitHubRead: Send + Sync {
    /// GET of an API path under `repos/`, as JSON.
    async fn api_json(&self, path: &str) -> Result<Value>;
    /// The unified diff of a pull request.
    async fn pr_diff(&self, repo: &str, number: u64) -> Result<Diff>;
}

/// The real client: the `gh` CLI when installed, otherwise public data over HTTPS. The token's
/// own scopes are the hard limit, as for the `github_read` tool.
pub struct GhCli;

#[async_trait]
impl GitHubRead for GhCli {
    async fn api_json(&self, path: &str) -> Result<Value> {
        let body = crate::github::api_get(path).await?;
        if body.ends_with(crate::github::TRUNCATED_MARK) {
            anyhow::bail!("the answer was too long to read whole");
        }
        serde_json::from_str(&body)
            .with_context(|| format!("GitHub answered {path} with something that is not JSON"))
    }

    async fn pr_diff(&self, repo: &str, number: u64) -> Result<Diff> {
        let (text, truncated) = crate::github::pr_diff_text(repo, number).await?;
        Ok(Diff { text, truncated })
    }
}

/// A client with no answers, for tests in other modules that only need discovery not to reach out.
#[cfg(test)]
pub(crate) fn fake_for_tests() -> fake::FakeGitHub {
    fake::FakeGitHub::default()
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Canned answers by path. A path with no answer is an error, as an unreachable repository is.
    #[derive(Default)]
    pub struct FakeGitHub {
        pub json: HashMap<String, Value>,
        pub diffs: HashMap<(String, u64), Diff>,
        pub asked: Mutex<Vec<String>>,
    }

    impl FakeGitHub {
        pub fn with(mut self, path: &str, value: Value) -> Self {
            self.json.insert(path.to_string(), value);
            self
        }
        pub fn with_diff(mut self, repo: &str, number: u64, text: &str, truncated: bool) -> Self {
            self.diffs.insert(
                (repo.to_string(), number),
                Diff {
                    text: text.to_string(),
                    truncated,
                },
            );
            self
        }
    }

    #[async_trait]
    impl GitHubRead for FakeGitHub {
        async fn api_json(&self, path: &str) -> Result<Value> {
            self.asked.lock().unwrap().push(path.to_string());
            self.json
                .get(path)
                .cloned()
                .with_context(|| format!("GitHub returned 404 for {path}"))
        }
        async fn pr_diff(&self, repo: &str, number: u64) -> Result<Diff> {
            self.asked
                .lock()
                .unwrap()
                .push(format!("diff {repo}#{number}"));
            self.diffs
                .get(&(repo.to_string(), number))
                .cloned()
                .with_context(|| format!("no diff for {repo}#{number}"))
        }
    }
}
