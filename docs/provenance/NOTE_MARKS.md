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

`GET /api/search` also takes `written_by=hq` (400 for any other value), `tag=<name>` (a whole tag,
any case) and `folder=<path>` (400 for `..`, a leading `/` or a backslash). The index narrows by
folder before the limit; a tag or `written_by` filter reads up to 500 index rows and checks each
note itself, and the file walk fallback checks each file before searching its text, so `limit`
counts only matches.
