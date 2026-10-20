# Backup and restore

`hq backup` writes one encrypted file holding your vault's notes, a consistent copy of its
database and, if you ask, your config. `hq restore` brings it back into a new folder. It needs no
network and runs no other program, so it is built to behave the same on a server, a laptop and a
phone. The tests run on macOS and, in CI, on Linux; the Android build of this crate has not been
run on a phone yet.

## Why this exists

The vault is your notes and your tasks. If the only copy lives on one machine, losing that machine
loses it. The updater keeps a few database snapshots before each update, on the same machine, and
that is not a backup. This is.

## The short version

```bash
hq backup keygen ~/hq-backup.key          # once; keep a copy somewhere else too
hq backup create --out ~/backups --key-file ~/hq-backup.key
hq backup verify ~/backups/hq-backup-20261020T101137Z.hqbak --key-file ~/hq-backup.key
hq restore ~/backups/hq-backup-20261020T101137Z.hqbak --into ~/restored-vault --key-file ~/hq-backup.key
HQ_VAULT_PATH=~/restored-vault hq status
hq backup status                          # when scheduled backups are on: did the last one work?
```

- `--out` is a file to create (its name ends `.hqbak`), or a folder, which is made if it is not
  there yet, and then the file is named `hq-backup-<UTC time>.hqbak`. An existing backup is never
  overwritten.
- Before a backup gets its final name, `create` reads it back and checks it, and removes it if that
  fails. Skip that with `--no-verify`. It exits with an error, after writing the file and listing
  what is missing, if some files or folders could not be read: that backup is incomplete.
- `restore` goes into a new or empty folder and refuses anything else. Nothing is put in the
  target until the whole backup has been read and checked, so a restore that fails the checks
  leaves the target as it was. (If the path was new and its parent folders did not exist, those
  stay. Moving the checked vault into an existing empty folder is a series of renames; if one
  fails the error says so and the folder is emptied by hand.)

## Keys

A backup is sealed with a key. Two ways to make one:

| | How | Good for |
|---|---|---|
| Key file | `hq backup keygen FILE` writes 64 hex characters, readable only by you | Anything automatic, and the stronger choice: nothing to guess |
| Passphrase | `--passphrase-file FILE` (first line) or `--passphrase-env` (`HQ_BACKUP_PASSPHRASE`) | Something you can remember. At least 10 characters. It never goes on the command line, where other users could see it |

Name one. With none named, `backup.key_file` from the config is used. A key file must be a plain
file (not a link) owned by you, with no group or world access; otherwise it is refused.

**Keep a copy of the key somewhere other than the machine that holds the backups**, such as a
password manager. Without it a backup cannot be opened, and nobody, including the HQ authors, can
recover it. Anyone who has both the key and a backup can read the backup.

## Scheduled backups

Off by default. In `config.yaml`:

```yaml
backup:
  enabled: true
  dir: /path/to/backups        # outside the vault
  key_file: /path/to/hq-backup.key
  every_hours: 24
  keep: 7                      # the newest 7 are kept, older ones are deleted
  include_config: false
```

A scheduled backup uses the key file only, so no secret sits in the config. The daemon checks
hourly and acts only when the newest backup in `dir` is older than `every_hours`. The backup runs
in the background, one at a time, so a big vault never holds up the daemon's other work; how it
went is written to `backup-status.json` in `dir` and shown by `hq backup status` (a failure
before any backup was tried, such as a missing key file or a setting that is not allowed, is
recorded there too once the backup folder exists; a folder that cannot be made is only logged),
and a failure is also logged. A backup that could not read some files or folders (they belong to
another user, say) is marked INCOMPLETE and fails the status, and then no complete backup is
deleted: a permissions problem must not wear away the good copies. The next backup replaces an
incomplete one, so they do not pile up. After each new backup it deletes all but the newest `keep`. It only ever deletes
files named exactly like backups, never the one it just wrote, and never lists a name dated more
than a day ahead (a clock that was wrong, or a file put there by someone else). An unfinished
`.partial` file left by a run that died is removed after a day.

A backup that stays on the machine it protects is not much of a backup. Copy `dir` somewhere else:
another computer, a cloud drive, a USB stick. The files are encrypted, so where they go is your
choice. HQ does not upload them.

## What is in a backup

- The vault's files, except logs, HQ's own lock files (`.daemon-lock`, `bot.lock` and the like,
  outside `Notebooks/`), other backups and the database files themselves. Anything under
  `Notebooks/` is yours and is kept, a `Cargo.lock` included.
- A consistent snapshot of the database, taken with SQLite's own `VACUUM INTO` so that writes
  happening at that moment cannot tear it.
- The vault's folders, empty ones included, because HQ expects some of them to exist.
- `config.yaml`, only with `--include-config` (or `backup.include_config`). It holds API keys and
  tokens, so it is off by default and is restored only to a path you name with `--config-out`.
- A manifest listing every file with its size and SHA-256.

Left out, and listed in the report and the manifest: symbolic links, files that are not plain
files, files or folders this user cannot read, files that are removed or replaced while the
backup runs, and any path a restore would refuse (more than 63 folders deep, or over 4096 bytes).
Never backed up, wherever they are: files ending `.hqbak` or `.hqbak.partial`, and folders named
`.hq-restore-...`. File modes are not kept: restored files are readable by you only, and an
executable bit is lost. A note being rewritten while the backup runs is taken only once it has been quiet for
a second; one that never settles is left out and listed rather than caught half written.

While it runs, a backup keeps plaintext (the database snapshot, and files over 8 MB held for the
quiet check) in a private folder with a random name in the system's temporary folder (`TMPDIR`
moves it), or beside the backup if that folder cannot be used or is too small for what this
vault needs. It is removed when the backup ends. One left by a run that was killed is removed by
the next backup or verify once it is a day old; until then it holds the database in the clear
(readable only by you). Nothing unencrypted is written to the backup folder in the normal case.

## Restoring

```bash
hq restore FILE --into DIR [--key-file F | --passphrase-file F | --passphrase-env] [--config-out PATH] [--no-reindex] [--max-size-gib N]
```

The restore is checked before anything is put in place:

1. Every part of the file is authenticated, and the whole stream is read to its end.
2. Every entry name is checked (no `..`, no absolute paths, no links), with limits on the number
   of entries and the total size, then each file's hash is compared with the manifest, and the
   manifest with the files actually present.
3. The stream holds nothing HQ never writes (no PAX or sparse records, no name longer than a
   path), so a hostile file cannot make the reader buffer gigabytes.
4. The database passes SQLite's integrity check, and is refused if it comes from a newer HQ than
   this one (a migration number above any this build has). Update HQ and try again.
5. Only then does the vault appear: in one rename when the target is a new folder, or moved in
   (the checked copy, item by item) when it is an existing empty one, such as a mount point or the
   current folder. It is readable only by you. A config file, if you named a place for it, is
   written last and never replaces a file; if that fails you get a warning, not an undone
   restore. The search index is then rebuilt.

A wrong key, a changed byte, a file cut short at any point, or a hostile file are all refused. A
wrong key is told apart from damage to the rest of the file, but not from damage to its first
64 KiB: both fail the first chunk, which is what a wrong key looks like.

A restore that is killed leaves a hidden `.hq-restore-...` folder next to the target (inside it,
for an existing empty folder), holding what it had unpacked, config included. A later restore
into the same target, or beside it, removes one a day old; into an existing folder it also says
when a younger one is why the folder is not empty. Delete a younger one by hand.

`hq backup verify FILE` does the same checks, the database check included, without restoring
anything (it uses a private work folder for the database alone, and removes it). Run it now and
then on a backup you have not opened: a backup nobody has opened is a hope, not a backup. A
restore or a verify refuses a backup that unpacks to more than 64 GiB unless you pass
`--max-size-gib`; the read-back that `create` does is sized from the backup it just wrote.

## The format, for anyone who needs to read it

Version 1. A 54-byte header in the clear (magic `HQBACKUP`, format, how the key was made, the
Argon2id cost and salt, a nonce base), then a gzip-compressed tar stream sealed with
ChaCha20-Poly1305 in 64 KiB chunks. Each chunk's nonce is the base with the chunk number mixed
in; its associated data is the whole header and a flag marking the last chunk. So a changed
header, a reordered, repeated or dropped chunk, and a file cut short at a chunk boundary all fail.
Passphrases go through Argon2id (64 MiB, 3 passes by default; a file may ask for at most 256 MiB,
10 passes and 4 lanes, so a hostile header cannot demand more of the machine). The code is in
`crates/hq-backup`.

## Limits

- It does not make a backup of a running machine's other state: the config unless asked, the
  environment, installed tools.
- A backup is not one instant. The database is snapshotted first and the notes are read over the
  seconds that follow, so a note saved during the backup can be newer than the database state.
  Notes changed afterwards are in the next one.
- A vault with over 5 million files and folders, or so many files that the list of them passes
  256 MB, cannot be backed up as one file; `create` says so instead of making a backup that a
  restore would refuse.
- The encryption library keeps its key in memory without wiping it when a backup ends. The key
  file and passphrase buffers this crate holds are wiped.
- Backups are not incremental: each is a whole copy, compressed. How much smaller it is depends
  on what the vault holds; text shrinks a lot, photos and PDFs hardly at all.
- A restore unpacks the whole vault into a work folder on the target's file system before it is
  put in place, so that file system needs about the vault's size free, and the backup file is
  read from wherever it is.
