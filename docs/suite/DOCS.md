# Docs: editing a note one block at a time

A doc is an ordinary note: headings, paragraphs, lists, tables, code and images (`docs/suite/HQMD.md`). On the phone, **Edit by block** (under the note's top bar) lists the note's blocks, and a tap opens one as markdown. The whole-note editor (the green **Edit** button) is still there; this is the structured way in, for a long note or a small change.

- **Change a block**: tap it, edit the markdown, Save. The new text must be one block (a paragraph, a list, a heading); two blocks in place of one are refused.
- **Formatting buttons**: below the text in the edit and add dialogs (below, because the selection's Cut and Copy bar floats above the text), a row of Bold, Italic, Code, Link, Heading, Bullets, Numbers and Quote. They write markdown into the field (`**bold**`, `*italic*`, `` `code` ``, `[text](https://)` with the address selected for typing, `## `, `- `, `1. `, `> `) around the selection or on the lines it touches; a second tap takes it off, and a line with another style swaps to the new one. The block is still saved as the markdown you see, so the formatting is exactly what the web and chat write.
- **Add a block**: **Add below** under any block, or **Add a block at the end**.
- **Delete a block**: inside a block's edit dialog.
- **Stale edits are refused**: the phone quotes the hash of the block it saw. If the block changed since (on another device, or by HQ), the edit is refused and nothing is written. The result must also read back as the blocks the edit produced; an edit that would merge with its neighbours when the note is read again is refused.
- **Nothing else moves**: the frontmatter and every other block are written back byte for byte. The note's own history is the undo.

`GET /api/suite/blocks?path=`, `POST /api/suite/blocks/edit`, `/insert` and `/delete` are the routes; they use the hqmd patch (`Op::Replace`, `InsertAfter`, `Delete`) that the format's guarantees are written for.

## Comments from a Word file

When a Word file is shrunk (`hq-suite-shrink`, which sets `ConvertOptions::word_comments` to the codec from `hq_suite_ingest::word_comment_codec()`; the importer itself does not know hqmd), the
comments in `word/comments.xml` come across as `comment` blocks (`HQMD.md`), each placed after the
block it was on:

- `on` is the hash of that block as written in the note, so the comment is not orphaned.
- `by` is the Word author, cleaned as the comment tools clean it (no control characters, quotes or
  backticks, 60 characters). `at` is the comment's day (`YYYY-MM-DD`), left out when Word has none.
- `state` is `resolved` when `commentsExtended.xml` marks it done, else `open`. A reply is its own
  comment, on the block of the comment it answers.
- A comment in a table cell sits on the table; one on a list item sits after the list.
- The report counts how many were carried. A comment with no body block to sit on (header, footer,
  note, never referenced) is **not carried** and is counted as a loss, so the fidelity tier is
  lossy. Nothing is anchored to the title as a stand-in.

A plain import (`hq vault import`, `convert_to_markdown`) leaves the option off and its output is
unchanged. The report now also counts tracked changes: inserted text is kept as plain text, deleted
text is dropped.

## Not done

Showing the formatting live as you type (the field is markdown, with buttons), moving a block, comments, and fill-in fields (`fields` is still reserved). A block edit is markdown, as the web and chat write it.
