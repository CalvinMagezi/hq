import { expect, test } from 'bun:test'
import { canSchedule, scheduleChoices } from './offersApi'

const offer = (kind: string) => ({ kind, line: 'l', reply: 'r', send: false })

test('only a next step can be scheduled', () => {
  expect(canSchedule(offer('next_look_up'))).toBe(true)
  expect(canSchedule(offer('pasted_list'))).toBe(false)
})

test('the choices are later today, tomorrow at nine and a week ahead at nine', () => {
  const now = new Date(2026, 10, 9, 14, 30)
  const seconds = (d: Date) => Math.floor(d.getTime() / 1000)
  expect(scheduleChoices(now).map((c) => c.at)).toEqual([
    seconds(new Date(2026, 10, 9, 17, 30)),
    seconds(new Date(2026, 10, 10, 9, 0)),
    seconds(new Date(2026, 10, 16, 9, 0)),
  ])
})
