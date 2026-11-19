# Document conversion and import

Notes for working under `crates/hq-convert/`, moved out of the root `CLAUDE.md` so an assistant loads them only when it is in this area. The root file has the rules that apply everywhere.

## Entry points

| What | Path |
|------|------|
| Importing files as notes (pdf, docx, xlsx, pptx, odt, rtf, html, xml, csv, images, old doc/ppt): `hq vault inventory`, `hq vault import`, `vault_inventory`, `vault_import`, `convert_to_markdown`, `POST /api/note/import` | `crates/hq-convert/src/ingest/` (entry `mod.rs`, sniffing `sniff.rs`, intermediate form and Markdown writer `ir.rs`, report `report.rs`, limits `package.rs`, one reader per format, PDF tables `pdf_layout.rs`, spreadsheet number formats `numfmt.rs` and `xlsx_formats.rs`, OCR confidence in `crates/hq-convert/src/ocr.rs`), `crates/hq-tools/src/vault/import.rs` (inventory, idempotent import by frontmatter `source`/`source_sha256`, archive, the two tools), `crates/hq-web/src/note_import.rs` (owner-only direct import route), `crates/hq-cli/src/commands/vault_import.rs`, `crates/hq-tools/src/convert.rs` (`convert_to_markdown`), connector approval in `crates/hq-mcp/src/gateway.rs` (`vault_import` is in `VAULT_WRITE_TOOLS`), taint in `crates/hq-agent/src/governance/taint.rs`, `crates/hq-convert/tests/corpus.rs`, `docs/NOTE_IMPORT.md` |
