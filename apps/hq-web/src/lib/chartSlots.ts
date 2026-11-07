// Chart SVG is built from validated numbers and escaped labels, then spliced in after DOMPurify,
// so the sanitizer profile stays HTML-only for untrusted markdown. The markdown can contain a
// slot marker of its own (the attribute has to survive the sanitizer), so each marker carries a
// random key it cannot guess, and only a marker this viewer made is ever filled.
const chartBySlot = new Map<string, string>()

const SLOT = /<div data-chart-slot="([0-9a-f]{32})"><\/div>/g

function slotKey(): string {
  const bytes = new Uint8Array(16)
  crypto.getRandomValues(bytes)
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')
}

/** Holds a rendered chart and returns the marker that stands in for it through the sanitizer. */
export function chartSlot(html: string): string {
  const key = slotKey()
  chartBySlot.set(key, html)
  return `<div data-chart-slot="${key}"></div>`
}

/** Puts each held chart back in place of its marker. A marker this viewer did not make stays empty. */
export function fillChartSlots(sanitized: string): string {
  return sanitized.replace(SLOT, (match, key: string) => {
    const html = chartBySlot.get(key)
    chartBySlot.delete(key)
    return html ?? match
  })
}
