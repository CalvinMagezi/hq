# Conversion fixtures

Small documents for the import pipeline tests (`tests/corpus.rs`). Everything in
them is invented: the company, the people, the phone numbers, the amounts. Nothing
here comes from a real vault, client or bank.

| File | What it checks |
|------|----------------|
| `invoice.docx` `.odt` `.rtf` `.doc` | one invoice in four formats: title, bold, a link, two tables, bullets |
| `invoice.pdf` | a PDF invoice: tables rebuilt from the page layout |
| `proposal.docx` `.odt` | headings, nested and numbered lists, a quote, a code block, a merged table cell |
| `tracker.xlsx` `.xls` `.ods` | dates that must not become serial numbers, a title row, two sheets |
| `training.pptx` `.odp` | slides, bullets with levels, speaker notes, a table, a picture |
| `statement.pdf` | a two-page mobile-money-style statement: one table across pages |
| `receipt.jpg` | a photographed receipt, for OCR |
| `page.html` `page-latin1.html` `page-utf16.html` | one page in three encodings |
| `records.xml` `notes.xml` | records become a table, other XML an outline |
| `data.csv` `data.tsv` `plain.txt` | delimited text and plain text |

`generate.py` makes the Office, HTML, XML and image files. The ODF, RTF, `.doc`,
`.xls` and PDF variants were saved from those with LibreOffice:

```bash
python3 generate.py out 52
cd out
soffice --headless --convert-to odt invoice.docx   # also rtf, doc, pdf
soffice --headless --convert-to ods tracker.xlsx   # also xls
soffice --headless --convert-to odp training.pptx
soffice --headless --convert-to pdf statement.docx
```

Tests that need poppler, LibreOffice or tesseract are `#[ignore]`d with a reason.
A larger set of real public files is opt-in: see `scripts/fetch-conversion-corpus.sh`.
