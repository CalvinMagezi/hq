# SQL migrations

A migration is **one new file here**: `NNN_what_it_does.sql`, where `NNN` is the next free number
(three digits) and the rest is lowercase words and underscores. Nothing else needs editing:
`crates/hq-db/build.rs` lists this folder in file name order and `src/migrations.rs` runs the files
in that order, once each, recording the file name (without `.sql`) in `schema_version`.

- **Never edit a migration that has shipped.** Databases that ran it will not run it again. Add a
  new one that changes the schema further.
- **Two branches can both pick the next number.** Git merges that cleanly, because they are
  different files. The test `migration_numbers_are_unique_and_in_order` fails on the second
  pull request: rename the later file to the next free number. If a build that people run already
  used the old name, also add it to `RENAMED` in `src/migrations.rs` so databases that recorded the
  old name do not run it twice.
- **Numbers are consecutive.** `012` is shared by two old files and `006` was never used; those are
  the only exceptions (`KNOWN_SHARED_NUMBERS` and `KNOWN_MISSING_NUMBERS` in the same test), and
  nothing new belongs in either list.
- **Finding the next number.** Run `scripts/next-migration-number.sh` for the next free `NNN`. Run
  it again after rebasing onto `main`, since another branch may have taken the number meanwhile.
