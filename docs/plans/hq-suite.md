# HQ Suite: plan

Status: M0 in progress. Owner-agreed scope; build notes are kept here, the language is in
`docs/suite/HQMD.md`, rules for the code are in `crates/suite/CLAUDE.md`.

## Problem and value

People who run HQ on a phone collect office files that arrive over chat and downloads. On a
budget phone (planning target 32 GB total, about 5 GB free; reference device Samsung Galaxy A32,
Android 13, 5.8 GB RAM, WebView 111) a hundred or two of them plus duplicates eat the space the
phone needs. The files are opaque to HQ until converted, and many of these users have no
computer, no paid cloud and expensive data.

The solution is a record that is the smallest faithful form of a file, originals that leave the
phone only with consent, and sharing back out in the formats the rest of the world uses.

How we will know (hypotheses until M0 measures real files; targets are set from measurements):

- Bytes freed per document-heavy phone, as a share of what those documents used.
- Zero silent loss: the restore drill passes every time and no shrink ever leaves a file with
  neither original nor record.
- Opening a ten-page doc and scrolling a 5,000-row sheet feel immediate on the reference phone.
- A docx or xlsx we produce opens in Word, Excel and Google's apps with no repair prompt.
- A person needs at most three choices to shrink a file: make it smaller, keep the original
  somewhere, undo.

Questions to keep asking while building:

1. "Replace Google" is a solution, not the problem. The problems are phone storage and data
   loss. A piece that serves neither is suspect.
2. The long tail of file fidelity is where effort disappears. Fix a small supported subset and
   be honest outside it.
3. Most people receive files and reply with something new. Priority: create and share well,
   then view imported files faithfully, then edit imported files.
4. Do not build all five tools before M1 proves the storage savings.

## Decisions

| # | Decision | Why |
|---|----------|-----|
| 1 | Built inside this repo under `crates/suite/`, behind the `suite` feature, extractable later | One release train and the crates it stands on |
| 2 | The vault stays plain files; notes are markdown; extensions live only in fenced blocks (hqmd) | HQ, search, import and every viewer keep working; recovery by hand |
| 3 | On the phone the record is the smallest faithful form; the original goes to a cold tier the user owns or to the 30-day trash, only after a preview and consent | Keeping both doubles the cost for those who can least afford it |
| 4 | No live co-editing, teams or sync engine in v1; no network services | Cuts the largest cost and risk; no server, account or recurring cost |
| 5 | Rust below the glass; the UI stays thin in the web app | Size, speed; text input and accessibility belong to the platform |
| 6 | Pandoc is not on the phone path | Large external binary; native readers and writers cover most formats |
| 7 | Reuse what is solid: SQLite and FTS5, zstd, calamine, rust_xlsxwriter, docx-rs, `hq-backup`, `hq-bridge`; Typst becomes an optional pack | Not where the value is |

## Layers

```
Hands     HQ web screens        office_* HQ tools        hq suite CLI
Tools     drive    docs    sheets    slides    dash
Core      md (hqmd)    calc    ingest    render
Storage   store    shrink    governor    sinks
Disk      notes (markdown)    media (content-addressed)    cache (disposable)
```

Each layer calls only the layer below it. Crates are added in the milestone that needs them.

## Storage model

Three classes: the **record** (notes and the media they link to; never evicted), **cold**
(originals that left the record, offloaded or waiting in `_trash/`), and **cache** (search
index, thumbnails, previews, `Exports/`; rebuildable, evicted first).

Shrinking one file: sniff, read into hqmd with a loss report, extract and hash media (stored
once), recompress by policy, write the note atomically, read it back and verify, classify the
fidelity tier, show a preview, and only on consent move the original. Every shrink writes a
journal entry first, so after a kill at any step the file is in exactly one of two states,
original kept or record complete, never neither. Apply is idempotent and resumable.

Never shrunk: encrypted, macro-enabled, damaged or unknown files. Scans and photographed
documents keep their page images (recompressed gently); OCR text is a search layer only.

## Milestones

| | Scope | Exit |
|---|-------|------|
| M0 | Spec, corpus, measurement, boundaries, no UI | Spec reviewed, ratios recorded from real files, encoder chosen, CI green |
| M1 | Store, shrink, governor | Kill-at-every-step tests pass; idempotence; a real shrink of a real set on the reference phone with measured savings; never neither original nor record |
| M2 | Docs, render, first screens | Round trips on the supported subset; a produced docx opens cleanly in Word and Google Docs; review the repo-placement decision |
| M3 | Sinks and recovery | A lost-phone drill on a wiped device restores and hashes match |
| M4 | Calc and Sheets | Differential tests pass; a 5,000-row sheet scrolls smoothly on the reference phone |
| M5 | Dashboard | Live blocks over sheet ranges and their render |
| M6 | Slides and pptx | Card editor and a native pptx writer |

Order: M1 proves the storage promise before any editor exists. Docs and Drive come before
Sheets because they are what people receive and share most. Slides last because it needs the
one writer that does not exist yet.

## Open questions

Blocking or near-blocking:

1. Image encoder (lossy WebP through the C library, pure-Rust JPEG, AVIF). Decide by measurement
   on the reference phone: size at equal visual quality, encode time, peak memory.
2. Native PDF on cheap phones: an embedded Typst is large. Optional pack or a small PDF writer.
3. OCR on the phone: unverified.
4. Whether `calamine` exposes enough formula information to keep formulas on xlsx import.
5. Whether a native pptx writer is worth writing or an existing crate suffices.
6. `request_delete_media` through a device action on Android 13.
7. Sink folders on Android: how the Rust process reads and writes a Storage Access Framework tree.
8. Where version diffs live: a new store or an extension of the provenance tables.
9. How the vault watcher and index treat non-markdown files and large sidecars, and
   `vault_inventory` skipping `_media/cas/`.
10. Key recovery design for sealed cold storage (phrase, shares, other).
11. Sealed per-object cold format: the `hq-backup` container or a lighter one.
12. Editor approach under the WebView 111 constraint.

Size budget: the suite adds at most 6 MB to the Android `hq` without optional packs. Measured in
M0, then made a hard CI limit.

## Status log

- M0 started: workspace folder, `hq-suite-md` (parser, printer, validator, patch API, golden
  corpus), the hqmd spec, the extraction rules and the test that enforces them.
- `hq suite measure` built: numbers-only report, candidate encoders behind `suite-candidates`
  (pure-Rust lossy WebP, libwebp, AVIF compile and round-trip).
- M1 storage layer built: `hq-suite-store` (atomic files, media by hash, trash in the vault's
  layout, the shrink journal) and the shrink pipeline in `hq-suite-shrink` with a crash matrix
  that injects a failure at every recorded step. `hq suite shrink | confirm | abandon | undo |
  jobs | recover` work against a real vault. See `docs/suite/SHRINK.md`.
- 2026-11-16, M0 measured and closed. Four real folders (1,098 files) on a laptop and a 162 MB
  sample on the Galaxy A32. **Encoder: lossy WebP through libwebp at quality 80.** At matched
  quality it is 17% smaller than JPEG 75, 2.5x faster than pure-Rust WebP, and AVIF costs seconds
  per picture on the phone and cannot be read back to verify. The losing crates are gone. The
  measuring tool had scored transparent pictures against the wrong background; fixed and tested.
  Numbers: `docs/suite/MEASURE.md`. Size budget: the suite adds 0.6 MB on the laptop build and
  0.65 MB on `aarch64-linux-android`, against 6 MiB.
- 2026-11-16, M1 finished in Rust: the storage governor (`hq-suite-governor`, `suite:` config,
  daemon task `suite-governor`, `hq suite storage [--tidy]`), shrink adopts the note an import
  already made (with its crash matrix), `vault_import` skips `_media/cas`, and the `/api/suite/*`
  routes (`crates/hq-web/src/suite_api.rs`, logic in `hq-suite-adapter::api`). The Android reader
  gets one bar for a note made from an office file (`ui/notes/SuiteBar.kt`): make it smaller,
  agree or not, undo. No new screen, tab or Activity (section 12 of the scope note).
- 2026-11-17: shrink links a Word document's pictures where they sat (`ConvertOptions::image_markers`, off by default, so `hq vault import` is unchanged; `hq-suite-shrink/src/place.rs`). On three real documents 17 of 18, 1 of 4 and 34 of 35 pictures landed in the text and the rest in the closing list.
- 2026-11-17: pictures a note links from the media store are drawn in the Android reader
  (`/api/suite/media`, `ui/chat/blocks/VaultImage.kt`; chat still blocks them), the `office_file`
  chat tool exists (`confirm` asks the owner, `docs/security/OWNER_APPROVAL.md`), and CI checks
  the size budget on pull requests that touch the suite (`.github/workflows/suite-size.yml`).
  The web note page has the same bar (`components/notes/SuiteBar.tsx`).

- 2026-11-17: M2 started with the Docs render and the regenerate check (`hq-suite-docs`; `hq suite docx <note>` and `hq suite check <file>`). On three real Word files (146, 43 and 194 blocks) the render reads back as the same record. A shrunk record renders back to Word with its pictures. Not yet checked in Word or Google Docs themselves.
- 2026-11-17: M3 started with the owner's default for key recovery (a written-down recovery code). `hq-backup::object` seals one object at a time; `hq-suite-sinks` is a folder sink; `hq suite sink add|list`, `offload`, `undo` (from the sink) and `restore-drill`. The pipeline's `offload` has its own crash matrix.

- 2026-11-17: M4 started: `hq-suite-calc`, a formula engine, and `hq suite calc-check`, which measures it against the answers Excel saved. First run on 57 real workbooks (86,082 formulas): 68.1% give Excel's answer, 0.4% a different one, 31.5% cannot be calculated yet. `docs/suite/SHEETS.md`.
- 2026-11-17: the lost-phone path is built and run end to end on a laptop (`hq suite sink register`, `restore-originals`).

- 2026-11-18: M6 started: `hq-suite-slides` and `hq suite pptx`, a native pptx writer (no template, no tool): title card, cards, nested bullets, numbered lists, tables, a two-column layout. Read back by the suite's own pptx reader and rendered correctly by LibreOffice; not yet opened in PowerPoint. Pictures are placed (as PNG, inside the vault only). `docs/suite/SLIDES.md`.

## Next

1. **More sinks (M3).** The folder sink exists (sealed per original, recovery code, restore drill). Missing: a sink chosen through Android's document picker, a paired-peer sink (built 2026-11-17: `hq suite sink peer-serve`, `add-peer`, `register-peer`; sealed on the phone, the peer holds ciphertext; setup of the forward is by hand), a cloud-folder sink, key shares as an alternative to one code, a lost-phone drill on a wiped device, and a button on the phone.
2. **Web editors** (Docs, Sheets) beyond the bar (milestone M2).
3. **Adoption beyond the neighbour.** A note an import placed under another name, or in
   `Notebooks/Imports/<month>/` for a file outside `Notebooks/`, is not adopted: the lookup is
   the note beside the file. An index lookup by `source_sha256` would close that.
4. PNG sources that are photographs stay lossless and often do not shrink; measure and decide.

Then M2 (Docs, the importer mapping for pptx and the other formats (docx is done), docx and pdf render, the
regenerate check, the editor spike on WebView 111).
