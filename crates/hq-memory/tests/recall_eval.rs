//! Recall quality: does HQ bring back the right memories for what was just said?
//!
//! A fixed corpus of memories (a fictional small-shop owner, placeholder names only) and a set of
//! messages with the memories each one should and should not bring back. A `Policy` is a way of
//! choosing memories for a message; every policy is scored the same way, so a change to recall is
//! judged against numbers instead of a feeling.
//!
//! `Policy::Recent` is what HQ does today: the highest-importance rows, whatever the message says.
//! It is the baseline every other policy has to beat.
//!
//! Run it with the report printed:
//!
//! ```text
//! cargo test -p hq-memory --test recall_eval -- --nocapture
//! HQ_EVAL_REPORT=/tmp/recall.md cargo test -p hq-memory --test recall_eval -- --nocapture
//! ```
//!
//! Fixtures are in `tests/fixtures/recall/`. A case has a `split` (`dev` to tune on, `test` to
//! gate on), a `kind`, and a row has a `lang` so other languages can be added as data later.

use hq_db::Database;
use hq_memory::querier::MemoryQuerier;
use hq_memory::{StoreMemoryParams, store_memory};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

const CORPUS: &str = include_str!("fixtures/recall/corpus.jsonl");
const CASES: &str = include_str!("fixtures/recall/cases.jsonl");

/// What the prompt block holds today, so the baseline matches the real thing.
const RECENT_LIMIT: i64 = 8;

#[derive(Debug, Deserialize)]
struct CorpusRow {
    id: String,
    summary: String,
    entities: Vec<String>,
    topics: Vec<String>,
    age_days: i64,
    importance: f64,
    /// `owner`, or `unknown_channel` for a legacy row that cannot be attributed.
    origin: String,
    /// `active`, or `superseded` when a newer row replaced it.
    status: String,
    #[allow(dead_code)]
    lang: String,
}

#[derive(Debug, Deserialize)]
struct Case {
    id: String,
    split: String,
    kind: String,
    /// Unused by `Policy::Recent`, which ignores what was said. Every policy that follows reads it.
    #[allow(dead_code)]
    message: String,
    #[allow(dead_code)]
    prior_message: Option<String>,
    /// `owner`, or `guest` for anyone else asking.
    identity: String,
    expect: Vec<String>,
    forbid: Vec<String>,
    #[allow(dead_code)]
    acceptable: Vec<String>,
    expect_silence: bool,
}

fn lines<T: for<'de> Deserialize<'de>>(text: &str) -> Vec<T> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad fixture line {l}: {e}")))
        .collect()
}

fn corpus() -> Vec<CorpusRow> {
    lines(CORPUS)
}

fn cases() -> Vec<Case> {
    lines(CASES)
}

/// The corpus loaded into a database, with a map from row ids back to fixture ids.
struct World {
    db: Database,
    fixture_of: HashMap<i64, String>,
    rows: HashMap<String, CorpusRow>,
}

impl World {
    fn build() -> Self {
        let db = Database::open_memory().expect("in-memory database");
        let mut fixture_of = HashMap::new();
        let mut rows = HashMap::new();
        for row in corpus() {
            let id = store_memory(
                &db,
                &StoreMemoryParams {
                    source: "chat-turn",
                    harness: "web",
                    raw_text: "",
                    summary: &row.summary,
                    entities: &row.entities,
                    topics: &row.topics,
                    importance: row.importance,
                    user_id: None,
                },
            )
            .expect("store memory");
            let at = (chrono::Utc::now() - chrono::Duration::days(row.age_days)).to_rfc3339();
            db.with_conn(|c| {
                c.execute(
                    "UPDATE memories SET created_at = ?1 WHERE id = ?2",
                    rusqlite::params![at, id],
                )?;
                Ok(())
            })
            .expect("backdate memory");
            fixture_of.insert(id, row.id.clone());
            rows.insert(row.id.clone(), row);
        }
        Self {
            db,
            fixture_of,
            rows,
        }
    }
}

/// A way of choosing memories for a message. New policies are added here and scored unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Policy {
    /// Today: the most important rows, the same ones whatever was said.
    Recent,
}

impl Policy {
    fn name(self) -> &'static str {
        match self {
            Policy::Recent => "recent (today)",
        }
    }
}

struct Retrieved {
    /// Fixture ids, best first.
    ids: Vec<String>,
    /// The text that would go to the model.
    block: String,
    elapsed: Duration,
}

fn retrieve(policy: Policy, world: &World, case: &Case) -> Retrieved {
    let started = Instant::now();
    let (ids, block) = match policy {
        Policy::Recent => {
            // The session builder gives anyone but the owner no memory at all.
            if case.identity != "owner" {
                (Vec::new(), String::new())
            } else {
                let ctx = MemoryQuerier::new(world.db.clone())
                    .get_recent_context(Some(RECENT_LIMIT), None)
                    .expect("recent context");
                let ids = ctx
                    .memories
                    .iter()
                    .map(|m| world.fixture_of[&m.id].clone())
                    .collect();
                (ids, ctx.formatted)
            }
        }
    };
    Retrieved {
        ids,
        block,
        elapsed: started.elapsed(),
    }
}

#[derive(Default, Debug)]
struct Scores {
    /// Cases that name memories to find.
    asked: usize,
    at3: usize,
    at5: usize,
    found: usize,
    reciprocal_rank: f64,
    /// Of those, cases where an outdated or forbidden memory came back too.
    stale: usize,
    /// Cases where nothing should come back.
    quiet_expected: usize,
    /// Of those, cases where something came back anyway.
    false_injection: usize,
    /// Memories returned to someone who is not the owner.
    guest_leak: usize,
    /// Legacy unattributable rows returned to the owner.
    unattributed_returned: usize,
    /// Cases (any kind) where a row that tries to break out of its fence came back.
    hostile_returned: usize,
    owner_cases: usize,
    tokens: Vec<usize>,
    micros: Vec<u128>,
}

fn is_hostile(row: &CorpusRow) -> bool {
    row.summary.contains("</vault_note>") || row.summary.contains("END_UNTRUSTED")
}

fn score(policy: Policy, world: &World, cases: &[&Case]) -> Scores {
    let mut s = Scores::default();
    for case in cases {
        let got = retrieve(policy, world, case);
        s.micros.push(got.elapsed.as_micros());
        if case.identity != "owner" {
            s.guest_leak += got.ids.len();
            continue;
        }
        s.owner_cases += 1;
        s.tokens.push(got.block.len() / 4);
        s.unattributed_returned += got
            .ids
            .iter()
            .filter(|id| world.rows[*id].origin != "owner")
            .count();
        if got.ids.iter().any(|id| is_hostile(&world.rows[id])) {
            s.hostile_returned += 1;
        }
        if case.kind == "hostile" {
            continue; // fence integrity is scored once there is a fence to check
        }
        if case.expect_silence {
            s.quiet_expected += 1;
            if !got.ids.is_empty() {
                s.false_injection += 1;
            }
        }
        if case.expect.is_empty() {
            continue;
        }
        s.asked += 1;
        let wanted: HashSet<&String> = case.expect.iter().collect();
        if let Some(rank) = got.ids.iter().position(|id| wanted.contains(id)) {
            s.found += 1;
            s.reciprocal_rank += 1.0 / (rank as f64 + 1.0);
            if rank < 3 {
                s.at3 += 1;
            }
            if rank < 5 {
                s.at5 += 1;
            }
        }
        let outdated = got
            .ids
            .iter()
            .any(|id| case.forbid.contains(id) || world.rows[id].status == "superseded");
        if outdated {
            s.stale += 1;
        }
    }
    s
}

fn percentile<T: Copy + Ord + Default>(values: &[T], p: f64) -> T {
    if values.is_empty() {
        return T::default();
    }
    let mut sorted = values.to_vec();
    sorted.sort();
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx]
}

fn pct(n: usize, of: usize) -> String {
    if of == 0 {
        "n/a".into()
    } else {
        format!("{:.0}%", 100.0 * n as f64 / of as f64)
    }
}

fn report(policy: Policy, slices: &[(&str, &Scores)]) -> String {
    let mut out = format!("### Recall: {}\n\n", policy.name());
    out.push_str(
        "| slice | asked | recall@3 | recall@5 | found | MRR | outdated returned | should be quiet | spoke anyway | tokens p50/p95 | µs p50/p95 |\n",
    );
    out.push_str("|---|---|---|---|---|---|---|---|---|---|---|\n");
    for (name, s) in slices {
        let mrr = if s.asked == 0 {
            "n/a".to_string()
        } else {
            format!("{:.2}", s.reciprocal_rank / s.asked as f64)
        };
        out.push_str(&format!(
            "| {name} | {} | {} | {} | {} | {mrr} | {} | {} | {} | {}/{} | {}/{} |\n",
            s.asked,
            pct(s.at3, s.asked),
            pct(s.at5, s.asked),
            pct(s.found, s.asked),
            pct(s.stale, s.asked),
            s.quiet_expected,
            pct(s.false_injection, s.quiet_expected),
            percentile(&s.tokens, 0.5),
            percentile(&s.tokens, 0.95),
            percentile(&s.micros, 0.5),
            percentile(&s.micros, 0.95),
        ));
    }
    let all = slices[0].1;
    out.push_str(&format!(
        "\nGuest answers carrying memories: {}. Unattributable rows returned to the owner: {}. \
         Owner cases that injected a row containing fence-like text: {} of {}. (This counts \
         presence only. It becomes a pass or fail once recall escapes such text.)\n",
        all.guest_leak, all.unattributed_returned, all.hostile_returned, all.owner_cases
    ));
    out
}

fn run(policy: Policy, world: &World, all: &[Case]) -> String {
    let by =
        |pred: &dyn Fn(&Case) -> bool| -> Vec<&Case> { all.iter().filter(|c| pred(c)).collect() };
    let everything = score(policy, world, &by(&|_| true));
    let test = score(policy, world, &by(&|c| c.split == "test"));
    let dev = score(policy, world, &by(&|c| c.split == "dev"));
    let mut per_kind: BTreeMap<String, Scores> = BTreeMap::new();
    for kind in ["positive", "paraphrase", "stale", "followup", "negative"] {
        per_kind.insert(kind.into(), score(policy, world, &by(&|c| c.kind == kind)));
    }
    let mut slices: Vec<(&str, &Scores)> = vec![
        ("all", &everything),
        ("test split", &test),
        ("dev split", &dev),
    ];
    for (kind, s) in &per_kind {
        slices.push((kind.as_str(), s));
    }
    report(policy, &slices)
}

fn write_report(text: &str) {
    println!("\n{text}");
    if let Ok(path) = std::env::var("HQ_EVAL_REPORT") {
        std::fs::write(&path, text).unwrap_or_else(|e| panic!("writing {path}: {e}"));
    }
}

#[test]
fn the_fixtures_agree_with_each_other() {
    let rows = corpus();
    let cases = cases();
    let ids: HashSet<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids.len(), rows.len(), "corpus ids are unique");
    assert!(rows.len() >= 60, "{} corpus rows", rows.len());
    assert!(cases.len() >= 70, "{} cases", cases.len());

    let case_ids: HashSet<&str> = cases.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(case_ids.len(), cases.len(), "case ids are unique");

    for case in &cases {
        for id in case
            .expect
            .iter()
            .chain(&case.forbid)
            .chain(&case.acceptable)
        {
            assert!(ids.contains(id.as_str()), "{} names unknown {id}", case.id);
        }
        assert!(
            case.split == "dev" || case.split == "test",
            "{} split",
            case.id
        );
        assert!(
            case.identity == "owner" || case.identity == "guest",
            "{} identity",
            case.id
        );
        if case.expect_silence {
            assert!(case.expect.is_empty(), "{} expects silence", case.id);
        } else {
            assert!(
                !case.expect.is_empty(),
                "{} is not a quiet case, so it must name what to find",
                case.id
            );
        }
        if case.kind == "followup" {
            assert!(case.prior_message.is_some(), "{} needs a prior", case.id);
        }
    }

    // Every kind is there, and both splits have enough to tune and gate on.
    for kind in [
        "positive",
        "paraphrase",
        "negative",
        "stale",
        "guest",
        "unattributed",
        "followup",
        "hostile",
    ] {
        assert!(cases.iter().any(|c| c.kind == kind), "no {kind} cases");
    }
    for split in ["dev", "test"] {
        let n = cases.iter().filter(|c| c.split == split).count();
        assert!(n >= 30, "{n} {split} cases");
    }
    // A superseded row is the older half of a pair that has a newer active one.
    for row in rows.iter().filter(|r| r.status == "superseded") {
        assert!(
            cases.iter().any(|c| c.forbid.contains(&row.id)),
            "{} is superseded but no case checks it stays out",
            row.id
        );
    }
}

#[test]
fn the_recent_policy_is_scored_as_the_baseline() {
    let world = World::build();
    let all = cases();
    let text = run(Policy::Recent, &world, &all);
    write_report(&text);

    let refs: Vec<&Case> = all.iter().collect();
    let first = score(Policy::Recent, &world, &refs);

    // The only promises made about today's behaviour: it never answers a guest from the
    // owner's memory, and it is deterministic, so a later policy is compared against a fixed bar.
    assert_eq!(first.guest_leak, 0, "a guest must get no memories");
    let again = score(Policy::Recent, &world, &refs);
    assert_eq!(first.found, again.found);
    assert_eq!(first.at3, again.at3);
    assert_eq!(first.false_injection, again.false_injection);
    assert!(first.asked >= 40, "{} cases ask for a memory", first.asked);
    assert!(first.quiet_expected >= 25, "{} quiet", first.quiet_expected);
}
