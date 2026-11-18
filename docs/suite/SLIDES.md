# Slides: a deck as a note, and a native PowerPoint file

A slides note is ordinary markdown with `suite_kind: slides` (`docs/suite/HQMD.md`). The level 1 heading is the deck title, and each level 2 heading starts a card.

```markdown
# Board review

Prepared for the board.

## Results

- Revenue **up** 12%
  - Uganda strongest

## Plan
<!-- layout: two-column -->

1. Hire
2. Train

---

| item | qty |
|---|---|
| Sugar | 40 |
```

```bash
hq suite pptx Notebooks/deck.md            # writes Notebooks/deck.pptx, never replacing a file
hq suite pptx Notebooks/deck.md --out ~/board.pptx
```

## What the file holds

- A title card (the deck title, and any text between the title and the first card as its subtitle), then one card per level 2 heading.
- Bullets (nested to four levels), numbered lists, paragraphs, **bold** and *italic*, code in a monospace font, and tables as real PowerPoint tables with borders.
- Layouts: `content` (the default) and `two-column`, picked by `<!-- layout: NAME -->` under the card's heading. In a two-column card a horizontal rule (`---`) splits the columns. A name the suite does not have is reported, and the card uses `content`.
- A real package, written by the suite with no template and no external tool: one slide master, two layouts, a theme and one part per card. The titles and bodies are placeholders, so PowerPoint's outline view and accessibility checker find them. The theme uses neutral grays only; a colour is the person's choice, not the writer's.

## How it was checked

- The package parts are written by hand and every one is checked to be well-formed XML, including with hostile text (control characters, `<`, `&`, quotes).
- The suite's own PowerPoint reader (`hq-convert`) reads a written deck back, and the title, every card, the bullets, the nested bullet and the table values are all there.
- LibreOffice opened a written deck and rendered every card correctly: title card, nested bullets, two columns, a table under code.

Not yet opened in PowerPoint or Keynote.

## What is not done

- Pictures are not placed in the file yet. `hq suite pptx` says how many a deck links and left out.
- Speaker notes, transitions, per-card colours and more layouts.
- A card editor on the phone. The reader shows a slides note as the markdown it is; the file is made on a computer.
