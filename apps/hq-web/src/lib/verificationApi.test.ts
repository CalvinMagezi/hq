import { afterEach, expect, test } from 'bun:test'
import {
  answerAudit,
  createSweep,
  fetchAudits,
  fetchBriefing,
  fetchSweep,
  fetchTaskVerification,
  requestSecondOpinion,
  sweepAction,
  vetoTask,
} from './verificationApi'
import { HqHttpError } from './hqAuth'

const realFetch = globalThis.fetch
afterEach(() => {
  globalThis.fetch = realFetch
})

interface Call {
  url: string
  method: string
  body: unknown
}

function serve(reply: unknown = {}, status = 200): Call[] {
  const calls: Call[] = []
  globalThis.fetch = (async (url: string, init: RequestInit = {}) => {
    calls.push({ url, method: init.method ?? 'GET', body: init.body ? JSON.parse(String(init.body)) : undefined })
    return new Response(JSON.stringify(reply), { status, headers: { 'content-type': 'application/json' } })
  }) as unknown as typeof fetch
  return calls
}

test('every owner action goes to the verification routes with the right verb and body', async () => {
  const calls = serve()
  await createSweep({ kind: 'tasks', ids: ['a', 'b'] })
  await fetchSweep('sw-1')
  await sweepAction('sw-1', 'run')
  await sweepAction('sw-1', 'pause')
  await sweepAction('sw-1', 'cancel')
  await sweepAction('sw-1', 'apply', true)
  await sweepAction('sw-1', 'undo')
  await answerAudit('au-1', 'disagree', 'the test is empty')
  await vetoTask('tk-1', 'needs a backfill')
  await requestSecondOpinion('tk-1')
  expect(calls).toEqual([
    { url: '/api/verification/sweeps', method: 'POST', body: { scope: { kind: 'tasks', ids: ['a', 'b'] } } },
    { url: '/api/verification/sweeps/sw-1', method: 'GET', body: undefined },
    { url: '/api/verification/sweeps/sw-1/run', method: 'POST', body: undefined },
    { url: '/api/verification/sweeps/sw-1/pause', method: 'POST', body: undefined },
    { url: '/api/verification/sweeps/sw-1/cancel', method: 'POST', body: undefined },
    { url: '/api/verification/sweeps/sw-1/apply', method: 'POST', body: { confirm: true } },
    { url: '/api/verification/sweeps/sw-1/undo', method: 'POST', body: undefined },
    { url: '/api/verification/audits/au-1', method: 'POST', body: { answer: 'disagree', note: 'the test is empty' } },
    { url: '/api/verification/holds/tk-1/veto', method: 'POST', body: { note: 'needs a backfill' } },
    { url: '/api/tasks/tk-1/reverify', method: 'POST', body: undefined },
  ])
})

test('apply does not confirm unless it is told to', async () => {
  const calls = serve()
  await sweepAction('sw-1', 'apply')
  expect(calls[0].body).toEqual({ confirm: false })
})

test('reads ask for what they mean and escape what they are given', async () => {
  const calls = serve()
  await fetchBriefing(72)
  await fetchAudits()
  await fetchAudits(10)
  await fetchTaskVerification('tk 1/../x')
  expect(calls.map((c) => c.url)).toEqual([
    '/api/verification/briefing?hours=72',
    '/api/verification/audits',
    '/api/verification/audits?n=10',
    '/api/tasks/tk%201%2F..%2Fx/verification',
  ])
})

test('a refusal arrives as the server worded it, with its status', async () => {
  serve({ error: 'applying moves 40 tasks, which is over 25: confirm it explicitly' }, 400)
  const err = await sweepAction('sw-1', 'apply').catch((e: unknown) => e)
  expect(err).toBeInstanceOf(HqHttpError)
  expect((err as HqHttpError).status).toBe(400)
  expect((err as HqHttpError).message).toContain('confirm it explicitly')
})
