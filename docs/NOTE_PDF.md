# Exporting notes as PDF

A vault note can be turned into a PDF you can send to someone who has never
seen the vault. The same renderer sits behind three entry points:

| Where | How |
|-------|-----|
| Web UI | Open a note and press the PDF button in the header. On a phone it opens the share sheet, on a desktop it downloads. |
| Terminal | `hq vault export-pdf <note> [-o file.pdf] [--brand <slug>]` (writes `<note>.pdf` in the current folder by default) |
| Agents | the `vault_export_pdf` tool (saves to `Exports/<note>.pdf` in the vault and returns a `web_path`) |
| HTTP | `GET /api/note/pdf?path=<note>[&brand=<slug>]` returns the file as `application/pdf` |

`<note>` is a vault-relative path, the same without `.md`, or a bare note name.

## What changes between the note and the PDF

- Frontmatter is dropped. The title comes from the `title:` key, else the first
  `# Heading`, else the file name, and is printed once at the top.
- `[[Note]]`, `[[Note|alias]]` and `[[Note#Heading]]` become plain text.
- `![[image.png]]` and `![](image.png)` embed the image when it resolves inside
  the vault (next to the note, from the vault root, or by file name).
- `> [!tip] Title` callouts become a bold label in a quote block.
- Code blocks are left exactly as written.

## What is never embedded

A PDF leaves the machine, so the exporter is strict about what goes in:

- Images that resolve outside the vault (absolute paths, `..`, symlinks that
  leave it) are replaced by `[image unavailable]`.
- Remote images are not fetched. They become a link.
- Raw HTML in a note is dropped.

## Engines

The first one found is used:

1. A Chromium-family browser run headless (Chrome, Chromium, Edge, Brave). Best
   output, and usually already installed.
2. WeasyPrint.
3. `xelatex` through pandoc.

pandoc is required for all three. Set `HQ_PDF_ENGINE=chromium|weasyprint|xelatex`
to force one, and `HQ_PDF_BROWSER=/path/to/browser` when the browser is not on
`PATH`. When running as root (a container), the browser is started with
`--no-sandbox`; set `HQ_PDF_NO_SANDBOX=1` to force that elsewhere.

`--brand` applies a registered brand's accent colour, font and light logo (see
`Notebooks/Projects/<Name>/Branding/brand.yaml`).

## Tests

```bash
cargo test -p hq-convert note_pdf
# real renders, needs pandoc and an engine:
cargo test -p hq-convert -- --ignored note_pdf_renders
cargo test -p hq-web -- --ignored note_pdf_endpoint_serves_a_pdf
```
