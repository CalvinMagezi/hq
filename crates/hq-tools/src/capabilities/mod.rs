//! What HQ can do, in the person's words, and which of it works on this install.
//!
//! One list (`assets/capabilities.yaml`, embedded in the binary) feeds every place HQ shows
//! itself: the empty chat on the web and phone apps (`/api/capabilities`), the agent's short
//! block of things it can offer, and the weekly adoption digest. Each entry is resolved against
//! the running install, so a person is only ever shown what works here, or what one step away
//! would make work.
//!
//! No language model is involved anywhere in this module. `next_step` ranks invented examples
//! with the on-device embedder.
//!
//! Which capabilities the person has used is adoption data, and it stays on the device: it orders
//! the chips the owner's own apps show and fills the weekly digest in their inbox, and nothing
//! else. The block a model sees ([`prompt_block`]) is built from the catalog and the install
//! alone and takes no usage at all, so no provider learns what the person does or does not use.
//! `docs/security/ADOPTION_DATA.md`.

pub mod adoption;
pub mod next_step;
pub mod offers;

use std::collections::HashSet;
use std::sync::LazyLock;

use hq_core::config::HqConfig;
use serde::{Deserialize, Serialize};

const CATALOG_YAML: &str = include_str!("../../assets/capabilities.yaml");

/// What an entry needs from this install besides its tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Need {
    /// A model to chat with: every capability is reached through a chat turn.
    Model,
    /// OCR on this machine (`tesseract`, or the Swift toolchain on macOS).
    Ocr,
    /// The `gws` command for Google Workspace.
    Email,
    /// Somewhere to run a coding agent: this machine (not a phone), or a configured host.
    Helpers,
}

/// One thing HQ can do for the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub id: String,
    pub group: String,
    pub says: String,
    pub example: String,
    pub tools: Vec<String>,
    #[serde(default)]
    pub needs: Vec<Need>,
    /// Worth mentioning to the model as something to offer. Off for what any chat assistant does
    /// unasked (saving and finding notes), so the block stays short.
    #[serde(default = "offered_by_default")]
    pub offer: bool,
}

fn offered_by_default() -> bool {
    true
}

fn parse_catalog(yaml: &str) -> Result<Vec<Capability>, serde_yaml::Error> {
    serde_yaml::from_str(yaml)
}

static CATALOG: LazyLock<Vec<Capability>> = LazyLock::new(|| {
    parse_catalog(CATALOG_YAML).unwrap_or_else(|e| {
        // A test parses the embedded list, so this is a broken build, not a broken install.
        tracing::error!("capabilities: the embedded catalog does not parse: {e}");
        Vec::new()
    })
});

/// The whole list, in suggestion order.
pub fn catalog() -> &'static [Capability] {
    &CATALOG
}

/// What this install has, for the needs that are not tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Install {
    pub model: bool,
    pub ocr: bool,
    pub email: bool,
    pub helpers: bool,
}

impl Install {
    /// Looks at the config and the machine. Cheap: a config read and two `PATH` lookups.
    pub fn detect(config: &HqConfig) -> Self {
        Self {
            model: config.has_llm_key(),
            ocr: hq_convert::OcrEngine::available(),
            email: which::which(hq_core::paths::resolve_gws_binary()).is_ok(),
            helpers: !config.profile.is_mobile() || !config.agent_host.hosts.is_empty(),
        }
    }

    /// The same install with a model taken as given. Reports about use need it: someone who
    /// chats with HQ has a model, and a command run outside the app (over `adb`, say) may not
    /// see the key the app hands HQ, which would make every capability look unavailable.
    pub fn assuming_model(self) -> Self {
        Self {
            model: true,
            ..self
        }
    }

    fn has(&self, need: Need) -> bool {
        match need {
            Need::Model => self.model,
            Need::Ocr => self.ocr,
            Need::Email => self.email,
            Need::Helpers => self.helpers,
        }
    }
}

/// The one setup step the web and phone apps can take the person through.
pub const MODEL_STEP: &str = "Add an AI service key in Settings";

/// Whether a capability works here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum Readiness {
    Ready,
    /// One step, which the person can take from the app, would make it work.
    OneStep {
        step: &'static str,
    },
    /// Not on this install; never shown.
    Unavailable,
}

fn readiness(cap: &Capability, install: &Install, has_tool: &dyn Fn(&str) -> bool) -> Readiness {
    if !cap.tools.iter().all(|t| has_tool(t)) {
        return Readiness::Unavailable;
    }
    let missing: Vec<Need> = cap
        .needs
        .iter()
        .copied()
        .filter(|n| !install.has(*n))
        .collect();
    match missing.as_slice() {
        [] => Readiness::Ready,
        [Need::Model] => Readiness::OneStep { step: MODEL_STEP },
        _ => Readiness::Unavailable,
    }
}

/// A capability as this install and this person see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Offer {
    pub id: String,
    pub group: String,
    pub says: String,
    pub example: String,
    #[serde(flatten)]
    pub readiness: Readiness,
    /// The person's own sessions have used one of its tools at least once.
    pub used: bool,
    #[serde(skip)]
    pub tools: Vec<String>,
    #[serde(skip)]
    pub offer: bool,
}

impl Offer {
    pub fn is_ready(&self) -> bool {
        self.readiness == Readiness::Ready
    }
}

/// Every entry of `catalog`, resolved. `has_tool` says whether a tool is registered for this
/// install; `used_tools` holds the tools the owner's own sessions have called successfully.
pub fn resolve(
    catalog: &[Capability],
    install: &Install,
    has_tool: &dyn Fn(&str) -> bool,
    used_tools: &HashSet<String>,
) -> Vec<Offer> {
    catalog
        .iter()
        .map(|cap| Offer {
            id: cap.id.clone(),
            group: cap.group.clone(),
            says: cap.says.clone(),
            example: cap.example.clone(),
            readiness: readiness(cap, install, has_tool),
            used: cap.tools.iter().any(|t| used_tools.contains(t)),
            tools: cap.tools.clone(),
            offer: cap.offer,
        })
        .collect()
}

/// The catalog's tools that the owner's own sessions have ever called successfully.
pub fn used_tools(db: &hq_db::Database) -> HashSet<String> {
    let tools: Vec<&str> = catalog()
        .iter()
        .flat_map(|c| c.tools.iter().map(String::as_str))
        .collect();
    db.with_conn(|c| hq_db::adoption::owner_has_used(c, &tools))
        .map(|v| v.into_iter().collect())
        .unwrap_or_default()
}

/// What to put on an empty chat: ready capabilities the person has not tried first, then the
/// ones they use, each group in catalog order.
pub fn suggestions(offers: &[Offer], n: usize) -> Vec<&Offer> {
    let ready = offers.iter().filter(|o| o.is_ready());
    ready
        .clone()
        .filter(|o| !o.used)
        .chain(ready.filter(|o| o.used))
        .take(n)
        .collect()
}

/// Lines in the agent's block, and its size in bytes. A phone sends this with every turn.
const BLOCK_MAX_LINES: usize = 8;
pub const BLOCK_MAX_BYTES: usize = 1024;

const BLOCK_HEAD: &str = "## Things you can offer\n\
HQ can also do these on this install. When one would clearly help with what the person is doing \
and they do not seem to know about it, offer it in one short sentence at the end of your reply, \
at most one per reply. Never list them.\n";

/// The block of capabilities the model may offer: what is ready on this install, in catalog
/// order. It takes no usage on purpose. What the person has or has not used never reaches a
/// model provider; choosing by usage happens only on the device (the chips, the digest).
pub fn prompt_block(
    catalog: &[Capability],
    install: &Install,
    has_tool: &dyn Fn(&str) -> bool,
) -> Option<String> {
    let mut block = String::from(BLOCK_HEAD);
    let mut lines = 0;
    let ready = catalog
        .iter()
        .filter(|c| c.offer && readiness(c, install, has_tool) == Readiness::Ready);
    for c in ready {
        let line = format!("- {} ({})\n", c.says, c.tools.join(", "));
        if lines == BLOCK_MAX_LINES || block.len() + line.len() > BLOCK_MAX_BYTES {
            break;
        }
        block.push_str(&line);
        lines += 1;
    }
    (lines > 0).then_some(block)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pill shows `says` as its label and `example` as the text it puts in the composer. Both
    /// have to fit a pill on a small phone.
    const MAX_PILL_TEXT_CHARS: usize = 40;

    #[test]
    fn what_a_pill_shows_fits_in_forty_characters() {
        let too_long: Vec<String> = catalog()
            .iter()
            .flat_map(|c| [("says", &c.says), ("example", &c.example)].map(|(f, v)| (c, f, v)))
            .filter(|(_, _, v)| v.chars().count() > MAX_PILL_TEXT_CHARS)
            .map(|(c, f, v)| format!("{}.{f} ({} chars): {v}", c.id, v.chars().count()))
            .collect();
        assert!(
            too_long.is_empty(),
            "shorten in capabilities.yaml: {too_long:#?}"
        );
    }

    fn all_tools(_: &str) -> bool {
        true
    }

    fn everything() -> Install {
        Install {
            model: true,
            ocr: true,
            email: true,
            helpers: true,
        }
    }

    #[test]
    fn the_embedded_catalog_parses_with_unique_ids_and_plain_words() {
        let cat = parse_catalog(CATALOG_YAML).expect("assets/capabilities.yaml parses");
        assert!(cat.len() >= 10);
        assert_eq!(catalog().len(), cat.len());
        let ids: HashSet<&str> = cat.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids.len(), cat.len(), "ids are unique");
        for c in &cat {
            assert!(!c.tools.is_empty(), "{} names its tools", c.id);
            assert!(
                c.needs.contains(&Need::Model),
                "{} is reached through chat",
                c.id
            );
            for t in &c.tools {
                assert!(
                    !c.says.contains(t.as_str()) && !c.example.contains(t.as_str()),
                    "{} shows the person a tool name",
                    c.id
                );
            }
            assert!(!c.says.contains('\u{2014}'), "{} uses an em dash", c.id);
        }
    }

    #[test]
    fn readiness_follows_tools_and_needs() {
        let cap = |needs: Vec<Need>| Capability {
            id: "x".into(),
            group: "notes".into(),
            says: "Do x".into(),
            example: "x".into(),
            tools: vec!["t".into()],
            needs,
            offer: true,
        };
        let no_model = Install {
            model: false,
            ..everything()
        };
        assert_eq!(
            readiness(&cap(vec![Need::Model]), &everything(), &all_tools),
            Readiness::Ready
        );
        assert_eq!(
            readiness(&cap(vec![Need::Model]), &no_model, &all_tools),
            Readiness::OneStep { step: MODEL_STEP }
        );
        assert_eq!(
            readiness(&cap(vec![Need::Model]), &everything(), &|_: &str| false),
            Readiness::Unavailable,
            "a tool this install does not register hides it"
        );
        let no_ocr = Install {
            ocr: false,
            ..no_model
        };
        assert_eq!(
            readiness(&cap(vec![Need::Model, Need::Ocr]), &no_ocr, &all_tools),
            Readiness::Unavailable,
            "only the model key is a step the app can take the person through"
        );
    }

    #[test]
    fn suggestions_put_untried_things_first_and_skip_what_does_not_work() {
        let used: HashSet<String> = ["vault_write_note".to_string()].into();
        let install = Install {
            ocr: false,
            ..everything()
        };
        let offers = resolve(catalog(), &install, &all_tools, &used);
        let picked = suggestions(&offers, 3);
        assert_eq!(picked.len(), 3);
        assert!(picked.iter().all(|o| o.is_ready() && !o.used));
        assert!(picked.iter().all(|o| o.id != "read-photo"));
        let all = suggestions(&offers, offers.len());
        assert_eq!(all.last().map(|o| o.id.as_str()), Some("save-note"));
    }

    #[test]
    fn the_model_block_is_small_and_says_nothing_about_use() {
        let block = prompt_block(catalog(), &everything(), &all_tools).unwrap();
        assert!(block.len() <= BLOCK_MAX_BYTES);
        assert!(block.matches("\n- ").count() <= BLOCK_MAX_LINES);
        assert!(block.contains("(vault_import)"));
        assert!(
            !block.contains("(vault_write_note)"),
            "saving notes goes without saying"
        );
        assert!(!block.to_lowercase().contains("not used"));
        assert!(!block.to_lowercase().contains("tried"));
        // Nothing to offer without a model key.
        let no_model = Install {
            model: false,
            ..everything()
        };
        assert_eq!(prompt_block(catalog(), &no_model, &all_tools), None);
    }

    #[test]
    fn an_offer_serializes_flat_for_the_apps() {
        let offers = resolve(catalog(), &Install::default(), &all_tools, &HashSet::new());
        let v = serde_json::to_value(&offers[0]).unwrap();
        assert_eq!(v["state"], "one_step");
        assert_eq!(v["step"], MODEL_STEP);
        assert_eq!(v["used"], false);
        assert!(v.get("tools").is_none());
    }
}
