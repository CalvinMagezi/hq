import { expect, test } from 'bun:test'
import { keyState, shouldRedirectToSetup } from './setupApi'

test('only a fresh install that can take a key is sent to setup', () => {
  expect(shouldRedirectToSetup({ needs_setup: true, available: true })).toBe(true)
  expect(shouldRedirectToSetup({ needs_setup: true, available: false })).toBe(false)
  expect(shouldRedirectToSetup({ needs_setup: false, available: true })).toBe(false)
})

test('an environment key is reported as such even when the config also has one', () => {
  expect(keyState({ configured: true, source: 'env' })).toBe('env')
  expect(keyState({ configured: false, source: 'env' })).toBe('env')
  expect(keyState({ configured: true, source: 'config' })).toBe('configured')
  expect(keyState({ configured: false, source: null })).toBe('not_set')
  expect(keyState({ configured: true })).toBe('configured')
})
