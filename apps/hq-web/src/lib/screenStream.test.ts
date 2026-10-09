import { expect, test } from 'bun:test'
import {
  FIRST_EVENT_TIMEOUT_MS,
  HEALTHY_EVENTS,
  HEALTHY_OPEN_MS,
  MAX_STREAM_RETRIES,
  chooseScreenMode,
  createSseParser,
  decideOnEnd,
  failuresAfterDrop,
  firstEventTimedOut,
  isHealthyConnection,
  parseEndReason,
  reconnectDelay,
  screenStatusText,
} from './screenStream'

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

test('end reasons parse, anything else is unavailable', () => {
  expect(parseEndReason('{"reason":"stopped"}')).toBe('stopped')
  expect(parseEndReason('{"reason":"time"}')).toBe('time')
  expect(parseEndReason('{"reason":"unavailable"}')).toBe('unavailable')
  expect(parseEndReason('{"reason":"weird"}')).toBe('unavailable')
  expect(parseEndReason('not json')).toBe('unavailable')
})

test('stopped moves to polling', () => {
  expect(decideOnEnd('stopped', 0)).toEqual({ action: 'poll' })
})

test('time reconnects at once and resets the counter', () => {
  expect(decideOnEnd('time', 2)).toEqual({ action: 'reconnect', delay: 0, failures: 0 })
})

test('unavailable backs off and eventually fails', () => {
  expect(decideOnEnd('unavailable', 0)).toEqual({ action: 'reconnect', delay: 1000, failures: 1 })
  expect(decideOnEnd('unavailable', 2)).toEqual({ action: 'reconnect', delay: 4000, failures: 3 })
  expect(decideOnEnd('unavailable', MAX_STREAM_RETRIES)).toEqual({ action: 'fail' })
})

test('one event does not reset the failure counter, a long or busy connection does', () => {
  expect(failuresAfterDrop(2, 100, 1)).toBe(3)
  expect(failuresAfterDrop(2, HEALTHY_OPEN_MS, 0)).toBe(1)
  expect(failuresAfterDrop(2, 100, HEALTHY_EVENTS)).toBe(1)
  expect(isHealthyConnection(HEALTHY_OPEN_MS - 1, HEALTHY_EVENTS - 1)).toBe(false)
})

test('a flapping connection reaches the polling fallback', () => {
  let failures = 0
  let delay: number | null = 0
  for (let i = 0; i < MAX_STREAM_RETRIES + 1 && delay !== null; i++) {
    failures = failuresAfterDrop(failures, 50, 1)
    delay = reconnectDelay(failures)
  }
  expect(delay).toBeNull()
})

test('first event timeout', () => {
  expect(firstEventTimedOut(FIRST_EVENT_TIMEOUT_MS - 1, 0)).toBe(false)
  expect(firstEventTimedOut(FIRST_EVENT_TIMEOUT_MS, 0)).toBe(true)
  expect(firstEventTimedOut(FIRST_EVENT_TIMEOUT_MS * 2, 1)).toBe(false)
})
