# Note marks in the notes API

Each note in `GET /api/note` (a note and a folder's `entries`), `/api/vault-signals` and
`/api/pinned` carries, read from `note_provenance` (`hq_db::note_marks`):

- `written_by`: `you`, `hq` or `import`. It is `hq` or `import` only when a record exists for the
  exact version on disk (matched by hash). A note nobody recorded, or one edited outside HQ's tools
  since its record, is `you`, in lists, search and the dot alike.
- `review`: `new`, `changed` or `null`.
  - `changed`: HQ wrote the version and an earlier version was the owner's (typed, unknown or
    imported), skipping HQ's own versions between. It survives opening. It clears only on Keep
    (`POST /api/note/review` with `accepted`) or when the owner writes the note again (Undo).
  - `new`: HQ wrote it and no earlier owner version is recorded. Clears on `POST /api/note/seen`
    for that version; a later HQ write is new again. A first record cannot tell a new note from
    HQ's first edit of a note that predates provenance, so the latter reads as `new`.
  - Only the owner's `accepted` settles a mark. `reviewed`, `disputed` and `superseded` can be set
    by agents and change nothing here.
- `why`: "Saved from your chat, 10 Nov", "Saved for a task, ...", "Imported, ...".
- `tags`: the front matter `tags` list, read only. One parser serves every route: it reads up to
  64 KB, refuses front matter with YAML anchors or aliases, nesting deeper than 8, or more than
  64 KB (no tags), and compares tags whole and case-folded.

`POST /api/note/seen {path}` reads the path as `GET /api/note` does, needs a `.md` note (404
otherwise), refuses a note over 4 MB (413) and keeps one row per note; the row follows the note
through a link copy and a rename. `GET /api/vault-signals` also returns `has_new` (bool): whether
any note is `new` or `changed`, for the quiet dot on Vault.

`POST /api/note/trash/restore` answers `restored_to`: `original` (its own folder), `folder` (its
folder was removed; it went to the nearest folder that still exists under `Notebooks/`) or `vault`
(none survives; it went to the top of the notes, `Notebooks/`). A name taken by another note still
gets the pre-existing ` (restored)` suffix, and the note's provenance follows it.

`GET /api/note/trash` also gives each item `deleted_by`: `hq` when the deleting call came from HQ
(set by the runtime from who made the call, never from a note or an argument), otherwise `you`; an
item nobody recorded reads as `you`.

## The owner's Trash controls and earlier text

These routes are the owner's. No tool, helper or MCP call reaches them (a test scans
`hq-tools` and `hq-mcp` for their paths).

- `POST /api/note/trash/purge {path}` deletes one Trash item for good and answers `purged: 1`.
  The path must be a note under `_trash/<day>/Notebooks/` (400 for `..`, a leading `/` or a
  backslash; 404 when it is already gone, so a purged item cannot be restored). Its provenance,
  opened-mark and earlier-text rows go too, unless a live note has taken the name or another
  Trash copy of it remains. `note_purge_log` keeps one line, the path and the time, never any text.
- `POST /api/note/trash/empty` does the same for everything the Trash lists and answers
  `purged: N`.
- Provenance stores only the hash of a version, not its text, so HQ keeps the text it replaces:
  before an HQ tool changes a note, `note_prev` (`hq_db::note_keep`) keeps the previous text, one
  row per note, up to 64 KB (a larger note keeps none). When that text is HQ's own and not yet
  kept by the owner, the row is left alone, so it stays the owner's version from before HQ's first
  change. It is dropped when the owner keeps the change, when the note is deleted, and with the
  provenance records at purge. Any write by someone other than HQ (the owner's save, a restore) drops it,
  and so does an HQ write that did not go through a keeping tool, so a text older than the owner's
  current version is never served. It also expires after 30 days, and the table is held to 5 MB
  (oldest first) by the daily clean-up. The path HQ is about to write comes from the model, so
  `Recorder::before_write` itself requires a `.md` note the note tools may write, reached through no
  symlink, a regular file, and reads at most 64 KB + 1 byte. Only the latest HQ change can be undone. Covered writers: the
  write, append, patch and frontmatter tools and the shortcut note creator; the person facts
  tool, imports and link capture do not keep earlier text.
- `GET /api/note/version?path=&id=` returns `content` (the body), `hash` and `saved_at` of the
  text before HQ's version `id` (ids come from `GET /api/note/versions`). 404 for any other id,
  or when nothing is kept.
- `POST /api/note/restore-version {path, id}` writes that text back as the owner, so the
  `changed` mark clears and the note reads as `you`, and answers the new `hash`. It restores only
  while the note on disk is still exactly HQ's change `id`; otherwise it answers 409 ("The note
  changed since HQ's edit, so it was not rolled back.") and writes nothing. Reading, comparing and
  writing happen under one per-note lock, which the owner's own save takes too. Once restored the
  kept text is gone, so a second call is 404.
- `POST /api/note/trash/empty` also answers `failed` (items it could not remove) and `success` is
  false when any failed. A purge re-checks the final file with `lstat` right before removing it
  and refuses a symlink; a window of a few instructions remains between that check and the
  removal.
- `deleted_by` is `you` only when the owner's own web handler deleted. A delete through any tool or
  MCP call, with or without an identity, reads `hq`.
- Reachability: tests send each of the four routes with no credential, a connector grant, and a
  Spark key (as bearer and `x-api-key`), on the main router and behind the paired-devices door, and
  expect 401, 403 or 404; the connector door's path test also covers them. When web sign-in is
  disabled (no web token) any local process can reach `/api`, as `DELETE /api/note` already could.

`GET /api/search` also takes `written_by=hq` (400 for any other value), `tag=<name>` (a whole tag,
any case) and `folder=<path>` (400 for `..`, a leading `/` or a backslash). The index narrows by
folder before the limit; a tag or `written_by` filter reads up to 500 index rows and checks each
note itself, and the file walk fallback checks each file before searching its text, so `limit`
counts only matches.
