import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { ANDROID_OUT, androidFile, load, placeholders, WEB_OUT, webFile } from '../../../../scripts/strings/build'
import { t, tn } from '.'
import { inlineCounts, inlineStrings } from '../../../../scripts/strings/inline'
import baseline from './inline-baseline.json'

describe('the catalogue', () => {
  const cat = load()

  test('the generated web and phone files are current', () => {
    expect(readFileSync(WEB_OUT, 'utf8')).toBe(webFile(cat))
    expect(readFileSync(ANDROID_OUT, 'utf8')).toBe(androidFile(cat))
  })

  test('every plural form uses the same names', () => {
    for (const [key, v] of Object.entries(cat)) {
      if (typeof v === 'string') continue
      const names = Object.values(v).map((s) => placeholders(s).sort().join(','))
      expect(new Set(names).size, key).toBe(1)
    }
  })

  test('no sentence uses an em dash', () => {
    for (const [key, v] of Object.entries(cat)) expect(JSON.stringify(v).includes('—'), key).toBe(false)
  })

  test('t and tn fill names and pick the plural form', () => {
    expect(t('state_starting', { seconds: 12 })).toBe('Starting HQ · 12 s')
    expect(tn('updates_cleared', 1)).toBe('1 update cleared')
    expect(tn('updates_cleared', 4)).toBe('4 updates cleared')
  })
})

describe('inline strings (N20 on the web)', () => {
  test('finds text a person reads', () => {
    expect(inlineStrings('<p>Hello there</p><input placeholder="Search notes" />')).toEqual(['Hello there', 'Search notes'])
    expect(inlineStrings('<p>{t("state_offline")}</p>')).toEqual([])
  })

  test('no file has more inline strings than it had when the catalogue arrived', () => {
    const was = baseline as Record<string, number>
    const grown = Object.entries(inlineCounts())
      .filter(([file, n]) => n > (was[file] ?? 0))
      .map(([file, n]) => `${file}: ${n} (was ${was[file] ?? 0}); take the words from apps/strings/en.yaml`)
    expect(grown).toEqual([])
  })
})
