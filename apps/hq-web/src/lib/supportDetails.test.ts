import { expect, test } from 'bun:test'
import { blankKeys, supportDetails } from './supportDetails'

test('keys and bearer tokens are blanked, versions stay', () => {
  const out = supportDetails(['HQ 0.9.2', 'key sk-abcdef123456', 'Authorization: Bearer abcdefgh12345678', '', 'x'.repeat(40)])
  expect(out).toBe('HQ 0.9.2\nkey [blanked]\nAuthorization: [blanked]\n[blanked]')
  expect(blankKeys('plain words')).toBe('plain words')
})
