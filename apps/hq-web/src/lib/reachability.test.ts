import { expect, test } from 'bun:test'
import { reachability, UNREACHABLE_GRACE_MS } from './reachability'

const LONG = UNREACHABLE_GRACE_MS + 1
const base = { wsConnected: false, signInNeeded: false, deviceOnline: true, disconnectedForMs: LONG }

test('a live socket is reachable', () => {
  expect(reachability({ ...base, wsConnected: true })).toBe('ok')
})

test('a short drop stays quiet', () => {
  expect(reachability({ ...base, disconnectedForMs: UNREACHABLE_GRACE_MS - 1 })).toBe('ok')
})

test('sign-in has its own prompt, so it is not reported as unreachable', () => {
  expect(reachability({ ...base, signInNeeded: true })).toBe('ok')
})

test('an online device that cannot reach the server points at the server path', () => {
  expect(reachability(base)).toBe('server_unreachable')
})

test('an offline device says so', () => {
  expect(reachability({ ...base, deviceOnline: false })).toBe('device_offline')
})
