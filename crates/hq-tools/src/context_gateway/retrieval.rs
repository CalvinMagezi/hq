//! Scoped retrieval: the one front door that turns a grant and an operation into entries.
//! Scope is checked on every source before anything is read, the excerpt budget is split
//! across sources, and every call is rate limited and audited with counts only.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};
use hq_db::{Database, context_audit};
use hq_memory::context_packet::{
    Candidate, ContextNeed, GapKind, PacketEntry, PacketGap, build_packet,
};
use hq_memory::graph_paths::{self, GraphOutcome, GraphPath, GraphQuery};
use hq_vault::VaultClient;

use super::{
    ContextEntry, ContextOp, ContextResult, GrantResolver, PrefixScope, ScopeCheck, SessionGrant,
    SourceId, Unavailable,
};

pub const RATE_WINDOW_SECS: i64 = 60;
pub const RATE_MAX_CALLS: u32 = 30;
pub const RATE_MAX_CHARS: usize = 200_000;
/// Below this a source cannot hold a useful excerpt, so it is reported instead of squeezed.
const MIN_SHARE_CHARS: usize = 80;
const SEARCH_OVERFETCH: usize = 3;
const ORIENT_BUDGET_CHARS: usize = 2_000;
pub const ASK_MAX_QUESTION_CHARS: usize = 500;
const ASK_MAX_RESULTS: usize = 5;
const ASK_BUDGET_CHARS: usize = 4_000;

#[derive(Default)]
struct Window {
    started: Option<DateTime<Utc>>,
    calls: u32,
    chars: usize,
}

/// Calls and returned characters per grant per window. In memory: a restart resets it, which
/// only ever gives a session a fresh window.
#[derive(Default)]
pub struct RateLimiter {
    windows: Mutex<HashMap<String, Window>>,
}

impl RateLimiter {
    /// Counts the call and says whether it may run. A refused call is not counted twice.
    fn admit(&self, key: &str, now: DateTime<Utc>) -> bool {
        let mut windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        let w = windows.entry(key.to_string()).or_default();
        let fresh = w
            .started
            .is_none_or(|at| now - at >= Duration::seconds(RATE_WINDOW_SECS));
        if fresh {
            *w = Window {
                started: Some(now),
                ..Window::default()
            };
        }
        if w.calls >= RATE_MAX_CALLS || w.chars >= RATE_MAX_CHARS {
            return false;
        }
        w.calls += 1;
        true
    }

    fn charge(&self, key: &str, chars: usize) {
        let mut windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(w) = windows.get_mut(key) {
            w.chars += chars;
        }
    }
}

pub struct Retriever<'a> {
    vault: &'a VaultClient,
    db: &'a Database,
    limiter: &'a RateLimiter,
}

impl<'a> Retriever<'a> {
    pub fn new(vault: &'a VaultClient, db: &'a Database, limiter: &'a RateLimiter) -> Self {
        Self { vault, db, limiter }
    }

    /// Resolves the session first. No grant means nothing is read and nothing is audited.
    pub fn retrieve(
        &self,
        resolver: &dyn GrantResolver,
        session_id: &str,
        op: &ContextOp,
        now: DateTime<Utc>,
    ) -> Result<ContextResult, Unavailable> {
        let grant = resolver.resolve(session_id)?;
        self.run(&grant, op, now)
    }

    pub fn run(
        &self,
        grant: &SessionGrant,
        op: &ContextOp,
        now: DateTime<Utc>,
    ) -> Result<ContextResult, Unavailable> {
        if grant.is_expired(now) {
            return Err(Unavailable::Expired);
        }
        let budget = grant.scope.max_chars;
        let key = format!("{}/{}", grant.session_id, grant.task_id);
        let result = if !grant.caps.contains(op.required_cap()) {
            refused(
                budget,
                GapKind::Denied { count: 1 },
                "this operation is not allowed",
            )
        } else if !self.limiter.admit(&key, now) {
            refused(
                budget,
                GapKind::RateLimited,
                "too many context calls, try again shortly",
            )
        } else {
            self.dispatch(grant, op, now)
        };
        self.limiter.charge(
            &key,
            result
                .entries
                .iter()
                .map(|e| e.excerpt.chars().count())
                .sum(),
        );
        self.audit(grant, op, &result, now);
        Ok(result)
    }

    fn dispatch(&self, grant: &SessionGrant, op: &ContextOp, now: DateTime<Utc>) -> ContextResult {
        let mut sink = Sink::default();
        let scope = PrefixScope;
        let total = grant.scope.max_chars;
        let (query, limit) = match op {
            ContextOp::Orient => {
                self.orient(grant, &scope, &mut sink);
                (String::new(), grant.scope.max_results)
            }
            ContextOp::Search { query, limit } => {
                let limit = (*limit).min(grant.scope.max_results);
                self.search(grant, &scope, query, limit, &mut sink);
                (query.clone(), limit)
            }
            ContextOp::Expand { from, depth } => {
                self.expand(grant, &scope, from, *depth, &mut sink);
                (String::new(), grant.scope.max_results)
            }
            ContextOp::Read { id, .. } => {
                // `section` is not honoured yet: excerpts are chosen by the packet builder.
                sink.add(
                    grant,
                    &scope,
                    vec![candidate(id.as_str(), "read", "read directly")],
                );
                (String::new(), 1)
            }
            ContextOp::Related { id } => {
                self.expand(grant, &scope, std::slice::from_ref(id), 1, &mut sink);
                (String::new(), grant.scope.max_results)
            }
            ContextOp::AskHq { question } => {
                let question = question.trim();
                if question.is_empty() || question.chars().count() > ASK_MAX_QUESTION_CHARS {
                    sink.gaps.push(gap(
                        GapKind::Missing,
                        "the question is empty or too long to answer",
                    ));
                    return self.fill(grant, sink, "", 1, 0, now);
                }
                self.search(grant, &scope, question, ASK_MAX_RESULTS, &mut sink);
                (question.to_string(), ASK_MAX_RESULTS)
            }
        };
        let budget = match op {
            ContextOp::Read { max_chars, .. } => (*max_chars).min(total),
            ContextOp::Orient => ORIENT_BUDGET_CHARS.min(total),
            ContextOp::AskHq { .. } => ASK_BUDGET_CHARS.min(total),
            _ => total,
        };
        self.fill(grant, sink, &query, limit.max(1), budget, now)
    }

    fn orient(&self, grant: &SessionGrant, scope: &PrefixScope, sink: &mut Sink) {
        match hq_memory::context_task::task_context(self.db, &grant.task_id, 0) {
            Ok(Some(ctx)) => {
                let found = ctx
                    .sources
                    .into_iter()
                    .map(|s| candidate(&s.path, s.via, &s.why))
                    .collect();
                sink.add(grant, scope, found);
            }
            Ok(None) => {}
            Err(_) => sink
                .gaps
                .push(gap(GapKind::RetrievalFailed, "the task could not be read")),
        }
    }

    fn search(
        &self,
        grant: &SessionGrant,
        scope: &PrefixScope,
        query: &str,
        limit: usize,
        sink: &mut Sink,
    ) {
        let mut found = Vec::new();
        for folder in &grant.scope.allow_prefixes {
            let prefix = format!("{}/", folder.as_str());
            let hits = self.db.with_conn(|c| {
                hq_db::search::keyword_search_in(c, query, limit * SEARCH_OVERFETCH, Some(&prefix))
            });
            match hits {
                Ok(list) => found.extend(
                    list.into_iter()
                        .map(|h| candidate(&h.note_path, "search", "keyword match for the query")),
                ),
                Err(_) => sink
                    .gaps
                    .push(gap(GapKind::RetrievalFailed, "search failed")),
            }
        }
        if found.is_empty() && sink.gaps.is_empty() {
            sink.gaps
                .push(gap(GapKind::NoMatch, "no note in scope matched the query"));
        }
        sink.add(grant, scope, found);
    }

    fn expand(
        &self,
        grant: &SessionGrant,
        scope: &PrefixScope,
        from: &[SourceId],
        depth: u8,
        sink: &mut Sink,
    ) {
        let (seeds, gap_for_seeds) = scope.partition(grant, from.to_vec());
        sink.gaps.extend(gap_for_seeds);
        if seeds.is_empty() {
            return;
        }
        let hops = depth.min(grant.scope.max_graph_depth) as usize;
        let query = GraphQuery {
            seeds: seeds.iter().map(stem).collect(),
            max_hops: hops.min(graph_paths::HARD_MAX_HOPS),
            ..GraphQuery::default()
        };
        match graph_paths::discover(self.db, self.vault, &query) {
            GraphOutcome::Unavailable { .. } => sink.gaps.push(gap(
                GapKind::GraphUnavailable,
                "graph discovery is unavailable",
            )),
            GraphOutcome::Paths(result) => {
                let (found, denied) = scoped_chains(grant, scope, &result.paths);
                sink.denied += denied;
                if found.is_empty() {
                    sink.gaps
                        .push(gap(GapKind::NoMatch, "no chain inside this scope"));
                }
                sink.add(grant, scope, found);
            }
        }
    }

    /// Reads each permitted source with its own share of the budget, so one large note cannot
    /// starve the rest. What does not fit is reported, never cut without a word.
    fn fill(
        &self,
        grant: &SessionGrant,
        sink: Sink,
        query: &str,
        limit: usize,
        budget: usize,
        now: DateTime<Utc>,
    ) -> ContextResult {
        let Sink {
            mut candidates,
            mut gaps,
            denied,
        } = sink;
        let mut seen = HashSet::new();
        candidates.retain(|c| seen.insert(c.path.clone()));
        let kept = candidates.len().min(limit);
        let share = budget / kept.max(1);
        let mut over = candidates.len() - kept;
        candidates.truncate(kept);
        if share < MIN_SHARE_CHARS && kept > 0 {
            over += kept;
            candidates.clear();
        }
        let mut entries = Vec::new();
        let mut denied = denied;
        for cand in candidates {
            let need = ContextNeed {
                refs: vec![cand.path.clone()],
                budget_chars: Some(share),
                max_sources: Some(1),
                ..ContextNeed::default()
            };
            let packet = build_packet(self.vault, Some(self.db), query, &need, now);
            gaps.extend(packet.gaps);
            for entry in packet.entries {
                // A moved note is found by name, so the path it landed on is checked again.
                match SourceId::parse(&entry.path) {
                    Ok(id) if PrefixScope.permits(grant, &id) => {
                        if self.larger_than(&entry.path, share) {
                            over += 1;
                        }
                        entries.push(to_entry(entry, &cand));
                    }
                    _ => denied += 1,
                }
            }
        }
        if over > 0 {
            gaps.push(gap(
                GapKind::OverBudget,
                &format!("{over} source(s) left out or shortened to fit the budget"),
            ));
        }
        if denied > 0 {
            gaps.push(gap(
                GapKind::Denied { count: denied },
                "sources outside this task's scope were left out",
            ));
        }
        ContextResult {
            entries,
            gaps,
            budget_chars: budget,
        }
    }

    /// An excerpt of a note bigger than its share is a shortened view, and the reader is told.
    fn larger_than(&self, path: &str, share: usize) -> bool {
        std::fs::metadata(self.vault.vault_path().join(path))
            .is_ok_and(|m| m.len() as usize > share)
    }

    fn audit(
        &self,
        grant: &SessionGrant,
        op: &ContextOp,
        result: &ContextResult,
        now: DateTime<Utc>,
    ) {
        let denied = result
            .gaps
            .iter()
            .map(|g| match g.kind {
                GapKind::Denied { count } => count,
                _ => 0,
            })
            .sum();
        let row = context_audit::AuditRow {
            session_id: &grant.session_id,
            task_id: &grant.task_id,
            mission_id: grant.mission_id.as_deref(),
            op: op_name(op),
            returned: result.entries.len(),
            denied,
        };
        // An audit failure must not turn into a read failure or leak through the result.
        let _ = self.db.with_conn(|c| {
            context_audit::record(c, &row, now)?;
            for entry in &result.entries {
                let trail = context_audit::TrailRow {
                    task_id: &grant.task_id,
                    session_id: &grant.session_id,
                    op: row.op,
                    source_id: &entry.id,
                    content_hash: &entry.content_hash,
                };
                context_audit::record_trail(c, &trail, now)?;
            }
            Ok(())
        });
    }
}

#[derive(Default)]
struct Sink {
    candidates: Vec<Candidate>,
    gaps: Vec<PacketGap>,
    denied: usize,
}

impl Sink {
    /// The one place candidates enter: anything that is not a permitted `SourceId` is counted
    /// and dropped before a read can happen.
    fn add(&mut self, grant: &SessionGrant, scope: &PrefixScope, found: Vec<Candidate>) {
        for cand in found {
            match SourceId::parse(&cand.path) {
                Ok(id) if scope.permits(grant, &id) => self.candidates.push(cand),
                _ => self.denied += 1,
            }
        }
    }
}

/// Keeps each chain up to its first edge that touches a source outside the grant. The edge and
/// everything after it are counted once and never expanded.
fn scoped_chains(
    grant: &SessionGrant,
    scope: &PrefixScope,
    paths: &[GraphPath],
) -> (Vec<Candidate>, usize) {
    let permitted = |note: &str| SourceId::parse(note).is_ok_and(|id| scope.permits(grant, &id));
    let mut out = Vec::new();
    let mut denied = 0;
    for path in paths {
        let mut notes_by_hop = Vec::new();
        for (hop, edge) in path.edges.iter().enumerate() {
            let one = GraphPath {
                nodes: Vec::new(),
                edges: vec![edge.clone()],
                score: 0.0,
                caveats: Vec::new(),
            };
            let notes = one.evidence_paths();
            let page_ok = path
                .nodes
                .get(hop + 1)
                .and_then(|n| n.page_path.as_deref())
                .is_none_or(permitted);
            if !page_ok || notes.iter().any(|n| !permitted(n)) {
                denied += 1;
                break;
            }
            notes_by_hop.push(notes);
        }
        // The chain text names every node on it, so it is built only from the edges kept: a
        // name past the cut would reveal what sits outside the scope.
        let kept = GraphPath {
            nodes: path
                .nodes
                .iter()
                .take(notes_by_hop.len() + 1)
                .cloned()
                .collect(),
            edges: path
                .edges
                .iter()
                .take(notes_by_hop.len())
                .cloned()
                .collect(),
            score: path.score,
            caveats: Vec::new(),
        };
        let chain = kept.describe();
        for note in notes_by_hop.into_iter().flatten() {
            out.push(Candidate {
                path: note,
                via: "graph".to_string(),
                relevance: format!("graph chain: {chain}"),
            });
        }
    }
    (out, denied)
}

fn to_entry(e: PacketEntry, cand: &Candidate) -> ContextEntry {
    let graph_path = (cand.via == "graph").then(|| cand.relevance.clone());
    ContextEntry {
        id: e.path,
        title: e.title,
        excerpt: e.excerpt,
        why: e.relevance,
        graph_path,
        freshness: e.freshness,
        provenance: e.provenance,
        content_hash: e.content_hash,
        retrieved_at: e.retrieved_at,
    }
}

fn candidate(path: &str, via: impl Into<String>, why: &str) -> Candidate {
    Candidate {
        path: path.to_string(),
        via: via.into(),
        relevance: why.to_string(),
    }
}

fn gap(kind: GapKind, detail: &str) -> PacketGap {
    PacketGap {
        kind,
        detail: detail.to_string(),
    }
}

fn refused(budget: usize, kind: GapKind, detail: &str) -> ContextResult {
    ContextResult {
        entries: Vec::new(),
        gaps: vec![gap(kind, detail)],
        budget_chars: budget,
    }
}

fn stem(id: &SourceId) -> String {
    std::path::Path::new(id.as_str())
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string()
}

fn op_name(op: &ContextOp) -> &'static str {
    match op {
        ContextOp::Orient => "orient",
        ContextOp::Search { .. } => "search",
        ContextOp::Expand { .. } => "expand",
        ContextOp::Read { .. } => "read",
        ContextOp::Related { .. } => "related",
        ContextOp::AskHq { .. } => "ask_hq",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_gateway::{Caps, SourceScope};
    use hq_memory::graph_paths::{EdgeStatus, PathEdge, PathNode};

    const IN_DIR: &str = "Notebooks/Projects/Open";
    const OUT_DIR: &str = "Notebooks/Projects/Closed";
    const SECRET: &str = "closedsecretmarker";

    struct Fixture {
        dir: tempfile::TempDir,
        db: Database,
        vault: VaultClient,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let vault = VaultClient::new(dir.path().to_path_buf()).unwrap();
        Fixture {
            dir,
            db: Database::open_memory().unwrap(),
            vault,
        }
    }

    impl Fixture {
        fn note(&self, rel: &str, body: &str) {
            let path = self.dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, format!("# {rel}\n\n{body}\n")).unwrap();
            self.db
                .with_conn(|c| hq_db::search::reconcile_path(c, self.dir.path(), rel).map(|_| ()))
                .unwrap();
        }

        fn run(&self, grant: &SessionGrant, op: &ContextOp) -> Result<ContextResult, Unavailable> {
            let limiter = RateLimiter::default();
            Retriever::new(&self.vault, &self.db, &limiter).run(grant, op, Utc::now())
        }
    }

    fn grant(max_chars: usize) -> SessionGrant {
        SessionGrant {
            session_id: "hs-a".into(),
            task_id: "t-1".into(),
            mission_id: Some("m-1".into()),
            scope: SourceScope {
                allow_prefixes: vec![SourceId::parse(IN_DIR).unwrap()],
                max_graph_depth: 2,
                max_results: 10,
                max_chars,
            },
            caps: Caps::READ,
            expires_at: None,
        }
    }

    fn read(path: &str) -> ContextOp {
        ContextOp::Read {
            id: SourceId::parse(path).unwrap(),
            section: None,
            max_chars: 4000,
        }
    }

    fn denied(r: &ContextResult) -> usize {
        r.gaps
            .iter()
            .map(|g| match g.kind {
                GapKind::Denied { count } => count,
                _ => 0,
            })
            .sum()
    }

    fn edge(from: &str, to: &str, evidence: &str) -> PathEdge {
        PathEdge {
            from: from.into(),
            to: to.into(),
            relation: "relates".into(),
            evidence_path: evidence.into(),
            evidence_updated_at: String::new(),
            sources: Vec::new(),
            status: EdgeStatus::Verified,
            reversed: false,
        }
    }

    fn chain(evidence: &[&str]) -> GraphPath {
        let names: Vec<String> = (0..=evidence.len()).map(|i| format!("n{i}")).collect();
        GraphPath {
            nodes: names
                .iter()
                .map(|n| PathNode {
                    name: n.clone(),
                    page_path: None,
                })
                .collect(),
            edges: evidence
                .iter()
                .enumerate()
                .map(|(i, e)| edge(&names[i], &names[i + 1], e))
                .collect(),
            score: 1.0,
            caveats: Vec::new(),
        }
    }

    #[test]
    fn an_out_of_scope_read_returns_nothing_and_leaks_no_name() {
        let f = fixture();
        let out = format!("{OUT_DIR}/Plan.md");
        f.note(&out, SECRET);
        let r = f.run(&grant(4000), &read(&out)).unwrap();
        assert!(r.entries.is_empty());
        assert_eq!(denied(&r), 1);
        let dump = serde_json::to_string(&r).unwrap();
        assert!(!dump.contains(SECRET) && !dump.contains("Plan") && !dump.contains("Closed"));
    }

    #[test]
    fn a_hop_to_an_out_of_scope_note_is_dropped_and_counted() {
        let g = grant(4000);
        let inside = format!("{IN_DIR}/A.md");
        let outside = format!("{OUT_DIR}/B.md");
        let hop1 = chain(&[&outside]);
        let hop2 = chain(&[&inside, &outside]);
        let (found, count) = scoped_chains(&g, &PrefixScope, &[hop1, hop2]);
        let paths: Vec<&str> = found.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, [inside.as_str()]);
        assert_eq!(count, 2);
        assert!(found.iter().all(|c| !c.relevance.contains("n2")));
    }

    #[test]
    fn search_stays_inside_the_folders_and_ignores_a_sibling_with_a_longer_name() {
        let f = fixture();
        f.note(&format!("{IN_DIR}/Mine.md"), "launch checklist alpha");
        f.note(&format!("{OUT_DIR}/Theirs.md"), "launch checklist alpha");
        f.note(
            "Notebooks/Projects/OpenElse/Near.md",
            "launch checklist alpha",
        );
        let op = ContextOp::Search {
            query: "launch checklist".into(),
            limit: 10,
        };
        let r = f.run(&grant(4000), &op).unwrap();
        let ids: Vec<&str> = r.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, [format!("{IN_DIR}/Mine.md")]);
    }

    #[test]
    fn one_huge_note_does_not_starve_the_others() {
        let f = fixture();
        f.note(
            &format!("{IN_DIR}/Huge.md"),
            &"filler words here. ".repeat(2000),
        );
        f.note(&format!("{IN_DIR}/Small1.md"), "small note one");
        f.note(&format!("{IN_DIR}/Small2.md"), "small note two");
        let op = ContextOp::Search {
            query: "note filler".into(),
            limit: 10,
        };
        let r = f.run(&grant(900), &op).unwrap();
        assert_eq!(r.entries.len(), 3);
        let share = 900 / 3;
        assert!(r.entries.iter().all(|e| e.excerpt.chars().count() <= share));
        assert!(r.gaps.iter().any(|g| g.kind == GapKind::OverBudget));
    }

    #[test]
    fn calls_past_the_window_limit_get_a_rate_limited_gap() {
        let limiter = RateLimiter::default();
        let now = Utc::now();
        for _ in 0..RATE_MAX_CALLS {
            assert!(limiter.admit("k", now));
        }
        assert!(!limiter.admit("k", now));
        assert!(limiter.admit("k", now + Duration::seconds(RATE_WINDOW_SECS)));

        let f = fixture();
        let g = grant(4000);
        let op = ContextOp::Orient;
        let lim = RateLimiter::default();
        let r = Retriever::new(&f.vault, &f.db, &lim);
        for _ in 0..RATE_MAX_CALLS {
            r.run(&g, &op, now).unwrap();
        }
        let blocked = r.run(&g, &op, now).unwrap();
        assert!(blocked.entries.is_empty());
        assert_eq!(blocked.gaps[0].kind, GapKind::RateLimited);
    }

    #[test]
    fn the_audit_row_holds_counts_and_no_content() {
        let f = fixture();
        f.note(&format!("{IN_DIR}/Ok.md"), "fine");
        f.note(&format!("{OUT_DIR}/No.md"), SECRET);
        f.run(&grant(4000), &read(&format!("{IN_DIR}/Ok.md")))
            .unwrap();
        f.run(&grant(4000), &read(&format!("{OUT_DIR}/No.md")))
            .unwrap();
        let rows =
            f.db.with_conn(|c| context_audit::counts_for_task(c, "t-1"))
                .unwrap();
        assert_eq!(
            rows,
            [("read".to_string(), 1, 0), ("read".to_string(), 0, 1)]
        );
        let text: String = f
            .db
            .with_conn(|c| {
                Ok(c.query_row(
                    "SELECT group_concat(session_id || task_id || op, '|') FROM context_gateway_audit",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert!(!text.contains(SECRET) && !text.contains("No.md"));
    }

    #[test]
    fn an_expired_grant_reads_nothing_and_writes_no_audit() {
        let f = fixture();
        f.note(&format!("{IN_DIR}/Ok.md"), "fine");
        let mut g = grant(4000);
        g.expires_at = Some(Utc::now() - Duration::seconds(1));
        let err = f.run(&g, &read(&format!("{IN_DIR}/Ok.md"))).unwrap_err();
        assert_eq!(err, Unavailable::Expired);
        let rows =
            f.db.with_conn(|c| context_audit::counts_for_task(c, "t-1"))
                .unwrap();
        assert!(rows.is_empty());
    }
}
