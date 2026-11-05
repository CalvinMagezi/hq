//! Words the native side shows, read from the one catalogue the web app and the phone use.

/// `apps/strings/en.yaml`, compiled in so the native words and the page's cannot say different things.
const CATALOGUE: &str = include_str!("../../../apps/strings/en.yaml");

/// The text of one catalogue key whose value is a single quoted line.
pub fn word(key: &str) -> String {
    let prefix = format!("{key}: \"");
    CATALOGUE
        .lines()
        .find_map(|l| l.strip_prefix(&prefix))
        .and_then(|v| v.strip_suffix('"'))
        .map(|v| v.replace("\\\"", "\""))
        .unwrap_or_else(|| panic!("the string catalogue has no line for {key}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_read_whole() {
        assert_eq!(word("hub_title"), "HQ Hub");
        assert_eq!(word("hub_window_phone"), "HQ · HQ on your phone");
    }
}
