# The vault client

Notes for working under `crates/hq-vault/`, moved out of the root `CLAUDE.md` so an assistant loads them only when it is in this area. The root file has the rules that apply everywhere.

## Entry points

| What | Path |
|------|------|
| Vault client | `crates/hq-vault/src/client.rs` |
| Links between notes: reading them out of text, saying what each one points at, changing only the target part (`extract`, `NoteSet::resolve`, `retarget`, `apply_edits`) | `crates/hq-vault/src/links.rs`, the shared corpus `crates/hq-vault/tests/fixtures/links_cases.json` (run by `tests/links_corpus.rs`) |
| The notes and attachments a link can name, listed by the index's rules | `crates/hq-vault/src/walk.rs` |
| Real-vault census and scale benchmark (opt-in, read-only) | `crates/hq-vault/tests/links_diag.rs` |
| Keeping links pointing where they pointed when notes move (`plan_moves`, `apply`; `reorg::move_note` and `move_folder` use it), who links here (`backlink_index`) | `crates/hq-vault/src/relink.rs` |
| Links in one note against the rest of the vault, for the write tools' warnings and `vault_links` | `crates/hq-vault/src/check.rs` |
| What could be repaired and which notes may be touched (`plan`, `may_touch`, tier A certain and B likely), and writing it under the note lock with a saved original, a manifest and undo (`repair::apply`, `repair::undo`) | `crates/hq-vault/src/fixes.rs`, `crates/hq-vault/src/repair.rs`. A repair never touches a sheet, deck, pinned note, unreviewed assistant note, `_graph/` or `Notebooks/Memories/`, or a note changed in the last ten minutes. |
| The notes check: dead and ambiguous links, missing headings, repeated notes, unreadable properties (`scan`, `scan_full`, `Report::apply_ignores`, `only_note`) | `crates/hq-vault/src/health.rs`, `docs/VAULT_HEALTH.md` |

## Rules

- **One link grammar.** Any code that reads a `[[wikilink]]` or `[text](note.md)` goes through `hq_vault::links`. It skips code, never guesses between two notes with one name (`Resolution::Ambiguous`), and edits only a link's target span. Do not add another regex or `find("[[")` scan. A case that parses wrong goes into `links_cases.json` first.
- **`walk.rs` repeats the index's note rules** (`hq_db::search::reconcile`), because `hq-db` does not depend on this crate. Change both together.
- Measured (release, warm cache, 2026-11-27, `cargo test -p hq-vault --release --test links_diag -- --ignored --nocapture`): walking, reading and resolving 20,000 notes with 240,000 links takes about half a second, so a scan needs no persistent link index. The check as a whole (`pipeline_benchmark`) adds the near-name search for each broken link: 22,000 broken links took about 9 s and planning on top of that under 1 s, because the report and the planner share one `check::NameIndex`. Share one index; building one per caller doubles the cost, and comparing a link with every note takes minutes.
