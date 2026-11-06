// Text a person reads, written inline in the web's components instead of taken from the catalogue
// (N20 on the web). `bun scripts/strings/inline.ts` rewrites the baseline once a file has fewer.

import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import { join, relative } from 'node:path'

export const WEB_SRC = join(import.meta.dir, '..', '..', 'apps/hq-web/src')
export const BASELINE = join(WEB_SRC, 'strings/inline-baseline.json')
const SCANNED = ['components', 'routes']
const WHOLLY_INTERPOLATED = /^\$\{.*\}$/

/** JSX text and literal title, aria-label and placeholder props. */
export function inlineStrings(source: string): string[] {
  const found: string[] = []
  for (const m of source.matchAll(/>\s*([^<>{}\n]*[A-Za-z]{2,}[^<>{}\n]*?)\s*</g)) found.push(m[1].trim())
  for (const m of source.matchAll(/\b(?:title|aria-label|placeholder)="([^"]*[A-Za-z]{2,}[^"]*)"/g)) {
    // Markup built as a string whose value is one interpolation (`${t('key')}`) takes its words from the catalogue.
    if (!WHOLLY_INTERPOLATED.test(m[1])) found.push(m[1])
  }
  return found
}

function tsxFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name)
    if (statSync(p).isDirectory()) return tsxFiles(p)
    return p.endsWith('.tsx') && !p.endsWith('.test.tsx') ? [p] : []
  })
}

/** Inline strings per file, keyed by the path under `src`. */
export function inlineCounts(): Record<string, number> {
  const out: Record<string, number> = {}
  for (const dir of SCANNED) {
    for (const file of tsxFiles(join(WEB_SRC, dir))) {
      const n = inlineStrings(readFileSync(file, 'utf8')).length
      if (n) out[relative(WEB_SRC, file)] = n
    }
  }
  return out
}

if (import.meta.main) {
  const counts = inlineCounts()
  writeFileSync(BASELINE, `${JSON.stringify(counts, null, 2)}\n`)
  console.log(`${Object.values(counts).reduce((a, b) => a + b, 0)} inline strings in ${Object.keys(counts).length} files.`)
}
