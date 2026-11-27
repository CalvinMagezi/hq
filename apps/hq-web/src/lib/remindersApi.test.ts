import { afterEach, expect, test } from 'bun:test'
import { cancelReminder, createReminder, doneReminder, fetchReminders, snoozeReminder } from './remindersApi'

const realFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = realFetch
})

function serverAnswers(status: number, body: unknown) {
  const sent: { url: string; init: RequestInit }[] = []
  globalThis.fetch = (async (url: string, init: RequestInit) => {
    sent.push({ url, init })
    return status === 204 ? new Response(null, { status }) : new Response(JSON.stringify(body), { status })
  }) as unknown as typeof fetch
  return sent
}

test('the list asks for the status it wants', async () => {
  const sent = serverAnswers(200, { time_zone: 'Africa/Kampala', now: 1, reminders: [] })
  await fetchReminders('done')
  expect(sent[0].url).toBe('/api/reminders?status=done')
  expect(sent[0].init.method).toBe('GET')
})

test('a new reminder sends its local time and zone as JSON', async () => {
  const sent = serverAnswers(201, {})
  await createReminder({ title: 'Call Moses', at: '2026-11-27T16:00', tz: 'Africa/Kampala', nudge: true })
  expect(sent[0].init.method).toBe('POST')
  expect(JSON.parse(String(sent[0].init.body))).toEqual({ title: 'Call Moses', at: '2026-11-27T16:00', tz: 'Africa/Kampala', nudge: true })
})

test('done, snooze and stop go to their own routes, with the id encoded', async () => {
  const sent = serverAnswers(200, {})
  await doneReminder('a/b')
  await snoozeReminder('a/b', 1_800_000_000)
  expect(sent[0].url).toBe('/api/reminders/a%2Fb/done')
  expect(sent[1].url).toBe('/api/reminders/a%2Fb/snooze')
  expect(JSON.parse(String(sent[1].init.body))).toEqual({ until: 1_800_000_000 })
  const gone = serverAnswers(204, null)
  await cancelReminder('x')
  expect(gone[0].init.method).toBe('DELETE')
})

test('a refused reminder carries the server words and status', async () => {
  serverAnswers(400, { error: 'that time has already passed (it is 4:00 pm today now)' })
  const err = await createReminder({ title: 'x', at: '2020-01-01T09:00', tz: 'UTC' }).catch((e) => e)
  expect(err.message).toContain('already passed')
  expect(err.status).toBe(400)
})
