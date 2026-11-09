# Shared links become a collection

A tester keeps links in Google Keep to read or try later and rarely returns to them. They asked
that sharing a link to HQ fetch what it is about, propose a tag and make a note, and that HQ then
use the collection: surface a relevant tip, or draw on it in a new project.

## Phase 1: built

Sharing a link into a note on the phone saves the note at once. The daemon task `link-capture`
then adds a short summary and proposed tags.

| Risk | What stops it |
|------|---------------|
| The page tries to instruct the model | One model call with no tools. The reply must parse as JSON with two fields. |
| The reply chooses where it is written | The server picks the path and fixed frontmatter keys (`source_url`, `proposed_tags`). |
| The summary beacons out when rendered | URLs are removed and Markdown is escaped. Tags must be plain slugs, three at most. |
| A private link reaches a third party | `web_fetch_direct` never uses the render fallback. Credentials in a URL are refused. |
| The fetch reaches the local network | The SSRF checks of `web_fetch` apply, redirects included. |
| The summary is trusted later | Recorded as written by `link-capture`, not the owner. Reading a vault note taints a session. |
| The owner's edit is overwritten | The note's hash is checked after the fetch and again right before the write. |
| Runaway cost | Three per run, fifty tries a day, three attempts per link. Data saver stops the task. |
| Page text goes to the AI provider | Only for a link the owner shared, and "Summarize links you share" turns it off. |

Not built: accepting a proposed tag with one tap, and sharing from the web app. Proposed tags are
shown in the note and held in `proposed_tags`.

## Phase 2: built

Goal: HQ brings up what the owner collected when it is relevant, without being asked. Saved notes
carry the tag `saved-item`.

1. **Import what exists.** Put a Google Takeout `Keep` folder in the vault and run `vault_import`
   on it. Each note keeps its title, text, checklist, link (`source_url`) and labels (as tags).
   Notes in Keep's trash are skipped, and the HTML twin of a JSON note is ignored.
2. **Find it during work.** `vault_search` with `tags: ["saved-item"]` finds things the owner
   saved about a subject. Results taint the session like any vault read.
3. **Surface it, chosen on the device.** The weekly task `collection-tip` matches open task titles
   against saved notes with the keyword index, needs two shared words, never repeats a note, and
   puts at most one item a week in the inbox. No model is involved, the item is device-only, and
   `daemon.collection_tip: false` turns it off.

Not built, on the owner's decision: a profile of what the owner likes. It is sensitive and would
go to a model in every session.
