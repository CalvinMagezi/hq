//! Forgetting: delete a memory and every copy of it that HQ made.
//!
//! A memory leaves traces beyond its own row. The insight notes written from it quote it, its
//! name is on concept pages, the line summarising an insight sits in `MEMORY.md`, and a pruned
//! copy may be in the archive. "Forget" removes all of those, keeps only a fingerprint so the
//! same words are not learned again, and says plainly what it could not reach.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use hq_db::Database;
use hq_db::memories_owner::add_tombstone;
use hq_db::search::remove_path_everywhere;
use hq_vault::VaultClient;
use rusqlite::OptionalExtension;
use serde::Serialize;

use crate::concept_pages::{frontmatter_string_list, yaml_string_list};
use crate::hash::content_hash;

/// Where HQ writes the notes it makes from several memories.
const INSIGHT_DIR: &str = "Notebooks/Memories";
const INSIGHTS_HEADER: &str = "## Agent Insights (Auto-Generated)";

/// What forgetting a memory could not reach. Shown to the owner, so nobody thinks more is gone
/// than is.
pub const NOT_REACHED: [&str; 2] = [
    "The chat it came from still holds what you said there.",
    "Backups made before now still hold it.",
];

/// What was removed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ForgetReport {
    /// Insights that were drawn from it, with the ones evolved from them.
    pub insights_removed: usize,
    /// The notes those insights were written into.
    pub insight_notes_removed: Vec<String>,
    /// Concept pages that no longer list it as a source.
    pub concept_pages_updated: usize,
    /// Concept pages that existed only because of it.
    pub concept_pages_removed: usize,
    /// Lines in the archive of aged-out memories.
    pub archive_entries_removed: usize,
}

/// Forgets memory `id`. Returns `None` when there is no such memory.
pub fn forget(db: &Database, vault_path: &Path, id: i64) -> Result<Option<ForgetReport>> {
    // The row, the fingerprint and the insights drawn from it go together or not at all.
    let Some(doomed) = db.with_conn(|conn| {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
        let Some(text) = tx
            .query_row(
                "SELECT COALESCE(NULLIF(summary, ''), content, '') FROM memories WHERE id = ?1",
                [id],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        else {
            return Ok(None);
        };
        add_tombstone(&tx, &content_hash(&text))?;
        let doomed = insights_to_remove(&tx, id)?;
        for (insight_id, _) in &doomed {
            tx.execute("DELETE FROM consolidations WHERE id = ?1", [insight_id])?;
        }
        tx.execute("DELETE FROM memories WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(Some(doomed))
    })?
    else {
        return Ok(None);
    };

    let mut report = ForgetReport {
        insights_removed: doomed.len(),
        ..ForgetReport::default()
    };

    // The notes written from the insights, found both by the path recorded for them and by the
    // memory numbers they list.
    let mut notes: Vec<String> = doomed
        .iter()
        .filter_map(|(_, path)| path.clone())
        .filter(|p| p.starts_with(&format!("{INSIGHT_DIR}/")) && !p.contains(".."))
        .collect();
    notes.extend(insight_notes_listing(vault_path, id));
    notes.sort();
    notes.dedup();
    for rel in notes {
        let file = vault_path.join(&rel);
        if file.is_file() && std::fs::remove_file(&file).is_ok() {
            db.with_conn(|conn| remove_path_everywhere(conn, &rel))?;
            report.insight_notes_removed.push(rel);
        }
    }

    if report.insights_removed > 0 {
        refresh_memory_md(db, vault_path)?;
    }
    drop_from_concept_pages(db, vault_path, id, &mut report)?;
    report.archive_entries_removed = drop_from_archive(vault_path, id)?;
    Ok(Some(report))
}

/// The insights that cite `id`, and every insight evolved from one of them, with the note each
/// was written into when that was recorded.
fn insights_to_remove(conn: &rusqlite::Connection, id: i64) -> Result<Vec<(i64, Option<String>)>> {
    struct Insight {
        id: i64,
        cites: Vec<i64>,
        parent: Option<i64>,
        note: Option<String>,
    }
    let mut stmt =
        conn.prepare("SELECT id, memory_ids, parent_id, note_path FROM consolidations")?;
    let all: Vec<Insight> = stmt
        .query_map([], |r| {
            let ids: String = r.get(1)?;
            Ok(Insight {
                id: r.get(0)?,
                cites: serde_json::from_str(&ids).unwrap_or_default(),
                parent: r.get(2)?,
                note: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    let mut doomed: HashSet<i64> = all
        .iter()
        .filter(|i| i.cites.contains(&id))
        .map(|i| i.id)
        .collect();
    // An insight that grew out of a removed one repeats part of it.
    loop {
        let before = doomed.len();
        for i in &all {
            if i.parent.is_some_and(|p| doomed.contains(&p)) {
                doomed.insert(i.id);
            }
        }
        if doomed.len() == before {
            break;
        }
    }
    Ok(all
        .into_iter()
        .filter(|i| doomed.contains(&i.id))
        .map(|i| (i.id, i.note))
        .collect())
}

/// Insight notes whose "Source Memories" list names memory `id`.
fn insight_notes_listing(vault_path: &Path, id: i64) -> Vec<String> {
    let marker = format!("**#{id}**");
    let Ok(entries) = std::fs::read_dir(vault_path.join(INSIGHT_DIR)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .filter(|e| std::fs::read_to_string(e.path()).is_ok_and(|text| text.contains(&marker)))
        .filter_map(|e| Some(format!("{INSIGHT_DIR}/{}", e.file_name().to_str()?)))
        .collect()
}

/// Rewrites the insight lines in `MEMORY.md` from the insights that are left. The usual refresh
/// leaves the file alone when none are left, which would keep the removed lines, so that case is
/// handled here.
fn refresh_memory_md(db: &Database, vault_path: &Path) -> Result<()> {
    let memory_md = vault_path.join("_system").join("MEMORY.md");
    if !memory_md.is_file() {
        return Ok(());
    }
    if crate::get_consolidation_history(db, 1)?.is_empty() {
        let text = std::fs::read_to_string(&memory_md)?;
        let without = without_insights_section(&text);
        if without != text {
            std::fs::write(&memory_md, without)?;
        }
        return Ok(());
    }
    crate::MemoryConsolidator::new(db.clone(), vault_path.to_path_buf()).refresh_memory_file()
}

/// `MEMORY.md` without the auto-generated insights section, up to the next section or the end.
fn without_insights_section(text: &str) -> String {
    let Some(start) = text.find(INSIGHTS_HEADER) else {
        return text.to_string();
    };
    let rest = &text[start + INSIGHTS_HEADER.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    let mut out = text[..start].trim_end().to_string();
    out.push('\n');
    let after = &rest[end..];
    if !after.trim().is_empty() {
        out.push_str(after);
        if !after.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// Takes `memory:{id}` off every concept page that lists it, and removes a page that HQ made
/// only from that memory and nobody has written in.
fn drop_from_concept_pages(
    db: &Database,
    vault_path: &Path,
    id: i64,
    report: &mut ForgetReport,
) -> Result<()> {
    let Ok(vault) = VaultClient::new(vault_path.to_path_buf()) else {
        return Ok(());
    };
    let pages = vault.list_notes_recursive("_graph").unwrap_or_default();
    let source = format!("memory:{id}");
    let mut changed = false;
    for rel in pages {
        let Ok(mut note) = vault.read_note(&rel) else {
            continue;
        };
        let mut refs = frontmatter_string_list(&note, "source_refs");
        if !refs.contains(&source) {
            continue;
        }
        refs.retain(|r| *r != source);
        changed = true;
        if refs.is_empty() && only_what_hq_wrote(&note.content) {
            if std::fs::remove_file(vault_path.join(&rel)).is_ok() {
                db.with_conn(|conn| remove_path_everywhere(conn, &rel))?;
                report.concept_pages_removed += 1;
            }
        } else {
            note.frontmatter
                .insert("source_refs".to_string(), yaml_string_list(&refs));
            vault.write_note(&rel, &note)?;
            report.concept_pages_updated += 1;
        }
    }
    if changed {
        // The entity tables are a cache built from these pages.
        crate::concept_pages::derive_entity_index(db, &vault)?;
    }
    Ok(())
}

/// Whether a concept page holds only the lines HQ writes itself: a title, and "related to" links.
/// A page with anything else has been written in, and is not ours to delete.
fn only_what_hq_wrote(content: &str) -> bool {
    content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .all(|l| {
            l.starts_with("# ") || l.starts_with("Related to ") || l.starts_with("Also related to ")
        })
}

/// Removes the memory from the monthly archive files of memories that aged out.
fn drop_from_archive(vault_path: &Path, id: i64) -> Result<usize> {
    let dir = vault_path.join("_archive").join("memories");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(0);
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|x| x != "jsonl") {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        let (mut kept, mut dropped) = (Vec::new(), 0usize);
        for line in text.lines() {
            let is_it = serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|v| v.get("id").and_then(|i| i.as_i64()))
                == Some(id);
            if is_it {
                dropped += 1;
            } else {
                kept.push(line);
            }
        }
        if dropped > 0 {
            let mut out = kept.join("\n");
            if !out.is_empty() {
                out.push('\n');
            }
            std::fs::write(&path, out)?;
            removed += dropped;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::concept_pages::upsert_concept_page;
    use crate::db::{StoreMemoryParams, store_consolidation, store_memory};

    struct Vault {
        db: Database,
        dir: tempfile::TempDir,
    }

    impl Vault {
        fn new() -> Self {
            Self {
                db: Database::open_memory().unwrap(),
                dir: tempfile::tempdir().unwrap(),
            }
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }

        fn memory(&self, summary: &str, entities: &[&str]) -> i64 {
            let entities: Vec<String> = entities.iter().map(ToString::to_string).collect();
            store_memory(
                &self.db,
                &StoreMemoryParams {
                    source: "chat-turn",
                    harness: "web",
                    raw_text: "",
                    summary,
                    entities: &entities,
                    topics: &[],
                    importance: 0.5,
                    user_id: None,
                },
            )
            .unwrap()
        }

        fn write(&self, rel: &str, text: &str) {
            let file = self.path().join(rel);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }

        fn exists(&self, rel: &str) -> bool {
            self.path().join(rel).exists()
        }

        fn read(&self, rel: &str) -> String {
            std::fs::read_to_string(self.path().join(rel)).unwrap_or_default()
        }

        fn rows(&self, table: &str) -> i64 {
            self.db
                .with_conn(|c| {
                    Ok(c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?)
                })
                .unwrap()
        }
    }

    #[test]
    fn there_is_nothing_to_forget_when_there_is_no_such_memory() {
        let v = Vault::new();
        assert_eq!(forget(&v.db, v.path(), 42).unwrap(), None);
        assert_eq!(
            v.rows("memory_tombstones"),
            0,
            "nothing is remembered about nothing"
        );
    }

    #[test]
    fn forgetting_removes_the_row_the_search_entry_and_leaves_only_a_fingerprint() {
        let v = Vault::new();
        let id = v.memory("Acme Feeds delivers on Thursdays", &[]);
        let other = v.memory("The loan is due on the 15th", &[]);

        let report = forget(&v.db, v.path(), id).unwrap().unwrap();

        assert_eq!(report, ForgetReport::default());
        assert_eq!(v.rows("memories"), 1);
        let left: i64 =
            v.db.with_conn(|c| Ok(c.query_row("SELECT id FROM memories", [], |r| r.get(0))?))
                .unwrap();
        assert_eq!(left, other);
        let hits =
            v.db.with_conn(|c| {
                hq_db::memories_search::search_keyword(
                    c,
                    "thursdays",
                    hq_db::memories_search::View::Owner,
                    5,
                )
            })
            .unwrap();
        assert!(hits.is_empty(), "not findable any more");
        let hash: String =
            v.db.with_conn(|c| {
                Ok(
                    c.query_row("SELECT content_hash FROM memory_tombstones", [], |r| {
                        r.get(0)
                    })?,
                )
            })
            .unwrap();
        assert_eq!(hash, content_hash("Acme Feeds delivers on Thursdays"));
        assert!(!hash.contains("Acme"), "the words themselves are not kept");
    }

    #[test]
    fn everything_made_from_the_memory_goes_with_it_and_what_is_not_its_own_stays() {
        let v = Vault::new();
        let gone = v.memory("Acme Feeds delivers on Thursdays", &["Acme Feeds"]);
        let kept = v.memory("The loan is due on the 15th", &["Acme Feeds"]);

        // An insight drawn from both, the note written from it and the line in MEMORY.md.
        store_consolidation(
            &v.db,
            &[gone, kept],
            "Suppliers and the loan share a calendar",
            &[],
        )
        .unwrap();
        let note = "Notebooks/Memories/2026-11-01-10-00-00-insight.md";
        v.write(
            note,
            &format!("---\nstatus: inferred\n---\n\n## Source Memories\n\n- **#{gone}** (chat-turn): Acme Feeds delivers on Thursdays\n"),
        );
        // An unrelated insight note must be left alone.
        let unrelated = "Notebooks/Memories/2026-10-01-10-00-00-insight.md";
        v.write(
            unrelated,
            "## Source Memories\n\n- **#999** (chat-turn): something else\n",
        );
        v.write(
            "_system/MEMORY.md",
            "# HQ Memory\n\n## Key Facts\n\n- I keep bees\n",
        );
        crate::MemoryConsolidator::new(v.db.clone(), v.path().to_path_buf())
            .refresh_memory_file()
            .unwrap();
        assert!(
            v.read("_system/MEMORY.md")
                .contains("Suppliers and the loan")
        );

        // Concept pages: one made only from this memory, one shared with another.
        let vault = VaultClient::new(v.path().to_path_buf()).unwrap();
        upsert_concept_page(
            &vault,
            "Acme Feeds",
            "concept",
            &[],
            &format!("memory:{gone}"),
        )
        .unwrap();
        upsert_concept_page(
            &vault,
            "Acme Feeds",
            "concept",
            &[],
            &format!("memory:{kept}"),
        )
        .unwrap();
        upsert_concept_page(
            &vault,
            "Thursday Run",
            "concept",
            &["Acme Feeds".into()],
            &format!("memory:{gone}"),
        )
        .unwrap();

        // The monthly archive of aged-out memories.
        v.write(
            "_archive/memories/2026-09.jsonl",
            &format!("{{\"id\":{gone},\"summary\":\"Acme Feeds delivers on Thursdays\"}}\n{{\"id\":777,\"summary\":\"another\"}}\n"),
        );

        let report = forget(&v.db, v.path(), gone).unwrap().unwrap();

        assert_eq!(report.insights_removed, 1);
        assert_eq!(report.insight_notes_removed, vec![note.to_string()]);
        assert!(!v.exists(note), "the note that quotes it is deleted");
        assert!(v.exists(unrelated), "a note about other memories is not");
        assert_eq!(v.rows("consolidations"), 0);
        let memory_md = v.read("_system/MEMORY.md");
        assert!(!memory_md.contains("Suppliers and the loan"), "{memory_md}");
        assert!(
            memory_md.contains("I keep bees"),
            "the owner's own facts stay"
        );

        assert_eq!(report.concept_pages_removed, 1, "{report:?}");
        assert!(!v.exists("_graph/thursday-run.md"));
        assert_eq!(report.concept_pages_updated, 1);
        let shared = v.read("_graph/acme-feeds.md");
        assert!(shared.contains(&format!("memory:{kept}")), "{shared}");
        assert!(!shared.contains(&format!("memory:{gone}")), "{shared}");

        assert_eq!(report.archive_entries_removed, 1);
        let archive = v.read("_archive/memories/2026-09.jsonl");
        assert!(
            archive.contains("another") && !archive.contains("Thursdays"),
            "{archive}"
        );
    }

    #[test]
    fn a_concept_page_the_owner_wrote_in_survives_with_the_reference_removed() {
        let v = Vault::new();
        let id = v.memory("Acme Feeds delivers on Thursdays", &["Acme Feeds"]);
        let vault = VaultClient::new(v.path().to_path_buf()).unwrap();
        upsert_concept_page(
            &vault,
            "Acme Feeds",
            "concept",
            &[],
            &format!("memory:{id}"),
        )
        .unwrap();
        let mut note = vault.read_note("_graph/acme-feeds.md").unwrap();
        note.content
            .push_str("\nMy contact there is Kofi, call before 9.\n");
        vault.write_note("_graph/acme-feeds.md", &note).unwrap();

        let report = forget(&v.db, v.path(), id).unwrap().unwrap();

        assert_eq!(
            (report.concept_pages_removed, report.concept_pages_updated),
            (0, 1)
        );
        let page = v.read("_graph/acme-feeds.md");
        assert!(page.contains("My contact there is Kofi"), "{page}");
        assert!(!page.contains(&format!("memory:{id}")));
    }

    #[test]
    fn insights_evolved_from_a_removed_one_go_too_and_the_last_line_leaves_memory_md() {
        let v = Vault::new();
        let id = v.memory("Acme Feeds delivers on Thursdays", &[]);
        store_consolidation(&v.db, &[id], "Thursday deliveries matter", &[]).unwrap();
        let first: i64 =
            v.db.with_conn(|c| Ok(c.query_row("SELECT id FROM consolidations", [], |r| r.get(0))?))
                .unwrap();
        crate::db::store_consolidation_chained(
            &v.db,
            &[],
            "Thursday deliveries matter more now",
            &[],
            first,
        )
        .unwrap();
        v.write(
            "_system/MEMORY.md",
            "# HQ Memory\n\n## Key Facts\n\n- I keep bees\n\n## Agent Insights (Auto-Generated)\n\n- [2026-11-01] (inferred) Thursday deliveries matter\n\n## Goals\n\n- Open a second shop\n",
        );

        let report = forget(&v.db, v.path(), id).unwrap().unwrap();

        assert_eq!(
            report.insights_removed, 2,
            "the insight and the one that grew from it"
        );
        assert_eq!(v.rows("consolidations"), 0);
        let memory_md = v.read("_system/MEMORY.md");
        assert!(!memory_md.contains("Thursday deliveries"), "{memory_md}");
        assert!(
            !memory_md.contains("Agent Insights"),
            "an empty section is dropped: {memory_md}"
        );
        assert!(
            memory_md.contains("I keep bees") && memory_md.contains("Open a second shop"),
            "{memory_md}"
        );
    }

    #[tokio::test]
    async fn the_ingester_skips_a_sentence_the_owner_had_forgotten() {
        let v = Vault::new();
        let id = v.memory("Asked: When does Acme deliver?", &[]);
        forget(&v.db, v.path(), id).unwrap().unwrap();
        let mut ingester = crate::MemoryIngester::new_local(v.db.clone(), v.path().to_path_buf());

        let stored = ingester
            .ingest(
                "user: When does Acme deliver?\n---\nassistant: Thursdays, I think.",
                "chat-turn",
                Some("web"),
                None,
                &crate::types::Trace::default(),
            )
            .await
            .unwrap();

        assert_eq!(stored, None, "the same words are not learned again");
        assert_eq!(v.rows("memories"), 0);
    }

    #[test]
    fn the_insights_section_is_cut_cleanly_whatever_surrounds_it() {
        let at_end = "# M\n\n## Key Facts\n\n- a\n\n## Agent Insights (Auto-Generated)\n\n- x\n";
        assert_eq!(
            without_insights_section(at_end),
            "# M\n\n## Key Facts\n\n- a\n"
        );
        let in_middle = "## Agent Insights (Auto-Generated)\n\n- x\n\n## Goals\n\n- g\n";
        assert_eq!(without_insights_section(in_middle), "\n\n## Goals\n\n- g\n");
        assert_eq!(
            without_insights_section("# nothing here\n"),
            "# nothing here\n"
        );
    }

    #[test]
    fn what_hq_wrote_on_a_page_is_told_from_what_someone_added() {
        assert!(only_what_hq_wrote(
            "# Acme\n\nRelated to [[kofi]].\n\nAlso related to [[bees]]."
        ));
        assert!(!only_what_hq_wrote("# Acme\n\nCall before 9."));
    }

    #[test]
    fn what_cannot_be_reached_is_said_plainly() {
        assert!(NOT_REACHED.iter().any(|s| s.contains("chat")));
        assert!(NOT_REACHED.iter().any(|s| s.contains("Backups")));
    }
}
