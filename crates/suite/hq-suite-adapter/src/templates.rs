//! Starter notes for the things people here keep on a phone: an invoice, a stock list, a savings
//! group's ledger, a school fees tracker, a sales dashboard and a board deck. Money is in UGX. Each
//! is an ordinary hqmd note with a little invented sample data to overwrite, so it opens as a working
//! sheet, deck or dashboard (formulas, tiles and charts included) the first time.

use std::path::Path;

use anyhow::{Result, bail};

use crate::author::write_note;

/// What `office_create` offers by name.
pub const NAMES: &[&str] = &[
    "invoice",
    "stock",
    "savings-group",
    "school-fees",
    "sales-dashboard",
    "board-deck",
];

/// A template: the `suite_kind` it makes and the text after the title heading.
struct Template {
    kind: &'static str,
    body: &'static str,
}

fn find(name: &str) -> Option<Template> {
    Some(match name {
        "invoice" => Template {
            kind: "sheet",
            body: INVOICE,
        },
        "stock" => Template {
            kind: "sheet",
            body: STOCK,
        },
        "savings-group" => Template {
            kind: "sheet",
            body: SAVINGS,
        },
        "school-fees" => Template {
            kind: "sheet",
            body: FEES,
        },
        "sales-dashboard" => Template {
            kind: "dashboard",
            body: SALES,
        },
        "board-deck" => Template {
            kind: "slides",
            body: DECK,
        },
        _ => return None,
    })
}

/// Makes a note from the named template and returns its vault-relative path.
pub fn create(vault: &Path, name: &str, title: &str, folder: Option<&str>) -> Result<String> {
    let title = title.trim();
    if title.is_empty() {
        bail!("a title is needed");
    }
    let Some(t) = find(name) else {
        bail!(
            "`{name}` is not a template; use one of: {}",
            NAMES.join(", ")
        );
    };
    write_note(
        vault,
        t.kind,
        title,
        folder,
        &format!("# {title}\n\n{}", t.body),
    )
}

const INVOICE: &str =
    "Fill in the items, quantities and prices. The amounts and the total work themselves out.

```sheet name=Invoice types=\"text,int,ugx,ugx\"
| item | qty | unit price | amount |
|---|---|---|---|
| Sample item | 2 | 15000 | =B2*C2 |
| Another item | 5 | 4000 | =B3*C3 |
| Delivery | 1 | 10000 | =B4*C4 |
| Total | =SUM(B2:B4) | | =SUM(D2:D4) |
```
";

const STOCK: &str =
    "One row per item. A row says Reorder when what is in stock is at or below the reorder level.

```sheet name=Stock types=\"text,int,int,ugx,ugx,text\"
| item | in stock | reorder at | unit cost | value | status |
|---|---|---|---|---|---|
| Sugar 1kg | 40 | 20 | 4200 | =B2*D2 | =IF(B2<=C2,\"Reorder\",\"OK\") |
| Salt 500g | 12 | 15 | 1500 | =B3*D3 | =IF(B3<=C3,\"Reorder\",\"OK\") |
| Cooking oil 1L | 25 | 10 | 8500 | =B4*D4 | =IF(B4<=C4,\"Reorder\",\"OK\") |
| Total | | | | =SUM(E2:E4) | |
```
";

const SAVINGS: &str = "One row per member for the month: what they put in, what they borrowed and what they paid back. The balance is what the group holds for them.

```sheet name=Ledger types=\"text,text,ugx,ugx,ugx,ugx\"
| member | month | saved | borrowed | repaid | balance |
|---|---|---|---|---|---|
| Member A | 2026-11 | 50000 | 0 | 0 | =C2-D2+E2 |
| Member B | 2026-11 | 50000 | 200000 | 20000 | =C3-D3+E3 |
| Member C | 2026-11 | 30000 | 0 | 0 | =C4-D4+E4 |
| Total | | =SUM(C2:C4) | =SUM(D2:D4) | =SUM(E2:E4) | =SUM(F2:F4) |
```
";

const FEES: &str =
    "One row per pupil. The balance is what is still owed and the status says who has cleared.

```sheet name=Fees types=\"text,text,ugx,ugx,ugx,text\"
| pupil | class | due | paid | balance | status |
|---|---|---|---|---|---|
| Pupil A | P5 | 450000 | 450000 | =C2-D2 | =IF(E2<=0,\"Cleared\",\"Owing\") |
| Pupil B | P5 | 450000 | 200000 | =C3-D3 | =IF(E3<=0,\"Cleared\",\"Owing\") |
| Pupil C | P6 | 500000 | 0 | =C4-D4 | =IF(E4<=0,\"Cleared\",\"Owing\") |
| Total | | =SUM(C2:C4) | =SUM(D2:D4) | =SUM(E2:E4) | |
```
";

const SALES: &str = "A small dashboard over the sales table below. Change a number in the table and the tiles and the chart follow.

```tile from=\"C1:C5\" fn=sum label=\"Total sales\" format=ugx
```

```tile from=\"C1:C5\" fn=avg label=\"Average per week\" format=ugx
```

```plot type=bar from=\"A1:C5\" x=week y=sales title=\"Sales by week\"
```

```sheet name=Sales types=\"text,text,ugx\"
| week | region | sales |
|---|---|---|
| Week 1 | North | 1200000 |
| Week 2 | North | 1500000 |
| Week 3 | South | 900000 |
| Week 4 | South | 1800000 |
```
";

const DECK: &str = "A short deck to edit.

## Where we are

- Revenue this quarter: add the number
- What went well
- What did not

## What we plan

1. Next step one
2. Next step two
3. Next step three
<!-- notes: Say who does what and by when. -->

## What we need from you

- A decision on the first item
- An introduction or a resource
";

#[cfg(test)]
mod tests {
    use super::*;

    fn made(name: &str) -> (tempfile::TempDir, String) {
        let d = tempfile::tempdir().unwrap();
        let rel = create(d.path(), name, "Test", None).unwrap();
        (d, rel)
    }

    #[test]
    fn every_template_makes_a_valid_note_of_its_kind_and_never_replaces_one() {
        for name in NAMES {
            let (d, rel) = made(name);
            let text = std::fs::read_to_string(d.path().join(&rel)).unwrap();
            assert!(text.starts_with("---\ntitle: "), "{name}");
            assert!(text.contains("suite_kind: "), "{name}");
            let again = create(d.path(), name, "Test", None).unwrap();
            assert_ne!(again, rel, "{name} replaced a note");
        }
        assert!(create(tempfile::tempdir().unwrap().path(), "nope", "T", None).is_err());
    }

    #[test]
    fn the_sheets_calculate_and_export_with_no_formula_left_as_text() {
        for name in [
            "invoice",
            "stock",
            "savings-group",
            "school-fees",
            "sales-dashboard",
        ] {
            let (d, rel) = made(name);
            let text = std::fs::read_to_string(d.path().join(&rel)).unwrap();
            let (bytes, line) = crate::author::sheet_bytes(&text, "xlsx").unwrap().unwrap();
            assert!(bytes.len() > 500, "{name}");
            assert!(!line.contains("saved as text"), "{name}: {line}");
        }
    }

    #[test]
    fn the_invoice_total_and_the_sales_dashboard_draw() {
        let (d, rel) = made("invoice");
        let view = crate::sheets::view(d.path(), &rel, 0, 0, 0).unwrap();
        // 2*15000 + 5*4000 + 1*10000 = 60000 in the total row's amount.
        let total = view.rows.last().unwrap().last().unwrap();
        assert!(total.text.contains("60,000"), "{}", total.text);
        let (d, rel) = made("sales-dashboard");
        let tile = crate::sheets::tile(d.path(), &rel, 0).unwrap();
        assert!(tile.text.contains("5,400,000"), "{}", tile.text);
        assert!(crate::sheets::plot(d.path(), &rel, 0).is_ok());
    }
}
