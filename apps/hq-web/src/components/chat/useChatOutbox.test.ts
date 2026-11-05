import { describe, expect, test } from 'bun:test'
import { lineState } from './useChatOutbox'

describe('the chat outbox line', () => {
  test('says why it waits, and that it is next once HQ is reachable', () => {
    expect(lineState('offline')).toBe('offline')
    expect(lineState('unreachable')).toBe('unreachable')
    expect(lineState('starting')).toBe('starting')
    expect(lineState('connected')).toBe('next')
    expect(lineState('reconnecting')).toBe('next')
  })
})
