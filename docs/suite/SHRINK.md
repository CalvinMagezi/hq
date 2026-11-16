# Making a document smaller

`hq suite shrink` turns a document in the vault into its record: a note holding the text and
tables, with the pictures stored once and recompressed. The original is set aside in the vault's
trash, only after the record is written and checked and only when you agree.

```bash
hq suite shrink Notebooks/Inbox/Imports/report.docx
```

It shows what it would do and moves nothing:

```text
Notebooks/Inbox/Imports/report.docx is ready to be made smaller.
  Original 581 KB, record 365 KB: 216 KB saved.
  The record is the note Notebooks/Inbox/Imports/report.md, with 1 picture made smaller and 1 kept as they were.
  The note does not keep page layout (margins, fonts, columns).
  Nothing else was left out.
  Your original has not been moved.
```

Say yes at the prompt (or pass `--yes`) and the original goes to `_trash/<day>/` where it stays for
30 days. Say no and nothing changes, except that a note was written; `hq suite abandon <job>`
removes it.

| Command | What it does |
|---------|--------------|
| `hq suite shrink <file> [--yes]` | Prepare, show the preview, move the original on a yes |
| `hq suite confirm <job>` | Agree to a prepared job |
| `hq suite abandon <job>` | Walk away from a prepared job: removes the new note |
| `hq suite undo <job>` | Bring the original back (within the trash's 30 days) |
| `hq suite jobs` | Where every job stands |
| `hq suite recover` | Finish or resume jobs a crash or restart interrupted |

## What is kept, what is not

- Kept: text, headings, lists, tables, links, and every picture (once, by content, shared with
  every other note that uses the same picture).
- Not kept: page layout (margins, fonts, columns, headers), and whatever the importer reports it
  cannot carry (charts, embedded objects, text boxes become inline text). The preview lists them.
- Pictures are appended in a "Pictures" section at the end of the note for now. Placing each one
  where it sat in the document arrives with the importer mapping in milestone M2.
- JPEG photographs are scaled to at most 1600 pixels on the long side and recompressed. PNG and
  WebP graphics are recompressed losslessly. A picture is replaced only if the new file is smaller;
  otherwise the original bytes are kept. These settings are provisional until the measurement
  (`hq suite measure`) decides the encoder.
- Not shrunk: password protected files, files HQ cannot read, scans and photographs of documents
  (they keep their page images; shrinking them comes later), anything under a folder HQ keeps for
  itself (`_media`, `_trash`, `_data`, `_system`), and files whose record would not be smaller.

## The safety model

Shrinking is several steps. Each can be repeated safely, and a job file in
`_data/suite/journal/` records the last one completed:

1. **Prepare.** Read the file, store its pictures in `_media/cas/`, draft the note (the draft
   text is kept beside the job file), write the note, read it back and check it: the text is the
   draft, it is valid hqmd, every picture it links to exists and its content matches its hash, and
   the original is still the file that was read. Nothing but unreferenced pictures and the new
   note have changed.
2. **Confirm.** Check again, because time has passed. Record the agreement and the trash path.
   Move the original (a rename, so it is atomic). Mark the note (`suite_original_state: trash`).
3. **Undo.** Move the original back, if the old place is free, and mark the note `kept`.

The rule the steps protect: the original is at its place, or in the trash path the job file
names, and whenever it is not at its place the record is complete. A crash anywhere leaves a state
that `hq suite recover` (and the next `prepare` of the same file) finishes or leaves for you to
decide: a job that has your agreement is carried through; one that does not waits at the preview.

Things that stop a move: the original changed since it was read, the note was edited since the
preview, a picture is missing or damaged, the old place is taken (undo). Each says so and moves
nothing.

## How it is tested

`crates/suite/hq-suite-shrink/tests/shrink_pipeline.rs` injects a simulated crash at each of the
recorded points of prepare, confirm and undo, then checks, before any recovery, that the original
is at its place or in the trash with a complete record, and after recovery that the job reaches its
end state. What this does not cover: a power cut inside a single file operation. For that the
suite relies on rename being atomic and on flushing files and folders before and after it
(`hq-suite-store::fsx`); the unfinished `.partial-` files such a cut can leave are swept at the
next run.

## Known limits (M1)

- Shrinking a file again after an undo writes a second note next to the first.
- A note that already exists for the file (from `hq vault import`) is not adopted; the shrink
  writes its own. Adopting it is next.
- Moving the original to a cold place you own (another device, an SD card) is milestone M3; today
  the only destination is the vault's trash.
- The trash's own 30-day clean-up deletes the original. The note then still says
  `suite_original_state: trash`; reconciling it to `gone` belongs to the storage governor.
