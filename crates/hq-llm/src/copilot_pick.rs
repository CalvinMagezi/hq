//! Choosing which Copilot model to link, by asking the seat rather than a catalog.
//!
//! Which models a Copilot seat can use depends on the plan and on what the organization
//! enabled, and the only official way to learn it from outside GitHub's own clients is to use
//! the model. So [`pick_first_usable`] walks a preference list and tries a tiny request on each
//! through whatever probe the caller supplies. It never falls to a model that was not listed,
//! so a seat with neither preferred model gets a report instead of a silent bigger model.

use std::future::Future;

/// Why a probe failed, as far as the text of the failure tells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeFailure {
    /// The CLI is not signed in (or the session expired). No other model will fare better.
    NotSignedIn,
    /// The seat does not offer this model, or the organization turned it off.
    ModelUnavailable,
    /// Anything else: a timeout, a network error, a message we do not recognise.
    Other,
}

/// Classify the failure text a Copilot CLI run produced. Sign-in problems are checked first,
/// with specific phrases, so "invalid or expired token" is never read as a missing model.
pub fn classify_failure(text: &str) -> ProbeFailure {
    let t = text.to_ascii_lowercase();
    let auth = [
        "not logged in",
        "not signed in",
        "gh auth login",
        "authentication",
        "unauthorized",
        "http 401",
        "expired",
        "bad credentials",
        "sign in",
        "log in to",
    ];
    if auth.iter().any(|w| t.contains(w)) {
        return ProbeFailure::NotSignedIn;
    }
    let model_gone = [
        "not available",
        "not found",
        "not supported",
        "not enabled",
        "disabled",
        "invalid model",
        "unknown model",
        "does not exist",
        "no access to",
        "model_not_found",
    ];
    if t.contains("model") && model_gone.iter().any(|w| t.contains(w)) {
        return ProbeFailure::ModelUnavailable;
    }
    ProbeFailure::Other
}

/// What the walk found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// `model` answered. `skipped` lists earlier preferences that did not, with the reason.
    Chosen {
        model: String,
        skipped: Vec<(String, ProbeFailure)>,
    },
    /// A try failed for a reason that says nothing about the model (timeout, network, an
    /// unrecognised message). Nothing further was tried, so a blip cannot link a lesser model.
    Inconclusive { model: String, detail: String },
    /// Every listed model was reported unavailable.
    NoneUsable { tried: Vec<(String, ProbeFailure)> },
    /// The CLI is not signed in, so nothing was tried past the first model.
    NotSignedIn { model: String },
}

/// Try `preference` in order with `probe`, which returns the failure text on error.
/// Blank and repeated entries are ignored. Stops at once when the CLI is not signed in.
pub async fn pick_first_usable<F, Fut>(preference: &[String], mut probe: F) -> Pick
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let mut seen: Vec<&str> = Vec::new();
    let mut tried = Vec::new();
    for model in preference.iter().map(|m| m.trim()).filter(|m| !m.is_empty()) {
        if seen.contains(&model) {
            continue;
        }
        seen.push(model);
        match probe(model.to_string()).await {
            Ok(()) => {
                return Pick::Chosen {
                    model: model.to_string(),
                    skipped: tried,
                };
            }
            Err(text) => match classify_failure(&text) {
                ProbeFailure::NotSignedIn => {
                    return Pick::NotSignedIn {
                        model: model.to_string(),
                    };
                }
                ProbeFailure::Other => {
                    return Pick::Inconclusive {
                        model: model.to_string(),
                        detail: text,
                    };
                }
                other => tried.push((model.to_string(), other)),
            },
        }
    }
    Pick::NoneUsable { tried }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs() -> Vec<String> {
        vec!["gpt-6-luna".into(), "claude-haiku-5.5".into()]
    }

    /// A seat that answers on exactly the models in `have`.
    fn seat(have: &'static [&'static str]) -> impl FnMut(String) -> std::future::Ready<Result<(), String>> {
        move |m| {
            std::future::ready(if have.contains(&m.as_str()) {
                Ok(())
            } else {
                Err(format!("error: model \"{m}\" is not available for your plan"))
            })
        }
    }

    #[tokio::test]
    async fn the_first_preference_wins_when_the_seat_has_it() {
        let got = pick_first_usable(&prefs(), seat(&["gpt-6-luna", "claude-haiku-5.5"])).await;
        assert_eq!(got, Pick::Chosen { model: "gpt-6-luna".into(), skipped: vec![] });
    }

    #[tokio::test]
    async fn the_second_is_used_when_the_first_is_not_enabled() {
        let got = pick_first_usable(&prefs(), seat(&["claude-haiku-5.5"])).await;
        assert_eq!(
            got,
            Pick::Chosen {
                model: "claude-haiku-5.5".into(),
                skipped: vec![("gpt-6-luna".into(), ProbeFailure::ModelUnavailable)],
            }
        );
    }

    #[tokio::test]
    async fn a_seat_with_neither_gets_a_report_not_a_bigger_model() {
        let got = pick_first_usable(&prefs(), seat(&["gpt-9-huge"])).await;
        match got {
            Pick::NoneUsable { tried } => assert_eq!(tried.len(), 2),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn a_missing_sign_in_stops_after_one_try() {
        let mut calls = 0;
        let got = pick_first_usable(&prefs(), |_| {
            calls += 1;
            std::future::ready(Err("You are not logged in. Run gh auth login".to_string()))
        })
        .await;
        assert_eq!(got, Pick::NotSignedIn { model: "gpt-6-luna".into() });
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn blank_and_repeated_entries_are_not_tried_twice() {
        let list = vec!["".to_string(), "a".to_string(), " a ".to_string(), "b".to_string()];
        let mut seen = Vec::new();
        let _ = pick_first_usable(&list, |m| {
            seen.push(m);
            std::future::ready(Err("model not available".to_string()))
        })
        .await;
        assert_eq!(seen, ["a", "b"]);
    }

    #[tokio::test]
    async fn a_timeout_ends_the_walk_instead_of_linking_a_lesser_model() {
        let got = pick_first_usable(&prefs(), |m| {
            std::future::ready(if m == "gpt-6-luna" { Err("timed out".to_string()) } else { Ok(()) })
        })
        .await;
        assert!(matches!(got, Pick::Inconclusive { ref model, .. } if model == "gpt-6-luna"), "{got:?}");
    }

    #[test]
    fn failure_text_is_classified() {
        use ProbeFailure::*;
        assert_eq!(classify_failure("Model 'x' is not available"), ModelUnavailable);
        assert_eq!(classify_failure("error: invalid model: x"), ModelUnavailable);
        assert_eq!(classify_failure("Invalid or expired token, cannot access models"), NotSignedIn);
        assert_eq!(classify_failure("Usage: copilot [options]  unknown option --model"), Other);
        assert_eq!(classify_failure("Authentication required. Run gh auth login"), NotSignedIn);
        assert_eq!(classify_failure("connection reset by peer"), Other);
        assert_eq!(classify_failure("your organization has disabled model gpt-6-luna"), ModelUnavailable);
    }
}
