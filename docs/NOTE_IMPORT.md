# Importing files as notes

Most vaults collect files that are not Markdown: PDFs, Word documents,
spreadsheets, slide decks, scans, exports from other tools. Search, the context
engine and the agents only see Markdown notes, so those files are invisible to
HQ until they are converted. This is the other direction of
[note export](NOTE_EXPORT.md): any file in the vault becomes a note that names
the file it came from.

| Where | How |
|-------|-----|
| Terminal | `hq vault inventory [folder]` lists the files that are not notes. `hq vault import <file>... \| <folder> \| --all [--apply]` converts them. |
| Agents (MCP) | `vault_inventory` (read-only), then `vault_import` with `path`, `paths` or `directory`. `convert_to_markdown` converts any file and returns the Markdown without writing anything. |

Both read only files inside the vault. Nothing outside it is touched.

## The cleanup flow

1. **Look.** `hq vault inventory` counts what is there and says, for each file,
   whether it is `pending`, `imported`, `changed` since it was imported, or
   `blocked` because this host lacks a tool it needs (OCR, LibreOffice).
2. **Preview.** `hq vault import Notebooks/Docs` (a folder, or `--all`) is a dry
   run: every file is converted and reported, nothing is written. `--preview 600`
   shows the start of each note.
3. **Write.** Add `--apply`. Each file becomes a note and the report says what
   was written. Running it again skips what is done.
4. **Tidy (optional).** `--archive` moves each original to
   `_archive/originals/<its old path>` after its note is written, and points the
   note's `source` at the new place. Originals are moved, never deleted, and files in
   `_media/` stay where they are because conversations refer to them.

A folder import leaves pictures (photos, screenshots, slide renders) out of the batch and
says how many it left, because reading dozens of them by OCR mostly makes notes nobody
wants. `--images` (`include_images` from an agent) brings them in, `--format pdf,docx` picks
formats, and a picture named on its own is always imported.

Named files (`hq vault import report.docx`) are imported straight away, and
`--dry-run` holds them back. From an agent, `path` and `paths` write at once, and
`directory` is a dry run until you pass `dry_run: false` and handles 10 files per
call by default (`limit`, at most 200; the result says how many are left).

## What you get

A note next to the original when it sits under `Notebooks/`, otherwise in
`Notebooks/Imports/<year-month>/`. `--destination` picks another folder. The
frontmatter records where it came from:

```yaml
title: Invoice
noteType: imported-document
source: _media/2026-09-19/Invoice.docx
source_sha256: 3f1b9e...
source_format: docx
converter: native:docx
conversion: ok            # or degraded
converted_at: 2026-10-19T10:16:17+00:00
tags: [imported, docx]
```

When something was left out, the note ends with an **Import notes** section that
lists it, and the result of the command lists it too. `conversion: degraded` means
the Markdown lacks something the file had (for example text that was read by OCR,
or a layout that could not be kept).

## Safe to repeat

- A file already imported and unchanged is skipped, wherever its note is now. HQ
  finds earlier imports by the `source` and `source_sha256` in the notes' frontmatter, so
  moving or renaming an imported note does not make the file look new again.
- A changed file is reported as `changed` and skipped, unless you pass
  `--on-conflict overwrite`. Overwrite replaces only notes an import wrote, and edits
  you made to one are lost.
- A name that another note already uses gets a short hash (`Report-3f1b9e.md`). A
  note written by hand is never replaced.
- HQ's own folders (`_system`, `_threads`, `_data`), hidden folders, `_trash`,
  `_archive` and `Exports` are not searched, and links are not followed.
- Every note records who wrote it (the owner from the terminal, the calling agent
  or connector from a tool) like any other vault write, and is added to the search
  index straight away when the database is there.

## Formats

| Format | Read by | Notes |
|--------|---------|-------|
| DOCX | native | headings, bold and italic, links, lists, tables, footnotes listed at the end, code styles |
| PPTX | native | one section per slide, bullets with their levels, tables, speaker notes |
| ODT, ODP | native | the same as their Microsoft counterparts |
| RTF | native | text, styles, lists, tables, Unicode |
| HTML | native | encoding detected (UTF-8, UTF-16, Latin-1 and others), scripts and styles dropped, tables kept |
| XML | native | records become a table, anything else an outline; XML that is not well formed is kept as a code block |
| XLSX, XLS | calamine, with number formats applied | one section per sheet; old XLS through LibreOffice when it is installed |
| ODS | native, from the text the sheet shows | one section per sheet |
| CSV, TSV, text | native | delimiter and encoding detected |
| PDF | poppler `pdftotext -layout` when present, else the text layer | tables rebuilt from column layout, including a table that runs across pages; a PDF with no text is read by OCR |
| Images | OCR (tesseract on Linux, Vision on macOS) | always `degraded`; with tesseract the note says the average confidence and lists the words it was least sure of, numbers first |
| DOC, PPT | LibreOffice, when installed | `blocked` in the inventory on a host without it |

The format comes from the file's content, and its name is only the fallback. A Word
file called `.pdf` is read as Word, and the report says the name was wrong.
Password-protected files are refused, not guessed at.

## What the host needs

The built-in readers need nothing installed. Three optional tools widen what a host can read:

| Tool | Package (Debian, Ubuntu) | What it adds |
|------|--------------------------|--------------|
| tesseract and pdftoppm | `tesseract-ocr poppler-utils` | scans, photos and PDFs with no text layer |
| pdftotext | `poppler-utils` | tables in PDFs |
| LibreOffice | `libreoffice-writer libreoffice-impress libreoffice-calc` (`--no-install-recommends`) | old `.doc`, `.ppt` and `.xls` files |

`hq doctor` lists which of them this machine has, under "Importing files as notes", and
`hq vault inventory` shows the same under "this host" and marks the files that need a missing
tool as `blocked`. On macOS, OCR uses the system's Vision framework, and `brew install poppler`
and `brew install --cask libreoffice` add the others. On a server, install them by hand: the
installer leaves them out (`deploy/README.md`).

## Limits

- Files over 64 MiB, archives with more than 20,000 parts or that unpack to more than
  256 MiB, and XML with millions of nodes are refused. Output is cut at 2,000,000
  characters and the cut is reported.
- Spreadsheet cells are shown the way the sheet shows them: `9.7%`, `UGX 2,700,000`,
  `(2,500)`, dates as `2026-07-01`, times as `15:36`. Formulas show their last computed
  value, and a file saved without computing them (some generated ones) has none.
  Fractions (`# ?/?`), conditional formats and engineering notation fall back to the
  plain number. An old `.xls` keeps its formats only when LibreOffice can re-save it first;
  otherwise its numbers show as stored, and the report says so.
- Pictures, charts and embedded objects are not carried into the note. The report
  counts them.
- In RTF, two tables with nothing between them are told apart only by a change in the
  number of cells per row (Word marks merged cells, so those stay in their table). Two
  adjacent tables with the same number of columns read as one.
- A scan is only as good as OCR. The confidence figure finds the words OCR doubted, but it
  can be confidently wrong (a digit clipped at the edge of a page read as another digit with 96%
  confidence in one of the test files), so check names, numbers and amounts against the original.
- Tesseract runs on one thread per picture with a three-minute limit. Left to itself it starts a
  thread per core in every process, and a few pictures at once took minutes instead of seconds.

## For agents

`vault_inventory` and `vault_import` live in the `vault` category (`hq_discover`
lists them). From claude.ai an import needs no approval unless a path, folder or
destination is in a system folder (`skills/`, `_system/`, `_commands/`, `agents/`
and so on), the same rule as other vault writes. A session that has run an import
counts as having read untrusted content, because the preview is the document's
text. Restricted audiences (family members, guests) cannot use either tool.

`convert_to_markdown` accepts an absolute path or one relative to the vault, returns
`markdown`, `title` and a `report`, and writes nothing. Use it to read a file without
adding a note.

## For developers

| What | Where |
|------|-------|
| Pipeline entry points, options, capabilities | `crates/hq-convert/src/ingest/mod.rs` |
| Signature-first sniffing | `ingest/sniff.rs` |
| Intermediate form and the one Markdown writer | `ingest/ir.rs` |
| Report and loss kinds | `ingest/report.rs` |
| Safe zip and XML reading, limits | `ingest/package.rs` |
| Readers | `ingest/docx.rs`, `pptx.rs`, `odf.rs`, `rtf.rs`, `html.rs`, `xmlconv.rs`, `sheet.rs`, `ods.rs`, `text.rs`, `pdf.rs`, `pdf_layout.rs`, `legacy.rs` |
| Spreadsheet number formats | `ingest/numfmt.rs` (format codes), `ingest/xlsx_formats.rs` (which cell has which) |
| OCR and its confidence | `crates/hq-convert/src/ocr.rs` |
| The vault flow (inventory, import, idempotency, archive) and the two tools | `crates/hq-tools/src/vault/import.rs` |
| Terminal commands | `crates/hq-cli/src/commands/vault_import.rs` |
| Connector approval and taint rules | `crates/hq-mcp/src/gateway.rs`, `crates/hq-agent/src/governance/taint.rs` |
| Sample documents and tests | `crates/hq-convert/tests/fixtures/corpus/`, `tests/corpus.rs` |

Try one file without a vault:

```bash
cargo run -p hq-convert --example convert -- path/to/file.docx          # a summary line
cargo run -p hq-convert --example convert -- --md path/to/file.docx     # the Markdown
cargo run -p hq-convert --example convert -- --json path/to/file.pdf    # the report
```

The fixtures are invented documents (an invoice in four formats, a proposal, a
tracker, a deck, a two-page statement, a receipt photo, HTML in three encodings).
For a wider check, `scripts/fetch-conversion-corpus.sh` downloads about seventy real
public test files into `target/conversion-corpus`, and
`cargo test -p hq-convert --test real_corpus -- --ignored` converts them and prints
a summary. Those tests need the network, so they are diagnostics and not part of CI.
