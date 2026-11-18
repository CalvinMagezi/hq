# Sheets: the formula engine

`hq-suite-calc` is a small spreadsheet formula engine: A1 references and ranges, the usual operator
precedence (`-2^2` is 4, `50%` is 0.5), errors that travel through a calculation, a cycle reported
as `#CIRC!` instead of looping, and a work stack so a chain of thousands of dependent formulas
cannot overflow. Functions: `SUM PRODUCT AVERAGE MIN MAX COUNT COUNTA IF IFS IFERROR SWITCH AND OR
NOT ABS INT SQRT ROUND ROUNDUP ROUNDDOWN MOD LEN CONCATENATE CONCAT ISNUMBER ISTEXT ISBLANK ISERROR
LEFT RIGHT UPPER LOWER TRIM ROW INDEX VLOOKUP PMT EDATE COUNTIF SUMIF SEARCH FIND SUBSTITUTE N XLOOKUP TEXT POWER QUOTIENT CHAR ROWS TEXTJOIN`, plus references between sheets (`=Stock!B2`, `='Price list'!A1:A9`). Anything else is `#NAME?`; it is never guessed.

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

The editor, the grid on the phone, number formats (UGX), the sidecar for large sheets and
references between sheets. This is the engine and its measurement, milestone M4's first step.

## On the phone

A `sheet` block in a note draws as a grid and a `tile` block as a labelled number, both computed by hq (`GET /api/suite/sheet`, `GET /api/suite/tile`). A tile reads a sheet in its own note for now: `from="[[Note]]!D2:D40"` with `fn` of `sum`, `count`, `avg`, `min`, `max` or `median`, and `format` such as `ugx`.

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

A `plot` block draws a chart from a range of a sheet in the same note: `type` is `bar`, `line`, `pie` or `donut`, `from` is a range whose first row names the columns, `x` is the column of labels and `y` the column of values (names are not case sensitive). hq calculates the values (a formula column is drawn as its results) and sends the chart in the format the chat already draws, so the phone uses its existing chart card. `GET /api/suite/plot?path=&index=`.

A plot that cannot be drawn says why instead of guessing: an unknown column, a value that is not a number, no rows, or more rows than a chart takes (50 for bar and line, 12 for pie and donut). One series per plot for now; `where` and `group` are reserved for the dashboard.

## The dashboard: `where` and `group`

Tiles and plots take two more attributes, so a note can hold a small dashboard over its sheets.

`where` keeps only the rows that match: `where="qty>30 and item=Sugar"`. Up to four conditions joined by `and`, each a column name from the range's first row, one of `= != <> > >= < <=`, and a number or text (text is not case sensitive). A comparison only holds between values of the same kind, and `!=` also holds between different kinds, as in a spreadsheet.

On a **tile**, a `where` makes the range's first row the column names and takes the number from the range's **last column**: `from="A1:C6" fn=sum where="region=North"` adds the `amount` of the North rows. Without a `where` the tile reads every number in the range, as before.

On a **plot**, `group=region` draws one series per value of that column (up to six, over the labels in the order they first appear, a label a group lacks counts as 0); it needs a bar or line chart. `where` filters the rows before anything is drawn.

A tile takes no `group` (it shows one number); a pie or donut takes none either. Anything that cannot be read or drawn says why instead of guessing: an unknown column, a condition without a value, more than four conditions, more than six groups.
