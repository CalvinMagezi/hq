# Chart blocks in chat

The web app draws a chart when an assistant reply contains a fenced code block
tagged `chart` whose body is one JSON object. Everything else in the reply
renders as normal markdown.

````
```chart
{
  "type": "line",
  "title": "Weekly signups",
  "xLabel": "Week",
  "yLabel": "Signups",
  "labels": ["W1", "W2", "W3"],
  "series": [
    { "name": "Web", "values": [12, 18, 15] },
    { "name": "Mobile", "values": [5, 9, 14] }
  ]
}
```
````

## Fields

| Field | Required | Notes |
|-------|----------|-------|
| `type` | yes | `line`, `bar`, `pie` or `donut` |
| `labels` | yes | Category names (x axis, or one per slice). Strings or numbers. |
| `series` | yes | Array of `{ "name": string, "values": number[] }`. Each `values` array must have exactly one number per label. |
| `title` | no | Shown above the chart. |
| `xLabel`, `yLabel` | no | Axis titles for line and bar charts. |

## Limits

- Line and bar: up to 50 labels and 6 series. A legend appears with 2 or more series.
- Pie and donut: up to 12 labels. Only the first series is drawn. Values must be zero or greater and must not all be zero. The legend lists each label with its value and percentage.
- Values must be finite numbers. `null`, strings and missing values make the payload invalid.
- Text is cut at 120 characters and x axis labels at 10 characters. When there are many labels, only some are printed on the axis.
- Colors are fixed and come from the app theme. A payload cannot set colors.

## When the payload is not usable

Invalid JSON, an unknown type, mismatched lengths, out-of-range values or an
empty payload never break the message. The block is shown as an ordinary code
block whose header says why it was not drawn, for example
`chart (not rendered: series 1 must have 3 values, one per label)`. A reply that
is still streaming shows this fallback until the closing fence arrives.

A Mermaid `pie` block (title and `"label" : number` rows only) is also drawn as a pie chart, since models often answer that way. Any other Mermaid diagram stays a code block.

HQ's web sessions are told about the `chart` block in their system prompt (`WEB_CHART_GUIDANCE` in `crates/hq-agent/src/builder/prompt.rs`), so charts arrive in this format without the user asking.

Also put the key numbers in the reply text, so the answer is complete for
clients that cannot draw charts (Telegram, Discord, the CLI).

Implementation: `apps/hq-web/src/lib/chart.ts`, hooked into the markdown
renderer in `apps/hq-web/src/components/MarkdownViewer.tsx`.
