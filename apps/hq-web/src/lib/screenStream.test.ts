import { expect, test } from 'bun:test'
import { MAX_STREAM_RETRIES, chooseScreenMode, createSseParser, reconnectDelay, screenStatusText } from './screenStream'

test('a complete event parses', () => {
  const p = createSseParser()
  expect(p.feed('event: screen\ndata: {"a":1}\n\n')).toEqual([{ event: 'screen', data: '{"a":1}' }])
})

test('events split at any byte boundary still parse', () => {
  const wire = ': keepalive\n\nevent: screen\ndata: one\n\nevent: end\ndata: {"reason":"x"}\n\n'
  for (let cut = 1; cut < wire.length; cut++) {
    const p = createSseParser()
    const got = [...p.feed(wire.slice(0, cut)), ...p.feed(wire.slice(cut))]
    expect(got).toEqual([
      { event: 'screen', data: 'one' },
      { event: 'end', data: '{"reason":"x"}' },
    ])
  }
})

test('CRLF split across chunks is one line break', () => {
  const p = createSseParser()
  expect([...p.feed('data: a\r'), ...p.feed('\n\r'), ...p.feed('\n')]).toEqual([{ event: 'message', data: 'a' }])
})

test('multi-line data joins with newline and comments are ignored', () => {
  const p = createSseParser()
  expect(p.feed(': hi\ndata: a\ndata:b\n\n')).toEqual([{ event: 'message', data: 'a\nb' }])
})

test('a blank line with no data emits nothing', () => {
  expect(createSseParser().feed('\n\nevent: x\n\n')).toEqual([])
})

test('reconnect backs off then gives up', () => {
  expect([1, 2, 3].map(reconnectDelay)).toEqual([1000, 2000, 4000])
  expect(reconnectDelay(MAX_STREAM_RETRIES + 1)).toBeNull()
})

test('mode chooser streams until failure or end', () => {
  expect(chooseScreenMode('connecting')).toBe('stream')
  expect(chooseScreenMode('live')).toBe('stream')
  expect(chooseScreenMode('failed')).toBe('poll')
  expect(chooseScreenMode('ended')).toBe('poll')
})

test('status text in plain words', () => {
  expect(screenStatusText('stream', 'live', 'live')).toBe('Live')
  expect(screenStatusText('poll', 'failed', 'live')).toBe('Updating every few seconds')
  expect(screenStatusText('poll', 'ended', 'snapshot')).toBe('Last saved view')
})
