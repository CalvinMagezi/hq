# hqmd: markdown with typed blocks

hqmd is the format of every note the HQ Suite writes. It is ordinary CommonMark with
GitHub-style tables, plus three small additions that live in places a plain markdown
viewer already knows how to show: typed fenced blocks, a few flat frontmatter keys, and
relative links to media. A person with a text editor, a file manager and no HQ can open,
read and rescue every note.

The reader and writer are the `hq-suite-md` crate (`crates/suite/hq-suite-md`). This page
is the language. The golden corpus in `crates/suite/hq-suite-md/tests/corpus/` is the
executable form of it.

## Principles

- **Compatible.** Remove every addition and what is left is a readable note. A viewer that
  has never heard of hqmd shows a typed block as a code block, which is a fair picture of it.
- **Nothing is dropped.** A block or a key this version does not know is kept byte for byte.
  Opening and saving a note changes no byte the user did not change.
- **Edits are small.** An edit names a block and replaces that block. The text around it is
  returned as it was.
- **Plain words in plain text.** Money keeps its meaning through a column type, never as a
  bare float. Dates are ISO 8601.

## Files

- UTF-8 text. `\n` or `\r\n` line endings; a note keeps whichever it has, and an edit uses
  the same one. A note with mixed endings is read and kept as is.
- A note may start with a frontmatter block between two `---` lines. If the first line is
  `---` and no closing line follows, the note has no frontmatter.
- A note is at most 16 MiB. Frontmatter is at most 64 KiB. Past those the checker reports an
  error and the editing API refuses to grow the note.

## Blocks

The body is read as top-level CommonMark blocks: headings, paragraphs, lists, quotes, code,
tables, HTML, rules, footnote definitions. The reader keeps each block's exact text and the
blank space before it. Text CommonMark does not report as a block, such as a link reference
definition, is kept as its own block, so it cannot be lost.

### Typed blocks

A typed block is a fenced code block whose header starts with one of the type names below.

````text
```sheet name=Stock types="text,int,ugx,ugx" id=stock
| item  | qty | price | total  |
|-------|-----|-------|--------|
| Sugar | 40  | 4200  | =B2*C2 |
```
````

Rules:

1. The opening fence starts in the first column and the block sits at the top level of the
   note. Inside a list or a quote, or after an indent, it is ordinary code. (The checker
   says so, at info level, so a person is not left wondering.)
2. The fence is three or more backticks or tildes, as in CommonMark. The writer picks a fence
   one longer than any run of the same character in the body, and uses tildes if a header
   value holds a backtick.
3. A fenced block whose header starts with any other word is code. This is how a block type
   from a newer version, or one not yet defined, survives an older reader: it is shown as
   code and saved as written. `fields` (Docs fill-in fields) and slide layout comments are
   reserved names. They are not part of the language yet, so they are kept as code or HTML
   and are not checked.

### The header

```text
header = type *( SP attr )
type   = lower *( lower / digit / "-" )
attr   = key "=" value
key    = lower *( lower / digit / "_" / "-" )
value  = bare / quoted
bare   = 1*( any character except space, tab or double quote )
quoted = DQUOTE *( any character except DQUOTE and "\" / "\\" / "\"" ) DQUOTE
```

The header is at most 1024 bytes with at most 24 attributes. A word with no `=`, a repeated
key, an unclosed quote or text after a closing quote is an error: the block is reported and
left alone, never repaired by guessing.

Every block may carry `id=`: lowercase letters, digits, `-` and `_`, up to 64 characters,
unique in the note. Edits do not need it (see "Editing"); links between blocks can use it.

When a header is written back, each attribute keeps the spelling it had (quoted or bare).
Attributes added by the suite are quoted unless they are letters, digits, `_`, `.` and `-`.

### Block types

#### `sheet`

A typed grid. The body is a GitHub table: a header row, a delimiter row, then data rows. Every
row has as many cells as the header. A cell starting with `=` is a formula. Write a pipe
inside a cell as `\|` and a backslash as `\\`. No blank lines inside the table.

| Attribute | Meaning |
|-----------|---------|
| `name` | Name used by references such as `[[Note#Stock]]!A1:B2`. Not empty. |
| `types` | One column type per column, comma separated. |
| `data` | Relative path of a compressed sidecar holding the rows. See "Large sheets". |
| `rows` | Total row count when the rows live in the sidecar. |
| `id` | Block id. |

Column types: `text`, `int`, `num`, `bool`, `date` (ISO 8601), `pct`, or a three-letter
lowercase currency code (`ugx`, `kes`, `usd`) for an amount of money. A cell holds the plain
number (`2700000`); the column type is what displays it as `UGX 2,700,000`. A currency code
the display code does not know is shown as the code itself. The words `text`, `int`, `num`,
`bool`, `date` and `pct` are never currency codes.

Limits: 100,000 cells inline and 16,384 columns. Larger sheets use a sidecar.

#### Large sheets

A sheet too big to keep in the note holds its header and a preview here, names the rest with
`data="_data/suite/sheets/<name>.csv.zst"` and states the full size with `rows=`. The sidecar
is a plain compressed file the note points at, never the only copy of anything else. The path
must be relative to the vault: forward slashes, no `..`, no empty or `.` parts, no drive
letter, no scheme. The sidecar's layout is fixed in milestone M4 from measurements on the
reference phone.

#### `tile`

One number computed from a range. The body is empty.

| Attribute | Meaning |
|-----------|---------|
| `from` | Required. A range, see "References". |
| `fn` | Required: `sum`, `count`, `avg`, `min`, `max` or `median`. |
| `label` | Text shown with the number. |
| `format` | A column type used to display the result. |
| `where` | Optional. Row filter such as `qty>30 and item=Sugar`; with it the range's first row names the columns and the number comes from the last column. |
| `group` | Not allowed on a tile. |

#### `plot`

A chart drawn from a range. The body is empty.

| Attribute | Meaning |
|-----------|---------|
| `type` | Required: `bar`, `line`, `pie` or `donut`. |
| `from` | Required. A range, header row first. |
| `x`, `y` | Required. Column names from the header row (the label and the value). |
| `title` | Optional. |
| `where` | Optional. Row filter, as on a tile. |
| `group` | Optional. Column to split into one series per value (bar and line only, up to six). |

#### `chart`

A chart with its data inline, as HQ chat already draws it (`docs/CHART_BLOCKS.md`). The body
is one JSON object. The only attribute is `id`. This block exists so that a note that holds a
chart from a chat answer is valid hqmd as it stands.

The dashboard's chart over a sheet is `plot`, not `chart`, because `chart` already means "JSON
body" in the chat renderer and the two must not be told apart by guessing.

## References

A range names cells, optionally in another note: `[[Stock]]!D2:D40`, `[[Notebooks/Shop/Stock#q3]]!B2`,
or just `B2:C9` for a sheet in the same note.

```text
range  = [ target "!" ] cell [ ":" cell ]
target = "[[" note [ "#" block ] "]]"
cell   = 1*3 UPPER 1*7 DIGIT          ; A1 .. XFD1048576, row never starting with 0
```

`block` picks a sheet in the note by its `id` or `name`; without it, the first sheet. A range
is stored with its top left first. The checker does not open other notes: it checks shape, and
it warns when a reference with no target points at a note that has no sheet.

## Frontmatter

Keys the suite adds are flat (no nesting), because the vault's own frontmatter tools take
only strings, numbers, booleans and lists. Keys HQ's importer already writes (`source`,
`source_sha256`, `source_format`, `converter`, `noteType`, `conversion`) are not repeated.

| Key | Values | Meaning |
|-----|--------|---------|
| `suite_kind` | `doc`, `sheet`, `slides`, `dashboard`, `drive-item` | Which tool owns the note |
| `suite_fidelity` | `native`, `safe`, `lossy`, `view` | How faithfully the note stands in for its source |
| `suite_original_state` | `kept`, `offloaded`, `trash`, `gone` | Where the original file is |
| `suite_original_sink` | a sink id | Required while `offloaded`, and only then |
| `suite_original_bytes` | whole number | Size of the original |
| `suite_shrunk_at` | ISO 8601 date or timestamp | When it was shrunk |
| `suite_saved_bytes` | whole number | Net bytes saved (may be negative) |

A `view` note must keep its original. Changing one key changes one line: every other line of
the frontmatter is returned as it was, and a key that holds a list or a multi-line value is
never rewritten as a scalar.

## Fidelity tiers

| Tier | Meaning | May be shrunk | May be edited |
|------|---------|---------------|---------------|
| `native` | Born as hqmd in the suite | not applicable | yes |
| `safe` | The import dropped nothing and the regenerate check passed | yes, after a preview | yes |
| `lossy` | The import dropped listed objects (charts, text boxes, embedded objects) | only with consent that names the losses | yes; the losses stay lost |
| `view` | Cannot be represented (encrypted, macros, damaged, unknown) | never | no; read-only |

Imports are one way and come with a loss report. Exports are one-way renders. Nothing promises
round-trip editing of someone else's complex file.

## Docs and slides

These conventions are carried by ordinary markdown, so they need no block type:

- A **doc** is headings, paragraphs, lists, tables, images and links. Anything else an import
  finds becomes a block the editor shows and does not change.
- A **slides** note has `suite_kind: slides`. The level 1 heading is the deck title and each
  level 2 heading starts a card. An optional HTML comment under the heading picks a layout:
  `<!-- layout: two-column -->`. Layout names are fixed in milestone M6.
- Images use relative links into `_media/cas/`: `![](../../_media/cas/ab/abcdef.webp)`.

## Editing

An edit is a patch: a list of operations that all apply or none do.

- A block is addressed by its position and the hash of its text (the first 12 hex characters of
  SHA-256 of the block without the blank lines around it). If the block changed since the
  caller looked, the edit is refused as stale and nothing is written. No hidden anchors are
  needed in the text.
- A patch may also carry the revision of the whole note (16 hex characters).
- All addresses in a patch refer to the note as it was before the patch, so operations do not
  shift each other. Two operations on one block conflict.
- Operations: replace a block with one block of markdown; insert markdown after a block or at
  the top; delete a block; set one frontmatter key.
- The result must read back as the blocks the edit produced. An edit that would merge with its
  neighbours when the note is read again (a list placed right after a list, text after a block
  with no closing fence) is refused as unstable.
- Replacing a block with its own text changes nothing.

## Guarantees

Each of these is a test in `crates/suite/hq-suite-md`:

1. Reading and printing any text, including broken text, returns the same bytes.
2. A plain markdown editor that changes the prose around typed blocks leaves them intact.
3. Setting a frontmatter key twice equals setting it once, and changes at most one line.
4. Inserting a block and deleting it again restores the note.
5. A failed patch changes nothing.
6. CRLF notes read as the same structure as their LF twins and stay CRLF after an edit.
7. A hostile note (deep nesting, huge headers, a million-cell table) is refused or reported,
   never allowed to hang or exhaust memory.

## Limits

| Limit | Value |
|-------|-------|
| Note size | 16 MiB |
| Frontmatter | 64 KiB |
| Header | 1024 bytes, 24 attributes |
| Inline sheet | 100,000 cells, 16,384 columns |
| Grid | A1 to XFD1048576 |
| Block hash | 12 hex characters |
| Revision | 16 hex characters |

## Versioning

New block types are added by name. An older reader sees the new block as code and saves it as
it was, so a note edited on a phone with a newer HQ survives a trip through an older one. A
change that would make an existing valid block invalid is not made; it gets a new name.

## Where this differs from the first draft

The scope note sketched the language before it was built. Three things changed on the way, each
for a reason:

- The dashboard chart over a range is `plot`, not `chart`, because `chart` already has a
  different meaning (JSON body) in HQ chat.
- Sheets gained `rows=`, so a note that points at a sidecar can show its size without reading it.
- `fields` is reserved, not defined. The fill-in syntax is decided in milestone M2 with the
  Docs editor, where it can be tested against real typing.
