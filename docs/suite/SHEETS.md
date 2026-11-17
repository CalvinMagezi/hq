# Sheets: the formula engine

`hq-suite-calc` is a small spreadsheet formula engine: A1 references and ranges, the usual operator
precedence (`-2^2` is 4, `50%` is 0.5), errors that travel through a calculation, a cycle reported
as `#CIRC!` instead of looping, and a work stack so a chain of thousands of dependent formulas
cannot overflow. Functions: `SUM PRODUCT AVERAGE MIN MAX COUNT COUNTA IF IFS IFERROR SWITCH AND OR
NOT ABS INT SQRT ROUND ROUNDUP ROUNDDOWN MOD LEN CONCATENATE CONCAT ISNUMBER ISTEXT ISBLANK ISERROR
LEFT RIGHT UPPER LOWER TRIM`. Anything else is `#NAME?`; it is never guessed.

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
