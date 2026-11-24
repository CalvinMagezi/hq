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
- Pictures: a picture linked in a card is placed under the card's text (stacked if there are several, each fitted to its column with its proportions kept), as PNG: the suite's WebP pictures are decoded and rewritten, since PowerPoint does not reliably open WebP. Only a file inside the vault, of a known image type, within 25 MB and 40 million pixels is placed; a remote, missing or damaged picture is left out and `hq suite pptx` says how many.
- Bullets (nested to four levels), numbered lists, paragraphs, **bold** and *italic*, code in a monospace font, and tables as real PowerPoint tables with borders.
- Layouts: `content` (the default) and `two-column`, picked by `<!-- layout: NAME -->` under the card's heading. In a two-column card a horizontal rule (`---`) splits the columns. A name the suite does not have is reported, and the card uses `content`.
- **Transitions and colours**, set by comments under a card's heading, or before the first card for the whole deck (a card's own comment wins):
  - `<!-- transition: fade -->`, with `push`, `wipe`, `split`, `cover`, `dissolve`, or `none` to cancel the deck's transition for one card. Add `slow`, `medium` or `fast`: `<!-- transition: push slow -->`.
  - `<!-- background: #1F3A5F -->` and `<!-- color: #FFFFFF -->`: the card's background and its text (titles, bullets, tables). Write `#RRGGBB` or `#RGB`. The writer never picks a colour: a card with no comment keeps the neutral theme.
  - A transition or colour the suite cannot use is named in `hq suite pptx`'s output, and so is a text colour that is hard to read on its background (contrast under 3:1, counting the theme's white and black for whichever you leave unset).
- **Speaker notes**: `<!-- notes: Say this first. -->` under a card's heading, or a comment that runs over several lines (blank lines separate paragraphs). A card with notes gets a notes page (and the file a notes master); a deck with none has no notes parts. A notes comment inside a code fence is code, not a note.
- A real package, written by the suite with no template and no external tool: one slide master, two layouts, a theme and one part per card. The titles and bodies are placeholders, so PowerPoint's outline view and accessibility checker find them. The theme uses neutral grays only; a colour is the person's choice, not the writer's.

## How it was checked

- The package parts are written by hand and every one is checked to be well-formed XML, including with hostile text (control characters, `<`, `&`, quotes).
- The suite's own PowerPoint reader (`hq-convert`) reads a written deck back, and the title, every card, the bullets, the nested bullet and the table values are all there.
- LibreOffice opened a written deck and rendered every card correctly: title card, nested bullets, two columns, a table under code, and pictures under text and in both columns of a two-column card. It also parsed the speaker notes into real notes pages with their exact text, and the suite's reader reads them back.

- Keynote opened a deck with transitions and colours and raised no repair prompt. It drew the colours as written and read the transitions back (push at slow speed, fade shown as its "dissolve", and none). LibreOffice read the same transitions.

Not yet opened in PowerPoint.

## What is not done

- More layouts, and per-run or per-shape colours (a colour applies to a whole card).
- A card editor on the phone. The reader shows a slides note as the markdown it is; the file is made on a computer.

## Editing cards: `office_edit` and `POST /api/suite/edit`

A card is the run of blocks from a `## ` heading to the next one. `cards` lists them (`GET /api/suite/cards?path=`): number, `hash` (of all the card's blocks), layout, transition, colours and notes. Then `add_card` (after `after_card`, or at the end), `replace_card` (the whole card, notes and look with it), `move_card` (`to`), `delete_card`, `set_notes` (empty clears) and `set_card_style` (`transition`, `background`, `color`, `layout`; `null` clears one). A card edit quotes the card's `hash`, so a card that changed since it was read is refused, and the other cards come back byte for byte.

Notes and look are written as the comments above (`<!-- notes: ... -->`, `<!-- transition: fade slow -->`) and their values are checked with the pptx writer's own parsers: a transition or colour the writer could not use is refused with the same sentence. A colour pair that is hard to read is allowed and comes back as a warning. An existing comment is changed in place; a new one goes under the card's heading. After every edit the deck is read back, and an edit that would make a different set of cards than asked (text with its own `## ` heading) is refused. Replace, delete and clearing notes are previewed first when an agent asks (see `SHEETS.md`).
