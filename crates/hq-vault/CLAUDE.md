# The vault client

Notes for working under `crates/hq-vault/`, moved out of the root `CLAUDE.md` so an assistant loads them only when it is in this area. The root file has the rules that apply everywhere.

## Entry points

| What | Path |
|------|------|
| Vault client | `crates/hq-vault/src/client.rs` |
| Links between notes: reading them out of text, saying what each one points at, changing only the target part (`extract`, `NoteSet::resolve`, `retarget`, `apply_edits`) | `crates/hq-vault/src/links.rs`, the shared corpus `crates/hq-vault/tests/fixtures/links_cases.json` (run by `tests/links_corpus.rs`) |
| The notes and attachments a link can name, listed by the index's rules | `crates/hq-vault/src/walk.rs` |
| Real-vault census and scale benchmark (opt-in, read-only) | `crates/hq-vault/tests/links_diag.rs` |

## Rules

- **One link grammar.** Any code that reads a `[[wikilink]]` or `[text](note.md)` goes through `hq_vault::links`. It skips code, never guesses between two notes with one name (`Resolution::Ambiguous`), and edits only a link's target span. Do not add another regex or `find("[[")` scan. A case that parses wrong goes into `links_cases.json` first.
- **`walk.rs` repeats the index's note rules** (`hq_db::search::reconcile`), because `hq-db` does not depend on this crate. Change both together.
- Measured (release, warm cache, 2026-11-26): walking, reading and resolving 20,000 notes with 240,000 links takes about half a second, so a scan needs no persistent link index yet. Re-measure with `scale_benchmark` before adding one.
