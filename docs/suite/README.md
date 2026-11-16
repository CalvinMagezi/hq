# HQ Suite

A small office suite inside HQ for people whose phone is their only computer. It keeps their
documents in the smallest form that is still faithful, helps them put the originals somewhere
they own, and shares work back out as Word, Excel, PDF and PowerPoint files.

Status: milestone M0 (spec, corpus, measurement) is built apart from the numbers from real files and
the reference phone; milestone M1 has its storage layer and the shrink pipeline, and is missing the
storage governor. Everything is on the command line; nothing is in the web app yet.

## The problem

Office files arrive over chat apps and downloads. On a budget phone (the planning target is 32 GB
with about 5 GB free), a hundred or two of them and their duplicates fill the space the phone
needs to run. They are also opaque to HQ until converted, and many of these users have no
computer, no paid cloud storage and expensive mobile data. Losing the phone must not mean losing
the files.

## What it does

- **Shrink.** Turn an imported file into its record: plain markdown plus media stored once and
  recompressed. Show a preview first. The original leaves the phone only with consent, to a
  place the user owns or to a 30-day trash. Nothing is lost silently.
- **Govern storage.** A budget the user sets, a plain report, and an order in which disposable
  data goes first. The record is never touched.
- **Share.** Render a note as docx, xlsx, pdf or pptx.
- **Edit small things well.** Docs, Sheets, Slides and a Dashboard as small editors over markdown.

Out of scope for v1: live co-editing, teams, any server or object store, macros, pivots, array
formulas, animation, and a desktop or native UI.

## Where things are

| What | Where |
|------|-------|
| The note format | [HQMD.md](HQMD.md) |
| The plan and milestones | [../plans/hq-suite.md](../plans/hq-suite.md) |
| Rules for the code | `crates/suite/CLAUDE.md` |
| Measuring real files | [MEASURE.md](MEASURE.md) |
| Making a document smaller | [SHRINK.md](SHRINK.md) |

## Why inside this repository

The suite stands on crates that change often and pin exact versions (the importer, the
exporter with its Typst pins, backup, pairing). A second repository would mean a cross-repo
change for nearly every new format, a second release train, and a second CI. HQ's own newer
parts (`hq-net`, `hq-edge`, `hq-bridge`, `hq-backup`) all landed as crates behind features. The
suite follows that, in its own folder, with a test that keeps it extractable. Revisit at the end
of M2. Extraction triggers (any two): a consumer outside HQ needs the engine without the hub;
suite release cadence or CI cost clearly hurts HQ; the binary budget cannot be met with feature
gating; the suite is to be licensed separately.
