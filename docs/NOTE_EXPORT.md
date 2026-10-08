# Exporting notes

A vault note can be turned into a file you can send to someone who has never
seen the vault: a PDF, a web page, a spreadsheet, a notebook and more. The same
renderer sits behind every entry point, and none of it needs an external tool.

| Where | How |
|-------|-----|
| Web UI | Open a note and press the PDF button in the header. On a phone it opens the share sheet, on a desktop it downloads. |
| Terminal | `hq vault export <note> --format <fmt> [-o file] [--brand <slug>] [--lang <language>]` writes `<note>.<ext>` in the current folder by default. `hq vault export-pdf <note>` is the PDF-only form. |
| Agents | the `vault_export` tool (saves to `Exports/<note>.<ext>` in the vault and returns a `web_path`). `vault_export_pdf` still works. |
| HTTP | `GET /api/note/export?path=<note>&format=<fmt>[&brand=<slug>][&languages=py,sh]`. `GET /api/note/pdf?path=<note>` is the PDF-only form. |

`<note>` is a vault-relative path, the same without `.md`, or a bare note name.

## Formats

| Format | You get | Notes |
|--------|---------|-------|
| `pdf` | A4 pages with a footer and page numbers | Built-in Typst engine |
| `docx` | A Word document with real heading styles, lists and tables | Opens in Word, LibreOffice and Google Docs; no template |
| `png` | The whole note as one tall image | Scaled down if it would exceed 16,000 px |
| `svg` | The whole note as one tall vector image | |
| `html` | One self-contained page, styles inline | Images are embedded in the file |
| `md` | Cleaned Markdown, title as the first heading | |
| `xlsx` | One sheet per table | Sheets are named after the heading above each table |
| `csv` | The table | Several tables come back as a zip of CSV files |
| `json` | One table is an array of records, several are an object keyed by heading | Column order is kept |
| `jsonl` | One record per line | With several tables each record carries a `_table` field |
| `xml` | `<tables><table><row><cell column="...">` | Column names go in an attribute |
| `latex` | `tabular` environments to paste into a document | |
| `ipynb` | A Jupyter notebook | Python blocks become code cells, everything else Markdown cells |
| `jira` | Jira and Confluence wiki markup | |
| `code` | The note's fenced code blocks | One block is a plain file, several are a zip named after their headings. `--lang` filters |

`pptx` is not written by this exporter yet. For a `pptx`, or a `docx` built on a
brand's Word template, use the `convert_from_markdown` tool, which still runs pandoc.

The table formats need a table: a note without one is an error, not an empty file.
In spreadsheets, numbers are stored as numbers, but values with leading zeros
(`007`) and very long digit strings (ids, phone numbers) stay text. Text that
starts like a formula (`=SUM(...)`) is never executed: XLSX stores it as a
string and CSV gets a leading apostrophe.

## What changes between the note and the file

- Frontmatter is dropped. The title comes from the `title:` key, else the first
  `# Heading`, else the file name, and is printed once at the top.
- `[[Note]]`, `[[Note|alias]]` and `[[Note#Heading]]` become plain text.
- `![[image.png]]` and `![](image.png)` embed the image when it resolves inside
  the vault (next to the note, from the vault root, or by file name).
- `> [!tip] Title` callouts become coloured boxes in PDF, PNG, SVG and HTML, and
  panels in Jira markup. Colours follow the callout type.
- Code blocks are left as written. In PDF, PNG and SVG, lines longer than about
  90 characters are wrapped with a hanging indent, because a page cannot scroll.

## What is never embedded

A file leaves the machine, so the exporter is strict about what goes in:

- Images that resolve outside the vault (absolute paths, `..`, symlinks that
  leave it) are replaced by `[image unavailable]`. Only png, jpg, gif, svg and
  webp under 25 MB are read.
- Remote images are not fetched.
- Raw HTML in a note is dropped.
- Links with a scheme other than `http`, `https`, `mailto` or `tel` lose their
  target, so a shared file cannot carry a `javascript:` or `file:` link.
- A corrupt image does not fail the export; it is left out and the rest renders.

## Fonts

PDF, PNG and SVG use Typst's built-in fonts plus whatever is installed on the
machine, and fall back per character, so a note mixing English and Japanese
works when a Japanese font is installed. A machine with no CJK font prints
boxes for those characters. To add fonts without installing them system-wide,
put them in `~/.hq/fonts` or point `HQ_EXPORT_FONT_DIR` at a folder (several
folders are separated like `PATH`).

`--brand` applies a registered brand's accent colour, font and name in the footer
(see `Notebooks/Projects/<Name>/Branding/brand.yaml`). Brand logos are not drawn yet.

## Older PDF engines

Set `HQ_PDF_ENGINE=chromium|weasyprint|xelatex` to send PDF export through the
previous pipeline instead: pandoc plus a headless Chromium-family browser,
WeasyPrint, or xelatex. That path needs pandoc and the chosen engine installed,
supports `HQ_PDF_BROWSER=/path/to/browser`, and starts the browser with
`--no-sandbox` when running as root (force it elsewhere with
`HQ_PDF_NO_SANDBOX=1`). It still flattens callouts to a bold label.

## Tests

```bash
cargo test -p hq-export
cargo test -p hq-convert note_pdf
cargo test -p hq-web note_export
# the legacy engines, needs pandoc and an engine:
cargo test -p hq-convert -- --ignored note_pdf_renders
```

To look at output while changing the renderer:

```bash
cargo run -p hq-export --example export -- note.md out.png
```
