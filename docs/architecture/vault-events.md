# Vault events, provenance and activity

How HQ notices that a note changed, keeps its derived data in step, remembers who wrote what,
and shows what is going on in a task. Read this before changing the watcher, the search index
writes, the provenance tables or the activity view.

The files in `.vault/` are the source of truth. Everything below is derived from them or sits
beside them, and can be rebuilt. A filesystem event is a hint that a path may have changed,
never a record of what happened.

## The pipeline

```
 editor / git / agent shell
          │  writes a file under Notebooks/
          ▼
  notify watcher (native events, or polling)        crates/hq-daemon/src/vault_watch/watcher.rs
          │  normalize: Created/Changed/Deleted/Renamed/Folder/Overflow
          ▼
  engine: debounce, coalesce, bounded queue         .../queue.rs   (pure state machine, time injected)
          │  "re-read this path" / "re-read this folder" / "scan everything"
          ▼
  pipeline: re-read the file, hash it, upsert or delete     .../pipeline.rs
          │                       │
          │ FTS5 index + sync_state + vault_cache     crates/hq-db/src/search/reconcile.rs
          ▼
  embedding worker: re-embed notes whose hash changed       crates/hq-daemon/src/embedding_worker.rs
```

- **Roots.** Only `Notebooks/` is watched. `_system`, `_threads`, `_data` and other folders HQ
  writes itself never enter the pipeline, so the index writing its own database (under
  `_data/`) cannot feed back into the watcher.
- **What counts as a note.** A regular `.md` file below `Notebooks/` whose path has no hidden
  component and does not pass through `_trash`. Symlinks are not followed, in files or in
  directories, and a note that becomes a symlink loses its index rows.
- **Content hash.** `b3:` followed by the BLAKE3 hash of the raw file. Embeddings carry the
  hash of the note they were made from, so an edit is detected by comparing hashes, not times.
- **Idempotent.** Handling a path re-reads the file as it is now. Duplicate or reordered
  events do the same thing twice and change nothing the second time.
- **Debounce and bounds.** Editors save in bursts and by replacing files. A path waits for
  `debounce_ms` of quiet (and no longer than a maximum wait) before it is read, so fifty
  saves are one index operation. The queue is bounded; when it fills it is dropped and
  replaced by a single scan of everything, so a flood costs work, never correctness.
- **Moves and deletes.** A rename is a delete at the old path and a create at the new one;
  graph links follow the note (`move_graph_links`). A delete clears the note's rows in
  `notes_fts`, `embeddings`, `sync_state`, `vault_cache` and `graph_links`.

### Reconciliation is the floor

Events can be lost: the kernel queue overflows, a watch limit is hit, a network or WSL2 drive
delivers none. The scheduled `embeddings` daemon task (every 30 minutes, first run five
minutes after start) runs the same full scan the watcher can ask for, `sync_index`, and
repairs anything the events missed. With the watcher off it is the only source of freshness.
So search freshness has two sources: events, for seconds-level latency, and the scan, for
correctness.

`hq vault repair` runs the same check on demand and reports orphaned derived rows (index or
embedding rows for notes that no longer exist); `hq vault repair --apply` removes them.

### Configuration

```yaml
vault_watch:
  enabled: true      # on by default; false leaves the scheduled scan as the only source
  mode: auto         # auto | native | poll
  debounce_ms: 300   # quiet time before a changed path is read (clamped to 50..10000)
  poll_secs: 10      # polling interval (never below 2)
```

`auto` uses native file events and falls back to polling when they cannot be used. Changes
to this block need a daemon restart.

**WSL2.** A Windows drive mounted into WSL2 (`/mnt/c/...`) delivers no inotify events. `auto`
detects this and polls instead; the status reports `backend: poll`. Keep the vault on the
Linux filesystem if you can: polling is correct but slower, and costs a directory walk every
`poll_secs`.

### Health

The daemon writes the watcher's and the embedding worker's state to
`_system/.daemon-metrics.json` under the keys `vault_watch` and `embedding_worker`:

| `vault_watch` field | Meaning |
|---|---|
| `state`, `detail`, `backend`, `roots` | running / off / failed, why, `native` or `poll`, what is watched |
| `last_notification_at`, `notifications`, `paths_ignored` | events seen and discarded |
| `events_queued`, `coalesced`, `queue_depth` | work waiting and work merged |
| `queue_overflows`, `watcher_overflows`, `dropped_events`, `watcher_errors` | where events were lost |
| `notes_indexed`, `notes_removed`, `notes_unchanged`, `files_failed` | outcomes of re-reads |
| `folder_syncs`, `full_scans`, `retries`, `abandoned` | wider repairs and failed work given up on |
| `last_scan_started_at`, `last_scan_finished_at` | the latest full scan |

`hq doctor` has a **Search index** section built from these: watcher state and backend, how
many notes are indexed against how many are on disk, how many embeddings are stale or
missing, and the embedding worker's last error. A watcher that is off, degraded to polling or
failing is called out there, with the scheduled scan named as the fallback.

### Embeddings

The worker wakes when the watcher indexes something (or every five minutes), waits a few
seconds for a burst to end, and embeds up to a batch of notes whose stored hash differs from
the file's. Failures back off (30 seconds doubling to 15 minutes) and three failures in a row
stop the cycle. Lite never embeds. Deleted notes lose their embeddings in the same step as
their index rows, and semantic search only returns notes that are still indexed.

Where vectors come from is `daemon.embedding_provider`:

```yaml
daemon:
  embedding_provider: local   # default: remote
```

- `remote` (the default): OpenRouter when `openrouter_api_key` is set, Ollama on this machine
  otherwise. Note text leaves the machine with every embedding.
- `local`: a static Model2Vec model (`potion-base-8M`, about 30 MB on disk and 36 MB in memory)
  runs in the `hq` process. The first use downloads three files from a pinned Hugging Face
  revision into `~/.hq/models/potion-base-8M` and checks each against a pinned SHA-256; a
  file that does not match is never installed. Files already there with the right hashes are
  used without the network, which is also how an offline machine is set up: copy them in.
  Local never falls back to a remote provider. It also embeds `vault_search` queries, so
  `hybrid` and `semantic` searches match notes by meaning; with `remote`, queries are not
  embedded and those modes are keyword searches. On a phone, `local` embeds even with
  `mobile.data_saver` on, since it uses no data after the model download.
- With `local`, the inbox triage (recent untagged notes in `Notebooks/`) asks no model: a note
  takes the tags that at least two of its closest tagged notes share (similarity 0.4 or more,
  three tags at most), or stays untagged. It also runs under the data saver. With `remote` it
  is an LLM classification and needs `openrouter_api_key`.
- The provider is fixed while `hq` runs: a config reload that changes it is refused, and a
  restart applies it.

Each vector is stored with its model's label (`model2vec/potion-base-8M@<revision>` for local),
and search compares a query only with vectors from the same model. Switching providers marks
every note for re-embedding; until that finishes, notes not yet re-embedded are found by
keyword only.

## Provenance: who wrote which version

`crates/hq-db/src/provenance.rs`, tables `note_provenance` and `note_review_events`
(migration 092). One row per version of a note, keyed by `(path, content_hash)`, so
attribution cannot drift onto a later edit.

A row says: the actor, the source kind (`human`, `agent`, `imported`, `generated`, `system`,
`unknown`), the session, the task, when it was recorded, and its review state. **Nothing is
inferred.** A write path that cannot say who it is records `unknown`; a version that no write
path recorded has no row at all, and a file edited in an editor or by a shell stays
unattributed. Attribution may be added later (`recorded_late`), never replaced.

### Where identity comes from

A tool learns who is calling only from reserved `_hq_*` arguments the runtime sets and a
caller cannot:

| Caller | How it is attested |
|---|---|
| claude.ai connector | the MCP gateway sets `_hq_connector` (and `_hq_caller_session` for a proven session) |
| HQ-spawned coding agent | `_hq_caller_session`, set only for a connection that proved its session with its token; the actor is the registered agent, the task its lease or mission |
| native agent session | the session strips every `_hq_*` key the model sent, then sets `_hq_native_session`, `_hq_native_agent` and `_hq_native_leases` (lease ids, never tokens); the task is read from the database and used only when the session's live leases agree on exactly one |
| the `hq` alias tool | passes the markers the runtime put on its own call to the tool it dispatches, and drops any the model wrote in the inner arguments |
| owner via the web app or `hq vault write` | recorded as `human`, actor `owner` |

The MCP gateway removes every `_hq_native_*` key from callers. If a call carries nothing
attested, it is `unknown`. That includes a local MCP client that connects with the API key
alone: the key proves the caller may use HQ, not which session or agent it is, so its writes are
recorded exactly but with no author. `note_provenance` says so in `writer_note`, and it says
something different for a file changed outside the tools.

Recorded at: `vault_write_note`, `vault_append_note`, `vault_patch_section`,
`vault_frontmatter_update`, person facts, the `vault_note` shortcut, `vault_move` (records
follow the note; referrers whose text really changed get a new version for the mover) and
`vault_delete` (a tombstone, purged by the daily cleanup after 90 days). A write inside a leased
task adds one `produced` link from the task to the note, and never changes the task's status.

`redact` wipes identifying fields on a record on request and keeps the row so counts add up.

## Review state

`unreviewed → reviewed / accepted / disputed / superseded`. Review state changes **only** through
`note_review`; writing, reading or searching never changes it, a new version starts
unreviewed, and writing the same text again leaves the verdict alone. Each decision keeps its
reviewer, reason and the full history. `note_provenance` reads the versions of a note with
their review history.

Who may do what:

- an agent in a session may mark a version `reviewed`, `disputed` or `superseded`; it cannot
  `accept` or reopen one, and it cannot review a version it wrote (same name or same session)
- the owner, a call with no attested identity (the key holder) and a claude.ai connector may
  accept; every connector call to `note_review` waits for the owner's approval
- `disputed` and `superseded` need a reason
- a version changed outside the tools has no record, so there is nothing to review until an
  earlier version is named by id
- none of the spark, handoff or tasks keys can call it

## The task activity view

`task_activity` (tool), `GET /api/tasks/{id}/activity` and `GET /api/initiatives/{id}/activity`
(REST), shown in the task drawer and in an initiative's **Briefing**. It is generated on every
request from the task system, the harness session registry and the provenance table; nothing
is stored and there is no hand-edited "latest activity" note.

It returns the task's fields and latest checkpoint, live leases, linked sessions with their
registry status, the notes tied to the task with author, task, session and review state,
decisions and findings split by review state (only notes that declare `type: decision` or
`type: finding`), open questions only where a checkpoint recorded them, a freshness time per
source, and a list of **gaps**: notes with no known author, notes edited after their last
recorded write, linked sessions the registry has never heard of, an `in_progress` task nothing
holds.

It never infers completion or ownership. Status comes from the task. An edit time is shown as
"version recorded", not as progress. Permissions: it is refused to restricted disclosure scopes
(it lists sessions and notes without per-space filtering), is not served to the spark, handoff
or tasks keys, answers 404 under `profile: lite`, and leaves notes under `_system`, `_threads`
and `_data` out. `task:sync` events wake open clients when a note is recorded or reviewed.

## Limits and honest caveats

- Only `Notebooks/` is watched. Notes elsewhere in the vault are indexed by the scheduled scan
  as before.
- Native events are not reliable on network filesystems and on Windows drives under WSL2;
  that is what the polling fallback and the scheduled scan are for.
- Provenance covers writes that go through HQ. A coding agent that writes with its own shell
  leaves an unattributed version, by design.
- The first scan after upgrading re-indexes notes whose stored hash used the old scheme, and
  re-embeds every note once.
