import { describe, expect, test } from 'bun:test'
import { replyAlert } from './replyAlerts'

describe('replyAlert', () => {
  test('says only that HQ answered, never the reply', () => {
    const { title, options } = replyAlert('t1')
    expect(title).toBe('HQ answered')
    expect(options.body).toBe('Tap to read.')
    expect(options.tag).toBe('hq-chat-t1')
    expect(JSON.stringify({ title, options })).not.toContain('secret plan')
  })
})
