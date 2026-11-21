# Sheets: the formula engine

`hq-suite-calc` is a small spreadsheet formula engine: A1 references and ranges, the usual operator
precedence (`-2^2` is 4, `50%` is 0.5), errors that travel through a calculation, a cycle reported
as `#CIRC!` instead of looping, and a work stack so a chain of thousands of dependent formulas
cannot overflow. Functions: `SUM PRODUCT AVERAGE MIN MAX COUNT COUNTA IF IFS IFERROR SWITCH AND OR
NOT ABS INT SQRT ROUND ROUNDUP ROUNDDOWN MOD LEN CONCATENATE CONCAT ISNUMBER ISTEXT ISBLANK ISERROR
LEFT RIGHT UPPER LOWER TRIM ROW INDEX VLOOKUP PMT EDATE COUNTIF SUMIF SEARCH FIND SUBSTITUTE N XLOOKUP TEXT POWER QUOTIENT CHAR ROWS TEXTJOIN LET`, and since the second batch (see "Second batch of functions" below) `TRUE FALSE IFNA COUNTIFS SUMIFS AVERAGEIF AVERAGEIFS MAXIFS MINIFS MEDIAN LARGE SMALL RANK RANK.EQ MATCH LOOKUP HLOOKUP VALUE REPT PROPER MID EXACT TRUNC SIGN CEILING FLOOR DATE YEAR MONTH DAY WEEKDAY EOMONTH DATEDIF`, plus references between sheets (`=Stock!B2`, `='Price list'!A1:A9`). Anything else is `#NAME?`; it is never guessed.

## How good it is: measured, not assumed

```bash
hq suite calc-check workbook.xlsx
```

Every formula in an Excel workbook has the value Excel saved with it. The check loads the workbook
into the engine, calculates every formula, and compares each answer with the saved one. The report
is counts only (no sheet names, cell addresses, formulas or values), so it can be shared; it also
counts the functions the engine lacks, which says what to build next.

First run, 57 workbooks of the owner's own (334 sheets, 86,082 formulas):

| Result | Formulas | Share |
|--------|----------|-------|
| Gave Excel's saved answer | 58,626 | 68.1% |
| Gave another answer | 303 | 0.4% |
| Could not be calculated | 27,153 | 31.5% |

Adding `INDEX ROW VLOOKUP PMT EDATE` moved the same 57 workbooks to 72.6% matching (62,468 of 86,082), with 447 giving another answer (those are formulas the engine now tries, so they were not counted before) and 23,167 still not calculated.

The 0.4% that give another answer are the ones to fix first. Of the 31.5% that could not be
calculated, about 11,000 read a cell that is itself one of these, and about 7,800 use syntax the
parser does not read yet (mostly references to another sheet, which needs a workbook rather than one
sheet). The functions named most often: `INDEX` (5,051), `ROW` (4,349), `EDATE` (2,259), `VLOOKUP`
(1,950), `LET` (1,595), `PMT` (1,080), `TEXT` (697).

The first run also showed the check's own bug (it compared against the wrong cell), which is why a
number here comes with the command that reproduces it.

## What is not built

Pivots, array results (`FILTER`, `SORT`, `UNIQUE`), table references (`Table1[Column]`), conditional formatting, and the sidecar for very large sheets. Editing, UGX formats, references between sheets and charts are built; see the sections below.

## On the phone

A `sheet` block in a note draws as a grid and a `tile` block as a labelled number, both computed by hq (`GET /api/suite/sheet`, `GET /api/suite/tile`). A tile reads a sheet in its own note, or in one other note of the same vault (see "Tiles and plots over another note" below): `from="[[Note]]!D2:D40"` with `fn` of `sum`, `count`, `avg`, `min`, `max` or `median`, and `format` such as `ugx`.

## Tiles and plots over another note

`from="[[Stock]]!D2:D40"` reads the `sheet` blocks of the note called Stock, `[[Stock#Prices]]!A1:C9` one of them by `name=`. `where`, `group`, `fn`, `format` and the chart kinds all work the same as over a sheet in the note itself.

- **Which note.** A name without a `/` is a file name (`Stock.md`, case does not matter) searched under the vault, skipping folders that start with `.` or `_` and never following a link. If two notes share the name the block is refused and the sentence lists both paths; write the folder to pick one, `[[Archive/Stock]]!...`. A name with a `/` is a vault-relative path without `.md`. Everything goes through `SuitePaths`, so an absolute path, `..` or a link out of the vault is refused.
- **One note deep.** Only the other note's `sheet` blocks are read. A tile or plot that sits in that note is not followed, and its formulas see its own note only.
- **Caps.** The other note must be 2 MB or less, and a name lookup visits at most 50,000 entries, 16 folders deep.
- **Missing things.** "There is no note called `X` in this vault", "the note `X` has no sheet called `Y`" and "`X` has no sheet" are said plainly, and `add_tile` / `add_plot` refuse to write such a block.
- **The web mirror.** The web and the phone do not resolve notes: they call `GET /api/suite/tile` and `GET /api/suite/plot`, which read the vault through the same code, so a cross-note block draws the same everywhere and a client must never try to read the other note itself. Dashboard snapshots (`hq-suite-adapter::snapshot`) use the same route into the code. The native `.xlsx` export leaves a plot over another note out (its points are not cells of the exported sheet) and counts it in the statement.

## Accuracy after the first fixes

Re-measured on the same 57 workbooks (86,082 formulas): 62,556 give Excel's saved answer (72.7%), **42 give another answer (0.05%)**, and 23,484 cannot be calculated yet (27.3%). The wrong answers fell from 447 to 42 after three fixes the first measurement pointed at:

- A cell the engine cannot calculate (`#NAME?`) is no longer hidden by `IFERROR` or `ISERROR`: the formula stays "cannot calculate" instead of returning the fallback with confidence. This alone removed about 290 wrong answers, and it is the rule that matters most: an unsupported cell must never turn into a plausible number.
- Values loaded from a file keep their type (`Sheet::set_value`): text that looks like a number stays text, and each Excel error keeps its kind.
- `EDATE` reproduces Excel's 1900 quirks (serial 0 is "0 January 1900", and 1900 counts as a leap year).

`hq suite calc-check` now also prints the wrong answers by leading function and by the kind of value on each side (counts only), so the next fix is chosen from numbers.

## Sheets that read each other

`Workbook` holds several sheets and lets a formula name another one, `Prices!B2` or `'Price list'!A1:A9` (names are not case sensitive). In a note, every `sheet` block is a sheet of one workbook, named by its `name=` (or `Sheet1`, `Sheet2` by position), so one block can read another. A name that does not exist leaves the formula "cannot calculate" rather than guessing. Excel's last column, XFD, is the limit; `ZZZ1` is not a cell.

The same 57 workbooks after this and two more fixes (text dates such as `2023-06-01` read as dates in arithmetic, and a difference within rounding noise of zero is 0 as in Excel): **75,248 of 86,082 formulas match Excel (87.4%), 43 differ (0.05%), 10,791 cannot be calculated yet (12.5%)**. A result within 1e-6 of the saved one, or within 1e-9 of its size, counts as the same number, because chains of money arithmetic differ from Excel by rounding noise.

Left to build, by how often the owner's workbooks use them: `LET`, `TEXT`, `COUNTIF`, `FILTER`, `XLOOKUP`, `SEARCH`, `SUBSTITUTE`, `N`, and some syntax the parser does not read yet.

## More functions

`COUNTIF` and `SUMIF` (with `>5`, `<=3`, `<>x` and `*`/`?` wildcards), `SEARCH`, `FIND`, `SUBSTITUTE`, `N`, `XLOOKUP` (exact match), `POWER`, `QUOTIENT`, `CHAR`, `ROWS` and `TEXTJOIN` were added from the owner's usage counts. `TEXT` reproduces number formats (`#,##0`, `0.00`, `0.0%`); a date or free-form format is left "cannot calculate" rather than guessed. Measured on the same 57 workbooks: **76,006 of 86,082 formulas match Excel (88.3%), 54 differ (0.06%), 10,022 cannot be calculated yet (11.6%)**. Still missing by count: `LET` (1,595), `FILTER`, `TODAY` and `NOW` (not repeatable, so not planned), `SUMPRODUCT`, `CHOOSE`.

`LET(x, 5, y, x*2, y+1)` is supported: each name stands for its value in what follows, a name bound again hides the earlier one, and names that build on names stop at a size limit. On its own it did not move the match rate, because the `LET` formulas in the owner's workbooks also use things the parser cannot read yet. The largest groups of unreadable syntax, by count: table references such as `Table1[Column]` (about 1,500), references to sheets that are not in the workbook (external links, about 2,000 together), and whole-column ranges such as `$B:$B` (about 500, which need a lookup that does not copy a million cells).

## Editing a cell on the phone

Press and hold a cell in a note's sheet to edit what is typed in it (a number, text or a formula). hq changes only that cell's line of the table and writes the note back otherwise byte for byte, then sends the sheet recalculated, so a total follows the new quantity. The edit is refused, and nothing is written, if the cell no longer holds what the phone showed (someone else changed it), if the text contains a `|` or a line break, or if the cell is outside the sheet. `POST /api/suite/sheet/cell` takes `{path, index, row, col, expected, text}`.

Not built: adding or deleting rows and columns, column types, and an undo for an edit (the note's own history is the undo).

## Charts from a sheet: `plot`

A `plot` block draws a chart from a range of a sheet in the same note (or another note, see below): `type` is `bar`, `line`, `pie`, `donut`, `area`, `stacked` or `scatter`, `from` is a range whose first row names the columns, `x` is the column of labels and `y` the column of values (names are not case sensitive). hq calculates the values (a formula column is drawn as its results) and sends the chart in the format the chat already draws, so the phone uses its existing chart card. `GET /api/suite/plot?path=&index=`.

A plot that cannot be drawn says why instead of guessing: an unknown column, a value that is not a number, no rows, or more rows than a chart takes (50 for bar and line, 12 for pie and donut). One series per plot unless `group` is used (below).

`area` and `stacked` (stacked bars) draw like `line` and `bar` over the labels, and take `group`. `scatter` places points by a numeric x: the `x` column must hold numbers, it takes no `group`, and the chart JSON carries the numbers as `xValues` (the same values, as text, are also in `labels`, so a reader that only knows categories still shows something). The web app and the Android app draw all three.

## The dashboard: `where` and `group`

Tiles and plots take two more attributes, so a note can hold a small dashboard over its sheets.

`where` keeps only the rows that match: `where="qty>30 and item=Sugar"`. Up to four conditions joined by `and`, each a column name from the range's first row, one of `= != <> > >= < <=`, and a number or text (text is not case sensitive). A comparison only holds between values of the same kind, and `!=` also holds between different kinds, as in a spreadsheet.

On a **tile**, a `where` makes the range's first row the column names and takes the number from the range's **last column**: `from="A1:C6" fn=sum where="region=North"` adds the `amount` of the North rows. Without a `where` the tile reads every number in the range, as before.

On a **plot**, `group=region` draws one series per value of that column (up to six, over the labels in the order they first appear, a label a group lacks counts as 0); it needs a bar, line, area or stacked chart. `where` filters the rows before anything is drawn.

A tile takes no `group` (it shows one number); a pie or donut takes none either. Anything that cannot be read or drawn says why instead of guessing: an unknown column, a condition without a value, more than four conditions, more than six groups.

### Layout

The web app draws a dashboard's tiles and plots in the order the note has them, as one grid, and lets a
person move a block earlier or later and give it a size (`size=small|wide|full`, see "Dashboard layout" in
`HQMD.md`). The filters above stay view-only; the layout is written into the note.

## Second batch of functions

Added to `hq-suite-calc` from the list of what the owner's workbooks still miss. **Not re-measured**: no real workbook was available when this was written, so the match rates above are the ones measured before it, and the effect of this batch on them is unknown until `hq suite calc-check` is run again. Every new function has unit tests that assert Excel's documented answers, error cases included (`crates/suite/hq-suite-calc/src/tests_more.rs`).

**Added:** `TRUE()`/`FALSE()` (the bare words `TRUE` and `FALSE` already worked), `IFNA`, `COUNTIFS`, `SUMIFS`, `AVERAGEIF`, `AVERAGEIFS`, `MAXIFS`, `MINIFS`, `MEDIAN`, `LARGE`, `SMALL`, `RANK` and `RANK.EQ`, `MATCH` (types 0, 1 and -1), `LOOKUP` (vector form), `HLOOKUP`, `VALUE`, `REPT`, `PROPER`, `MID`, `EXACT`, `TRUNC`, `SIGN`, `CEILING`, `FLOOR`, `DATE`, `YEAR`, `MONTH`, `DAY`, `WEEKDAY` (types 1, 2, 3 and 11 to 17), `EOMONTH`, `DATEDIF` (`Y`, `M`, `D`, `YM`).

**`TEXT`** now reproduces date formats (`yyyy-mm-dd`, `dd/mm/yyyy`, `dd.mm.yyyy`, `mmm yyyy`, `d mmmm`, `dddd`, `ddd`, `yy`, `m/d/yy`), currency-like number formats (`$#,##0.00`, `"UGX "#,##0`, `#,##0 "UGX"`, `\U\G\X #,##0`, a currency sign such as the euro), `0`, `#`, `#,##0`, `0.##` (with Excel's trailing point, `2.`), zero padding (`000`), percent, a second section for negatives (`#,##0;(#,##0)`) and a third for zero. A text that is not a number comes back unchanged and a truth value as `TRUE` or `FALSE`, as in Excel.

**Rounding** now works on the 15 significant digits Excel keeps instead of on the binary number underneath, so `ROUND(1.005,2)` is 1.01 and `ROUNDUP(2.3*3,0)` is 7 (the first used to give 1). `TEXT` uses the same rule.

**Whole-column ranges** (`$B:$B`, `B:B`, `Sheet!A:C`, `B2:B1048576`) are read. The engine stops at the last row anything is written in, in the whole workbook, so a million empty cells are never copied. Cutting a column back changes nothing for sums, counts, lookups or `INDEX` (`INDEX(B:B, 500000)` is still an empty cell), but it would change a count of blanks, so `COUNTIF(A:A,"")`, `COUNTIF(A:A,"=")` and the same in `COUNTIFS`/`SUMIF` are refused as "cannot calculate". `ROWS(A:A)` is 1,048,576.

**Behaviour fixed on the way**, each of which could have given another answer before: `COUNTIF(range, 0)` no longer counts blank cells; `COUNTIF(range, "<>")` counts the cells that are not blank, `"="` only blank ones, and `""` blank cells and empty texts; the text `"inf"` or `"nan"` is no longer read as a number; `VLOOKUP` with approximate matching ignores the blank rows below a table.

### What stays "cannot calculate", and why

The rule is unchanged: an unsupported thing is `#NAME?` ("cannot calculate"), never a plausible number. `hq_suite_calc::NOT_PLANNED` lists the functions left out on purpose with the reason, and `why_unsupported(name)` returns it; they are also absent from `FUNCTIONS`, so the calc-check report counts each by name.

| Left out | Why |
|----------|-----|
| `TODAY`, `NOW`, `RAND`, `RANDBETWEEN` | The answer depends on the clock or chance, so a saved value cannot be repeated. |
| `OFFSET`, `INDIRECT` | Volatile: what they read depends on a calculation or a text. |
| `SUMPRODUCT` | Works on whole arrays, and the engine has no array values. |
| `CHOOSE` | Not built yet. |
| `UNIQUE`, `SORT`, `FILTER` | They return an array that spills over several cells; a cell here holds one value, so there is no safe result to show. |
| Table references (`Table1[Column]`) | The workbook model has no tables (the loader reads cells, not table definitions), so nothing can resolve the name. They fail to parse and stay "cannot calculate". |

Smaller refusals inside functions that exist:

- `MATCH` with type 1 or -1, `LOOKUP` and an approximate `HLOOKUP` need sorted data. Excel's binary search over an unsorted list, or one that mixes texts and numbers, depends on how it probes, so such a list gives "cannot calculate". A trailing run of blank cells is allowed. `MATCH(x, range, -1)` with repeated values, `MATCH` with `~` escapes, `LOOKUP` over a table (array form) and a blank lookup value are refused too.
- `LARGE` and `SMALL` with a fractional `k`, `AVERAGEIF` with a sum range of another shape.
- `DATEDIF` with `"MD"` or `"YD"` (Excel's own answers are inconsistent around month ends and leap days).
- A date given as text other than `2024-03-15` (`15/03/2024`, `March 5`), and `VALUE` of a text with a currency sign, a time, odd thousands commas or another date style: Excel's reading depends on the locale.
- `TEXT` with a time (`hh:mm`), colours (`[Red]`), currency tags (`[$USD]`), fractions, scientific, a scaling comma (`0,`), fill characters, an unquoted letter, or a small negative that shows as zero (`-0.001` as `0.00`, which Excel shows with a sign).

### Left for the web mirror

`apps/hq-web/src/lib/sheetCalc.ts` is a separate TypeScript calculator for the grid and has only five functions. It does not follow this engine, so a cell it shows can differ from the one hq calculates until it either learns these functions or asks hq for the values.

## Charts in the `.xlsx`

A `plot` block that reads a plain range (no `where`, no `group`) is written as a native chart: bar, line, pie, donut, area (`c:areaChart`), stacked bar (`c:barChart` with `grouping=stacked`, `overlap=100`) or scatter (`c:scatterChart` with a numeric x), with the chart reading the sheet's own cells (so changing a cell in Excel moves the chart) and carrying a cached copy so a viewer that does not recalculate still draws it. Series colours are written explicitly so every program draws the same chart. A plot that filters or groups has no single range, so it is left out and the export's one-line statement says how many. `hq-suite-xlsx::write_xlsx_with_charts` is the writer; `hq-suite-adapter::charts` maps a note's plots onto it. Checked in LibreOffice (`cargo test -p hq-suite-xlsx --test libreoffice -- --ignored`, set `XLSX_KEEP=<dir>` to keep the PDF); not yet opened in Excel.
## Editing more than a cell: `office_edit` and `POST /api/suite/edit`

The agent's `office_edit` tool and the apps' `POST /api/suite/edit` run the same code (`hq-suite-adapter::edit`). Every action reads the note, builds the note as it would be after the change, and writes it back as one hqmd patch, so a block that changed since it was read is refused and every other byte of the note stays as it was.

Sheet actions (`sheet` is the sheet block's number from 0; row 0 is the header row): `set_cells` (a rectangle of `values` with the `expected` text of each cell), `add_rows` (at the end, or before `at`), `delete_rows`, `add_column` (on the right: `name`, `type`, `default`), `set_column_type` (declares the type; cells are not converted) and `add_sheet`. The structural ones quote the sheet block's `hash`. Cell text follows the single-cell rules (2,000 characters, no `|` or line break); a formula must pass the xlsx writer's safety check and parse in the engine; a sheet holds at most 100,000 cells and 16,384 columns; a sheet with a `data=` sidecar is not edited.

**Formulas that read rows.** Adding rows in the middle and deleting rows move the references that read the rows below, the way Excel and Sheets do (`ref_shift.rs`). Formulas on the sheet and on other sheets of the note (`Name!A1`, `'Price list'!A1`, any case) are rewritten, and so is the `from=` of every tile and plot that reads the sheet in this note. A range with an insert inside it (or at its last row) grows, one that starts at the insert moves whole, and a range just above the insert stays as it is. Deleting shrinks ranges; a reference to a deleted row becomes `#REF!`, which the sheet shows as an error. `$` rows move too. Text in quotes, function names, whole-column references (`A:A`) and other notes' ranges are left byte for byte. The reply says what moved ("Adjusted 6 formulas and 1 chart"). The one refusal: deleting every row a tile or plot reads. Adding rows at the end and adding a column on the right move nothing.

**Preview then apply.** An agent's call that deletes anything, or touches more than 50 cells, writes nothing and answers with a summary and a `token`. The same call with the `token` applies it, and only if the note is still at the revision the preview was made from. A single-cell or single-block edit is one step. The apps' calls (the owner's own) are one step; `dry_run: true` asks what a call would do.

**Dashboards.** `add_tile` and `add_plot` build the block from structured arguments, draw it with the evaluators above before writing, and refuse with the evaluator's own sentence if it would not draw.

**Telling open pages.** Every suite write sends `{"type": "note_changed", "path", "hash"}` on the socket, and the web app reloads the note it has open. The existing `notes:change` event is coarser (a count and a time from the search index) and still arrives.

## Columns: insert and delete

`add_column` takes an optional `at` (a column number from 0) to insert before that column; without it the column goes on the right. `delete_column` takes `col` and, as a stale guard, optionally the column's header as `expected`. Both rewrite the header, the rule line, every row and the block's `types=` list, and move every reference the way Excel does: formulas in this sheet and in other sheets that name it, whole-column references such as `B:B`, and the `from=` range of tiles and plots. A reference to a deleted column becomes `#REF!` (the formula engine shows an error for it), and a tile or plot whose range would lose all its columns is refused. A tile or plot that names the deleted column by header in `x=`, `y=`, `group=` or `where=` is refused too, since those names do not shift. The last column cannot be deleted, and for the agent `delete_column` is previewed before it is applied.
