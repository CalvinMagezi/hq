//! The shared link corpus (`tests/fixtures/links_cases.json`) run against `hq_vault::links`.
//!
//! The web app and the Android app read the same file for the token fields, so a link that
//! parses one way here and another way in a renderer fails a test in one place or the other.
//! The corpus is the eval: every case has a hand-written truth, and there is no tolerance.

use hq_vault::links::{NoteInput, NoteSet, Resolution, extract};
use serde_json::Value;

const CORPUS: &str = include_str!("fixtures/links_cases.json");

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn note_set(corpus: &Value) -> NoteSet {
    let notes = corpus["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| NoteInput {
            path: str_of(n, "path").unwrap().to_string(),
            aliases: n["aliases"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        });
    let attachments = corpus["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string);
    NoteSet::new(notes, attachments)
}

#[test]
fn every_case_in_the_corpus_reads_and_resolves_as_written() {
    let corpus: Value = serde_json::from_str(CORPUS).expect("corpus is valid JSON");
    let set = note_set(&corpus);
    let mut checked = 0;
    let mut failures: Vec<String> = Vec::new();

    for case in corpus["cases"].as_array().unwrap() {
        let name = str_of(case, "name").unwrap();
        let from = str_of(case, "from").unwrap();
        let text = str_of(case, "text").unwrap();
        let want = case["links"].as_array().unwrap();
        let got = extract(text);

        if got.len() != want.len() {
            let found: Vec<&str> = got.iter().map(|l| l.target.as_str()).collect();
            failures.push(format!(
                "{name}: expected {} links, found {found:?}",
                want.len()
            ));
            continue;
        }
        for (link, truth) in got.iter().zip(want) {
            checked += 1;
            let mut problems = Vec::new();
            if link.target != str_of(truth, "target").unwrap() {
                problems.push(format!("target {:?}", link.target));
            }
            if link.heading.as_deref() != str_of(truth, "heading") {
                problems.push(format!("heading {:?}", link.heading));
            }
            if link.block.as_deref() != str_of(truth, "block") {
                problems.push(format!("block {:?}", link.block));
            }
            if link.alias.as_deref() != str_of(truth, "alias") {
                problems.push(format!("alias {:?}", link.alias));
            }
            if link.embed != truth["embed"].as_bool().unwrap_or(false) {
                problems.push(format!("embed {}", link.embed));
            }
            if link.in_frontmatter != truth["in_frontmatter"].as_bool().unwrap_or(false) {
                problems.push(format!("in_frontmatter {}", link.in_frontmatter));
            }
            let resolved = set.resolve(from, link);
            let ok = match (str_of(truth, "resolution").unwrap(), &resolved) {
                ("unique", Resolution::Unique(p)) => Some(p.as_str()) == str_of(truth, "path"),
                ("attachment", Resolution::Attachment(p)) => {
                    Some(p.as_str()) == str_of(truth, "path")
                }
                ("ambiguous", Resolution::Ambiguous(ps)) => {
                    let mut want: Vec<&str> = truth["paths"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(Value::as_str)
                        .collect();
                    want.sort_unstable();
                    let mut have: Vec<&str> = ps.iter().map(String::as_str).collect();
                    have.sort_unstable();
                    want == have
                }
                ("external", Resolution::External)
                | ("same_note", Resolution::SameNote)
                | ("dangling", Resolution::Dangling) => true,
                _ => false,
            };
            if !ok {
                problems.push(format!("resolved {resolved:?}"));
            }
            if !problems.is_empty() {
                failures.push(format!(
                    "{name}: {} -> {}",
                    link.target,
                    problems.join(", ")
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {checked} links wrong:\n{}",
        failures.len(),
        failures.join("\n")
    );
    println!("link corpus: {checked} links checked, all as written");
}

/// Links in code and in broken syntax are the ones a rewrite must never touch. This reads the
/// corpus for them: no case may report a link whose text sits in a code span.
#[test]
fn nothing_inside_code_is_ever_reported() {
    let corpus: Value = serde_json::from_str(CORPUS).unwrap();
    for case in corpus["cases"].as_array().unwrap() {
        let text = str_of(case, "text").unwrap();
        for link in extract(text) {
            assert!(
                link.target != "Nope",
                "{}: a link that must be ignored was reported",
                str_of(case, "name").unwrap()
            );
        }
    }
}
