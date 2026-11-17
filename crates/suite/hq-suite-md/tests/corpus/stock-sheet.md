---
title: Stock
suite_kind: sheet
suite_fidelity: native
---
# Stock

Counted on the first of the month.

```sheet name=Stock types="text,int,ugx,ugx" id=stock
| item  | qty | price | total  |
|-------|-----|-------|--------|
| Sugar | 40  | 4200  | =B2*C2 |
| Salt  | 25  | 1800  | =B3*C3 |
| Soap  | 12  | 2500  | =B4*C4 |
```

Large history lives in a sidecar:

```sheet name=History data="_data/suite/sheets/stock-history.csv.zst" rows=48210 types="date,text,int"
| day        | item  | qty |
|------------|-------|-----|
| 2026-01-02 | Sugar | 38  |
| 2026-01-03 | Sugar | 35  |
```
