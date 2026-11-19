---
name: office-authoring
description: Make a spreadsheet, document or deck as a file the owner opens
hints: [office_create, office_export, make a spreadsheet, build a spreadsheet, put this in excel, write a brief, write a document, make a deck, make slides, slide deck, xlsx, docx, pptx]
provenance:
  mintedBy: hq
---

# Office authoring

You make office files in two steps: write a note, then export it. The note is the truth and stays in the vault. The file is a copy made from it, saved under `Exports/` in the vault, where the owner's apps can open it.

Never write a deliverable anywhere else. A file in a scratch folder, `/tmp` or your own working directory is invisible to the owner. If a script already made a file, call `file_deliver` with its path and say nothing about where it came from.

## The two steps

1. `office_create` with `kind`:
   - `sheet`: give `sheets`, each with `columns` (a name and a `type`: text, int, num, bool, date, pct, or a currency code such as ugx or usd) and `rows`. A cell that starts with `=` is a formula, written the way a spreadsheet writes it (`=B2*C2`, `=SUM(D2:D9)`). Use formulas for totals and products so the owner can change a number and see the rest follow. Leave a cell blank when you do not know the value.
   - `doc`: give `content` as Markdown. Use headings, short paragraphs and tables.
   - `slides`: give `content` as Markdown with one `## heading` per slide and a few bullets under each.
2. `office_export` with the note path and a `format`: xlsx or csv for a sheet, docx for a document, pptx for a deck, pdf for any of them.

The export result has a `fidelity` line saying what was kept. Pass that line on to the owner in your reply, in your own words, together with the name of the file. Do not paste a link: the chat shows the file as a card by itself.

## Data you gathered

- When the figures come from the web, add a `source` column, an `as of` date column, a `retail or wholesale` column and a `confidence` column to every row, and fill them with what you actually read. For price comparisons, `local-price-research` has the rules.
- Write a blank where you could not verify a figure. Never fill a gap with a guess or a number from memory.
- Say in your reply which figures are missing and why.

## When to ask first

Ask one short question before you start when the request leaves out something that changes the file a lot: how many rows or slides, which columns, which currency, who the audience is. A small request does not need a question: make it, and offer to change it.

## What not to do

- Do not use `convert_from_markdown` or a shell to make the deliverable. They are for other jobs.
- Do not make a second file when the owner asks you to change the first: edit the note with `vault_patch_section` or write a new version of it, then export again.
- Do not promise layout. The files carry content, formulas and number formats, not charts or pictures made by hand.
