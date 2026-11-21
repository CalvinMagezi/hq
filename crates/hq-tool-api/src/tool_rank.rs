//! Offline ranking of tools against a free-text query.
//!
//! One small scorer shared by every place a model (or a hint) has to find a tool: native
//! `tool_search`, the MCP gateway's `hq_discover` and the per-turn "likely tools" hint. No LLM,
//! no network, no usage data: the score depends only on the query and the catalogue text, so the
//! same inputs always give the same order.
//!
//! Scoring is a small BM25-flavoured sum. Each query word is stemmed, optionally widened by a
//! short synonym table, and matched against the tool's name, keywords, hint and description. A
//! word that appears in few tools counts for more than one that appears in many.

use std::collections::{HashMap, HashSet};

/// The text of one tool the ranker looks at. Borrowed, so ranking allocates no copies of it.
#[derive(Debug, Clone, Copy)]
pub struct RankDoc<'a> {
    pub name: &'a str,
    pub hint: &'a str,
    pub description: &'a str,
    pub keywords: &'a [&'a str],
}

/// Words too common to say anything about what a person wants.
const STOP_WORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "into", "you", "are", "need", "want",
    "can", "please", "have", "has", "was", "will", "would", "could", "should", "how", "what",
    "when", "any", "all", "get", "use", "using", "make", "help",
];

/// Everyday words mapped to the words tool names and hints use. Kept short and generic: each
/// entry is a way people say a thing that the catalogue says differently. The expansion counts
/// for less than the word the person actually typed.
const SYNONYMS: &[(&str, &[&str])] = &[
    ("spreadsheet", &["sheet", "office", "table"]),
    ("excel", &["sheet", "office"]),
    ("presentation", &["deck", "slides", "office"]),
    ("slides", &["deck", "presentation"]),
    ("remind", &["reminder", "task", "schedule"]),
    ("todo", &["task"]),
    ("email", &["mail", "gmail", "message"]),
    ("pdf", &["convert", "export", "document"]),
    ("document", &["doc", "file", "office"]),
    ("note", &["vault", "notes"]),
    ("notes", &["vault", "note"]),
    ("remember", &["memory", "vault"]),
    ("google", &["search", "web"]),
    ("website", &["web", "fetch", "url"]),
    ("internet", &["web", "search"]),
    ("shrink", &["compress", "smaller"]),
    ("photo", &["image", "picture"]),
    ("picture", &["image", "photo"]),
    ("calendar", &["event", "schedule"]),
    ("meeting", &["calendar", "event"]),
];

/// Weight of a word the person typed versus one the synonym table added.
const TYPED: f32 = 1.0;
const WIDENED: f32 = 0.5;

/// Where in a tool a word was found, and what that is worth.
const IN_NAME: f32 = 3.0;
const IN_KEYWORDS: f32 = 2.5;
const IN_HINT: f32 = 2.0;
const IN_DESCRIPTION: f32 = 1.0;

/// Longest slice of a description the ranker reads, so a long description does not outweigh a
/// name that fits.
const DESCRIPTION_CHARS: usize = 200;

/// Cut a word to a rough stem: enough that "exports" and "exporting" meet "export", not a full
/// stemmer. Words of four letters or fewer are left alone.
fn stem(word: &str) -> String {
    let w = word;
    for suffix in ["ing", "ed", "es", "s"] {
        if w.chars().count() > 4
            && let Some(rest) = w.strip_suffix(suffix)
            && rest.chars().count() >= 3
        {
            return rest.to_string();
        }
    }
    w.to_string()
}

/// Split text into lowercase stems of three or more characters, in order, without stop words.
/// `snake_case` and `kebab-case` names split on their separators like any other text.
pub fn terms(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3 && !STOP_WORDS.contains(w))
        .map(stem)
        .collect()
}

fn token_set(text: &str) -> HashSet<String> {
    terms(text).into_iter().collect()
}

/// The query's words and their weights: typed words first, then any widened by the synonym table.
fn weighted_query(query: &str) -> Vec<(String, f32)> {
    let typed: Vec<String> = {
        let mut t = terms(query);
        t.sort();
        t.dedup();
        t
    };
    let mut out: Vec<(String, f32)> = typed.iter().map(|t| (t.clone(), TYPED)).collect();
    let typed_set: HashSet<&String> = typed.iter().collect();
    let raw: HashSet<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_string)
        .collect();
    let mut widened: Vec<String> = Vec::new();
    for (word, extra) in SYNONYMS {
        if raw.contains(*word) || typed_set.contains(&stem(word)) {
            widened.extend(extra.iter().map(|e| stem(e)));
        }
    }
    widened.sort();
    widened.dedup();
    for w in widened {
        if !typed_set.contains(&w) {
            out.push((w, WIDENED));
        }
    }
    out
}

struct Indexed {
    name: HashSet<String>,
    keywords: HashSet<String>,
    hint: HashSet<String>,
    description: HashSet<String>,
}

fn index(doc: &RankDoc<'_>) -> Indexed {
    let desc: String = doc.description.chars().take(DESCRIPTION_CHARS).collect();
    Indexed {
        name: token_set(doc.name),
        keywords: doc.keywords.iter().flat_map(|k| terms(k)).collect(),
        hint: token_set(doc.hint),
        description: token_set(&desc),
    }
}

/// Rank `docs` against `query`. Returns indices into `docs`, best first, at most `max_results`.
/// Documents that match no word are left out. Ties keep the order of `docs`, so a caller that
/// passes a sorted catalogue gets a stable result.
pub fn rank(docs: &[RankDoc<'_>], query: &str, max_results: usize) -> Vec<usize> {
    rank_scored(docs, query, max_results)
        .into_iter()
        .map(|(i, _)| i)
        .collect()
}

/// Like [`rank`], with each document's score, so a caller can apply a threshold.
pub fn rank_scored(docs: &[RankDoc<'_>], query: &str, max_results: usize) -> Vec<(usize, f32)> {
    let q = weighted_query(query);
    if q.is_empty() || docs.is_empty() || max_results == 0 {
        return Vec::new();
    }
    let indexed: Vec<Indexed> = docs.iter().map(index).collect();

    // How many tools mention each query word anywhere, for the rarity weight.
    let mut df: HashMap<&str, usize> = HashMap::new();
    for (term, _) in &q {
        let n = indexed
            .iter()
            .filter(|d| {
                d.name.contains(term)
                    || d.keywords.contains(term)
                    || d.hint.contains(term)
                    || d.description.contains(term)
            })
            .count();
        df.insert(term.as_str(), n);
    }
    let total = docs.len() as f32;

    let mut scored: Vec<(usize, f32)> = indexed
        .iter()
        .enumerate()
        .filter_map(|(i, d)| {
            let mut score = 0.0_f32;
            for (term, weight) in &q {
                let place = if d.name.contains(term) {
                    IN_NAME
                } else if d.keywords.contains(term) {
                    IN_KEYWORDS
                } else if d.hint.contains(term) {
                    IN_HINT
                } else if d.description.contains(term) {
                    IN_DESCRIPTION
                } else {
                    continue;
                };
                let n = df.get(term.as_str()).copied().unwrap_or(1).max(1) as f32;
                let idf = (1.0 + total / n).ln();
                // A word that is also in the hint adds a little on top of a name match, so the
                // tool that names and describes it beats the one that only names it.
                let bonus = if place == IN_NAME && d.hint.contains(term) {
                    0.5
                } else {
                    0.0
                };
                score += weight * (place + bonus) * idf;
            }
            (score > 0.0).then_some((i, score))
        })
        .collect();
    // Stable sort: equal scores keep catalogue order.
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(max_results);
    scored
}

/// Edit distance between two short strings, counting single-character inserts, deletes and
/// replacements. Names are a few dozen characters, so the plain two-row version is plenty.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// The names closest to a name that does not exist, best first, at most `n`. Used for "did you
/// mean": a typo (`vault_serach`) is near by edit distance, and a guess in other words
/// (`search_vault`, `find_notes`) shares words with the real name. Returns indices into `names`;
/// a name with neither a small edit distance nor a shared word is left out.
pub fn nearest(names: &[&str], wanted: &str, n: usize) -> Vec<usize> {
    let wanted_lower = wanted.trim().to_lowercase();
    if wanted_lower.is_empty() || n == 0 {
        return Vec::new();
    }
    let wanted_terms = token_set(&wanted_lower);
    let close = (wanted_lower.chars().count() / 3).max(2);
    let mut found: Vec<(usize, usize, usize, usize)> = names
        .iter()
        .enumerate()
        .filter_map(|(i, name)| {
            let lower = name.to_lowercase();
            let dist = edit_distance(&wanted_lower, &lower);
            let shared = token_set(&lower).intersection(&wanted_terms).count();
            let is_close = dist <= close;
            (is_close || shared > 0).then_some((
                usize::from(!is_close),
                dist,
                usize::MAX - shared,
                i,
            ))
        })
        .collect();
    // Close spellings first, then smaller distance, then more shared words, then catalogue order.
    found.sort();
    found.into_iter().take(n).map(|(_, _, _, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc<'a>(name: &'a str, hint: &'a str) -> RankDoc<'a> {
        RankDoc {
            name,
            hint,
            description: "",
            keywords: &[],
        }
    }

    fn catalog() -> Vec<RankDoc<'static>> {
        vec![
            doc("vault_inventory", "List notes in the vault"),
            doc("office_create", "Make a spreadsheet, document or dashboard"),
            doc("office_export", "Export an office file"),
            doc("web_fetch", "Fetch a page"),
            doc("task_create", "Add a task or reminder"),
        ]
    }

    fn names(q: &str, n: usize) -> Vec<&'static str> {
        let docs = catalog();
        rank(&docs, q, n)
            .into_iter()
            .map(|i| docs[i].name)
            .collect()
    }

    #[test]
    fn a_long_sentence_finds_the_tool_it_describes() {
        let got = names(
            "I need to make a spreadsheet and a dashboard for the team",
            5,
        );
        assert_eq!(got.first().copied(), Some("office_create"));
    }

    #[test]
    fn a_name_and_hint_match_beats_a_name_only_match() {
        assert_eq!(names("office", 5), vec!["office_export", "office_create"]);
        assert_eq!(names("export office file", 5)[0], "office_export");
    }

    #[test]
    fn everyday_words_reach_the_tool_through_the_synonym_table() {
        assert_eq!(
            names("remind me tomorrow", 5).first().copied(),
            Some("task_create")
        );
        assert!(names("excel", 5).iter().all(|n| n.starts_with("office_")));
        assert_eq!(names("excel", 5).len(), 2);
    }

    #[test]
    fn plural_and_ing_forms_meet_the_base_word() {
        assert_eq!(
            names("exporting files", 5).first().copied(),
            Some("office_export")
        );
        assert_eq!(names("notes", 5).first().copied(), Some("vault_inventory"));
    }

    #[test]
    fn the_cap_holds_and_nonsense_or_empty_matches_nothing() {
        assert_eq!(names("office", 1).len(), 1);
        assert!(names("zzzz qqqq", 5).is_empty());
        assert!(names("", 5).is_empty());
        assert!(names("office", 0).is_empty());
    }

    #[test]
    fn ties_keep_catalogue_order_so_results_are_stable() {
        let docs = vec![
            doc("alpha_thing", "does widgets"),
            doc("beta_thing", "does widgets"),
        ];
        assert_eq!(rank(&docs, "widgets", 5), vec![0, 1]);
    }

    #[test]
    fn a_rare_word_outweighs_a_common_one() {
        let docs = vec![
            doc("a_tool", "work with files"),
            doc("b_tool", "work with files"),
            doc("c_tool", "work with files and invoices"),
        ];
        assert_eq!(rank(&docs, "files invoices", 3)[0], 2);
    }

    #[test]
    fn keywords_and_description_are_searched() {
        let kw: &[&str] = &["spreadsheet"];
        let docs = vec![
            doc("zzz_one", "does something"),
            RankDoc {
                name: "zzz_two",
                hint: "does something",
                description: "Reads receipts from a scanned photo",
                keywords: kw,
            },
        ];
        assert_eq!(rank(&docs, "spreadsheet", 5), vec![1]);
        assert_eq!(rank(&docs, "receipts", 5), vec![1]);
    }

    #[test]
    fn nearest_catches_a_typo_and_a_reworded_name() {
        let names = [
            "vault_search",
            "vault_write_note",
            "web_fetch",
            "task_create",
        ];
        let pick = |wanted: &str| -> Vec<&str> {
            nearest(&names, wanted, 3)
                .into_iter()
                .map(|i| names[i])
                .collect()
        };
        assert_eq!(pick("vault_serach")[0], "vault_search");
        assert_eq!(pick("search_vault")[0], "vault_search");
        assert_eq!(pick("fetch_web")[0], "web_fetch");
        assert!(pick("completely_unrelated_thing").is_empty());
        assert!(nearest(&names, "", 3).is_empty());
        assert_eq!(nearest(&names, "vault", 1).len(), 1);
    }
}
