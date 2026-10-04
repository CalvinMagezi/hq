# Vault graph capability audit (FR-063)

This audit was written before the FR-063 code and describes the graph as it
stood on `main`. The last section says what changed and what did not.

## What exists

`memory_entity_graph(seeds[], hops, limit)` in `crates/hq-tools/src/vault/graph.rs`
calls `hq_memory::entity_graph::spreading_activation`. It reads two SQLite
tables created by migration 058:

- `entity_nodes(id, canonical, display_name, entity_type, mention_count, created_at, updated_at)`
- `entity_edges(source_id, target_id, relationship, weight, source_memory_id, updated_at)`,
  primary key `(source_id, target_id, relationship)`

The tables are a derived cache. The source of truth is the set of concept pages
under `_graph/*.md` in the vault (`crates/hq-memory/src/concept_pages.rs`).
`derive_entity_index` rebuilds both tables from those pages; the consolidator
and the forgetter call it. Writers never touch the tables directly any more
(`ingester.rs` and `consolidator.rs` write pages).

## Findings

| Area | Finding |
|------|---------|
| Schema | Nodes are entities, not notes. Nothing links a node to a source note except its concept page, which was not recorded. |
| Edge semantics | Only wikilinks between concept pages become edges, all with relationship `linked`. They are stored undirected (`min(id)` is the source). The `co_occurs` and `relates_to` types in `types.rs` are never written by the current derive path. |
| Provenance | Derived edges always had `source_memory_id = NULL`. No edge pointed at supporting evidence. |
| Time | `updated_at` on an edge was the rebuild time, not the time the evidence changed. There was no confidence or verification status. |
| Traversal | `spreading_activation` returns a ranked list of entities with an activation score. It never returns the path that produced a score, so a result cannot be explained. It has a visited set (cycle safe) and a hop limit, but one edge query per node visited. |
| Lookup | Seeds match `canonical` exactly (lowercase, trimmed). No fuzzy match, no alias handling. |
| Update behavior | The only maintenance path was `derive_entity_index`: delete both tables, then re-insert across many separate statements with no transaction. A failure midway left a partial graph that looked healthy. There was no incremental update, no freshness marker, and no cleanup for moved or deleted pages except the next full rebuild. |
| Failure modes | If the tables are empty or stale the tool returns an empty or outdated list with no signal. Nothing degrades to vault search. |
| Search coverage | FTS indexes `Notebooks/` only, so `_graph/` pages are not searchable. A fallback to vault search reaches Notebooks notes, not concept pages. Direct references reach any page. |
| Consumers | `memory_entity_graph`, `hq memory` CLI, and `MemoryForgetter::archive_stale_concept_pages` (which reads edge counts). The Spark read-only allowlist includes the tool. |

Verdict: the facility is an entity-neighborhood lookup over a derived index.
It has no typed relations, no source provenance per edge, no path output, no
freshness signal and no safe update story. It is worth extending because the
page-to-index derivation and its consumers already work.

## What FR-063 changes

Everything below extends the existing tables and pages. There is no second graph.

- Migration 063 adds `entity_nodes.page_path`, `page_stamp`, and
  `entity_edges.evidence_path`, `evidence_updated_at`, `confidence`, plus a
  one-row `entity_index_state` (`ok` or `degraded`).
- Concept pages may state a typed relation on a list line:
  `- criticizes: [[project-apollo]] (src: Notebooks/Meetings/review.md)`.
  `concept_pages::add_relation` writes one. Typed edges keep their direction.
  Plain wikilinks stay untyped `linked` edges.
- `graph_index` holds maintenance: an atomic rebuild (pages are read first, one
  transaction), per-page `reindex_page` (create, edit, move, delete; idempotent),
  `reconcile` (applies only the drift between vault and index), and
  `check_freshness`. Any failure marks the index `degraded`.
- `graph_paths::discover` returns bounded chains with hop, result, expansion and
  size limits, cycle-free paths, and a per-edge check that re-reads the evidence
  note. An edge whose evidence is gone or no longer mentions both endpoints is
  dropped. An edge supported only by a concept page is returned as tentative.
  `supersedes` and `contradicts` neighbors appear as caveats on a path.
- A stale or degraded index returns `Unavailable` with a pointer to vault
  search, never old results. `context_packet` turns that into a gap and carries
  on with direct references and search.
- `memory_entity_graph` gained `mode: "paths"`. The default mode is unchanged.

## Not done, on purpose

- Nothing writes typed relations automatically yet. The consolidator still
  emits plain `Also related to [[x]]` links. Extracting `criticizes` or
  `decided_for` from notes needs an LLM step that deserves its own change.
- The daemon still runs the full rebuild after consolidation. `reindex_page` and
  `reconcile` are available but not wired into note-write events.
- Seed matching is still exact on the canonical name.
