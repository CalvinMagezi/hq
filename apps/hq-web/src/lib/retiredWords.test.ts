import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { inlineStrings } from '../../../../scripts/strings/inline'
import { load } from '../../../../scripts/strings/build'

// Words retired everywhere a person reads them (docs/plans/ux-run/web/wave-0/02-shared-parts.md, check 2).
const RETIRED = /\blive\b|Saved · |Can't reach HQ|HQ is not answering|HQ didn't reply|Checking…|Saved on this PC|Tailscale/i
// The same in source: the phrases anywhere, and "live" as a whole string a condition can show.
const RETIRED_IN_SOURCE = /(['"`])live\1|Saved · |Can't reach HQ|HQ is not answering|HQ didn't reply|Checking…|Saved on this PC|Tailscale/

/** Places whose words are rewritten by a later spec, each with that spec. */
const LATER: Record<string, string> = {
  'routes/settings.tsx': 'Settings › Integrations is rewritten in web/wave-4/08-settings.md',
}

function tsxUnder(dir: string): string[] {
  return Array.from(new Bun.Glob('**/*.tsx').scanSync(dir)).filter((p) => !p.endsWith('.test.tsx')).map((p) => join(dir, p))
}

describe('retired words', () => {
  test('the pattern catches each retired phrase', () => {
    for (const s of ['live', 'Saved · 3 min', "Can't reach HQ", 'HQ is not answering', "HQ didn't reply", 'Checking…', 'Saved on this PC', 'Tailscale']) {
      expect(RETIRED.test(s), s).toBe(true)
    }
  })

  test('no catalogue sentence uses one', () => {
    const hits = Object.entries(load()).filter(([, v]) => RETIRED.test(JSON.stringify(v)))
    expect(hits.map(([k]) => k)).toEqual([])
  })

  test('no text a person reads in a component uses one', () => {
    const src = join(import.meta.dir, '..')
    const files = tsxUnder(src).filter((file) => !(file.slice(src.length + 1) in LATER))
    const hits = files.flatMap((file) => {
      const source = readFileSync(file, 'utf8')
      const shown = inlineStrings(source).filter((s) => RETIRED.test(s))
      const picked = source.match(RETIRED_IN_SOURCE) ? [source.match(RETIRED_IN_SOURCE)![0]] : []
      return [...shown, ...picked].map((s) => `${file.slice(src.length + 1)}: ${s}`)
    })
    expect(hits).toEqual([])
  })
})
