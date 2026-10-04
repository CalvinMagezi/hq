import { expect, test } from 'bun:test'
import { MAX_BACKOFF_MS, backoffDelay, createPoller } from './usePolled'

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))

test('backoff doubles per failure and is capped', () => {
  expect(backoffDelay(3000, 0)).toBe(3000)
  expect(backoffDelay(3000, 1)).toBe(6000)
  expect(backoffDelay(3000, 2)).toBe(12000)
  expect(backoffDelay(3000, 50)).toBe(MAX_BACKOFF_MS)
})

test('a slow request is never overlapped by the next poll', async () => {
  let active = 0
  let peak = 0
  let calls = 0
  const poller = createPoller({
    load: async () => {
      calls += 1
      active += 1
      peak = Math.max(peak, active)
      await sleep(60)
      active -= 1
      return calls
    },
    onData: () => {},
    onError: () => {},
    everyMs: 10,
  })
  poller.start()
  await sleep(300)
  poller.stop()
  expect(peak).toBe(1)
  expect(calls).toBeGreaterThan(1)
})

test('a refresh during a request runs once more after it instead of in parallel', async () => {
  let active = 0
  let peak = 0
  let calls = 0
  const poller = createPoller({
    load: async () => {
      calls += 1
      active += 1
      peak = Math.max(peak, active)
      await sleep(40)
      active -= 1
    },
    onData: () => {},
    onError: () => {},
    everyMs: 10_000,
  })
  poller.start()
  void poller.refresh()
  void poller.refresh()
  await sleep(200)
  poller.stop()
  expect(peak).toBe(1)
  expect(calls).toBe(2)
})

test('failures back off the next poll and keep reporting the error', async () => {
  const stamps: number[] = []
  const errors: string[] = []
  const poller = createPoller({
    load: async () => {
      stamps.push(Date.now())
      throw new Error('host unreachable')
    },
    onData: () => {},
    onError: (m) => errors.push(m),
    everyMs: 20,
  })
  poller.start()
  await sleep(400)
  poller.stop()
  expect(errors[0]).toBe('host unreachable')
  const gaps = stamps.slice(1).map((t, i) => t - stamps[i]!)
  expect(gaps.length).toBeGreaterThanOrEqual(2)
  expect(gaps[1]!).toBeGreaterThan(gaps[0]! * 1.4)
})

test('a stopped poller ignores a response that lands late', async () => {
  const seen: number[] = []
  const poller = createPoller({
    load: async () => {
      await sleep(30)
      return 1
    },
    onData: (d) => seen.push(d),
    onError: () => {},
    everyMs: 10,
  })
  poller.start()
  poller.stop()
  await sleep(80)
  expect(seen).toEqual([])
})

test('a load that throws synchronously reports the error and keeps polling', async () => {
  const errors: string[] = []
  let calls = 0
  const poller = createPoller<number>({
    load: () => {
      calls += 1
      throw new Error('sync boom')
    },
    onData: () => {},
    onError: (m) => errors.push(m),
    everyMs: 10,
  })
  poller.start()
  await sleep(150)
  poller.stop()
  expect(errors[0]).toBe('sync boom')
  expect(calls).toBeGreaterThan(1)
})
