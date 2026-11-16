---
title: Shop dashboard
suite_kind: dashboard
suite_fidelity: native
---
# Shop dashboard

```tile from="[[Stock]]!D2:D40" fn=sum label="Stock value" format=ugx
```

```tile from="[[Stock]]!B2:B40" fn=count label=Items
```

```plot type=bar from="[[Stock]]!A2:D40" x=item y=total title="Value by item"
```

A small chart with its data inline, as chat draws it:

```chart
{
  "type": "bar",
  "labels": ["Mon", "Tue", "Wed"],
  "series": [{ "name": "Sales", "values": [12, 18, 15] }]
}
```
