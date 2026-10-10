//! Derived task relationship index (FR-069). Logically separate from the vault
//! and memory graphs: own tables (migration 063), no runtime dependency on
//! hq-memory, and every failure here degrades to plain task listing.
//!
//! Two kinds of link, never mixed. Explicit links (parent, sub-task,
//! depends_on) are read live from the task tables. Inferred links are cheap
//! deterministic similarity (TF-IDF over title and description, rarity-weighted
//! tag overlap, same initiative), stored with the evidence that produced them.

use anyhow::Result;
use rusqlite::{Connection, params};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap, HashSet};

const MIN_SCORE: f64 = 0.2;
const MAX_EDGES_PER_TASK: usize = 10;
const WEIGHT_TEXT: f64 = 0.6;
const WEIGHT_TAGS: f64 = 0.3;
const WEIGHT_INITIATIVE: f64 = 0.1;
const TITLE_TERM_BOOST: f64 = 3.0;
const MIN_TERM_LEN: usize = 3;
const MAX_EVIDENCE_TERMS: usize = 5;
pub const DEFAULT_SYNC_BUDGET: usize = 200;
const MAX_SYNC_ROUNDS: usize = 20;
pub const MAX_RESULTS: usize = 25;
/// A new task is called a likely duplicate at this score or above. Higher than
/// `MIN_SCORE`, which only decides what is worth showing as related.
const DUPLICATE_MIN_SCORE: f64 = 0.45;
/// Most similar open tasks, and most similar completed tasks, `similar_to_text` returns.
/// Counted apart so any number of completed lookalikes cannot hide an open one.
pub const MAX_SIMILAR: usize = 5;
/// Scored candidates looked up for status before the split, bounding the queries.
const MAX_CANDIDATES: usize = 200;
/// Completed similar tasks needed before an estimate is suggested.
const MIN_ESTIMATE_SAMPLES: usize = 2;
/// A leased time shorter than this is noise, not a sample of how long the work takes.
const MIN_SAMPLE_SECONDS: i64 = 60;
/// Suggested estimates are rounded to this many minutes.
const ESTIMATE_ROUNDING_MINUTES: i64 = 5;
const MAX_EXPLICIT: usize = 50;

pub const KIND_PARENT: &str = "parent";
pub const KIND_SUBTASK: &str = "subtask";
pub const KIND_DEPENDS_ON: &str = "depends_on";
pub const KIND_DEPENDENT: &str = "dependent";

const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "into", "are", "not", "can", "all", "any",
    "task", "tasks", "should", "when", "then", "than", "has", "have", "was", "were", "will", "its",
    "our", "you", "your", "via", "per", "use", "using", "build", "make", "add", "new",
];

#[derive(Debug, Clone, Serialize)]
pub struct TaskRef {
    pub id: String,
    pub display_id: String,
    pub title: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExplicitLink {
    pub kind: &'static str,
    pub task: TaskRef,
}

#[derive(Debug, Clone, Serialize)]
pub struct InferredLink {
    pub task: TaskRef,
    pub score: f64,
    /// The features behind the score: shared terms, shared tags, same initiative.
    pub evidence: Value,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncReport {
    pub tasks: usize,
    pub reindexed: usize,
    pub removed: usize,
    /// Tasks whose text changed but were not re-derived this call (budget hit).
    pub stale_remaining: usize,
}

struct Doc {
    id: String,
    initiative_id: String,
    tags: BTreeSet<String>,
    term_counts: HashMap<String, f64>,
    fingerprint: String,
}

struct Corpus {
    docs: Vec<Doc>,
    vectors: Vec<HashMap<String, f64>>,
    norms: Vec<f64>,
    tag_weight: HashMap<String, f64>,
}

fn tokenize(text: &str, boost: f64, into: &mut HashMap<String, f64>) {
    for word in text.split(|c: char| !c.is_alphanumeric()) {
        let word = word.to_lowercase();
        let has_letter = word.chars().any(|c| c.is_alphabetic());
        if word.chars().count() < MIN_TERM_LEN || !has_letter || STOPWORDS.contains(&word.as_str())
        {
            continue;
        }
        *into.entry(word).or_insert(0.0) += boost;
    }
}

/// FNV-1a: stable across Rust versions, unlike `DefaultHasher`.
fn fnv1a(text: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn load_docs(conn: &Connection) -> Result<Vec<Doc>> {
    let mut tags: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut stmt = conn.prepare("SELECT task_id, tag FROM task_tags")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (task_id, tag) = row?;
        tags.entry(task_id).or_default().insert(tag.to_lowercase());
    }
    let mut stmt = conn.prepare(
        "SELECT id, initiative_id, title, COALESCE(description, '') FROM tasks ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let mut docs = Vec::new();
    for row in rows {
        let (id, initiative_id, title, description) = row?;
        let tags = tags.remove(&id).unwrap_or_default();
        let mut term_counts = HashMap::new();
        tokenize(&title, TITLE_TERM_BOOST, &mut term_counts);
        tokenize(&description, 1.0, &mut term_counts);
        let joined_tags = tags.iter().cloned().collect::<Vec<_>>().join(",");
        let fingerprint = fnv1a(&format!(
            "{initiative_id}\n{title}\n{description}\n{joined_tags}"
        ));
        docs.push(Doc {
            id,
            initiative_id,
            tags,
            term_counts,
            fingerprint,
        });
    }
    Ok(docs)
}

impl Corpus {
    fn new(docs: Vec<Doc>) -> Self {
        let n = docs.len() as f64;
        let mut term_df: HashMap<&str, f64> = HashMap::new();
        let mut tag_df: HashMap<&str, f64> = HashMap::new();
        for doc in &docs {
            for term in doc.term_counts.keys() {
                *term_df.entry(term).or_insert(0.0) += 1.0;
            }
            for tag in &doc.tags {
                *tag_df.entry(tag).or_insert(0.0) += 1.0;
            }
        }
        let vectors: Vec<HashMap<String, f64>> = docs
            .iter()
            .map(|d| {
                d.term_counts
                    .iter()
                    .map(|(term, tf)| {
                        let idf = ((n + 1.0) / (term_df[term.as_str()] + 1.0)).ln() + 1.0;
                        (term.clone(), (1.0 + tf.ln()) * idf)
                    })
                    .collect()
            })
            .collect();
        let norms = vectors
            .iter()
            .map(|v| v.values().map(|w| w * w).sum::<f64>().sqrt())
            .collect();
        // A tag on every task has weight 0, so routing tags like "hq" never link anything.
        let tag_weight = tag_df
            .iter()
            .map(|(t, df)| ((*t).to_string(), ((n + 1.0) / (df + 1.0)).ln()))
            .collect();
        Corpus {
            docs,
            vectors,
            norms,
            tag_weight,
        }
    }

    /// Score and explain one pair, or `None` when there is no shared feature.
    fn compare(&self, i: usize, j: usize) -> Option<(f64, Value)> {
        let (a, b) = (&self.docs[i], &self.docs[j]);
        let mut shared: Vec<(&String, f64)> = self.vectors[i]
            .iter()
            .filter_map(|(t, wa)| self.vectors[j].get(t).map(|wb| (t, wa * wb)))
            .collect();
        let dot: f64 = shared.iter().map(|(_, p)| p).sum();
        let denom = self.norms[i] * self.norms[j];
        let cosine = if denom > 0.0 { dot / denom } else { 0.0 };
        let shared_tags: Vec<&String> = a.tags.intersection(&b.tags).collect();
        let weigh = |tags: &mut dyn Iterator<Item = &String>| -> f64 {
            tags.map(|t| self.tag_weight.get(t).copied().unwrap_or(0.0))
                .sum()
        };
        let union_weight = weigh(&mut a.tags.union(&b.tags));
        let tag_overlap = if union_weight > 0.0 {
            weigh(&mut shared_tags.iter().copied()) / union_weight
        } else {
            0.0
        };
        if cosine <= 0.0 && tag_overlap <= 0.0 {
            return None;
        }
        let same_initiative = a.initiative_id == b.initiative_id;
        let score = WEIGHT_TEXT * cosine
            + WEIGHT_TAGS * tag_overlap
            + if same_initiative {
                WEIGHT_INITIATIVE
            } else {
                0.0
            };
        if score < MIN_SCORE {
            return None;
        }
        shared.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(y.0)));
        let terms: Vec<&String> = shared
            .iter()
            .take(MAX_EVIDENCE_TERMS)
            .map(|(t, _)| *t)
            .collect();
        let evidence = json!({
            "shared_terms": terms,
            "shared_tags": shared_tags,
            "same_initiative": same_initiative,
            "text_similarity": round3(cosine),
            "tag_overlap": round3(tag_overlap),
        });
        Some((score, evidence))
    }
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn delete_task_edges(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM task_graph_edges WHERE task_a = ?1 OR task_b = ?1",
        params![id],
    )?;
    Ok(())
}

fn index_task(conn: &Connection, corpus: &Corpus, i: usize) -> Result<()> {
    let id = &corpus.docs[i].id;
    delete_task_edges(conn, id)?;
    let mut scored: Vec<(usize, f64, Value)> = (0..corpus.docs.len())
        .filter(|j| *j != i)
        .filter_map(|j| corpus.compare(i, j).map(|(s, e)| (j, s, e)))
        .collect();
    scored.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then(corpus.docs[a.0].id.cmp(&corpus.docs[b.0].id))
    });
    for (j, score, evidence) in scored.into_iter().take(MAX_EDGES_PER_TASK) {
        let other = &corpus.docs[j].id;
        let (lo, hi) = if id < other { (id, other) } else { (other, id) };
        conn.execute(
            "INSERT OR REPLACE INTO task_graph_edges (task_a, task_b, score, evidence) VALUES (?1, ?2, ?3, ?4)",
            params![lo, hi, score, evidence.to_string()],
        )?;
    }
    conn.execute(
        "INSERT INTO task_graph_nodes (task_id, fingerprint) VALUES (?1, ?2)
         ON CONFLICT(task_id) DO UPDATE SET fingerprint = ?2, indexed_at = datetime('now')",
        params![id, corpus.docs[i].fingerprint],
    )?;
    Ok(())
}

/// Brings the index up to date: prunes deleted tasks, re-derives at most
/// `budget` tasks whose text, tags or initiative changed. Scores of untouched
/// pairs keep the corpus statistics they were computed with until `rebuild`.
/// Repeats `sync` until nothing is left stale, so one call always leaves a usable index. A
/// single budgeted pass left 167 of 367 tasks unindexed and inferred links empty.
pub fn sync_to_current(conn: &Connection, budget: usize) -> Result<SyncReport> {
    let mut total = sync(conn, budget)?;
    for _ in 1..MAX_SYNC_ROUNDS {
        if total.stale_remaining == 0 {
            break;
        }
        let round = sync(conn, budget)?;
        total.reindexed += round.reindexed;
        total.removed += round.removed;
        total.stale_remaining = round.stale_remaining;
    }
    Ok(total)
}

pub fn sync(conn: &Connection, budget: usize) -> Result<SyncReport> {
    let tx = conn.unchecked_transaction()?;
    let mut indexed: HashMap<String, String> = HashMap::new();
    {
        let mut stmt = tx.prepare("SELECT task_id, fingerprint FROM task_graph_nodes")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (id, fp) = row?;
            indexed.insert(id, fp);
        }
    }
    let corpus = Corpus::new(load_docs(&tx)?);
    let live: HashSet<&str> = corpus.docs.iter().map(|d| d.id.as_str()).collect();
    let mut report = SyncReport {
        tasks: corpus.docs.len(),
        ..Default::default()
    };
    for gone in indexed.keys().filter(|id| !live.contains(id.as_str())) {
        delete_task_edges(&tx, gone)?;
        tx.execute(
            "DELETE FROM task_graph_nodes WHERE task_id = ?1",
            params![gone],
        )?;
        report.removed += 1;
    }
    let stale: Vec<usize> = (0..corpus.docs.len())
        .filter(|i| indexed.get(&corpus.docs[*i].id) != Some(&corpus.docs[*i].fingerprint))
        .collect();
    for i in stale.iter().take(budget) {
        index_task(&tx, &corpus, *i)?;
        report.reindexed += 1;
    }
    report.stale_remaining = stale.len().saturating_sub(budget);
    tx.commit()?;
    Ok(report)
}

/// Drops the whole index and re-derives it with fresh corpus statistics.
pub fn rebuild(conn: &Connection) -> Result<SyncReport> {
    conn.execute_batch("DELETE FROM task_graph_edges; DELETE FROM task_graph_nodes;")?;
    sync(conn, usize::MAX)
}

pub fn resolve_task(conn: &Connection, id_or_display_id: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT id FROM tasks WHERE id = ?1 OR display_id = ?1")?;
    let mut rows = stmt.query(params![id_or_display_id])?;
    Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
}

fn ref_from_row(r: &rusqlite::Row, offset: usize) -> rusqlite::Result<TaskRef> {
    Ok(TaskRef {
        id: r.get(offset)?,
        display_id: r.get(offset + 1)?,
        title: r.get(offset + 2)?,
        status: r.get(offset + 3)?,
    })
}

const EXPLICIT_SQL: &str = "
    SELECT 'parent', t.id, t.display_id, t.title, t.status FROM tasks c JOIN tasks t ON t.id = c.parent_task_id WHERE c.id = ?1
    UNION ALL SELECT 'subtask', t.id, t.display_id, t.title, t.status FROM tasks t WHERE t.parent_task_id = ?1
    UNION ALL SELECT 'depends_on', t.id, t.display_id, t.title, t.status FROM task_dependencies d JOIN tasks t ON t.id = d.depends_on_task_id WHERE d.task_id = ?1
    UNION ALL SELECT 'dependent', t.id, t.display_id, t.title, t.status FROM task_dependencies d JOIN tasks t ON t.id = d.task_id WHERE d.depends_on_task_id = ?1
    LIMIT ?2";

/// Links the user or an agent declared. Read live, so never stale.
pub fn explicit_links(conn: &Connection, task_id: &str) -> Result<Vec<ExplicitLink>> {
    let mut stmt = conn.prepare(EXPLICIT_SQL)?;
    let rows = stmt.query_map(params![task_id, MAX_EXPLICIT as i64], |r| {
        let kind: String = r.get(0)?;
        Ok((kind, ref_from_row(r, 1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (kind, task) = row?;
        let kind = match kind.as_str() {
            KIND_PARENT => KIND_PARENT,
            KIND_SUBTASK => KIND_SUBTASK,
            KIND_DEPENDS_ON => KIND_DEPENDS_ON,
            _ => KIND_DEPENDENT,
        };
        out.push(ExplicitLink { kind, task });
    }
    Ok(out)
}

/// Inferred neighbours of `task_id`, best first, one hop, capped at `limit`.
pub fn inferred_links(conn: &Connection, task_id: &str, limit: usize) -> Result<Vec<InferredLink>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.display_id, t.title, t.status, e.score, e.evidence
         FROM task_graph_edges e
         JOIN tasks t ON t.id = CASE WHEN e.task_a = ?1 THEN e.task_b ELSE e.task_a END
         WHERE e.task_a = ?1 OR e.task_b = ?1
         ORDER BY e.score DESC, t.display_id LIMIT ?2",
    )?;
    let limit = limit.min(MAX_RESULTS) as i64;
    let rows = stmt.query_map(params![task_id, limit], |r| {
        let evidence: String = r.get(5)?;
        Ok(InferredLink {
            task: ref_from_row(r, 0)?,
            score: round3(r.get(4)?),
            evidence: serde_json::from_str(&evidence).unwrap_or(Value::Null),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Same-initiative listing used when the graph has nothing (or fails).
pub fn fallback_listing(conn: &Connection, task_id: &str, limit: usize) -> Result<Vec<TaskRef>> {
    let mut stmt = conn.prepare(
        "SELECT o.id, o.display_id, o.title, o.status FROM tasks t JOIN tasks o
             ON o.initiative_id = t.initiative_id AND o.id <> t.id
         WHERE t.id = ?1 ORDER BY o.updated_at DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![task_id, limit.min(MAX_RESULTS) as i64], |r| {
        ref_from_row(r, 0)
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[cfg(test)]
mod tests;

/// A task like the one being written, with why it is similar.
#[derive(Debug, Clone, Serialize)]
pub struct SimilarTask {
    pub task: TaskRef,
    pub score: f64,
    pub evidence: Value,
}

/// Tasks that read like a task with this title, description, tags and initiative,
/// highest score first. Nothing is written and nothing is indexed: the text is
/// scored against the live tasks the way the stored index would score it, so it
/// works for a task that does not exist yet. `exclude_id` leaves out the task
/// that was just created from this text.
pub fn similar_to_text(
    conn: &Connection,
    exclude_id: Option<&str>,
    initiative_id: &str,
    title: &str,
    description: &str,
    tags: &[String],
) -> Result<Vec<SimilarTask>> {
    let mut docs = load_docs(conn)?;
    docs.retain(|d| Some(d.id.as_str()) != exclude_id);
    let mut term_counts = HashMap::new();
    tokenize(title, TITLE_TERM_BOOST, &mut term_counts);
    tokenize(description, 1.0, &mut term_counts);
    docs.push(Doc {
        id: String::new(),
        initiative_id: initiative_id.to_string(),
        tags: tags.iter().map(|t| t.to_lowercase()).collect(),
        term_counts,
        fingerprint: String::new(),
    });
    let corpus = Corpus::new(docs);
    let new = corpus.docs.len() - 1;
    let mut scored: Vec<(f64, usize, Value)> = (0..new)
        .filter_map(|j| corpus.compare(new, j).map(|(score, evidence)| (score, j, evidence)))
        .filter(|(score, _, _)| *score >= DUPLICATE_MIN_SCORE)
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.truncate(MAX_CANDIDATES);
    let all: Vec<SimilarTask> = scored
        .into_iter()
        .filter_map(|(score, j, evidence)| {
            conn.query_row(
                "SELECT id, display_id, title, status FROM tasks WHERE id = ?1",
                params![&corpus.docs[j].id],
                |r| ref_from_row(r, 0),
            )
            .ok()
            .map(|task| SimilarTask { task, score: round3(score), evidence })
        })
        .collect();
    let (mut open, mut done): (Vec<_>, Vec<_>) = all.into_iter().partition(|s| s.task.status != "complete");
    open.truncate(MAX_SIMILAR);
    done.truncate(MAX_SIMILAR);
    open.extend(done);
    open.sort_by(|a, b| b.score.total_cmp(&a.score));
    Ok(open)
}

/// An estimate suggested by how long similar completed tasks took.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EstimateSuggestion {
    pub minutes: i64,
    /// Display ids of the completed tasks it is based on.
    pub based_on: Vec<String>,
}

/// The median leased time of similar completed tasks, rounded to five minutes,
/// when at least `MIN_ESTIMATE_SAMPLES` of them have real leased time. A hint with
/// its evidence, never a value that is written for anyone.
pub fn suggest_estimate(conn: &Connection, similar: &[SimilarTask]) -> Result<Option<EstimateSuggestion>> {
    let mut samples: Vec<(i64, String)> = Vec::new();
    for s in similar.iter().filter(|s| s.task.status == "complete") {
        let seconds = crate::tasks::leased_seconds(conn, &s.task.id)?;
        if seconds >= MIN_SAMPLE_SECONDS {
            samples.push((seconds, s.task.display_id.clone()));
        }
    }
    if samples.len() < MIN_ESTIMATE_SAMPLES {
        return Ok(None);
    }
    samples.sort();
    let mid = samples.len() / 2;
    let median = if samples.len().is_multiple_of(2) {
        (samples[mid - 1].0 + samples[mid].0) / 2
    } else {
        samples[mid].0
    };
    let minutes = ((median / 60 + ESTIMATE_ROUNDING_MINUTES / 2) / ESTIMATE_ROUNDING_MINUTES * ESTIMATE_ROUNDING_MINUTES)
        .max(ESTIMATE_ROUNDING_MINUTES);
    Ok(Some(EstimateSuggestion { minutes, based_on: samples.into_iter().map(|(_, id)| id).collect() }))
}
