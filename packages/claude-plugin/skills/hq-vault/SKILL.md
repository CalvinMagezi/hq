---
name: hq-vault
description: Read and write HQ's shared markdown vault. Use when you learn something other sessions or agents will need, such as a decision, a how-to, a finding or a project fact, or when you need background before starting work. Triggers on notes, remember this, document this, vault, knowledge base, or when the tools vault_search, vault_read, vault_write_note, vault_patch_section or vault_frontmatter_update are available.
---

# Using the vault

The vault is HQ's shared memory: markdown notes with YAML frontmatter that every session and agent can read. Tools are reached with `hq_call(tool, args)`.

## Where things go

Tasks track work. Notes hold knowledge. If another session or agent will need a fact next week, it belongs in a note, not in your local memory and not buried in a task comment.

## Reading

- `vault_search` finds notes by keyword, meaning, or recency. Run it before you write anything, and before you start work in an area you have not seen.
- `vault_read` reads one note by path. Use `start_line` and `end_line` for a slice of a long one.
- `vault_read_section` reads only the part under one heading.
- `vault_batch_read` reads up to 20 notes in one call.

## Writing

1. Search first. If a note on the topic exists, update it.
2. `vault_patch_section` replaces one section of an existing note. `vault_append_note` adds to the end. Prefer these over rewriting the whole note.
3. `vault_write_note` creates a note or overwrites one. Use it for a new note, not to edit an old one.
4. `vault_frontmatter_update` changes tags, status or other frontmatter without touching the body.

Keep to one topic per note, with a title someone would search for. Link related notes with `[[note name]]`. State facts plainly and say where they came from.

## Do not store

Secrets, tokens, passwords, and anything the user asked you to keep private.
