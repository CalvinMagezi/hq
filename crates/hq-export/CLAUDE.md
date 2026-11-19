# Note export

Notes for working under `crates/hq-export/`, moved out of the root `CLAUDE.md` so an assistant loads them only when it is in this area. The root file has the rules that apply everywhere.

## Entry points

| What | Path |
|------|------|
| Note export (pdf, docx, png, svg, html, md, xlsx, csv, json, jsonl, xml, latex, ipynb, jira, code): `hq vault export --format`, `hq vault export-pdf`, `vault_export`, `vault_export_pdf`, `/api/note/export`, `/api/note/pdf` | `crates/hq-export/` (document model `doc.rs`, writers, Typst layout `typst_markup.rs` and `render.rs`, vault glue `note.rs`), `crates/hq-convert/src/note_pdf.rs` (note preparation, legacy PDF engines behind `HQ_PDF_ENGINE`), `crates/hq-tools/src/convert.rs`, `crates/hq-web/src/vault_api.rs`, `docs/NOTE_EXPORT.md` |
