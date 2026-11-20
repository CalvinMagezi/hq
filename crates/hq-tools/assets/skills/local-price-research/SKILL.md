---
name: local-price-research
description: Compare prices across places or sources. Use the unit the market sells in, two independent sources per place, date and retail or wholesale on every figure, verify the source URLs. Not for one lookup.
hints: [compare prices, price comparison, prices in, market prices]
provenance:
  mintedBy: hq
---

# Local price research

A price is only useful in the unit and market where people pay it. Use this for requests that compare prices across places or sources. A single quick question ("how much is a litre of petrol today") needs one good source and one sentence; skip this skill.

## 1. Settle the unit first

Before searching for figures, find out how this good is actually sold in each place: by the bunch, sack, crate, dozen, piece, litre, kilogram, per month, per night. One search for the trade unit is enough ("how is X sold in <place>").

- Give the figure in that trade unit and, when a source supports it, a per-standard-unit figure (per kg, per litre) as well. Label each with its unit. Never convert between them yourself unless a source states a weight or size, and then say which one you used.
- If only one of the two exists, give that one and say the other was not found.
- If a source gives a range, keep the range. Do not collapse it to a midpoint.

## 2. Two independent sources per place

For each place, look for at least two sources that do not copy each other: a market report, a government or trade statistics page, a news item, a retailer or marketplace listing, a price-tracking site. One aggregator page counts as one source however many rows it has, and a page that repeats another counts as the same source.

- Stop searching a place when two independent sources are found, or when a few distinct queries and sites turned up nothing more.
- If only one source exists for a place, say so in that place's row. If sources disagree, show both figures and the gap. Do not average them away.
- A district stands for a region only when you say so. Name the place each figure is for.

## 3. Record the quality of every figure

Every price you present, in chat, a note or a sheet, carries these four things: the source (a name and a URL), the date it was seen or published, whether it is retail or wholesale (or unknown), and a confidence of high, medium or low with a few words of why.

In a sheet or table add them as columns: `source`, `date seen`, `retail or wholesale`, `confidence`. Leave a cell blank when you do not know it. A blank is honest; a guess is not. `office-authoring` has the file mechanics.

## 4. Verifier helpers get the sources

When you hand the figures to a verifier helper, give it each figure with its source URL and tell it to fetch every URL and compare the page with the figure. Do not pass numbers as given.

- Brief it to return, per figure: confirmed, different (with the number it found), or could not fetch.
- A figure whose source could not be fetched, by you or the helper, is marked `unverified` in the output, in the table and in the sentence that quotes it. Say that no independent check was made.
- Do not describe a figure as verified because a helper agreed without opening the source.

## 5. A dashboard is a page with a chart

When the person asks for a dashboard, or for a comparison they can open, make a page, not an export of your notes.

- Write ONE self-contained `.html` file: no external scripts, fonts or images, so it works offline. Save it under `Exports/` in the vault and attach it to the reply.
- Draw one bar chart per comparison, in inline SVG or CSS bars: one bar per place, the unit on the axis. A range is a bar with whiskers, or two values, never a midpoint.
- Add a small table with source, date, retail or wholesale, and confidence for every figure.
- Set an explicit light page background and dark text colours on both `html` and `body`, so the page reads in any theme.
- A place with no figure shows the words "no figure found". Never draw a zero bar for it.
- The spreadsheet is a separate `.xlsx`: numeric price columns (low and high as numbers, the unit in its own column) plus the quality columns from section 3.
- Never present an export of the research note as the dashboard.

## 6. Check the figures with a helper

After drafting and before the final answer, hand the table with its source URLs to a verifier helper through `spawn_subagents`. Section 4 says what to brief. Report its result in the answer in one plain sentence. If the person asked for a helper, always do this.

## 7. Caveats: short, next to the number

Put a caveat beside the figure it limits, in a few words: "one source", "wholesale", "seen 2026-03", "unverified". Do not collect caveats in a footnote or a closing paragraph. Lead the answer with the figures in their trade unit, then the table.

## Do not

- Do not hard-code a unit or a market from memory. Find it for the place asked about.
- Do not run this for a request that only needs one lookup, and do not delay a simple answer to run it.
- Do not present a figure without a date. If the page shows none, write "date not shown".
