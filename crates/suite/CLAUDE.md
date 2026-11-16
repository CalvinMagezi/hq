# CLAUDE.md for `crates/suite/`

The HQ Suite (Drive, Docs, Sheets, Slides, Dashboard) lives here. Read the repo's root
`CLAUDE.md` first; this file adds what is special about this folder. The plan is
`docs/plans/hq-suite.md`, the note format is `docs/suite/HQMD.md`, the overview is
`docs/suite/README.md`.

## Why a separate folder

The suite is built inside HQ so it shares one release train, one CI and the crates it stands on
(`hq-convert`, `hq-export`, `hq-backup`, `hq-bridge`, `hq-db`, `hq-vault`). It is built so it
can be lifted out later. The rules below are what keep that possible, and a test enforces them.

## Extraction rules

1. No suite crate except `hq-suite-adapter` depends on `hq-agent`, `hq-relay`, `hq-llm`,
   `hq-web`, `hq-mcp`, `hq-tools`, `hq-cli`, `hq-daemon` or `hq-memory`.
2. Only `hq-suite-adapter` registers tools, reads `suite:` config, schedules daemon tasks,
   speaks device actions or adds REST routes.
3. No suite crate reads `~/.hq`, an environment variable, the clock for a decision, or a vault
   path of its own. Paths come in through a small `SuitePaths` value (in `hq-suite-store`).
4. No suite crate opens a network connection or a listening port. The only networking is HQ's
   existing pairing, reached through the adapter by the peer sink.
5. Frontmatter keys the suite adds are flat and start with `suite_` (see `HQMD.md`).

`crates/suite/hq-suite-adapter/tests/boundary.rs` reads every suite manifest and the shipped
source (`src/`) of every crate but the adapter, and fails on a breach of rules 1, 2 (starting
programs), 3 and 4. It also checks that every suite crate is listed below and is a workspace
member, and that its own scanner still sees planted breaches. A later change that needs to cross a line has
to delete the guard first and say why in review.

## Crates

| Crate | Job | May depend on |
|-------|-----|---------------|
| `hq-suite-md` | hqmd: parse, print, validate, patch. Pure, no I/O. | `pulldown-cmark`, `serde*`, `sha2`, `hex`, `chrono` |
| `hq-suite-ingest` | Reads office files for the suite: pictures inside packages, fidelity tier projection from the importer's report | `hq-convert`, `hq-suite-md` |
| `hq-suite-store` | Durable files: atomic writes, media stored by hash, trash in the vault's layout, the shrink journal | `hq-suite-md`, `sha2`, `serde*` |
| `hq-suite-shrink` | Picture decoding with caps, the encoders (lossy WebP through libwebp for photographs), the measurement report, the shrink pipeline (prepare, confirm, undo, recover) | `hq-convert`, `hq-suite-ingest`, `hq-suite-md`, `hq-suite-store`, `image`, `webp` |
| `hq-suite-adapter` | The only crate that knows HQ: config, tools, CLI glue, daemon task, device actions, REST | everything |

The plan names more crates (governor, sinks, render, calc, one per tool).
They are added in the milestone that needs them, not before: an empty crate is debt.
A crate that is added gets a row here and an Entry Points row in the root `CLAUDE.md` if it
has an entry point.

## Working here

- `cargo test -p hq-suite-md` and friends, not the whole workspace.
- Tests use synthetic content only. Real files never enter the repo. The golden corpus under
  `hq-suite-md/tests/corpus/` is invented; regenerate outlines with
  `UPDATE_GOLDEN=1 cargo test -p hq-suite-md --test corpus` and read the diff.
- Anything a person did not ask to change is returned byte for byte. If a change to the parser
  or printer breaks that, the change is wrong.
- User-facing strings go through a string table from the first screen so more languages can be
  added. English first.
- The measurement tool prints numbers and format classes only: no file names, no paths, no
  content. Keep it that way.
- Adoption data about the suite stays on the device (`docs/security/ADOPTION_DATA.md`).
