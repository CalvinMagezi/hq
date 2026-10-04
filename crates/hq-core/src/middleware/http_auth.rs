//! Shared header parsing for `AGENTHQ_API_KEY` style protection.

/// Prefer non-empty `x-api-key`; otherwise use the Bearer token (already stripped of `Bearer `).
#[inline]
pub fn resolve_api_key_candidate<'a>(x_api_key: &'a str, bearer_token: &'a str) -> &'a str {
    if !x_api_key.is_empty() {
        x_api_key
    } else {
        bearer_token
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_x_api_key() {
        assert_eq!(resolve_api_key_candidate("abc", "def"), "abc");
    }

    #[test]
    fn falls_back_to_bearer() {
        assert_eq!(resolve_api_key_candidate("", "tok"), "tok");
    }
}
