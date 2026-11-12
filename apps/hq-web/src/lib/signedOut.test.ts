import { describe, expect, test } from 'bun:test'
import { refusedNow } from './hqAuth'
import { signedOutTitle } from '~/components/SignInPrompt'

const res = (status: number, headers: Record<string, string> = {}) => new Response(null, { status, headers })

describe('refusedNow', () => {
  test('a 401 for the token held now wipes', () => {
    expect(refusedNow(res(401), 't1', 't1')).toBe(true)
    expect(refusedNow(res(401), null, null)).toBe(true)
  })
  test('a copy from the worker cache wipes nothing', () => {
    expect(refusedNow(res(401, { 'x-hq-cache': '1' }), 't1', 't1')).toBe(false)
  })
  test('an older request finishing after a fresh sign-in wipes nothing', () => {
    expect(refusedNow(res(401), 'old', 'new')).toBe(false)
  })
  test('anything but a 401 wipes nothing', () => {
    expect(refusedNow(res(403), 't1', 't1')).toBe(false)
  })
})

describe('signedOutTitle', () => {
  test('names the place', () => {
    expect(signedOutTitle('/notes/x')).toBe('Sign in to see your Vault')
    expect(signedOutTitle('/chat')).toBe('Sign in to see your chats')
    expect(signedOutTitle('/inbox')).toBe('Sign in to see what needs you')
    expect(signedOutTitle('/settings/devices')).toBe('Sign in to see your settings')
  })
})
