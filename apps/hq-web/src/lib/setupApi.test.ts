import { expect, test } from 'bun:test'
import { shouldRedirectToSetup } from './setupApi'

test('only a fresh install that can take a key is sent to setup', () => {
  expect(shouldRedirectToSetup({ needs_setup: true, available: true })).toBe(true)
  expect(shouldRedirectToSetup({ needs_setup: true, available: false })).toBe(false)
  expect(shouldRedirectToSetup({ needs_setup: false, available: true })).toBe(false)
})
