import { describe, expect, test } from 'bun:test'
import { epochInZone, groupOf, groupReminders, localString, parseLocalInput, repeatSpec, timeChoices } from './reminderTimes'

const KAMPALA = 'Africa/Kampala'
const LONDON = 'Europe/London'
// Friday 27 November 2026, 13:20 in Kampala (10:20 UTC).
const NOW = Date.UTC(2026, 10, 27, 10, 20)

describe('wall clock times in the owner zone', () => {
  test('Kampala is three hours ahead of UTC', () => {
    expect(epochInZone(KAMPALA, 2026, 11, 27, 16, 0)).toBe(Date.UTC(2026, 10, 27, 13, 0))
    expect(localString(NOW, KAMPALA)).toBe('2026-11-27T13:20')
  })

  test('a time the clocks skip lands after the gap, and a daily time keeps its wall clock across a change', () => {
    // London springs forward at 01:00 on 29 March 2026: 01:30 does not exist.
    expect(epochInZone(LONDON, 2026, 3, 29, 1, 30)).toBe(Date.UTC(2026, 2, 29, 1, 0))
    // It falls back at 02:00 BST on 25 October 2026: 01:30 happens twice, and the first is BST (00:30 UTC).
    expect(epochInZone(LONDON, 2026, 10, 25, 1, 30)).toBe(Date.UTC(2026, 9, 25, 0, 30))
    const before = epochInZone(LONDON, 2026, 3, 28, 9, 0)
    const after = epochInZone(LONDON, 2026, 3, 29, 9, 0)
    expect(after - before).toBe(23 * 3_600_000)
  })

  test('what a datetime-local input holds is read in the given zone', () => {
    expect(parseLocalInput('2026-11-27T16:00', KAMPALA)).toBe(Date.UTC(2026, 10, 27, 13, 0))
    expect(parseLocalInput('soon', KAMPALA)).toBeNull()
  })
})

describe('the quick times', () => {
  test('an hour, this evening and tomorrow morning, in the owner zone', () => {
    const choices = timeChoices(NOW, KAMPALA)
    expect(choices.map((c) => c.key)).toEqual(['hour', 'evening', 'tomorrow'])
    expect(localString(choices[0].at, KAMPALA)).toBe('2026-11-27T14:20')
    expect(localString(choices[1].at, KAMPALA)).toBe('2026-11-27T18:00')
    expect(localString(choices[2].at, KAMPALA)).toBe('2026-11-28T09:00')
  })

  test('this evening is left out when it is nearly now or past', () => {
    const lateAfternoon = epochInZone(KAMPALA, 2026, 11, 27, 17, 45)
    expect(timeChoices(lateAfternoon, KAMPALA).map((c) => c.key)).toEqual(['hour', 'tomorrow'])
    const night = epochInZone(KAMPALA, 2026, 11, 27, 22, 0)
    expect(timeChoices(night, KAMPALA).map((c) => c.key)).toEqual(['hour', 'tomorrow'])
  })

  test('next Monday is the coming Monday, never today', () => {
    const monday = timeChoices(NOW, KAMPALA, { monday: true }).find((c) => c.key === 'monday')!
    expect(localString(monday.at, KAMPALA)).toBe('2026-11-30T09:00')
    const onAMonday = epochInZone(KAMPALA, 2026, 11, 30, 8, 0)
    const next = timeChoices(onAMonday, KAMPALA, { monday: true }).find((c) => c.key === 'monday')!
    expect(localString(next.at, KAMPALA)).toBe('2026-12-07T09:00')
  })
})

describe('repeat', () => {
  test('weekly and monthly follow the day it first rings', () => {
    const friday = epochInZone(KAMPALA, 2026, 11, 27, 16, 0)
    expect(repeatSpec('weekly', friday, KAMPALA)).toBe('weekly:5')
    expect(repeatSpec('monthly', friday, KAMPALA)).toBe('monthly:27')
    expect(repeatSpec('daily', friday, KAMPALA)).toBe('daily')
    expect(repeatSpec('weekdays', friday, KAMPALA)).toBe('weekdays')
    expect(repeatSpec('none', friday, KAMPALA)).toBeUndefined()
  })

  test('the weekday is the owner zone day, not the browser day', () => {
    // 23:30 UTC on Friday is already Saturday 02:30 in Kampala.
    const lateUtc = Date.UTC(2026, 10, 27, 23, 30)
    expect(repeatSpec('weekly', lateUtc, KAMPALA)).toBe('weekly:6')
    expect(repeatSpec('weekly', lateUtc, 'UTC')).toBe('weekly:5')
  })
})

describe('groups', () => {
  test('overdue, today, tomorrow and later, soonest first, empty groups dropped', () => {
    const at = (day: number, hour: number) => epochInZone(KAMPALA, 2026, 11, day, hour, 0)
    const items = [
      { id: 'later', due_epoch: at(30, 9) / 1000 },
      { id: 'tomorrow', due_epoch: at(28, 9) / 1000 },
      { id: 'today-b', due_epoch: at(27, 18) / 1000 },
      { id: 'overdue', due_epoch: at(27, 8) / 1000 },
      { id: 'today-a', due_epoch: at(27, 15) / 1000 },
    ]
    const groups = groupReminders(items, NOW, KAMPALA)
    expect(groups.map((g) => g.group)).toEqual(['overdue', 'today', 'tomorrow', 'later'])
    expect(groups[1].items.map((i) => i.id)).toEqual(['today-a', 'today-b'])
    expect(groupReminders([items[0]], NOW, KAMPALA).map((g) => g.group)).toEqual(['later'])
    expect(groupReminders([], NOW, KAMPALA)).toEqual([])
  })

  test('late tonight in UTC is tomorrow in Kampala', () => {
    const due = Date.UTC(2026, 10, 27, 22, 0)
    expect(groupOf(due, NOW, KAMPALA)).toBe('tomorrow')
    expect(groupOf(due, NOW, 'UTC')).toBe('today')
  })
})
